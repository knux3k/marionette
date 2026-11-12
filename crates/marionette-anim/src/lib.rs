//! Marionette skeletal animation.
//!
//! An `Animator` component plays clips from the entity's glTF `Model`. Each tick (in
//! [`Stage::Effects`], after logic and physics) the module advances every layer by the fixed
//! step, moves layer weights toward their targets (cross-fades), samples and blends the clips,
//! and writes a [`Pose`] (one model-space matrix per glTF node). The renderer skins meshes
//! with that pose on the GPU.
//!
//! Determinism: layer times and weights live in the `Animator` component, so they are saved,
//! hashed and restored with the scene. The pose is recomputed from them with plain
//! arithmetic (no `sin`/`cos`/`slerp`), on one thread, in scene-id order. Model files are
//! read only in [`Module::init`], never during a tick.

pub mod sample;

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_assets::rig::{Rig, load_rig};
use marionette_core::builtin::{DEFAULT_DT, FixedStep, SimClock};
use marionette_core::module::{EventLog, Module, SceneDir, Stage};
use marionette_core::scene::SceneId;
use marionette_core::spatial::{Model, Pose};
use marionette_core::{Diagnostic, Errors, Registry, Result};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

pub mod codes {
    /// The model file of an animated entity cannot be read.
    pub const ASSET: &str = "anim.asset";
    /// A layer names a clip the model does not have.
    pub const UNKNOWN_CLIP: &str = "anim.unknown_clip";
    /// The entity has an `Animator` but no `Model`.
    pub const NO_MODEL: &str = "anim.no_model";
}

/// One clip playing on an `Animator`.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct AnimLayer {
    /// Clip name in the model file; `#0`, `#1`, ... pick unnamed clips by position.
    pub clip: String,
    /// Seconds into the clip.
    pub time: f32,
    /// Playback rate: 1 is normal speed, 0 holds, negative plays backwards.
    pub speed: f32,
    /// Wraps around at the end; otherwise holds the last frame (and logs `anim_finished`).
    pub looping: bool,
    /// Blend weight now (0..1). Layer poses are averaged by weight.
    pub weight: f32,
    /// Weight this layer moves toward. A layer whose weight and target are both 0 is removed.
    pub target_weight: f32,
    /// Seconds a full 0 to 1 weight change takes (0 = immediate).
    pub fade: f32,
}

/// Plays skeletal animation clips of the entity's `Model` (a glTF file with animations),
/// blended by layer weight. Change it through the `anim_play`, `anim_blend` and `anim_stop`
/// operations or by editing the layers directly. An empty list shows the rest pose.
#[derive(Component, Reflect, Debug, Clone, Default, PartialEq)]
pub struct Animator {
    pub layers: Vec<AnimLayer>,
}

impl Animator {
    /// Plays `clip` from the start, fading every other layer out over `fade` seconds. If
    /// `clip` is already playing it keeps its time and fades back to full weight.
    pub fn play(&mut self, clip: &str, fade: f32, speed: f32, looping: bool) {
        let fade = fade.max(0.0);
        for l in &mut self.layers {
            l.target_weight = 0.0;
            l.fade = fade;
        }
        match self.layers.iter_mut().find(|l| l.clip == clip) {
            Some(l) => {
                l.target_weight = 1.0;
                l.speed = speed;
                l.looping = looping;
            }
            None => self.layers.push(AnimLayer {
                clip: clip.to_string(),
                time: 0.0,
                speed,
                looping,
                weight: if fade > 0.0 && !self.layers.is_empty() {
                    0.0
                } else {
                    1.0
                },
                target_weight: 1.0,
                fade,
            }),
        }
        if fade == 0.0 {
            for l in &mut self.layers {
                l.weight = l.target_weight;
            }
            self.layers.retain(|l| l.weight > 0.0);
        }
    }

    /// Sets the target weight of `clip` (adding it, from time 0, if missing) without
    /// touching other layers: for mixing, e.g. 70 % walk and 30 % run.
    pub fn blend(&mut self, clip: &str, weight: f32, fade: f32, speed: f32, looping: bool) {
        let weight = weight.max(0.0);
        match self.layers.iter_mut().find(|l| l.clip == clip) {
            Some(l) => {
                l.target_weight = weight;
                l.fade = fade.max(0.0);
                l.speed = speed;
                l.looping = looping;
                if fade <= 0.0 {
                    l.weight = weight;
                }
            }
            None => self.layers.push(AnimLayer {
                clip: clip.to_string(),
                time: 0.0,
                speed,
                looping,
                weight: if fade > 0.0 { 0.0 } else { weight },
                target_weight: weight,
                fade: fade.max(0.0),
            }),
        }
        self.layers.retain(|l| l.weight > 0.0 || l.target_weight > 0.0);
    }

