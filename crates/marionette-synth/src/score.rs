//! Offline rendering: a [`Score`] (timed notes and streams as plain data) renders to a buffer
//! with the same `Synth` code the audio thread runs, split into blocks of any size. The result
//! is bit-identical whatever the block size, so a score can be checked numerically and pinned
//! by hash, and the same patches then sound the same on a real device.

use crate::engine::{Command, Synth, create};
use crate::stream::{StreamSettings, open_stream};
use crate::voice::Patch;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Notes, streams and gain changes over time. Times are seconds.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Score {
    pub rate: u32,
    /// Polyphony limit.
    pub voices: usize,
    /// Length to render in seconds (0 = until the last event plus 1 s of tail).
    pub seconds: f64,
    /// Block size to render in (any value gives the same samples; tests vary it).
    pub block: usize,
    pub notes: Vec<ScoreNote>,
    pub streams: Vec<ScoreStream>,
    pub master: Vec<ScoreMaster>,
}

/// One note. Give the pitch as `note` (MIDI number, 69 = A4) or `hz`.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct ScoreNote {
    pub at: f64,
    /// Seconds held before release.
    pub dur: f64,
    pub note: Option<f32>,
    pub hz: Option<f32>,
    pub velocity: f32,
    /// A preset name (`pluck`, `pad`, `bass`, `lead`, `thump`, `wind`, `bell`) or `null` with
    /// `custom` set.
    pub patch: Option<String>,
    pub custom: Option<Patch>,
}

