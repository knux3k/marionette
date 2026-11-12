//! Marionette audio.
//!
//! The simulation never touches a sound card. In [`Stage::Effects`] it decides which sounds
//! start this tick (from `AudioSource` triggers) and writes a `sound` event to the
//! `EventLog` with the gain and stereo pan heard by the `AudioListener`. Headless runs stop
//! there, so tests can assert on what played and when. In windowed mode a [`Player`] reads
//! new `sound` events after each frame and plays them with kira (WAV/OGG files or built-in
//! sounds).

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::Registry;
use marionette_core::actions::ActionState;
use marionette_core::builtin::{Position, SimClock};
use marionette_core::module::{EventLog, Module, Stage};
use marionette_core::scene::SceneId;
use marionette_core::spatial::Transform;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// When an `AudioSource` plays.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub enum SoundTrigger {
    /// On the first tick (tick 0).
    OnStart,
    /// When this entity starts touching another (needs physics).
    OnCollision,
    /// When this entity's character controller jumps.
    OnJump,
    /// When an input action is pressed.
    OnAction { action: String },
}

/// A sound attached to an entity. `sound` is a WAV/OGG file relative to the scene or a
/// built-in sound (`builtin:beep`, `builtin:jump`, `builtin:thud`).
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct AudioSource {
    pub sound: String,
    /// Linear gain (1.0 = as recorded).
    pub volume: f32,
    pub looping: bool,
    pub trigger: SoundTrigger,
}

/// Where sounds are heard from (usually on the camera). Pan follows the listener's right
/// (+X) axis; gain falls off as `1 / (1 + distance)`.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct AudioListener {
    /// Master gain.
    pub volume: f32,
}

pub struct AudioModule;

impl Module for AudioModule {
    fn name(&self) -> &'static str {
        "audio"
    }
    fn register(&self, r: &mut Registry) {
        r.register_component::<AudioSource>();
        r.register_component::<AudioListener>();
    }
    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(play_sounds.in_set(Stage::Effects));
    }
}

fn place(world: &World, e: Entity) -> ([f32; 3], [f32; 4]) {
    let t = world.get::<Transform>(e).copied().unwrap_or_default();
    let p = world
        .get::<Position>(e)
        .map_or([t.translation.x, t.translation.y, t.translation.z], |p| {
            [p.x, p.y, p.z]
        });
    (p, [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w])
}

/// Rotates (1, 0, 0) by quaternion q (x, y, z, w), using only + - *.
fn right_axis(q: [f32; 4]) -> [f32; 3] {
    let [x, y, z, w] = q;
    [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y + w * z),
        2.0 * (x * z - w * y),
    ]
}

/// A listener: (position, rotation quaternion) and master gain.
pub type Listener = (([f32; 3], [f32; 4]), f32);

/// Gain and pan of a source heard by a listener.
pub fn spatialize(listener: Option<Listener>, source: [f32; 3], volume: f32) -> (f32, f32) {
    let Some(((lp, lq), lv)) = listener else {
        return (volume, 0.0);
    };
    let d = [source[0] - lp[0], source[1] - lp[1], source[2] - lp[2]];
    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let gain = volume * lv / (1.0 + dist);
    if dist < 1e-6 {
        return (gain, 0.0);
    }
    let r = right_axis(lq);
    let pan = ((d[0] * r[0] + d[1] * r[1] + d[2] * r[2]) / dist).clamp(-1.0, 1.0);
    (gain, pan)
}

fn play_sounds(world: &mut World) {
    let tick = world.resource::<SimClock>().tick;
    let mut q = world.query::<(Entity, &SceneId, &AudioSource)>();
    let mut sources: Vec<(u64, Entity, AudioSource)> =
        q.iter(world).map(|(e, id, s)| (id.0, e, s.clone())).collect();
    if sources.is_empty() {
        return;
    }
    sources.sort_by_key(|(id, _, _)| *id);
    let mut lq = world.query::<(Entity, &SceneId, &AudioListener)>();
    let listener = lq
        .iter(world)
        .min_by_key(|(_, id, _)| id.0)
        .map(|(e, _, l)| (place(world, e), l.volume));
    let this_tick: Vec<(String, u64, Option<u64>)> = world
        .resource::<EventLog>()
        .of_tick(tick)
        .filter_map(|e| e.entity.map(|a| (e.kind.clone(), a, e.other)))
        .collect();
    let involved = |kind: &str, id: u64| {
        this_tick
            .iter()
            .any(|(k, a, b)| k == kind && (*a == id || *b == Some(id)))
    };
    let actions = world.get_resource::<ActionState>().cloned().unwrap_or_default();
    let mut played = Vec::new();
    for (id, e, s) in &sources {
        let fire = match &s.trigger {
            SoundTrigger::OnStart => tick == 0,
            SoundTrigger::OnCollision => involved("collision_started", *id),
            SoundTrigger::OnJump => involved("jump", *id),
            SoundTrigger::OnAction { action } => actions.just_pressed(action),
        };
        if fire {
            let (pos, _) = place(world, *e);
            let (gain, pan) = spatialize(listener, pos, s.volume);
            played.push((*id, s.sound.clone(), gain, pan, s.looping));
        }
    }
    let mut log = world.resource_mut::<EventLog>();
    for (id, sound, gain, pan, looping) in played {
        log.push(
            tick,
            "sound",
            Some(id),
            None,
            json!({ "sound": sound, "gain": gain, "pan": pan, "looping": looping }),
        );
    }
}

