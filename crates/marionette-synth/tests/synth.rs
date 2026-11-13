//! Numeric proofs for the synth: pitch, sample-accurate timing, block-size independence,
//! voice stealing, clicks, streaming and custom generators. No sound card is used.

use marionette_synth::dsp::Wave;
use marionette_synth::*;
use std::path::Path;

fn note(at: f64, dur: f64, n: f32, patch: &str) -> score::ScoreNote {
    score::ScoreNote {
        at,
        dur,
        note: Some(n),
        patch: Some(patch.into()),
        ..Default::default()
    }
}

fn chord_score(block: usize) -> Score {
    let mut s = Score {
        block,
        seconds: 2.0,
        ..Default::default()
    };
    for i in 0..12 {
        s.notes.push(note(
            i as f64 * 0.11,
            0.4,
            48.0 + i as f32 * 2.0,
            ["pluck", "pad", "bass", "lead", "bell"][i % 5],
        ));
    }
    s.notes.push(note(0.5, 0.3, 36.0, "thump"));
    s.notes.push(note(0.2, 1.0, 60.0, "wind"));
    s
}

#[test]
fn sine_pitch_and_level() {
    let patch = Patch {
        gain: 0.5,
        amp: dsp::Adsr {
            attack: 0.001,
            decay: 0.01,
            sustain: 1.0,
            release: 0.01,
        },
        ..Patch::default()
    };
    let s = Score {
        seconds: 1.0,
        notes: vec![score::ScoreNote {
            dur: 0.9,
            hz: Some(440.0),
            custom: Some(patch),
            ..Default::default()
        }],
        ..Default::default()
    };
    let r = render_score(&s, Path::new(".")).unwrap();
    let rep = analyze(&r.frames, r.rate);
    let hz = rep.dominant_hz.unwrap();
    assert!((hz - 440.0).abs() < 1.5, "pitch {hz}");
    // Centre pan is -3 dB per channel: 0.5 * 0.7071.
    assert!((rep.peak - 0.3536).abs() < 0.01, "peak {}", rep.peak);
    assert!(rep.dc.abs() < 1e-3 && rep.clipped == 0 && rep.non_finite == 0);
    assert!(rep.max_step < 0.1, "click {}", rep.max_step);
}

#[test]
fn waves_have_the_right_pitch() {
    for wave in [Wave::Saw, Wave::Square, Wave::Triangle] {
        let patch = Patch {
            wave,
            amp: dsp::Adsr {
                attack: 0.001,
                decay: 0.01,
                sustain: 1.0,
                release: 0.01,
            },
            ..Patch::default()
        };
        let s = Score {
            seconds: 1.0,
            notes: vec![score::ScoreNote {
                dur: 0.9,
                hz: Some(500.0),
                custom: Some(patch),
                ..Default::default()
            }],
            ..Default::default()
        };
        let r = render_score(&s, Path::new(".")).unwrap();
        let rep = analyze(&r.frames, r.rate);
        assert!(
            (rep.dominant_hz.unwrap() - 500.0).abs() < 2.0,
            "{wave:?} {:?}",
            rep.dominant_hz
        );
        assert!(
            rep.peak > 0.05 && rep.peak <= 1.0 && rep.dc.abs() < 0.01,
            "{wave:?} {rep:?}"
        );
    }
}

#[test]
fn output_is_identical_for_any_block_size() {
    let reference = hash_frames(&render_score(&chord_score(256), Path::new(".")).unwrap().frames);
    for block in [1, 7, 64, 480, 4096] {
        let h = hash_frames(&render_score(&chord_score(block), Path::new(".")).unwrap().frames);
        assert_eq!(h, reference, "block {block}");
    }
}

