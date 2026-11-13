//! Music layers (decision 0075): stems of one piece that start together and fade in and out
//! as the game's intensity changes ("vertical layering"). The levels live in the `Music`
//! component and move on the fixed tick, so they are saved, hashed and replayed like any
//! other state; the player only follows them.

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::builtin::{DEFAULT_DT, FixedStep, SimClock};
use marionette_core::module::EventLog;
use marionette_core::scene::SceneId;
use serde_json::json;

/// One stem. All stems of a `Music` should have the same length so their loops stay aligned.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct MusicLayer {
    /// WAV/OGG file relative to the scene, or a built-in stem (`builtin:music_pad`,
    /// `builtin:music_bass`, `builtin:music_drums`, `builtin:music_lead`).
    pub sound: String,
    /// The layer plays while the music's intensity is at least this.
    pub at: f32,
    /// Gain of this stem at full level.
    pub mix: f32,
    /// Level now, 0..1 (moved by the engine; start at 0).
    pub level: f32,
}

/// Layered music, not positional. Raise `intensity` to bring layers in.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct Music {
    pub layers: Vec<MusicLayer>,
    pub intensity: f32,
    /// Master gain.
    pub volume: f32,
    /// Seconds for a layer to fade fully in or out (0 = instant).
    pub fade: f32,
    /// False fades every layer out (the player stops the stems once all are silent).
    pub playing: bool,
}

impl Music {
    /// Level each layer is heading to.
    pub fn target(&self, layer: &MusicLayer) -> f32 {
        if self.playing && self.intensity >= layer.at {
            1.0
        } else {
            0.0
        }
    }

    /// Moves every layer's level toward its target. Returns (layer index, came in) for each
    /// layer that left silence or fell silent.
    pub fn advance(&mut self, dt: f32) -> Vec<(usize, bool)> {
        let mut changed = Vec::new();
        for i in 0..self.layers.len() {
            let target = self.target(&self.layers[i]);
            let l = &mut self.layers[i];
            let before = l.level;
            if self.fade <= 0.0 {
                l.level = target;
            } else if l.level < target {
                l.level = (l.level + dt / self.fade).min(target);
            } else if l.level > target {
                l.level = (l.level - dt / self.fade).max(target);
            }
            if (l.level - target).abs() < 1e-5 {
                l.level = target;
            }
            if before == 0.0 && l.level > 0.0 {
                changed.push((i, true));
            } else if before > 0.0 && l.level == 0.0 {
                changed.push((i, false));
            }
        }
        changed
    }

    /// Gain of layer `i` now (level × mix × volume).
    pub fn gain(&self, i: usize) -> f32 {
        self.layers.get(i).map_or(0.0, |l| l.level * l.mix * self.volume)
    }
}

/// Tick system: moves layer levels and logs `music_layer` events (`in` / `out`).
pub(crate) fn advance_music(world: &mut World) {
    let dt = world.get_resource::<FixedStep>().map_or(DEFAULT_DT, |s| s.dt);
    let tick = world.get_resource::<SimClock>().map_or(0, |c| c.tick);
    let mut q = world.query::<(Entity, &SceneId, &Music)>();
    let mut all: Vec<(u64, Entity)> = q.iter(world).map(|(e, id, _)| (id.0, e)).collect();
    if all.is_empty() {
        return;
    }
    all.sort_by_key(|(id, _)| *id);
    let mut events = Vec::new();
    for (id, e) in all {
        let Some(mut m) = world.get::<Music>(e).cloned() else {
            continue;
        };
        let changed = m.advance(dt);
        for (i, came_in) in changed {
            events.push((
                id,
                json!({ "layer": i, "sound": m.layers[i].sound, "state": if came_in { "in" } else { "out" } }),
            ));
        }
        if world.get::<Music>(e) != Some(&m) {
            world.entity_mut(e).insert(m);
        }
    }
    if let Some(mut log) = world.get_resource_mut::<EventLog>() {
        for (id, detail) in events {
            log.push(tick, "music_layer", Some(id), None, detail);
        }
    }
}

/// Length of every built-in stem: two bars at 120 beats per minute.
pub const STEM_SECONDS: f32 = 4.0;

