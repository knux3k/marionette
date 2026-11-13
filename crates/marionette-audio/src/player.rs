//! Windowed-mode playback through kira. The simulation decides what starts (the `sound`
//! log); the player keeps one voice per playing sound and, every frame, re-hears it from the
//! current listener (or a tracked head pose) so sounds follow moving sources and turning
//! heads between fixed ticks. Music stems start on one clock tick, so they stay in sync.

use crate::music::Music;
use crate::{AudioSource, Ear, Heard, OPEN_HZ, SoundSpatial, hear, listener_ear, load_sound, place};
use bevy_ecs::prelude::*;
use kira::clock::{ClockHandle, ClockSpeed, ClockTime};
use kira::effect::filter::{FilterBuilder, FilterHandle};
use kira::sound::PlaybackState;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{Decibels, Panning, PlaybackRate, StartTime, Tween};
use marionette_core::module::EventLog;
use marionette_core::scene::SceneId;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// Most voices followed at once (bounded, MEMORY-AND-CRASHES.md). Past it, new one-shots
/// still play but keep the gain and pan they started with.
pub const MAX_VOICES: usize = 48;
/// Most `Music` entities playing at once (kira has 8 clocks).
const MAX_MUSIC: usize = 4;
/// Parameter changes glide over this long, so per-frame updates do not crackle.
const GLIDE: Duration = Duration::from_millis(30);

fn db(gain: f32) -> Decibels {
    if gain > 1e-3 {
        Decibels(20.0 * gain.log10())
    } else {
        Decibels::SILENCE
    }
}

fn glide() -> Tween {
    Tween {
        duration: GLIDE,
        ..Default::default()
    }
}

struct Voice {
    id: u64,
    sound: String,
    looping: bool,
    handle: StaticSoundHandle,
    /// Per-voice track with a low-pass, for sources with `rear_muffle`.
    filter: Option<(TrackHandle, FilterHandle)>,
    last_position: Option<[f32; 3]>,
}

struct MusicVoice {
    sounds: Vec<String>,
    stems: Vec<StaticSoundHandle>,
    _clock: ClockHandle,
}

