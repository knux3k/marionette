//! Animation operations: list a model's clips, play, blend and stop clips on an entity's
//! `Animator` (scene file or live session, through the same validated edit path as every
//! other edit), and read where a bone is.

use crate::edit::{EditOutput, EditTarget, apply};
use crate::observe::{WorldTarget, with_world};
use crate::{Agent, Op, codes, op};
use marionette_anim::{AnimOverlays, Animator, BlendPoint, BlendSpaces};
use marionette_assets::glam::{Mat4, Quat, Vec3};
use marionette_assets::rig::{Property, Rig, load_rig};
use marionette_core::Registry;
use marionette_core::scene::{Scene, SceneEntity};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "anim_clips",
            "List the animation clips (name, seconds, animated nodes), nodes, skins and morph \
             targets of a model: `asset` (a glTF path relative to the project root) or the \
             `Model` of entity `id` in `scene`.",
            false,
            clips,
        ),
        op(
            "anim_play",
            "Play a clip on an entity's Animator (added if missing), cross-fading every other \
             clip out over `fade` seconds. Target: `scene` (file) or `session` (live world).",
            true,
            play,
        ),
        op(
            "anim_blend",
            "Set one clip's blend weight (fading over `fade` seconds) without touching other \
             clips, e.g. walk 0.7 + run 0.3. Adds the clip if it is not playing.",
            true,
            blend,
        ),
        op(
            "anim_stop",
            "Fade out one clip, or every clip when `clip` is omitted, over `fade` seconds.",
            true,
            stop,
        ),
        op(
            "anim_space",
            "Define or steer a blend space: clips mixed by parameter `x` (and `y`), e.g. idle at \
             0, walk at 1.5, run at 4 by speed. `points` (re)defines it; `x`/`y` set where it \
             goes (gliding at `rate` units/s). Play it with anim_play using its `name` as clip.",
            true,
            space,
        ),
        op(
            "anim_overlay",
            "Play a clip on a named overlay over the base pose: limited to `mask` bones (with \
             their children), replacing them or `additive`, at strength `weight` (fading over \
             `fade`). E.g. a wave on the upper body while walking. `stop: true` fades it out.",
            true,
            overlay,
        ),
        op(
            "anim_pose",
            "World position and rotation of an animated entity's nodes (bones) after `ticks` \
             (scene) or now (session). `nodes` limits the list.",
            false,
            pose,
        ),
    ]
}

pub mod anim_codes {
    pub const NO_MODEL: &str = "anim.no_model";
    pub const UNKNOWN_CLIP: &str = "anim.unknown_clip";
    pub const ASSET: &str = "anim.asset";
    pub const UNKNOWN_NODE: &str = "anim.unknown_node";
}

