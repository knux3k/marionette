//! Synth operations (decision 0220): render procedural and streamed audio to numbers and a WAV
//! file without a sound card, and list the preset sounds. An agent cannot listen, so a score
//! (timed notes and streams as JSON) renders offline with the same code the audio thread runs,
//! and the answer is a report: level, clipping, clicks, pitch, timing and a hash.

use crate::{Agent, Op, op, write_atomic};
use marionette_core::{Diagnostic, Errors};
use marionette_synth::{
    PRESETS, Patch, Report, Score, StatsSnapshot, analyze, hash_frames, render_score, wav_stereo_bytes,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "synth_render",
            "Render a synth score offline; report level, clipping, clicks, pitch, timing as numbers.",
            false,
            render,
        ),
        op(
            "synth_patches",
            "Synth presets and every Patch field with its default.",
            false,
            patches,
        ),
    ]
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderInput {
    // Report: peak, rms, dc, clipped, max_step (clicks), dominant_hz, first_sound_frame, hash and
    // stats (stolen, dropped, late, underruns). Same code as the real audio thread.
    /// Inline score: `{rate, voices, seconds, block, notes:[{at, dur, note|hz, velocity,
    /// patch|custom}], streams:[{file, at, gain, looping}], master:[{at, gain}]}`. Times in seconds.
    pub score: Option<Value>,
    /// Score JSON file, relative to the project.
    pub file: Option<String>,
    /// Where stream files are found, relative to the project (default: the project root).
    pub base: Option<String>,
    /// Write the render here as a 16-bit stereo WAV (relative to the project).
    pub wav: Option<String>,
    /// Also return rms per window of this many milliseconds (at most 400 windows).
    pub envelope_ms: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
pub struct RenderOutput {
    #[schemars(with = "Value")]
    pub report: Report,
    #[schemars(with = "Value")]
    pub stats: StatsSnapshot,
    /// Hash of the rendered samples: the same score gives the same hash everywhere.
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wav: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub envelope: Vec<f32>,
}

fn bad(msg: String) -> Errors {
    Diagnostic::error("synth.bad_score", msg).into()
}

fn render(agent: &mut Agent, input: RenderInput) -> Result<RenderOutput, Errors> {
    let score: Score = match (input.score, &input.file) {
        (Some(v), _) => serde_json::from_value(v).map_err(|e| bad(format!("bad score: {e}")))?,
        (None, Some(f)) => {
            let text = std::fs::read_to_string(agent.resolve(f)?)
                .map_err(|e| bad(format!("cannot read {f}: {e}")))?;
            serde_json::from_str(&text).map_err(|e| bad(format!("bad score in {f}: {e}")))?
        }
        (None, None) => return Err(bad("give `score` or `file`".into())),
    };
    let base = match &input.base {
        Some(b) => agent.resolve(b)?,
        None => agent.resolve(".")?,
    };
    let r = render_score(&score, &base).map_err(bad)?;
    let report = analyze(&r.frames, r.rate);
    let hash = hash_frames(&r.frames);
    let envelope = match input.envelope_ms {
        Some(ms) if ms > 0 => {
            let w = (u64::from(ms) * u64::from(r.rate) / 1000).max(1) as usize;
            r.frames
                .chunks(w)
                .take(400)
                .map(|c| {
                    let s: f32 = c.iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum();
                    (s / (c.len() * 2) as f32).sqrt()
                })
                .collect()
        }
        _ => Vec::new(),
    };
    if let Some(w) = &input.wav {
        write_atomic(&agent.resolve(w)?, w, &wav_stereo_bytes(&r.frames, r.rate))?;
    }
    Ok(RenderOutput {
        report,
        stats: r.stats,
        hash,
        wav: input.wav,
        envelope,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchesInput {}

#[derive(Serialize, JsonSchema)]
pub struct PatchesOutput {
    pub presets: Vec<String>,
    /// Every Patch field at its default.
    #[schemars(with = "Value")]
    pub default: Patch,
    /// The presets in full.
    pub patches: serde_json::Map<String, Value>,
}

fn patches(_: &mut Agent, _: PatchesInput) -> Result<PatchesOutput, Errors> {
    let mut map = serde_json::Map::new();
    for n in PRESETS {
        map.insert(
            n.into(),
            serde_json::to_value(Patch::preset(n).expect("preset exists")).unwrap_or_default(),
        );
    }
    Ok(PatchesOutput {
        presets: PRESETS.iter().map(|s| (*s).into()).collect(),
        default: Patch::default(),
        patches: map,
    })
}
