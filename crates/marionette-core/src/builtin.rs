//! Built-in components, resources and systems: enough for a scene to simulate something.
//!
//! - `random_impulse`: entities with `RandomImpulse` get a seeded random kick each tick.
//! - `follow`: entities with `Follow` accelerate toward another entity.
//! - `integrate`: `Position += Velocity * dt`.
//!
//! All arithmetic is plain `f32` add/multiply (no transcendental functions), and every
//! system that consumes randomness visits entities in stable-id order.

use crate::module::Stage;
use crate::registry::Registry;
use crate::rng::SimRng;
use crate::scene::SceneId;
use crate::spatial;
use crate::ui;
use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use std::collections::BTreeMap;

/// Default tick length when the scene has no `FixedStep` resource.
pub const DEFAULT_DT: f32 = 1.0 / 60.0;

/// A reference to another entity by its stable scene id. Serialized as a bare number.
/// The validator reports a `scene.dangling_ref` error if the id does not exist.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SceneRef(pub u64);

/// World-space position in meters.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Linear velocity in meters per second.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct Velocity {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Adds a seeded random velocity change of up to `strength` per axis each tick.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct RandomImpulse {
    pub strength: f32,
}

/// Accelerates toward `target` with acceleration `gain * offset`.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct Follow {
    pub target: SceneRef,
    pub gain: f32,
}

/// A movement request in world space, written by one system and read by another without
/// either depending on the other (decision 0232): navigation writes it, a character
/// controller (`PlanetCharacter` in physics) or a game's own system reads it. `velocity` is
/// in m/s; a reader uses the part along the ground and caps it at its own speed. `jump`
/// asks for one jump; the reader sets it back to false.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct MoveIntent {
    pub velocity: spatial::Vec3,
    pub jump: bool,
}

/// Length of one simulation tick in seconds.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct FixedStep {
    pub dt: f32,
}

/// Number of ticks simulated so far. Internal; part of the state hash.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SimClock {
    pub tick: u64,
}

/// Registers every built-in type.
pub fn register_builtins(registry: &mut Registry) {
    registry.register_component::<Position>();
    registry.register_component::<Velocity>();
    registry.register_component::<RandomImpulse>();
    registry.register_component::<Follow>();
    registry.register_component::<MoveIntent>();
    registry.register_component::<crate::jobdata::JobData>();
    registry.register_component::<crate::jobdata::JobDraw>();
    registry.register_resource::<FixedStep>();
    // Data read by renderers (see `spatial`).
    registry.register_component::<spatial::Transform>();
    registry.register_component::<spatial::LookAt>();
    registry.register_component::<spatial::Camera>();
    registry.register_component::<spatial::Model>();
    registry.register_component::<spatial::Material>();
    registry.register_component::<spatial::ModelPlacement>();
    registry.register_component::<spatial::Sprite>();
    registry.register_component::<spatial::SpriteRegion>();
    registry.register_component::<spatial::DirectionalLight>();
    registry.register_resource::<spatial::AmbientLight>();
    // Opt-in graphics features (decision 0063): absent means the plain look.
    registry.register_component::<spatial::PointLight>();
    registry.register_component::<spatial::SpotLight>();
    registry.register_component::<spatial::Emissive>();
    registry.register_resource::<spatial::Shadows>();
    registry.register_resource::<spatial::ToneMapping>();
    registry.register_resource::<spatial::Fog>();
    registry.register_resource::<spatial::SkyLight>();
    // More opt-in looks (decisions 0064 to 0066).
    registry.register_resource::<spatial::Bloom>();
    registry.register_component::<spatial::CastShadows>();
    registry.register_resource::<spatial::Environment>();
    registry.register_component::<ui::UiNode>();
    registry.register_component::<ui::UiRect>();
    registry.register_component::<ui::UiText>();
    registry.register_component::<ui::UiImage>();
    registry.register_component::<ui::UiButton>();
}

/// Adds the built-in systems to `schedule`, in a fixed order.
pub fn add_builtin_systems(schedule: &mut Schedule) {
    schedule.add_systems((random_impulse, follow, integrate).chain().in_set(Stage::Logic));
    schedule.add_systems(advance_clock.in_set(Stage::End));
}

fn dt(step: &Option<Res<FixedStep>>) -> f32 {
    step.as_ref().map_or(DEFAULT_DT, |s| s.dt)
}

fn random_impulse(mut rng: ResMut<SimRng>, mut query: Query<(&SceneId, &RandomImpulse, &mut Velocity)>) {
    // Query order depends on storage layout; sort so random draws go to entities
    // in the same order on every run.
    let mut items: Vec<_> = query.iter_mut().collect();
    items.sort_by_key(|(id, _, _)| id.0);
    for (_, impulse, mut vel) in items {
        vel.x += impulse.strength * rng.next_signed_f32();
        vel.y += impulse.strength * rng.next_signed_f32();
        vel.z += impulse.strength * rng.next_signed_f32();
    }
}

fn follow(
    step: Option<Res<FixedStep>>,
    targets: Query<(&SceneId, &Position)>,
    mut followers: Query<(&Follow, &Position, &mut Velocity)>,
) {
    let dt = dt(&step);
    let positions: BTreeMap<u64, Position> = targets.iter().map(|(id, p)| (id.0, *p)).collect();
    for (follow, pos, mut vel) in &mut followers {
        let Some(target) = positions.get(&follow.target.0) else {
            continue; // dangling refs are reported by the validator
        };
        let k = follow.gain * dt;
        vel.x += (target.x - pos.x) * k;
        vel.y += (target.y - pos.y) * k;
        vel.z += (target.z - pos.z) * k;
    }
}

fn integrate(step: Option<Res<FixedStep>>, mut query: Query<(&Velocity, &mut Position)>) {
    let dt = dt(&step);
    for (vel, mut pos) in &mut query {
        pos.x += vel.x * dt;
        pos.y += vel.y * dt;
        pos.z += vel.z * dt;
    }
}

fn advance_clock(mut clock: ResMut<SimClock>) {
    clock.tick += 1;
}
