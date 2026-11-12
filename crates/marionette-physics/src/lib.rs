//! Marionette physics: Rapier 3D and 2D behind scene components, stepped once per fixed
//! tick in [`Stage::Physics`]. Built with Rapier's `enhanced-determinism` feature.
//!
//! Turn physics on with the `PhysicsSettings` resource. Bodies are built from `RigidBody` +
//! `Collider` + `Transform` when the scene loads; afterwards physics writes `Transform`,
//! `Velocity`, `AngularVelocity` and `CharacterState` back every tick, so the state hash
//! covers every body. Rapier's internal caches are saved in snapshots so a restored session
//! continues exactly.

mod backend;
pub mod components;
mod dims;

use bevy_ecs::prelude::*;
use components::*;
use marionette_core::actions::ActionState;
use marionette_core::builtin::{DEFAULT_DT, FixedStep, SimClock};
use marionette_core::module::{EventLog, Module, SceneDir, Stage};
use marionette_core::scene::SceneId;
use marionette_core::{Diagnostic, Errors, Registry, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod codes {
    pub const MISSING: &str = "physics.missing_collider";
    pub const POSITION: &str = "physics.position_conflict";
    pub const SHAPE: &str = "physics.shape";
    pub const SNAPSHOT: &str = "physics.snapshot";
}

/// The physics world of the scene (3D or 2D). Not a scene component: rebuilt from
/// components on load and carried in snapshots.
#[derive(Resource, Serialize, Deserialize)]
pub enum Physics {
    D3(Box<dims::d3::State>),
    D2(Box<dims::d2::State>),
}

pub struct PhysicsModule;

/// Spawned entities whose body could not be built (reported once as a `physics_error`
/// event). Not simulation state: rebuilt empty on load and restore.
#[derive(Resource, Default)]
struct FailedBodies(std::collections::BTreeSet<u64>);

fn scene_ids(world: &mut World) -> BTreeMap<u64, Entity> {
    let mut q = world.query::<(Entity, &SceneId)>();
    q.iter(world).map(|(e, id)| (id.0, e)).collect()
}

impl Module for PhysicsModule {
    fn name(&self) -> &'static str {
        "physics"
    }

    fn register(&self, r: &mut Registry) {
        r.register_resource::<PhysicsSettings>();
        r.register_component::<RigidBody>();
        r.register_component::<Collider>();
        r.register_component::<AngularVelocity>();
        r.register_component::<CharacterController>();
        r.register_component::<CharacterState>();
    }

    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(physics_tick.in_set(Stage::Physics));
    }

    fn init(&self, world: &mut World) -> Result<()> {
        world.remove_resource::<FailedBodies>();
        let Some(settings) = world.get_resource::<PhysicsSettings>().copied() else {
            world.remove_resource::<Physics>();
            return Ok(());
        };
        let dir = world
            .get_resource::<SceneDir>()
            .map(|d| d.0.clone())
            .unwrap_or_default();
        let ids = scene_ids(world);
        let physics = match settings.mode {
            PhysicsMode::Dim3 => Physics::D3(Box::new(dims::d3::build(world, &ids, &dir)?)),
            PhysicsMode::Dim2 => Physics::D2(Box::new(dims::d2::build(world, &ids, &dir)?)),
        };
        world.insert_resource(physics);
        Ok(())
    }

    fn save(&self, world: &World) -> Option<Vec<u8>> {
        let p = world.get_resource::<Physics>()?;
        Some(postcard::to_allocvec(p).expect("physics state serializes"))
    }

    fn load(&self, world: &mut World, bytes: &[u8]) -> Result<()> {
        let p: Physics = postcard::from_bytes(bytes).map_err(|e| {
            Errors::from(Diagnostic::error(
                codes::SNAPSHOT,
                format!("cannot restore physics state: {e}"),
            ))
        })?;
        world.insert_resource(p);
        world.remove_resource::<FailedBodies>();
        Ok(())
    }
}

/// The physics step, as one exclusive system so the order of every write is fixed.
fn physics_tick(world: &mut World) {
    let Some(mut physics) = world.remove_resource::<Physics>() else {
        return;
    };
    let dt = world.get_resource::<FixedStep>().map_or(DEFAULT_DT, |s| s.dt);
    let g = world
        .get_resource::<PhysicsSettings>()
        .map_or([0.0, -9.81, 0.0], |s| [s.gravity.x, s.gravity.y, s.gravity.z]);
    let tick = world.resource::<SimClock>().tick;
    let actions = world.get_resource::<ActionState>().cloned().unwrap_or_default();
    let ids = scene_ids(world);
    // Entities spawned or despawned since the last step (by scripts) gain or lose bodies.
    let dir = world
        .get_resource::<SceneDir>()
        .map(|d| d.0.clone())
        .unwrap_or_default();
    let mut failed = world.remove_resource::<FailedBodies>().unwrap_or_default();
    let errors = match &mut physics {
        Physics::D3(s) => dims::d3::sync(s, world, &ids, &dir, &mut failed.0),
        Physics::D2(s) => dims::d2::sync(s, world, &ids, &dir, &mut failed.0),
    };
    world.insert_resource(failed);
    if !errors.is_empty()
        && let Some(mut log) = world.get_resource_mut::<EventLog>()
    {
        for errs in errors {
            for d in errs.0 {
                log.push(
                    tick,
                    "physics_error",
                    d.entity,
                    None,
                    serde_json::json!({ "code": d.code, "message": d.message }),
                );
            }
        }
    }
    match &mut physics {
        Physics::D3(s) => dims::d3::tick(s, world, &ids, &actions, dt, g, tick),
        Physics::D2(s) => dims::d2::tick(s, world, &ids, &actions, dt, g, tick),
    }
    world.insert_resource(physics);
}

/// A ray hit: the entity, distance along the ray, hit point and surface normal.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct RayHit {
    pub entity: u64,
    pub distance: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

/// Casts a ray in the world's physics scene (2D ignores z). `None` if nothing is hit or the
/// scene has no physics.
pub fn raycast(world: &World, origin: [f32; 3], direction: [f32; 3], max_distance: f32) -> Option<RayHit> {
    let hit = match world.get_resource::<Physics>()? {
        Physics::D3(s) => dims::d3::raycast(s, origin, direction, max_distance),
        Physics::D2(s) => dims::d2::raycast(s, origin, direction, max_distance),
    }?;
    let len =
        (direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2]).sqrt();
    let d = if len > 0.0 {
        direction.map(|c| c / len)
    } else {
        direction
    };
    Some(RayHit {
        entity: hit.0,
        distance: hit.1,
        point: [
            origin[0] + d[0] * hit.1,
            origin[1] + d[1] * hit.1,
            origin[2] + d[2] * hit.1,
        ],
        normal: hit.2,
    })
}
