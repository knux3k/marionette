//! Audio operations (decision 0075): steer layered music, and hear the mix as numbers. An
//! agent cannot listen, so `audio_mix` reports what the listener hears from every source
//! (gain, pan, low-pass, behind or not) and every music layer's level.

use crate::edit::{EditOutput, EditTarget, apply};
use crate::observe::{WorldTarget, with_world};
use crate::{Agent, Op, codes, op};
use marionette_audio::{AudioSource, Music, SoundSpatial, hear, listener_ear};
use marionette_core::scene::SceneId;
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "audio_music",
            "Steer an entity's layered Music: set `intensity` (layers whose `at` is at most it \
             fade in), `playing`, `volume` or `fade` seconds. Target: `scene` or `session`.",
            true,
            music,
        ),
        op(
            "audio_mix",
            "What the listener hears now (session) or after `ticks` (scene), as numbers: per \
             AudioSource its gain, pan (-1 left .. 1 right), low-pass hertz and whether it is \
             behind; per Music layer its level and gain. Sorted loudest first, paged by `limit`.",
            false,
            mix,
        ),
    ]
}

#[derive(Deserialize, JsonSchema)]
pub struct MusicInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub id: u64,
    pub intensity: Option<f32>,
    pub playing: Option<bool>,
    pub volume: Option<f32>,
    /// Seconds for a full layer fade.
    pub fade: Option<f32>,
}

fn music(agent: &mut Agent, input: MusicInput) -> Result<EditOutput, Errors> {
    let engine = agent.engine().clone();
    let registry = engine.registry();
    let entry = registry.get("Music").expect("the audio module registers Music");
    let id = input.id;
    let (target, diff) = apply(agent, &input.target, |scene, shown| {
        let e = scene.entities.iter_mut().find(|e| e.id == id).ok_or_else(|| {
            Errors::from(
                Diagnostic::error(codes::NO_ENTITY, format!("{shown} has no entity {id}")).entity(id),
            )
        })?;
        let v = e.components.get("Music").ok_or_else(|| {
            Errors::from(
                Diagnostic::error(codes::BAD_INPUT, format!("entity {id} has no Music"))
                    .entity(id)
                    .fix("add one with component_set (layers: sound, at, mix, level 0)"),
            )
        })?;
        let value = registry.from_json(entry, v).map_err(|m| {
            Errors::from(
                Diagnostic::error(codes::BAD_INPUT, format!("entity {id} has an invalid Music: {m}"))
                    .entity(id),
            )
        })?;
        let mut m = *value.into_any().downcast::<Music>().expect("registered as Music");
        if let Some(x) = input.intensity {
            m.intensity = x;
        }
        if let Some(x) = input.playing {
            m.playing = x;
        }
        if let Some(x) = input.volume {
            m.volume = x.max(0.0);
        }
        if let Some(x) = input.fade {
            m.fade = x.max(0.0);
        }
        e.components.insert("Music".into(), registry.to_json(&m));
        Ok(())
    })?;
    Ok(EditOutput { target, id, diff })
}

#[derive(Deserialize, JsonSchema)]
pub struct MixInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    /// Most sources to list. Default 20.
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct HeardSource {
    pub id: u64,
    pub sound: String,
    pub gain: f32,
    pub pan: f32,
    pub lowpass_hz: f32,
    pub behind: bool,
    /// Has `SoundSpatial` (otherwise the plain `1 / (1 + distance)` rule).
    pub spatial: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct HeardLayer {
    pub id: u64,
    pub layer: usize,
    pub sound: String,
    pub level: f32,
    pub gain: f32,
}

#[derive(Serialize, JsonSchema)]
pub struct MixOutput {
    pub tick: u64,
    /// False when the world has no AudioListener (sources are then heard at their volume).
    pub listener: bool,
    pub sources: Vec<HeardSource>,
    /// Sources left out by `limit`.
    pub more: usize,
    pub music: Vec<HeardLayer>,
}

fn mix(agent: &mut Agent, input: MixInput) -> Result<MixOutput, Errors> {
    with_world(agent, &input.target, |_, sim, _| {
        let world = &mut sim.world;
        let ear = listener_ear(world);
        let mut q = world.query::<(
            bevy_ecs::entity::Entity,
            &SceneId,
            &AudioSource,
            Option<&SoundSpatial>,
        )>();
        let found: Vec<(bevy_ecs::entity::Entity, u64, AudioSource, Option<SoundSpatial>)> = q
            .iter(world)
            .map(|(e, id, s, sp)| (e, id.0, s.clone(), sp.copied()))
            .collect();
        let mut sources: Vec<HeardSource> = found
            .into_iter()
            .map(|(e, id, s, sp)| {
                let t = world
                    .get::<marionette_core::spatial::Transform>(e)
                    .copied()
                    .unwrap_or_default();
                let pos = world
                    .get::<marionette_core::builtin::Position>(e)
                    .map_or([t.translation.x, t.translation.y, t.translation.z], |p| {
                        [p.x, p.y, p.z]
                    });
                let h = hear(ear.as_ref(), pos, [0.0; 3], s.volume, sp.as_ref());
                let behind = ear.is_some_and(|ear| {
                    let probe = SoundSpatial {
                        rear_muffle: 1.0,
                        ..SoundSpatial::default()
                    };
                    hear(Some(&ear), pos, [0.0; 3], 1.0, Some(&probe)).lowpass_hz < marionette_audio::OPEN_HZ
                });
                HeardSource {
                    id,
                    sound: s.sound,
                    gain: h.gain,
                    pan: h.pan,
                    lowpass_hz: h.lowpass_hz,
                    behind,
                    spatial: sp.is_some(),
                }
            })
            .collect();
        sources.sort_by(|a, b| b.gain.total_cmp(&a.gain).then(a.id.cmp(&b.id)));
        let limit = input.limit.unwrap_or(20);
        let more = sources.len().saturating_sub(limit);
        sources.truncate(limit);
        let mut mq = world.query::<(&SceneId, &Music)>();
        let mut music: Vec<HeardLayer> = mq
            .iter(world)
            .flat_map(|(id, m)| {
                m.layers
                    .iter()
                    .enumerate()
                    .map(|(i, l)| HeardLayer {
                        id: id.0,
                        layer: i,
                        sound: l.sound.clone(),
                        level: l.level,
                        gain: m.gain(i),
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        music.sort_by_key(|l| (l.id, l.layer));
        Ok(MixOutput {
            tick: sim.tick(),
            listener: ear.is_some(),
            sources,
            more,
            music,
        })
    })
}
