//! `marionette audio-check` (decision 0220): the real-device proof for the synth. Nothing here
//! can hear; it counts. The audio callback times itself, streams count empty reads, the queue
//! counts what it dropped, so a clean run means the device never starved.

use marionette_agent::engine::{SoundPlayer, synth};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

/// A 12 s looping chord bed, rendered offline, so the check exercises a disk stream with no
/// files in the repository.
fn make_stream_file(dir: &Path) -> Result<PathBuf, String> {
    let mut score = synth::Score { seconds: 12.0, ..Default::default() };
    for (i, chord) in [[48.0, 55.0, 64.0], [45.0, 52.0, 60.0], [41.0, 48.0, 57.0], [43.0, 50.0, 59.0]].iter().enumerate() {
        for n in chord {
            score.notes.push(synth::score::ScoreNote {
                at: i as f64 * 3.0,
                dur: 2.8,
                note: Some(*n),
                patch: Some("pad".into()),
                ..Default::default()
            });
        }
    }
    let r = synth::render_score(&score, dir)?;
    let path = dir.join("bed.wav");
    std::fs::write(&path, synth::wav_stereo_bytes(&r.frames, r.rate)).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn run(seconds: f64, voices: usize, stream: Option<&str>, json_out: bool) -> ExitCode {
    let dir = match marionette_host::temp_dir("audio-check") {
        Ok(d) => d,
        Err(e) => {
            eprintln!("cannot make a temp folder: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut player = match SoundPlayer::new(dir.path().to_path_buf()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let mut host = match player.start_synth(voices.max(8) * 2) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let stream_path = match stream {
        Some(p) => PathBuf::from(p),
        None => match make_stream_file(dir.path()) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("cannot prepare the stream: {e}");
                return ExitCode::FAILURE;
            }
        },
    };
    let stats = host.handle.stats().clone();
    let (source, stream_handle) = match synth::open_stream(
        &stream_path,
        &stats,
        synth::StreamSettings { looping: true, gain: 0.5, ..Default::default() },
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    host.handle.install(0, Box::new(source));

    // Wait for the first callback so the rate is the device's.
    let start = Instant::now();
    while stats.now() == 0 && start.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(10));
    }
    if stats.now() == 0 {
        eprintln!("the audio device never asked for samples");
        return ExitCode::from(2);
    }
    let presets = ["pluck", "pad", "bass", "lead", "bell", "thump"];
    let mut id = 0u32;
    let mut scheduled = 0u64;
    let begin = Instant::now();
    let mut next_note = 0.0f64;
    let mut peak_voices = 0;
    while begin.elapsed().as_secs_f64() < seconds {
        let t = begin.elapsed().as_secs_f64();
        // Keep notes scheduled 150 ms ahead, sample accurate, at `voices` per second.
        while next_note < t + 0.15 {
            let at = stats.now() + host.handle.frames((next_note - t).max(0.0) + 0.05);
            let p = synth::Patch::preset(presets[id as usize % presets.len()]).expect("preset");
            let freq = synth::dsp::midi_hz(36.0 + (id * 7 % 48) as f32);
            if host.handle.note_on(at, id, freq, 0.7, p) {
                host.handle.note_off(at + host.handle.frames(1.0), id);
                scheduled += 1;
            }
            id += 1;
            next_note += 1.0 / voices.max(1) as f64;
        }
        peak_voices = peak_voices.max(stats.voices.load(std::sync::atomic::Ordering::Relaxed));
        host.handle.free_trash();
        std::thread::sleep(Duration::from_millis(5));
    }
    let snap = synth::StatsSnapshot::of(&stats);
    let pass = snap.slow_blocks == 0 && snap.underruns == 0 && snap.dropped == 0 && stream_handle.error().is_none();
    let report = json!({
        "pass": pass,
        "seconds": seconds,
        "rate": snap.rate,
        "frames_played": snap.frame,
        "notes_scheduled": scheduled,
        "peak_voices": peak_voices,
        "stats": snap,
        "stream_underruns": stream_handle.underruns(),
        "stream_frames": stream_handle.played(),
        "stream_error": stream_handle.error(),
        "note": "slow_blocks counts callbacks that took longer than the audio they produced; max_block_us is the longest callback",
    });
    if json_out {
        println!("{report}");
    } else {
        println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
    }
    drop(stream_handle);
    drop(host);
    if pass { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