    /// Fades out `clip` (or every layer when `None`) over `fade` seconds.
    pub fn stop(&mut self, clip: Option<&str>, fade: f32) {
        for l in &mut self.layers {
            if clip.is_none_or(|c| c == l.clip) {
                l.target_weight = 0.0;
                l.fade = fade.max(0.0);
                if fade <= 0.0 {
                    l.weight = 0.0;
                }
            }
        }
        self.layers.retain(|l| l.weight > 0.0 || l.target_weight > 0.0);
    }
}

/// Rigs loaded in [`Module::init`], keyed by model asset string. Runtime state, not scene
/// data: rebuilt from files whenever a scene is loaded, reloaded or restored.
#[derive(Resource, Default)]
pub struct Rigs(pub BTreeMap<String, Arc<Rig>>);

pub struct AnimModule;

impl Module for AnimModule {
    fn name(&self) -> &'static str {
        "anim"
    }

    fn register(&self, r: &mut Registry) {
        r.register_component::<Animator>();
    }

    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(animate.in_set(Stage::Effects));
    }

    fn init(&self, world: &mut World) -> Result<()> {
        let dir = world
            .get_resource::<SceneDir>()
            .map(|d| d.0.clone())
            .unwrap_or_default();
        let mut errors = Vec::new();
        let mut rigs: BTreeMap<String, Arc<Rig>> = BTreeMap::new();
        for (id, entity) in animated(world) {
            let Some(model) = world.get::<Model>(entity) else {
                errors.push(
                    Diagnostic::error(
                        codes::NO_MODEL,
                        format!("entity {id} has an Animator but no Model"),
                    )
                    .entity(id)
                    .component("Animator")
                    .fix("add a Model whose asset is a glTF file with animations"),
                );
                continue;
            };
            let asset = model.asset.clone();
            if !rigs.contains_key(&asset) {
                match load_rig(&asset, &dir) {
                    Ok(rig) => {
                        rigs.insert(asset.clone(), Arc::new(rig));
                    }
                    Err(msg) => {
                        errors.push(
                            Diagnostic::error(codes::ASSET, msg)
                                .entity(id)
                                .component("Model")
                                .fix("check the path (relative to the scene file)"),
                        );
                        continue;
                    }
                }
            }
            let rig = &rigs[&asset];
            if let Some(animator) = world.get::<Animator>(entity) {
                for (i, layer) in animator.layers.iter().enumerate() {
                    if rig.clip(&layer.clip).is_none() {
                        let names: Vec<String> = rig.clips.iter().map(|c| c.name.clone()).collect();
                        errors.push(
                            Diagnostic::error(
                                codes::UNKNOWN_CLIP,
                                format!("`{asset}` has no clip `{}`", layer.clip),
                            )
                            .entity(id)
                            .component("Animator")
                            .field(format!("layers/{i}/clip"))
                            .fix(if names.is_empty() {
                                "this model has no animation clips".to_string()
                            } else {
                                format!("clips in this model: {}", names.join(", "))
                            }),
                        );
                    }
                }
            }
        }
        if !errors.is_empty() {
            return Err(Errors(errors));
        }
        world.insert_resource(Rigs(rigs));
        // Pose straight away, so a loaded or restored scene renders posed before its first tick.
        write_poses(world, None);
        Ok(())
    }
}

/// Entities with an `Animator`, sorted by scene id.
fn animated(world: &mut World) -> Vec<(u64, Entity)> {
    let mut q = world.query_filtered::<(Entity, &SceneId), With<Animator>>();
    let mut v: Vec<(u64, Entity)> = q.iter(world).map(|(e, id)| (id.0, e)).collect();
    v.sort_by_key(|(id, _)| *id);
    v
}

/// Advances one layer by `dt`. Returns true when a non-looping clip reaches its end now.
fn advance(layer: &mut AnimLayer, duration: f32, dt: f32) -> bool {
    let before = layer.time;
    layer.time += dt * layer.speed;
    let mut finished = false;
    if duration > 0.0 {
        if layer.looping {
            if layer.time >= duration || layer.time < 0.0 {
                layer.time = layer.time.rem_euclid(duration);
            }
        } else if layer.time >= duration {
            layer.time = duration;
            finished = before < duration;
        } else if layer.time <= 0.0 {
            layer.time = 0.0;
            finished = before > 0.0 && layer.speed < 0.0;
        }
    } else {
        layer.time = 0.0;
    }
    if layer.weight != layer.target_weight {
        if layer.fade <= 0.0 {
            layer.weight = layer.target_weight;
        } else {
            let step = dt / layer.fade;
            layer.weight = if layer.weight < layer.target_weight {
                (layer.weight + step).min(layer.target_weight)
            } else {
                (layer.weight - step).max(layer.target_weight)
            };
        }
    }
    finished
}

