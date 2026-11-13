//! Marionette audio.
//!
//! The simulation never touches a sound card. In [`Stage::Effects`] it decides which sounds
//! start this tick (from `AudioSource` triggers) and writes a `sound` event to the
//! `EventLog` with the gain and stereo pan heard by the `AudioListener`. Headless runs stop
//! there, so tests can assert on what played and when. In windowed mode a [`Player`] reads
//! new `sound` events after each frame and plays them with kira (WAV/OGG files or built-in
//! sounds).
//!
//! Decision 0075 adds, all opt-in so existing scenes hear and hash the same:
//! - [`SoundSpatial`] on a source: distance curve, muffling behind the listener's head and
//!   Doppler pitch. [`hear`] is the one formula, used by the tick (for the log) and by the
//!   player every frame, so voices follow moving sources and a tracked head between ticks.
//! - [`music::Music`]: stems that start together and fade in and out by an intensity value.
//! - The player keeps a voice per playing sound and stops looping sounds whose entity or
//!   `AudioSource` is gone.

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::Registry;
use marionette_core::actions::ActionState;
use marionette_core::builtin::{Position, SimClock};
use marionette_core::module::{EventLog, Module, Stage};
use marionette_core::scene::SceneId;
use marionette_core::spatial::Transform;
use serde_json::json;
use std::path::Path;

pub mod music;
mod player;
mod synth_host;

pub use marionette_synth as synth;
pub use music::{Music, MusicLayer};
pub use player::{MAX_VOICES, Player};
pub use synth_host::SynthHost;

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
/// built-in sound (`builtin:beep`, `builtin:jump`, `builtin:thud`; music stems in `Music`).
/// Add `SoundSpatial` for 3D distance, rear muffling and Doppler.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct AudioSource {
    pub sound: String,
    /// Linear gain (1.0 = as recorded).
    pub volume: f32,
    pub looping: bool,
    pub trigger: SoundTrigger,
}

/// Where sounds are heard from (usually on the camera, or the head in VR). Pan follows the
/// listener's right (+X) axis; front is its -Z axis. Without a `SoundSpatial` on the source,
/// gain falls off as `1 / (1 + distance)`.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct AudioListener {
    /// Master gain.
    pub volume: f32,
}

/// How gain falls with distance (between `min_distance` and `max_distance`).
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub enum Rolloff {
    /// `min_distance / distance`: natural falloff; held at its `max_distance` value beyond.
    Inverse,
    /// Straight line from full gain at `min_distance` to silence at `max_distance`.
    Linear,
    /// No distance falloff (only direction).
    Constant,
}

/// 3D settings for the `AudioSource` on the same entity.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct SoundSpatial {
    /// Full gain inside this distance (meters); also where panning reaches full width.
    pub min_distance: f32,
    /// See [`Rolloff`].
    pub max_distance: f32,
    pub rolloff: Rolloff,
    /// 0..1: low-pass when the sound is behind the listener (1 = down to 4 kHz straight
    /// behind). Stereo panning alone cannot tell front from back; this can.
    pub rear_muffle: f32,
    /// Doppler pitch shift: 0 off, 1 real speed of sound. Player only (needs velocities).
    pub doppler: f32,
}

impl Default for SoundSpatial {
    fn default() -> Self {
        Self {
            min_distance: 1.0,
            max_distance: 50.0,
            rolloff: Rolloff::Inverse,
            rear_muffle: 0.5,
            doppler: 0.0,
        }
    }
}

/// The listener as the formula needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ear {
    pub position: [f32; 3],
    /// Rotation quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    /// Meters per second (Doppler only).
    pub velocity: [f32; 3],
    /// Master gain.
    pub volume: f32,
}

/// How one source sounds to the listener.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heard {
    pub gain: f32,
    /// -1 left .. 1 right.
    pub pan: f32,
    /// Low-pass cutoff in hertz (20000 = open).
    pub lowpass_hz: f32,
    /// Playback rate (1 = as recorded).
    pub pitch: f32,
}

/// Cutoff meaning "no filtering".
pub const OPEN_HZ: f32 = 20_000.0;
/// Speed of sound, meters per second.
pub const SPEED_OF_SOUND: f32 = 343.0;

fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let [vx, vy, vz] = v;
    // v + 2w(q×v) + 2 q×(q×v), with + - * only.
    let t = [
        2.0 * (y * vz - z * vy),
        2.0 * (z * vx - x * vz),
        2.0 * (x * vy - y * vx),
    ];
    [
        vx + w * t[0] + (y * t[2] - z * t[1]),
        vy + w * t[1] + (z * t[0] - x * t[2]),
        vz + w * t[2] + (x * t[1] - y * t[0]),
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// How a source at `source` (moving at `source_velocity`) with gain `volume` sounds to `ear`.
/// Without `spatial` this is the original rule: `1 / (1 + distance)` and pan only.
pub fn hear(
    ear: Option<&Ear>,
    source: [f32; 3],
    source_velocity: [f32; 3],
    volume: f32,
    spatial: Option<&SoundSpatial>,
) -> Heard {
    let open = Heard {
        gain: volume,
        pan: 0.0,
        lowpass_hz: OPEN_HZ,
        pitch: 1.0,
    };
    let Some(ear) = ear else {
        return open;
    };
    let d = [
        source[0] - ear.position[0],
        source[1] - ear.position[1],
        source[2] - ear.position[2],
    ];
    let dist = dot(d, d).sqrt();
    let Some(sp) = spatial else {
        let gain = volume * ear.volume / (1.0 + dist);
        if dist < 1e-6 {
            return Heard { gain, ..open };
        }
        let pan = (dot(d, right_axis(ear.rotation)) / dist).clamp(-1.0, 1.0);
        return Heard { gain, pan, ..open };
    };
    let min = sp.min_distance.max(1e-3);
    let max = sp.max_distance.max(min);
    let clamped = dist.clamp(min, max);
    let falloff = match sp.rolloff {
        Rolloff::Inverse => min / clamped,
        Rolloff::Linear => {
            if max > min {
                1.0 - (clamped - min) / (max - min)
            } else {
                1.0
            }
        }
        Rolloff::Constant => 1.0,
    };
    let gain = volume * ear.volume * falloff;
    if dist < 1e-6 {
        return Heard { gain, ..open };
    }
    let u = [d[0] / dist, d[1] / dist, d[2] / dist];
    // Narrow the stereo image inside min_distance so a sound at the head does not jump sides.
    let width = (dist / min).min(1.0);
    let pan = (dot(u, rotate(ear.rotation, [1.0, 0.0, 0.0])) * width).clamp(-1.0, 1.0);
    let behind = (-dot(u, rotate(ear.rotation, [0.0, 0.0, -1.0]))).max(0.0);
    let lowpass_hz = OPEN_HZ / (1.0 + 4.0 * sp.rear_muffle.clamp(0.0, 1.0) * behind);
    let pitch = if sp.doppler > 0.0 {
        let c = SPEED_OF_SOUND;
        let k = sp.doppler;
        // Positive when moving toward the other.
        let listener_in = dot(ear.velocity, u) * k;
        let source_in = -dot(source_velocity, u) * k;
        ((c + listener_in) / (c - source_in).max(1.0)).clamp(0.5, 2.0)
    } else {
        1.0
    };
    Heard {
        gain,
        pan,
        lowpass_hz,
        pitch,
    }
}

pub struct AudioModule;

impl Module for AudioModule {
    fn name(&self) -> &'static str {
        "audio"
    }
    fn register(&self, r: &mut Registry) {
        r.register_component::<AudioSource>();
        r.register_component::<AudioListener>();
        r.register_component::<SoundSpatial>();
        r.register_component::<Music>();
    }
    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems((play_sounds, music::advance_music).chain().in_set(Stage::Effects));
    }
}