impl Default for ScoreNote {
    fn default() -> Self {
        Self { at: 0.0, dur: 0.5, note: None, hz: None, velocity: 1.0, patch: None, custom: None }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct ScoreStream {
    /// Path relative to the folder given to `render_score`.
    pub file: String,
    pub at: f64,
    pub gain: f32,
    pub looping: bool,
}

impl Default for ScoreStream {
    fn default() -> Self {
        Self { file: String::new(), at: 0.0, gain: 1.0, looping: false }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ScoreMaster {
    pub at: f64,
    pub gain: f32,
}

/// Names `Patch::preset` knows.
pub const PRESETS: [&str; 7] = ["pluck", "pad", "bass", "lead", "thump", "wind", "bell"];

impl Patch {
    /// A named starting sound. Unknown names return `None`.
    pub fn preset(name: &str) -> Option<Patch> {
        use crate::dsp::{Adsr, FilterKind, Wave};
        let a = |attack, decay, sustain, release| Adsr { attack, decay, sustain, release };
        Some(match name {
            "pluck" => Patch {
                wave: Wave::Saw,
                filter: FilterKind::LowPass,
                cutoff: 600.0,
                filter_env: 3.0,
                filter_adsr: a(0.001, 0.15, 0.0, 0.1),
                amp: a(0.001, 0.25, 0.0, 0.1),
                ..Patch::default()
            },
            "pad" => Patch {
                wave: Wave::Saw,
                wave2: Wave::Saw,
                mix2: 0.8,
                detune_cents: 9.0,
                filter: FilterKind::LowPass,
                cutoff: 1400.0,
                amp: a(0.6, 0.5, 0.8, 1.2),
                vibrato_cents: 6.0,
                gain: 0.3,
                ..Patch::default()
            },
            "bass" => Patch {
                wave: Wave::Square,
                wave2: Wave::Sine,
                mix2: 0.6,
                detune_cents: -1200.0,
                filter: FilterKind::LowPass,
                cutoff: 500.0,
                filter_env: 2.0,
                filter_adsr: a(0.002, 0.2, 0.2, 0.1),
                amp: a(0.003, 0.2, 0.7, 0.1),
                ..Patch::default()
            },
            "lead" => Patch {
                wave: Wave::Square,
                wave2: Wave::Saw,
                mix2: 0.4,
                filter: FilterKind::LowPass,
                cutoff: 3000.0,
                resonance: 1.5,
                amp: a(0.01, 0.1, 0.7, 0.2),
                vibrato_hz: 5.5,
                vibrato_cents: 12.0,
                gain: 0.35,
                ..Patch::default()
            },
            "thump" => Patch {
                wave: Wave::Sine,
                noise: 0.2,
                filter: FilterKind::LowPass,
                cutoff: 200.0,
                filter_env: 2.0,
                filter_adsr: a(0.001, 0.08, 0.0, 0.05),
                amp: a(0.001, 0.18, 0.0, 0.05),
                gain: 0.9,
                ..Patch::default()
            },
            "wind" => Patch {
                wave: Wave::Noise,
                filter: FilterKind::BandPass,
                cutoff: 700.0,
                resonance: 3.0,
                amp: a(1.5, 0.5, 1.0, 2.0),
                vibrato_hz: 0.2,
                gain: 0.4,
                ..Patch::default()
            },
            "bell" => Patch {
                wave: Wave::Sine,
                wave2: Wave::Sine,
                mix2: 0.5,
                detune_cents: 1586.0,
                amp: a(0.001, 1.5, 0.0, 1.0),
                gain: 0.4,
                ..Patch::default()
            },
            _ => return None,
        })
    }
}

/// What `render_score` returns.
#[derive(Debug)]
pub struct Rendered {
    pub frames: Vec<[f32; 2]>,
    pub rate: u32,
    pub stats: crate::engine::StatsSnapshot,
}

fn secs_to_frames(s: f64, rate: u32) -> u64 {
    (s * f64::from(rate)).round().max(0.0) as u64
}

/// Renders a score. `base` resolves stream files. Fails on an unknown preset, missing pitch
/// or unreadable stream; nothing is rendered then. Output is capped at 10 minutes.
pub fn render_score(score: &Score, base: &Path) -> Result<Rendered, String> {
    let rate = if score.rate == 0 { 48_000 } else { score.rate.clamp(8_000, 192_000) };
    let voices = if score.voices == 0 { 64 } else { score.voices.min(1024) };
    let block = if score.block == 0 { 256 } else { score.block.min(1 << 16) };
    let (mut synth, mut handle) = create(rate, voices);

    // Queue is bounded: feed it in order and let the synth drain as time passes. Events are
    // sorted by time, so the pending table sees each in good time.
    struct Ev {
        at: u64,
        cmd: Command,
    }
    let mut events: Vec<Ev> = Vec::new();
    let mut last = 0.0f64;
    for (i, n) in score.notes.iter().enumerate() {
        let patch = match (&n.custom, &n.patch) {
            (Some(p), _) => *p,
            (None, Some(name)) => Patch::preset(name)
                .ok_or_else(|| format!("note {i}: unknown patch `{name}` (pluck, pad, bass, lead, thump, wind, bell)"))?,
            (None, None) => Patch::default(),
        };
        let hz = match (n.hz, n.note) {
            (Some(h), _) => h,
            (None, Some(m)) => crate::dsp::midi_hz(m),
            (None, None) => return Err(format!("note {i} needs `note` (MIDI number) or `hz`")),
        };
        let at = secs_to_frames(n.at, rate);
        let off = secs_to_frames(n.at + n.dur.max(0.0), rate);
        let id = i as u32;
        events.push(Ev { at, cmd: Command::NoteOn { at, id, freq: hz, velocity: n.velocity, patch } });
        events.push(Ev { at: off, cmd: Command::NoteOff { at: off, id } });
        last = last.max(n.at + n.dur + patch.amp.release as f64);
    }
    for m in &score.master {
        let at = secs_to_frames(m.at, rate);
        events.push(Ev { at, cmd: Command::Master { at, gain: m.gain } });
        last = last.max(m.at);
    }
    let mut streams = Vec::new();
    for s in &score.streams {
        let settings = StreamSettings {
            looping: s.looping,
            gain: s.gain,
            start_at: secs_to_frames(s.at, rate),
            blocking: true,
        };
        let (src, h) = open_stream(&base.join(&s.file), handle.stats(), settings)?;
        handle.install(streams.len() as u8, Box::new(src));
        streams.push(h);
        last = last.max(s.at);
    }
    // Stable sort keeps note-on before note-off at the same frame only if pushed so; make it
    // explicit: offs after ons at equal time by pushing order (sort_by_key is stable).
    events.sort_by_key(|e| e.at);
    let total = if score.seconds > 0.0 { score.seconds } else { last + 1.0 }.min(600.0);
    let total_frames = secs_to_frames(total, rate) as usize;

    let mut out = vec![[0.0f32; 2]; total_frames];
    let mut pos = 0usize;
    let mut queue = events.into_iter().peekable();
    while pos < total_frames {
        let n = block.min(total_frames - pos);
        let end = (pos + n) as u64;
        // Hand over every event due before the end of this block, as far as the queue allows.
        while let Some(e) = queue.peek() {
            if e.at >= end + (block as u64) * 4 {
                break;
            }
            let e = queue.next().expect("peeked");
            if !handle.send(e.cmd) {
                return Err("too many events scheduled at once".into());
            }
        }
        synth.render(&mut out[pos..pos + n]);
        pos += n;
    }
    let stats = crate::engine::StatsSnapshot::of(synth_stats(&synth));
    drop(streams);
    Ok(Rendered { frames: out, rate, stats })
}

fn synth_stats(s: &Synth) -> &crate::engine::Stats {
    s.stats()
}