/// Plays `sound` events and `Music` through the default audio device (windowed mode).
pub struct Player {
    manager: kira::AudioManager<kira::DefaultBackend>,
    cache: BTreeMap<String, StaticSoundData>,
    base_dir: PathBuf,
    seen: u64,
    voices: Vec<Voice>,
    music: BTreeMap<u64, MusicVoice>,
    head: Option<([f32; 3], [f32; 4])>,
    last_ear: Option<[f32; 3]>,
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
            voices: Vec::new(),
            music: BTreeMap::new(),
            head: None,
            last_ear: None,
        })
    }

    /// The tracked head pose in world space (position, rotation x y z w), set every frame by
    /// a headset module. While set it replaces the `AudioListener` entity's pose (its
    /// `volume` still applies). `None` goes back to the entity.
    pub fn set_head(&mut self, head: Option<([f32; 3], [f32; 4])>) {
        self.head = head;
    }

    /// Voices followed now.
    pub fn voices(&self) -> usize {
        self.voices.len()
    }

    fn data(&mut self, sound: &str) -> Result<StaticSoundData, String> {
        if let Some(d) = self.cache.get(sound) {
            return Ok(d.clone());
        }
        let d = load_sound(sound, &self.base_dir)?;
        self.cache.insert(sound.to_string(), d.clone());
        Ok(d)
    }

    /// Plays every `sound` event added since the last call (without moving existing voices;
    /// see [`Player::update`]). Problems are returned, not fatal.
    pub fn play_new(&mut self, log: &EventLog) -> Vec<String> {
        let mut problems = Vec::new();
        let new: Vec<(Option<u64>, serde_json::Value)> = log
            .since(self.seen)
            .filter(|e| e.kind == "sound")
            .map(|e| (e.entity, e.detail.clone()))
            .collect();
        self.seen = log.end();
        for (entity, detail) in new {
            let Some(sound) = detail["sound"].as_str() else {
                continue;
            };
            let base = match self.data(sound) {
                Ok(d) => d,
                Err(m) => {
                    problems.push(m);
                    continue;
                }
            };
            let looping = detail["looping"] == true;
            let heard = Heard {
                gain: detail["gain"].as_f64().unwrap_or(1.0) as f32,
                pan: detail["pan"].as_f64().unwrap_or(0.0) as f32,
                lowpass_hz: detail["lowpass_hz"].as_f64().unwrap_or(OPEN_HZ as f64) as f32,
                pitch: 1.0,
            };
            let mut data = base.volume(db(heard.gain)).panning(Panning(heard.pan));
            if looping {
                data = data.loop_region(..);
            }
            let follow = entity.is_some() && self.voices.len() < MAX_VOICES;
            let filtered = follow && detail.get("lowpass_hz").is_some();
            let result = if filtered {
                let mut b = TrackBuilder::new().persist_until_sounds_finish(true);
                let fh = b.add_effect(FilterBuilder::new().cutoff(heard.lowpass_hz as f64));
                match self.manager.add_sub_track(b) {
                    Ok(mut track) => track
                        .play(data)
                        .map(|h| (h, Some((track, fh))))
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            } else {
                self.manager
                    .play(data)
                    .map(|h| (h, None))
                    .map_err(|e| e.to_string())
            };
            match result {
                Ok((handle, filter)) => {
                    if follow && let Some(id) = entity {
                        self.voices.push(Voice {
                            id,
                            sound: sound.to_string(),
                            looping,
                            handle,
                            filter,
                            last_position: None,
                        });
                    }
                }
                Err(err) => problems.push(format!("cannot play {sound}: {err}")),
            }
        }
        problems
    }

    /// Follows the world: re-hears every voice from the listener (or tracked head) now,
    /// stops looping voices whose entity or `AudioSource` is gone, and plays `Music`.
    /// `dt` is the real seconds since the last call (for Doppler velocities).
    pub fn update(&mut self, world: &mut World, dt: f32) -> Vec<String> {
        let mut problems = Vec::new();
        let mut ear = listener_ear(world);
        if let Some((position, rotation)) = self.head {
            let volume = ear.map_or(1.0, |e| e.volume);
            ear = Some(Ear {
                position,
                rotation,
                velocity: [0.0; 3],
                volume,
            });
        }
        if let Some(e) = ear.as_mut() {
            if let Some(last) = self.last_ear
                && dt > 0.0
            {
                e.velocity = [
                    (e.position[0] - last[0]) / dt,
                    (e.position[1] - last[1]) / dt,
                    (e.position[2] - last[2]) / dt,
                ];
            }
            self.last_ear = Some(e.position);
        }
        let mut q = world.query::<(Entity, &SceneId, &AudioSource, Option<&SoundSpatial>)>();
        let sources: BTreeMap<u64, (Entity, AudioSource, Option<SoundSpatial>)> = q
            .iter(world)
            .map(|(e, id, s, sp)| (id.0, (e, s.clone(), sp.copied())))
            .collect();
        let mut keep = Vec::with_capacity(self.voices.len());
        for mut v in std::mem::take(&mut self.voices) {
            if v.handle.state() == PlaybackState::Stopped {
                continue;
            }
            let Some((entity, source, spatial)) = sources.get(&v.id).filter(|(_, s, _)| s.sound == v.sound)
            else {
                if v.looping {
                    v.handle.stop(glide());
                }
                // One-shots finish where they were.
                continue;
            };
            let (pos, _) = place(world, *entity);
            let velocity = match (v.last_position, dt > 0.0) {
                (Some(l), true) => [(pos[0] - l[0]) / dt, (pos[1] - l[1]) / dt, (pos[2] - l[2]) / dt],
                _ => [0.0; 3],
            };
            v.last_position = Some(pos);
            let h = hear(ear.as_ref(), pos, velocity, source.volume, spatial.as_ref());
            v.handle.set_volume(db(h.gain), glide());
            v.handle.set_panning(Panning(h.pan), glide());
            v.handle.set_playback_rate(PlaybackRate(h.pitch as f64), glide());
            if let Some((_, f)) = v.filter.as_mut() {
                f.set_cutoff(h.lowpass_hz as f64, glide());
            }
            keep.push(v);
        }
        self.voices = keep;
        self.update_music(world, &mut problems);
        problems
    }

    fn update_music(&mut self, world: &mut World, problems: &mut Vec<String>) {
        let mut q = world.query::<(&SceneId, &Music)>();
        let all: BTreeMap<u64, Music> = q.iter(world).map(|(id, m)| (id.0, m.clone())).collect();
        // Stop music whose entity is gone, whose stems changed, or that fell silent.
        let ids: Vec<u64> = self.music.keys().copied().collect();
        for id in ids {
            let stop = match all.get(&id) {
                None => true,
                Some(m) => {
                    let sounds: Vec<&String> = m.layers.iter().map(|l| &l.sound).collect();
                    sounds != self.music[&id].sounds.iter().collect::<Vec<_>>()
                        || (!m.playing && m.layers.iter().all(|l| l.level == 0.0))
                }
            };
            if stop && let Some(mut mv) = self.music.remove(&id) {
                for s in &mut mv.stems {
                    s.stop(glide());
                }
            }
        }
        for (id, m) in &all {
            if let Some(mv) = self.music.get_mut(id) {
                for (i, s) in mv.stems.iter_mut().enumerate() {
                    s.set_volume(db(m.gain(i)), glide());
                }
                continue;
            }
            if !m.playing || self.music.len() >= MAX_MUSIC {
                continue;
            }
            match self.start_music(m) {
                Ok(mv) => {
                    self.music.insert(*id, mv);
                }
                Err(e) => {
                    if !problems.contains(&e) {
                        problems.push(e);
                    }
                }
            }
        }
    }

    fn start_music(&mut self, m: &Music) -> Result<MusicVoice, String> {
        let mut clock = self
            .manager
            .add_clock(ClockSpeed::TicksPerSecond(100.0))
            .map_err(|e| format!("cannot start music: {e}"))?;
        let start = StartTime::ClockTime(ClockTime {
            clock: clock.id(),
            ticks: 1,
            fraction: 0.0,
        });
        let mut stems = Vec::new();
        for (i, l) in m.layers.iter().enumerate() {
            let data = self
                .data(&l.sound)?
                .loop_region(..)
                .volume(db(m.gain(i)))
                .start_time(start);
            stems.push(
                self.manager
                    .play(data)
                    .map_err(|e| format!("cannot play {}: {e}", l.sound))?,
            );
        }
        clock.start();
        Ok(MusicVoice {
            sounds: m.layers.iter().map(|l| l.sound.clone()).collect(),
            stems,
            _clock: clock,
        })
    }
}