pub(crate) fn place(world: &World, e: Entity) -> ([f32; 3], [f32; 4]) {
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

/// Gain and pan of a source heard by a listener (the original rule; see [`hear`]).
pub fn spatialize(listener: Option<Listener>, source: [f32; 3], volume: f32) -> (f32, f32) {
    let ear = listener.map(|((position, rotation), volume)| Ear {
        position,
        rotation,
        velocity: [0.0; 3],
        volume,
    });
    let h = hear(ear.as_ref(), source, [0.0; 3], volume, None);
    (h.gain, h.pan)
}

/// The listener of `world` (lowest scene id with an `AudioListener`), from its entity pose.
pub fn listener_ear(world: &mut World) -> Option<Ear> {
    let mut lq = world.query::<(Entity, &SceneId, &AudioListener)>();
    let (e, l) = lq
        .iter(world)
        .min_by_key(|(_, id, _)| id.0)
        .map(|(e, _, l)| (e, l.volume))?;
    let (position, rotation) = place(world, e);
    Some(Ear {
        position,
        rotation,
        velocity: [0.0; 3],
        volume: l,
    })
}

fn play_sounds(world: &mut World) {
    let tick = world.resource::<SimClock>().tick;
    let mut q = world.query::<(Entity, &SceneId, &AudioSource, Option<&SoundSpatial>)>();
    let mut sources: Vec<(u64, Entity, AudioSource, Option<SoundSpatial>)> = q
        .iter(world)
        .map(|(e, id, s, sp)| (id.0, e, s.clone(), sp.copied()))
        .collect();
    if sources.is_empty() {
        return;
    }
    sources.sort_by_key(|(id, _, _, _)| *id);
    let ear = listener_ear(world);
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
    for (id, e, s, spatial) in &sources {
        let fire = match &s.trigger {
            SoundTrigger::OnStart => tick == 0,
            SoundTrigger::OnCollision => involved("collision_started", *id),
            SoundTrigger::OnJump => involved("jump", *id),
            SoundTrigger::OnAction { action } => actions.just_pressed(action),
        };
        if fire {
            let (pos, _) = place(world, *e);
            let h = hear(ear.as_ref(), pos, [0.0; 3], s.volume, spatial.as_ref());
            let mut detail = json!({ "sound": s.sound, "gain": h.gain, "pan": h.pan, "looping": s.looping });
            if spatial.is_some() {
                detail["lowpass_hz"] = json!(h.lowpass_hz);
            }
            played.push((*id, detail));
        }
    }
    let mut log = world.resource_mut::<EventLog>();
    for (id, detail) in played {
        log.push(tick, "sound", Some(id), None, detail);
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
            .or_else(|| music::builtin_stem(name))
            .ok_or_else(|| {
                format!(
                    "unknown built-in sound `{name}`; built-ins are beep, jump, thud, music_pad, \
                 music_bass, music_drums, music_lead"
                )
            })?;
        return StaticSoundData::from_cursor(std::io::Cursor::new(wav_bytes(&samples, 44_100)))
            .map_err(|e| format!("cannot decode built-in sound: {e}"));
    }
    StaticSoundData::from_file(base_dir.join(sound)).map_err(|e| format!("cannot load sound {sound}: {e}"))
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

    fn ear(rotation: [f32; 4]) -> Ear {
        Ear {
            position: [0.0; 3],
            rotation,
            velocity: [0.0; 3],
            volume: 1.0,
        }
    }

    #[test]
    fn distance_curves() {
        let e = ear([0.0, 0.0, 0.0, 1.0]);
        let sp = SoundSpatial {
            min_distance: 2.0,
            max_distance: 10.0,
            rolloff: Rolloff::Inverse,
            rear_muffle: 0.0,
            doppler: 0.0,
        };
        let g = |d: f32, sp: &SoundSpatial| hear(Some(&e), [0.0, 0.0, -d], [0.0; 3], 1.0, Some(sp)).gain;
        assert_eq!(g(1.0, &sp), 1.0);
        assert_eq!(g(4.0, &sp), 0.5);
        assert_eq!(g(40.0, &sp), 0.2);
        let lin = SoundSpatial {
            rolloff: Rolloff::Linear,
            ..sp
        };
        assert_eq!(g(6.0, &lin), 0.5);
        assert_eq!(g(12.0, &lin), 0.0);
        let flat = SoundSpatial {
            rolloff: Rolloff::Constant,
            ..sp
        };
        assert_eq!(g(9.0, &flat), 1.0);
    }

    #[test]
    fn behind_is_muffled_and_turning_the_head_moves_the_sound() {
        let sp = SoundSpatial {
            rear_muffle: 1.0,
            ..SoundSpatial::default()
        };
        let front = ear([0.0, 0.0, 0.0, 1.0]);
        let h = hear(Some(&front), [0.0, 0.0, -5.0], [0.0; 3], 1.0, Some(&sp));
        assert_eq!((h.pan, h.lowpass_hz), (0.0, OPEN_HZ));
        let h = hear(Some(&front), [0.0, 0.0, 5.0], [0.0; 3], 1.0, Some(&sp));
        assert_eq!(h.lowpass_hz, 4_000.0);
        // Head turned 90 degrees left (about +Y): the sound ahead is now on the right.
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let left = ear([0.0, s, 0.0, s]);
        let h = hear(Some(&left), [0.0, 0.0, -5.0], [0.0; 3], 1.0, Some(&sp));
        assert!((h.pan - 1.0).abs() < 1e-5, "{h:?}");
        assert!((h.lowpass_hz - OPEN_HZ).abs() < 1.0);
        // Inside min_distance the image narrows instead of jumping sides.
        let h = hear(Some(&front), [0.25, 0.0, 0.0], [0.0; 3], 1.0, Some(&sp));
        assert!((h.pan - 0.25).abs() < 1e-5);
    }

    #[test]
    fn doppler_raises_pitch_when_approaching() {
        let sp = SoundSpatial {
            doppler: 1.0,
            ..SoundSpatial::default()
        };
        let e = ear([0.0, 0.0, 0.0, 1.0]);
        let toward = hear(Some(&e), [0.0, 0.0, -10.0], [0.0, 0.0, 34.3], 1.0, Some(&sp));
        assert!((toward.pitch - 1.0 / 0.9).abs() < 1e-4, "{toward:?}");
        let away = hear(Some(&e), [0.0, 0.0, -10.0], [0.0, 0.0, -34.3], 1.0, Some(&sp));
        assert!(away.pitch < 1.0);
        let off = hear(
            Some(&e),
            [0.0, 0.0, -10.0],
            [0.0, 0.0, 34.3],
            1.0,
            Some(&SoundSpatial::default()),
        );
        assert_eq!(off.pitch, 1.0);
    }

    #[test]
    fn music_stems_decode() {
        let d = load_sound("builtin:music_drums", Path::new(".")).unwrap();
        assert_eq!(d.frames.len(), 176_400);
    }
}