#[test]
fn notes_start_on_the_exact_frame() {
    let (mut synth, mut h) = create(48_000, 8);
    let patch = Patch {
        wave: Wave::Noise,
        amp: dsp::Adsr {
            attack: 0.0,
            decay: 0.1,
            sustain: 1.0,
            release: 0.01,
        },
        ..Patch::default()
    };
    assert!(h.note_on(1000, 1, 440.0, 1.0, patch));
    assert!(h.note_on(3001, 2, 440.0, 1.0, patch));
    let mut out = vec![[0.0; 2]; 4000];
    // Render in awkward blocks so the commands land mid-block.
    let (a, b) = out.split_at_mut(777);
    synth.render(a);
    synth.render(b);
    let first = out.iter().position(|f| f[0] != 0.0).unwrap();
    assert_eq!(first, 1000);
    assert_eq!(synth.stats().late.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(synth.active_voices(), 2);
    h.note_off(3500, 1);
}

#[test]
fn late_commands_run_now_and_are_counted() {
    let (mut synth, mut h) = create(48_000, 4);
    let mut out = vec![[0.0; 2]; 512];
    synth.render(&mut out);
    h.note_on(10, 1, 440.0, 1.0, Patch::default());
    synth.render(&mut out);
    assert_eq!(synth.stats().late.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(synth.active_voices(), 1);
}

#[test]
fn voice_stealing_stays_bounded_and_clean() {
    let mut s = Score {
        seconds: 3.0,
        voices: 16,
        ..Default::default()
    };
    for i in 0..300 {
        s.notes
            .push(note(i as f64 * 0.005, 1.0, 40.0 + (i % 40) as f32, "pad"));
    }
    let r = render_score(&s, Path::new(".")).unwrap();
    let rep = analyze(&r.frames, r.rate);
    assert!(r.stats.stolen > 100, "{:?}", r.stats);
    assert!(r.stats.voices <= 16);
    assert_eq!(rep.non_finite, 0);
    assert!(rep.peak <= 1.0, "limiter keeps the output inside full scale");
}

#[test]
fn release_does_not_click() {
    let s = Score {
        seconds: 1.5,
        notes: vec![
            note(0.0, 0.2, 57.0, "pad"),
            note(0.1, 0.3, 64.0, "bell"),
            note(0.2, 0.1, 50.0, "pad"),
        ],
        ..Default::default()
    };
    let r = render_score(&s, Path::new(".")).unwrap();
    assert!(analyze(&r.frames, r.rate).max_step < 0.05);
}

#[test]
fn master_gain_glides() {
    let s = Score {
        seconds: 1.0,
        notes: vec![note(0.0, 0.9, 69.0, "lead")],
        master: vec![score::ScoreMaster { at: 0.5, gain: 0.0 }],
        ..Default::default()
    };
    let r = render_score(&s, Path::new(".")).unwrap();
    let tail = analyze(&r.frames[44_000..], r.rate);
    assert!(tail.peak < 1e-3, "{tail:?}");
    assert!(analyze(&r.frames, r.rate).max_step < 0.1);
}

struct Ping {
    left: usize,
}
impl Generator for Ping {
    fn render(&mut self, out: &mut [[f32; 2]], _frame: u64, _rate: f32) {
        for f in out.iter_mut().take(self.left) {
            f[0] += 0.25;
            f[1] += 0.25;
        }
        self.left = self.left.saturating_sub(out.len());
    }
    fn finished(&self) -> bool {
        self.left == 0
    }
}

#[test]
fn custom_generators_mix_in_and_come_back() {
    let (mut synth, mut h) = create(48_000, 4);
    assert!(h.install(0, Box::new(Ping { left: 300 })));
    let mut out = vec![[0.0; 2]; 256];
    synth.render(&mut out);
    assert_eq!(out[0], [0.25, 0.25]);
    synth.render(&mut out);
    assert_eq!(out[43], [0.25, 0.25]);
    assert_eq!(out[44], [0.0, 0.0]);
    h.free_trash(); // the finished generator was handed back, not dropped on the audio thread
    assert_eq!(
        synth
            .stats()
            .trash_overflow
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

fn write_tone(dir: &Path, name: &str, hz: f32, secs: f32, rate: u32) -> std::path::PathBuf {
    let n = (secs * rate as f32) as usize;
    let frames: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let v = (std::f32::consts::TAU * hz * i as f32 / rate as f32).sin() * 0.4;
            [v, v]
        })
        .collect();
    let p = dir.join(name);
    std::fs::write(&p, wav_stereo_bytes(&frames, rate)).unwrap();
    p
}

#[test]
fn streams_play_from_disk_at_the_right_pitch_and_time() {
    let dir = marionette_host::temp_dir("synth-stream").unwrap();
    write_tone(dir.path(), "a.wav", 220.0, 1.0, 48_000);
    let s = Score {
        seconds: 2.0,
        streams: vec![score::ScoreStream {
            file: "a.wav".into(),
            at: 0.25,
            gain: 1.0,
            looping: false,
        }],
        ..Default::default()
    };
    let r = render_score(&s, dir.path()).unwrap();
    let rep = analyze(&r.frames, r.rate);
    assert_eq!(
        rep.first_sound_frame,
        Some(12_001),
        "starts on the frame (sine is 0 at its first sample)"
    );
    assert!((rep.dominant_hz.unwrap() - 220.0).abs() < 1.0, "{rep:?}");
    assert!((rep.peak - 0.4).abs() < 0.01);
    // Plays once: nothing after 1.25 s.
    assert!(analyze(&r.frames[61_000..], r.rate).peak < 1e-4);
}

#[test]
fn streams_loop_and_resample_without_holes() {
    let dir = marionette_host::temp_dir("synth-loop").unwrap();
    write_tone(dir.path(), "b.wav", 441.0, 0.3, 44_100); // file rate differs from the synth's
    let s = Score {
        seconds: 2.0,
        streams: vec![score::ScoreStream {
            file: "b.wav".into(),
            at: 0.0,
            gain: 1.0,
            looping: true,
        }],
        ..Default::default()
    };
    let r = render_score(&s, dir.path()).unwrap();
    let rep = analyze(&r.frames, r.rate);
    assert!((rep.dominant_hz.unwrap() - 441.0).abs() < 3.0, "{rep:?}");
    // Looping after 0.3 s must still make sound at 1.9 s.
    assert!(analyze(&r.frames[90_000..], r.rate).rms > 0.1);
    assert_eq!(r.stats.underruns, 0);
}

#[test]
fn missing_stream_file_is_an_error_not_a_panic() {
    let s = Score {
        streams: vec![score::ScoreStream {
            file: "nope.wav".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(render_score(&s, Path::new(".")).unwrap_err().contains("nope.wav"));
}

#[test]
fn bad_scores_fail_with_a_reason() {
    let s = Score {
        notes: vec![score::ScoreNote::default()],
        ..Default::default()
    };
    assert!(render_score(&s, Path::new(".")).unwrap_err().contains("note"));
    let s = Score {
        notes: vec![note(0.0, 1.0, 60.0, "nope")],
        ..Default::default()
    };
    assert!(
        render_score(&s, Path::new("."))
            .unwrap_err()
            .contains("unknown patch")
    );
}

#[test]
fn render_cost_leaves_headroom() {
    // Debug build, so a loose bound: 64 sustained voices for 1 s of audio in well under 1 s.
    // The release figure is measured by the `bench` example and recorded in the proofs.
    let (mut synth, mut h) = create(48_000, 64);
    for i in 0..64 {
        h.note_on(0, i, 100.0 + i as f32 * 20.0, 1.0, Patch::preset("pad").unwrap());
    }
    let mut out = vec![[0.0; 2]; 480];
    let t = std::time::Instant::now();
    for _ in 0..100 {
        synth.render(&mut out);
    }
    let per_block = t.elapsed().as_secs_f64() / 100.0;
    assert!(
        per_block < 0.010,
        "480 frames (10 ms of audio) took {per_block} s"
    );
}