/// Built-in sounds as mono samples at 44.1 kHz (generated, so no files are needed).
pub fn builtin_samples(name: &str) -> Option<Vec<f32>> {
    const RATE: f32 = 44_100.0;
    let n = |secs: f32| (secs * RATE) as usize;
    let tau = std::f32::consts::TAU;
    Some(match name {
        "beep" => (0..n(0.15))
            .map(|i| (tau * 880.0 * i as f32 / RATE).sin() * 0.4 * (1.0 - i as f32 / n(0.15) as f32))
            .collect(),
        "jump" => (0..n(0.25))
            .map(|i| {
                let t = i as f32 / RATE;
                (tau * (300.0 * t + 1200.0 * t * t)).sin() * 0.35 * (1.0 - t / 0.25)
            })
            .collect(),
        "thud" => {
            let mut s: u32 = 0x1234_5678;
            (0..n(0.2))
                .map(|i| {
                    s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                    let t = i as f32 / RATE;
                    (noise * 0.3 + (tau * 70.0 * t).sin() * 0.5) * (-t * 25.0).exp()
                })
                .collect()
        }
        _ => return None,
    })
}

/// Encodes mono samples as a 16-bit PCM WAV file.
pub fn wav_bytes(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    b
}

/// Loads a sound as kira data (no audio device needed): a `builtin:` name or a WAV/OGG file.
pub fn load_sound(
    sound: &str,
    base_dir: &Path,
) -> Result<kira::sound::static_sound::StaticSoundData, String> {
    use kira::sound::static_sound::StaticSoundData;
    if let Some(name) = sound.strip_prefix("builtin:") {
        let samples = builtin_samples(name)
            .ok_or_else(|| format!("unknown built-in sound `{name}`; built-ins are beep, jump, thud"))?;
        return StaticSoundData::from_cursor(std::io::Cursor::new(wav_bytes(&samples, 44_100)))
            .map_err(|e| format!("cannot decode built-in sound: {e}"));
    }
    StaticSoundData::from_file(base_dir.join(sound)).map_err(|e| format!("cannot load sound {sound}: {e}"))
}

/// Plays `sound` events from the log through the default audio device (windowed mode).
pub struct Player {
    manager: kira::AudioManager<kira::DefaultBackend>,
    cache: BTreeMap<String, kira::sound::static_sound::StaticSoundData>,
    base_dir: PathBuf,
    seen: u64,
}

impl Player {
    pub fn new(base_dir: PathBuf) -> Result<Self, String> {
        let manager = kira::AudioManager::<kira::DefaultBackend>::new(kira::AudioManagerSettings::default())
            .map_err(|e| format!("cannot open the audio device: {e}"))?;
        Ok(Self {
            manager,
            cache: BTreeMap::new(),
            base_dir,
            seen: 0,
        })
    }

    /// Plays every `sound` event added since the last call. Problems are returned, not fatal.
    pub fn play_new(&mut self, log: &EventLog) -> Vec<String> {
        let mut problems = Vec::new();
        for e in log.since(self.seen).filter(|e| e.kind == "sound") {
            let Some(sound) = e.detail["sound"].as_str() else {
                continue;
            };
            if !self.cache.contains_key(sound) {
                match load_sound(sound, &self.base_dir) {
                    Ok(d) => {
                        self.cache.insert(sound.to_string(), d);
                    }
                    Err(m) => {
                        problems.push(m);
                        continue;
                    }
                }
            }
            let gain = e.detail["gain"].as_f64().unwrap_or(1.0) as f32;
            let db = if gain > 1e-4 { 20.0 * gain.log10() } else { -60.0 };
            let mut data = self.cache[sound]
                .volume(kira::Decibels(db))
                .panning(kira::Panning(e.detail["pan"].as_f64().unwrap_or(0.0) as f32));
            if e.detail["looping"] == true {
                data = data.loop_region(..);
            }
            if let Err(err) = self.manager.play(data) {
                problems.push(format!("cannot play {sound}: {err}"));
            }
        }
        self.seen = log.end();
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_sounds_decode_through_kira_without_a_device() {
        for name in ["beep", "jump", "thud"] {
            let data = load_sound(&format!("builtin:{name}"), Path::new(".")).unwrap();
            assert_eq!(data.sample_rate, 44_100);
            assert!(data.frames.len() > 1000);
        }
    }

    #[test]
    fn wav_files_decode() {
        let dir = std::env::temp_dir().join(format!("marionette-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("t.wav"),
            wav_bytes(&builtin_samples("beep").unwrap(), 44_100),
        )
        .unwrap();
        let data = load_sound("t.wav", &dir).unwrap();
        assert_eq!(data.frames.len(), builtin_samples("beep").unwrap().len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn panning_follows_the_listener() {
        let listener = Some((([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]), 1.0));
        let (g, p) = spatialize(listener, [3.0, 0.0, 0.0], 1.0);
        assert_eq!((g, p), (0.25, 1.0));
        let (_, p) = spatialize(listener, [-2.0, 0.0, 0.0], 1.0);
        assert_eq!(p, -1.0);
        let (_, p) = spatialize(listener, [0.0, 0.0, -5.0], 1.0);
        assert_eq!(p, 0.0);
    }
}