/// Built-in stems (mono, 44.1 kHz, 4 s loops in A minor at 120 BPM). Every frequency is a
/// multiple of 0.25 Hz or every note ends at zero, so the loops have no click.
pub fn builtin_stem(name: &str) -> Option<Vec<f32>> {
    const RATE: f32 = 44_100.0;
    let n = (STEM_SECONDS * RATE) as usize;
    let tau = std::f32::consts::TAU;
    let beat = 0.5; // seconds
    let mut out = vec![0.0f32; n];
    match name {
        "music_pad" => {
            // A minor triad, just intonation, slow swell twice per loop.
            for (i, s) in out.iter_mut().enumerate() {
                let t = i as f32 / RATE;
                let swell = 0.75 + 0.25 * (tau * 0.5 * t).sin();
                let v: f32 = [220.0, 264.0, 330.0].iter().map(|f| (tau * f * t).sin()).sum();
                *s = v * 0.09 * swell;
            }
        }
        "music_bass" => {
            // One note per beat: A A A A F F G G (just ratios of 110 Hz).
            let notes = [110.0, 110.0, 110.0, 110.0, 88.0, 88.0, 99.0, 99.0];
            for (i, s) in out.iter_mut().enumerate() {
                let t = i as f32 / RATE;
                let k = ((t / beat) as usize).min(notes.len() - 1);
                let local = t - k as f32 * beat;
                let env = (local * 200.0).min(1.0) * (1.0 - local / beat).max(0.0);
                let f = notes[k];
                *s = ((tau * f * t).sin() + 0.3 * (tau * 2.0 * f * t).sin()) * 0.3 * env;
            }
        }
        "music_drums" => {
            // Kick on every beat, hat on every off-beat.
            let mut seed: u32 = 0x9e37_79b9;
            for (i, s) in out.iter_mut().enumerate() {
                let t = i as f32 / RATE;
                let local = t % beat;
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                let kick = (tau * (50.0 * local + 300.0 * local * (0.15 - local).max(0.0))).sin()
                    * (-local * 18.0).exp()
                    * 0.6;
                let off = local - beat * 0.5;
                let hat = if off >= 0.0 {
                    noise * (-off * 60.0).exp() * 0.15
                } else {
                    0.0
                };
                *s = kick + hat;
            }
        }
        "music_lead" => {
            // Sixteenth-note arpeggio over A minor, each note with its own short envelope.
            let notes = [440.0, 528.0, 660.0, 880.0, 660.0, 528.0, 440.0, 330.0];
            let step = beat / 4.0;
            for (i, s) in out.iter_mut().enumerate() {
                let t = i as f32 / RATE;
                let k = (t / step) as usize;
                let local = t - k as f32 * step;
                let f = notes[k % notes.len()];
                let env = (local * 400.0).min(1.0) * (1.0 - local / step).max(0.0);
                let tri = 2.0 * (2.0 * (f * t - (f * t + 0.5).floor())).abs() - 1.0;
                *s = tri * 0.12 * env;
            }
        }
        _ => return None,
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn music(intensity: f32) -> Music {
        Music {
            layers: ["a", "b", "c"]
                .iter()
                .enumerate()
                .map(|(i, s)| MusicLayer {
                    sound: s.to_string(),
                    at: i as f32,
                    mix: 1.0,
                    level: 0.0,
                })
                .collect(),
            intensity,
            volume: 0.5,
            fade: 1.0,
            playing: true,
        }
    }

    #[test]
    fn layers_follow_intensity_and_fade() {
        let mut m = music(1.0);
        assert_eq!(m.advance(0.5), vec![(0, true), (1, true)]);
        assert_eq!(m.layers[0].level, 0.5);
        assert_eq!(m.layers[2].level, 0.0);
        assert!(m.advance(0.5).is_empty());
        assert_eq!(m.gain(1), 0.5);
        m.intensity = 0.0;
        m.advance(0.25);
        assert_eq!(m.layers[1].level, 0.75);
        assert_eq!(m.advance(1.0), vec![(1, false)]);
        m.playing = false;
        assert_eq!(m.advance(1.0), vec![(0, false)]);
        assert!(m.layers.iter().all(|l| l.level == 0.0));
    }

    #[test]
    fn stems_loop_without_a_click() {
        for name in ["music_pad", "music_bass", "music_drums", "music_lead"] {
            let s = builtin_stem(name).unwrap();
            assert_eq!(s.len(), 176_400);
            let (first, last) = (s[0], s[s.len() - 1]);
            assert!((first - last).abs() < 0.05, "{name}: {first} vs {last}");
            assert!(s.iter().all(|v| v.abs() <= 1.0), "{name} clips");
            assert!(s.iter().any(|v| v.abs() > 0.05), "{name} is silent");
        }
    }
}