/// The animation step: advance layers, then pose every animated entity.
fn animate(world: &mut World) {
    let dt = world.get_resource::<FixedStep>().map_or(DEFAULT_DT, |s| s.dt);
    write_poses(world, Some(dt));
}

/// Poses every animated entity; with `dt`, advances its layers first.
fn write_poses(world: &mut World, dt: Option<f32>) {
    let Some(rigs) = world.get_resource::<Rigs>().map(|r| r.0.clone()) else {
        return;
    };
    let tick = world.get_resource::<SimClock>().map_or(0, |c| c.tick);
    let mut finished: Vec<(u64, String)> = Vec::new();
    for (id, entity) in animated(world) {
        let Some(rig) = world
            .get::<Model>(entity)
            .and_then(|m| rigs.get(&m.asset))
            .cloned()
        else {
            continue;
        };
        let Some(mut animator) = world.get::<Animator>(entity).cloned() else {
            continue;
        };
        if let Some(dt) = dt {
            for layer in &mut animator.layers {
                let duration = rig.clip(&layer.clip).map_or(0.0, |c| c.duration);
                if advance(layer, duration, dt) {
                    finished.push((id, layer.clip.clone()));
                }
            }
            animator
                .layers
                .retain(|l| l.weight > 0.0 || l.target_weight > 0.0);
            if world.get::<Animator>(entity) != Some(&animator) {
                world.entity_mut(entity).insert(animator.clone());
            }
        }
        let layers: Vec<(&marionette_assets::rig::Clip, f32, f32)> = animator
            .layers
            .iter()
            .filter_map(|l| rig.clip(&l.clip).map(|c| (c, l.time, l.weight)))
            .collect();
        let local = sample::blend(&rig, &layers);
        let pose = Pose {
            nodes: local.model_space(&rig),
            morph_weights: local.morph_weights,
        };
        world.entity_mut(entity).insert(pose);
    }
    if !finished.is_empty()
        && let Some(mut log) = world.get_resource_mut::<EventLog>()
    {
        for (id, clip) in finished {
            log.push(tick, "anim_finished", Some(id), None, json!({ "clip": clip }));
        }
    }
}

/// Model-space matrix (column-major) of the node called `node` in `entity`'s current pose,
/// or of the rest pose if the entity is not animated.
pub fn node_matrix(world: &World, entity: Entity, node: &str) -> Option<[f32; 16]> {
    let model = world.get::<Model>(entity)?;
    let rig = world.get_resource::<Rigs>()?.0.get(&model.asset)?.clone();
    let i = rig.node(node)?;
    match world.get::<Pose>(entity) {
        Some(p) => p.nodes.get(i).copied(),
        None => rig.rest_pose().get(i).copied(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(clip: &str, weight: f32) -> AnimLayer {
        AnimLayer {
            clip: clip.into(),
            time: 0.0,
            speed: 1.0,
            looping: true,
            weight,
            target_weight: weight,
            fade: 0.0,
        }
    }

    #[test]
    fn play_cross_fades_and_removes_faded_layers() {
        let mut a = Animator {
            layers: vec![layer("walk", 1.0)],
        };
        a.play("wave", 0.5, 1.0, true);
        assert_eq!(a.layers.len(), 2);
        assert_eq!((a.layers[0].target_weight, a.layers[1].weight), (0.0, 0.0));
        for _ in 0..30 {
            for l in &mut a.layers {
                advance(l, 1.0, 1.0 / 60.0);
            }
        }
        assert_eq!(a.layers[0].weight, 0.0);
        assert_eq!(a.layers[1].weight, 1.0);
        a.stop(Some("walk"), 0.0);
        assert_eq!(a.layers.len(), 1);
        a.play("idle", 0.0, 1.0, false);
        assert_eq!(
            a.layers,
            vec![layer("idle", 1.0)]
                .into_iter()
                .map(|mut l| {
                    l.looping = false;
                    l
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn looping_wraps_and_one_shot_finishes_once() {
        let mut l = layer("x", 1.0);
        l.time = 0.99;
        assert!(!advance(&mut l, 1.0, 0.02));
        assert!((l.time - 0.01).abs() < 1e-6);
        l.looping = false;
        l.time = 0.99;
        assert!(advance(&mut l, 1.0, 0.02));
        assert_eq!(l.time, 1.0);
        assert!(!advance(&mut l, 1.0, 0.02));
    }
}