#[derive(Deserialize, JsonSchema)]
pub struct ClipsInput {
    /// glTF file relative to the project root.
    pub asset: Option<String>,
    /// Scene file whose entity `id` has the `Model` to inspect (instead of `asset`).
    pub scene: Option<String>,
    pub id: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct ClipInfo {
    pub name: String,
    pub seconds: f32,
    /// Names of the nodes this clip moves.
    pub nodes: Vec<String>,
    /// True when the clip animates morph target (blend shape) weights.
    pub morph: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct SkinInfo {
    pub name: String,
    pub joints: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ClipsOutput {
    pub asset: String,
    pub clips: Vec<ClipInfo>,
    pub nodes: Vec<String>,
    pub skins: Vec<SkinInfo>,
    /// Nodes with morph targets and how many each has.
    pub morph_targets: Vec<(String, usize)>,
}

fn rig_error(asset: &str, msg: String) -> Errors {
    Diagnostic::error(anim_codes::ASSET, msg)
        .fix(format!(
            "check that `{asset}` is a glTF/GLB file (relative to the scene file)"
        ))
        .into()
}

fn clips(agent: &mut Agent, input: ClipsInput) -> Result<ClipsOutput, Errors> {
    let (asset, dir) = match (&input.asset, &input.scene, input.id) {
        (Some(a), None, None) => (a.clone(), agent.resolve(".")?),
        (None, Some(scene), Some(id)) => {
            let path = agent.resolve(scene)?;
            let parsed = marionette_core::scene::parse(&crate::read_text(&path, scene)?)
                .map_err(|e| e.with_file(scene))?;
            (model_of(&parsed, id, scene)?, parent(&path))
        }
        _ => {
            return Err(
                Diagnostic::error(codes::TARGET, "anim_clips needs `asset`, or `scene` and `id`")
                    .fix("pass a glTF path as `asset`, or a scene file and entity id")
                    .into(),
            );
        }
    };
    let rig = load_rig(&asset, &dir).map_err(|m| rig_error(&asset, m))?;
    Ok(ClipsOutput {
        clips: rig
            .clips
            .iter()
            .map(|c| {
                let mut nodes: Vec<String> = c
                    .channels
                    .iter()
                    .map(|ch| rig.nodes[ch.node].name.clone())
                    .collect();
                nodes.dedup();
                ClipInfo {
                    name: c.name.clone(),
                    seconds: c.duration,
                    nodes,
                    morph: c.channels.iter().any(|ch| ch.property == Property::Weights),
                }
            })
            .collect(),
        nodes: rig.nodes.iter().map(|n| n.name.clone()).collect(),
        skins: rig
            .skins
            .iter()
            .map(|s| SkinInfo {
                name: s.name.clone(),
                joints: s
                    .joints
                    .iter()
                    .map(|&j| {
                        rig.nodes
                            .get(j)
                            .map_or_else(|| format!("#{j}"), |n| n.name.clone())
                    })
                    .collect(),
            })
            .collect(),
        morph_targets: rig
            .morph_weights
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.is_empty())
            .map(|(i, w)| (rig.nodes[i].name.clone(), w.len()))
            .collect(),
        asset,
    })
}

fn parent(path: &Path) -> PathBuf {
    path.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// The `Model` asset of entity `id` in `scene`.
fn model_of(scene: &Scene, id: u64, shown: &str) -> Result<String, Errors> {
    let e = scene.entities.iter().find(|e| e.id == id).ok_or_else(|| {
        Errors::from(Diagnostic::error(codes::NO_ENTITY, format!("{shown} has no entity {id}")).entity(id))
    })?;
    e.components
        .get("Model")
        .and_then(|m| m.get("asset"))
        .and_then(|a| a.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            Diagnostic::error(anim_codes::NO_MODEL, format!("entity {id} has no Model"))
                .entity(id)
                .fix("add a Model whose asset is a glTF file with animations")
                .into()
        })
}

/// The directory relative model paths of an edit target resolve against.
fn target_dir(agent: &mut Agent, target: &EditTarget) -> Result<PathBuf, Errors> {
    let scene = match (&target.scene, &target.session) {
        (Some(file), None) => file.clone(),
        (None, Some(id)) => crate::session::get_mut(agent, id)?.scene.clone(),
        _ => {
            return Err(Diagnostic::error(
                codes::TARGET,
                "an edit needs exactly one of `scene` and `session`",
            )
            .into());
        }
    };
    Ok(parent(&agent.resolve(&scene)?))
}

#[derive(Deserialize, JsonSchema)]
pub struct PlayInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub id: u64,
    /// Clip name (see anim_clips); `#0` is the first clip.
    pub clip: String,
    /// Cross-fade seconds. Default 0.2.
    pub fade: Option<f32>,
    /// Playback rate. Default 1.
    pub speed: Option<f32>,
    /// Default true.
    pub looping: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BlendInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub id: u64,
    pub clip: String,
    /// Target weight (0 fades it out).
    pub weight: f32,
    /// Seconds for a full 0 to 1 change. Default 0.2.
    pub fade: Option<f32>,
    /// Default 1.
    pub speed: Option<f32>,
    /// Default true.
    pub looping: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct StopInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub id: u64,
    /// Clip to stop; all clips when omitted.
    pub clip: Option<String>,
    /// Fade-out seconds. Default 0.2.
    pub fade: Option<f32>,
}

/// Entity `id`'s component `name` as `T` (default when absent).
fn read_component<T: bevy_reflect::Reflect + Default>(
    registry: &Registry,
    e: &SceneEntity,
    name: &str,
) -> Result<T, Errors> {
    let id = e.id;
    let Some(v) = e.components.get(name) else {
        return Ok(T::default());
    };
    let entry = registry
        .get(name)
        .expect("the anim module registers its components");
    let value = registry.from_json(entry, v).map_err(|m| {
        Errors::from(
            Diagnostic::error(
                codes::BAD_INPUT,
                format!("entity {id} has an invalid {name}: {m}"),
            )
            .entity(id),
        )
    })?;
    Ok(*value
        .into_any()
        .downcast::<T>()
        .expect("registered under this name"))
}

/// Fails with `anim.unknown_clip` unless the model (or a blend space) provides `clip`.
fn check_clip(rig: &Rig, spaces: &BlendSpaces, asset: &str, id: u64, clip: &str) -> Result<(), Errors> {
    match marionette_anim::blend::missing_clip(rig, Some(spaces), clip) {
        None => Ok(()),
        Some(missing) => Err(Diagnostic::error(
            anim_codes::UNKNOWN_CLIP,
            format!("`{asset}` has no clip `{missing}`"),
        )
        .entity(id)
        .fix(marionette_anim::clip_names_fix(rig))
        .into()),
    }
}

/// Edits entity `id`'s Animator with `change`, after checking `clip` exists in its model.
fn edit_animator(
    agent: &mut Agent,
    target: &EditTarget,
    id: u64,
    clip: Option<&str>,
    change: impl Fn(&mut Animator),
) -> Result<EditOutput, Errors> {
    let dir = target_dir(agent, target)?;
    let engine = agent.engine().clone();
    let registry = engine.registry();
    let entry = registry
        .get("Animator")
        .expect("the anim module registers Animator");
    let (target_name, diff) = apply(agent, target, |scene, shown| {
        let asset = model_of(scene, id, shown)?;
        if let Some(clip) = clip {
            let rig: Rig = load_rig(&asset, &dir).map_err(|m| rig_error(&asset, m))?;
            let e = scene
                .entities
                .iter()
                .find(|e| e.id == id)
                .expect("model_of found it");
            let spaces: BlendSpaces = read_component(registry, e, "BlendSpaces")?;
            check_clip(&rig, &spaces, &asset, id, clip)?;
        }
        let e = scene
            .entities
            .iter_mut()
            .find(|e| e.id == id)
            .expect("model_of found it");
        let mut animator = match e.components.get("Animator") {
            Some(v) => {
                let value = registry.from_json(entry, v).map_err(|m| {
                    Errors::from(
                        Diagnostic::error(
                            codes::BAD_INPUT,
                            format!("entity {id} has an invalid Animator: {m}"),
                        )
                        .entity(id),
                    )
                })?;
                *value
                    .into_any()
                    .downcast::<Animator>()
                    .expect("registered as Animator")
            }
            None => Animator::default(),
        };
        change(&mut animator);
        e.components
            .insert("Animator".into(), registry.to_json(&animator));
        Ok(())
    })?;
    Ok(EditOutput {
        target: target_name,
        id,
        diff,
    })
}

fn play(agent: &mut Agent, input: PlayInput) -> Result<EditOutput, Errors> {
    edit_animator(agent, &input.target, input.id, Some(&input.clip), |a| {
        a.play(
            &input.clip,
            input.fade.unwrap_or(0.2),
            input.speed.unwrap_or(1.0),
            input.looping.unwrap_or(true),
        )
    })
}

fn blend(agent: &mut Agent, input: BlendInput) -> Result<EditOutput, Errors> {
    edit_animator(agent, &input.target, input.id, Some(&input.clip), |a| {
        a.blend(
            &input.clip,
            input.weight,
            input.fade.unwrap_or(0.2),
            input.speed.unwrap_or(1.0),
            input.looping.unwrap_or(true),
        )
    })
}

fn stop(agent: &mut Agent, input: StopInput) -> Result<EditOutput, Errors> {
    edit_animator(agent, &input.target, input.id, None, |a| {
        a.stop(input.clip.as_deref(), input.fade.unwrap_or(0.2))
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct SpacePoint {
    pub clip: String,
    pub x: f32,
    /// Default 0 (1D spaces).
    pub y: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SpaceInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub id: u64,
    /// Space name; play it with anim_play `clip: <name>`.
    pub name: String,
    /// Clips and where they sit; replaces the space's points. Required to create a space.
    pub points: Option<Vec<SpacePoint>>,
    /// Keep every clip at the same phase (walk/run cycles). Default true when creating.
    pub sync: Option<bool>,
    /// Parameter target. Omitted keeps the current one.
    pub x: Option<f32>,
    pub y: Option<f32>,
    /// Units per second toward the target (0 = jump). Omitted keeps the current rate.
    pub rate: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct OverlayInput {
    #[serde(flatten)]
    pub target: EditTarget,
    pub id: u64,
    /// Overlay name (created if missing).
    pub name: String,
    /// Clip or blend space to play on the overlay (cross-fades its other clips out).
    pub clip: Option<String>,
    /// Bone names (each with its children); replaces the mask. Empty = whole model.
    pub mask: Option<Vec<String>>,
    /// Add the clip's motion instead of replacing bones.
    pub additive: Option<bool>,
    /// Strength target 0..1. Default 1 (0 with `stop`).
    pub weight: Option<f32>,
    /// Seconds for a full strength change and the clip cross-fade. Default 0.2.
    pub fade: Option<f32>,
    pub speed: Option<f32>,
    /// Default true.
    pub looping: Option<bool>,
    /// Fade the overlay out (its clips stop once silent).
    pub stop: Option<bool>,
}

/// Edits entity `id`'s components `name` (as `T`) and Animator (added if missing).
fn edit_with<T: bevy_reflect::Reflect + bevy_reflect::PartialReflect + Default>(
    agent: &mut Agent,
    target: &EditTarget,
    id: u64,
    name: &'static str,
    clips: Vec<String>,
    change: impl FnOnce(&mut T, &BlendSpaces) -> Result<(), Errors>,
) -> Result<EditOutput, Errors> {
    let dir = target_dir(agent, target)?;
    let engine = agent.engine().clone();
    let registry = engine.registry();
    let (target_name, diff) = apply(agent, target, |scene, shown| {
        let asset = model_of(scene, id, shown)?;
        let e = scene
            .entities
            .iter_mut()
            .find(|e| e.id == id)
            .expect("model_of found it");
        let mut value: T = read_component(registry, e, name)?;
        let spaces: BlendSpaces = read_component(registry, e, "BlendSpaces")?;
        change(&mut value, &spaces)?;
        // Check clips against the spaces as they are after the change.
        let spaces_after: BlendSpaces = match (&value as &dyn bevy_reflect::Reflect)
            .as_any()
            .downcast_ref::<BlendSpaces>()
        {
            Some(s) => s.clone(),
            None => spaces,
        };
        if !clips.is_empty() {
            let rig: Rig = load_rig(&asset, &dir).map_err(|m| rig_error(&asset, m))?;
            for c in &clips {
                check_clip(&rig, &spaces_after, &asset, id, c)?;
            }
        }
        e.components.insert(name.into(), registry.to_json(&value));
        if !e.components.contains_key("Animator") {
            e.components
                .insert("Animator".into(), registry.to_json(&Animator::default()));
        }
        Ok(())
    })?;
    Ok(EditOutput {
        target: target_name,
        id,
        diff,
    })
}

fn space(agent: &mut Agent, input: SpaceInput) -> Result<EditOutput, Errors> {
    let clips: Vec<String> = input.points.iter().flatten().map(|p| p.clip.clone()).collect();
    let name = input.name.clone();
    edit_with::<BlendSpaces>(agent, &input.target, input.id, "BlendSpaces", clips, |s, _| {
        let exists = s.get(&name).is_some();
        match (&input.points, exists) {
            (Some(points), _) => {
                let points = points
                    .iter()
                    .map(|p| BlendPoint {
                        clip: p.clip.clone(),
                        x: p.x,
                        y: p.y.unwrap_or(0.0),
                    })
                    .collect();
                let sync = input
                    .sync
                    .unwrap_or_else(|| s.get(&name).is_none_or(|sp| sp.sync));
                s.define(&name, points, sync);
            }
            (None, false) => {
                return Err(Diagnostic::error(
                    codes::BAD_INPUT,
                    format!("entity {} has no blend space `{name}`", input.id),
                )
                .entity(input.id)
                .fix("pass `points` to create it")
                .into());
            }
            (None, true) => {
                if let Some(sync) = input.sync {
                    s.spaces
                        .iter_mut()
                        .filter(|sp| sp.name == name)
                        .for_each(|sp| sp.sync = sync);
                }
            }
        }
        let sp = s.get(&name).expect("defined above");
        let (x, y) = (input.x.unwrap_or(sp.target_x), input.y.unwrap_or(sp.target_y));
        s.set_param(&name, x, y, input.rate);
        Ok(())
    })
}

fn overlay(agent: &mut Agent, input: OverlayInput) -> Result<EditOutput, Errors> {
    let clips: Vec<String> = input.clip.iter().cloned().collect();
    edit_with::<AnimOverlays>(agent, &input.target, input.id, "AnimOverlays", clips, |o, _| {
        let fade = input.fade.unwrap_or(0.2);
        let ov = o.entry(&input.name);
        if let Some(mask) = &input.mask {
            ov.mask = mask.clone();
        }
        if let Some(a) = input.additive {
            ov.additive = a;
        }
        if input.stop == Some(true) {
            ov.set_weight(input.weight.unwrap_or(0.0), fade);
            if ov.target_weight == 0.0 {
                ov.stop(None, fade);
            }
        } else {
            ov.set_weight(input.weight.unwrap_or(1.0), fade);
        }
        if let Some(clip) = &input.clip {
            ov.play(
                clip,
                fade,
                input.speed.unwrap_or(1.0),
                input.looping.unwrap_or(true),
            );
        }
        Ok(())
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct PoseInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    pub id: u64,
    /// Node names to report; all nodes when omitted.
    pub nodes: Option<Vec<String>>,
}

#[derive(Serialize, JsonSchema)]
pub struct NodePose {
    pub name: String,
    /// World position.
    pub position: [f32; 3],
    /// World rotation (x, y, z, w).
    pub rotation: [f32; 4],
}

#[derive(Serialize, JsonSchema)]
pub struct PoseOutput {
    pub tick: u64,
    pub nodes: Vec<NodePose>,
}

fn pose(agent: &mut Agent, input: PoseInput) -> Result<PoseOutput, Errors> {
    with_world(agent, &input.target, |_, sim, _| {
        let id = input.id;
        let world = &mut sim.world;
        let mut q = world.query::<(bevy_ecs::entity::Entity, &marionette_core::scene::SceneId)>();
        let entity = q
            .iter(world)
            .find(|(_, s)| s.0 == id)
            .map(|(e, _)| e)
            .ok_or_else(|| {
                Errors::from(Diagnostic::error(codes::NO_ENTITY, format!("no entity {id}")).entity(id))
            })?;
        let model = world
            .get::<marionette_core::spatial::Model>(entity)
            .ok_or_else(|| {
                Errors::from(
                    Diagnostic::error(anim_codes::NO_MODEL, format!("entity {id} has no Model")).entity(id),
                )
            })?;
        let rig = match world
            .get_resource::<marionette_anim::Rigs>()
            .and_then(|r| r.0.get(&model.asset))
        {
            Some(r) => r.clone(),
            None => {
                let dir = world
                    .get_resource::<marionette_core::module::SceneDir>()
                    .map(|d| d.0.clone())
                    .unwrap_or_default();
                std::sync::Arc::new(load_rig(&model.asset, &dir).map_err(|m| rig_error(&model.asset, m))?)
            }
        };
        let names: Vec<String> = match &input.nodes {
            Some(n) => n.clone(),
            None => rig.nodes.iter().map(|n| n.name.clone()).collect(),
        };
        let t = world
            .get::<marionette_core::spatial::Transform>(entity)
            .copied()
            .unwrap_or_default();
        let translation = world.get::<marionette_core::builtin::Position>(entity).map_or(
            Vec3::new(t.translation.x, t.translation.y, t.translation.z),
            |p| Vec3::new(p.x, p.y, p.z),
        );
        let root = Mat4::from_scale_rotation_translation(
            Vec3::new(t.scale.x, t.scale.y, t.scale.z),
            Quat::from_xyzw(t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w).normalize(),
            translation,
        );
        let mut out = Vec::new();
        for name in names {
            let m = marionette_anim::node_matrix(world, entity, &name)
                .or_else(|| rig.node(&name).map(|i| rig.rest_pose()[i]))
                .ok_or_else(|| {
                    Errors::from(
                        Diagnostic::error(
                            anim_codes::UNKNOWN_NODE,
                            format!("`{}` has no node `{name}`", model.asset),
                        )
                        .entity(id)
                        .fix("anim_clips lists the node names"),
                    )
                })?;
            let (_, rot, pos) = (root * Mat4::from_cols_array(&m)).to_scale_rotation_translation();
            out.push(NodePose {
                name,
                position: pos.to_array(),
                rotation: rot.to_array(),
            });
        }
        Ok(PoseOutput {
            tick: sim.tick(),
            nodes: out,
        })
    })
}
