//! Marionette physics: Rapier 3D and 2D behind scene components, stepped once per fixed
//! tick in [`Stage::Physics`]. Built with Rapier's `enhanced-determinism` feature.
//!
//! Turn physics on with the `PhysicsSettings` resource. Bodies are built from `RigidBody` +
//! `Collider` + `Transform` when the scene loads; afterwards physics writes `Transform`,
//! `Velocity`, `AngularVelocity` and `CharacterState` back every tick, so the state hash
//! covers every body. Rapier's internal caches are saved in snapshots so a restored session
//! continues exactly.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![deny(clippy::clone_on_ref_ptr)]

mod backend;
pub mod components;
mod dims;
pub mod game;
pub mod planet;

use bevy_ecs::prelude::*;
use components::*;
pub use game::{
    BodyState, Contact, GameBody, Hit, QueryOptions, Target, add_body, body_contacts, body_keys, body_state,
    overlap, push_body, raycast_all, raycast_first, remove_body, set_body_gravity_scale, set_body_pose,
    set_body_velocity, shape_cast,
};
use marionette_core::actions::ActionState;
use marionette_core::builtin::{DEFAULT_DT, FixedStep, SimClock};
use marionette_core::module::{EventLog, Module, RayCaster, SceneDir, Stage};
pub use marionette_core::module::{RayFilter, RayHit};
use marionette_core::scene::SceneId;
use marionette_core::{Diagnostic, Errors, Registry, Result};
pub use planet::{GravityWell, GravityZone, PlanetCharacter, PlanetState, SphereFluid};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod codes {
    pub const MISSING: &str = "physics.missing_collider";
    pub const POSITION: &str = "physics.position_conflict";
    pub const CHILD: &str = "physics.child_body";
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

/// The physics step as a system set, inside `Stage::Physics`. Game systems that set body
/// velocities or poses this tick (a carried object) run `.before(PhysicsStep)`; ones that
/// read the results (copying game bodies back into game data) run `.after(PhysicsStep)`.
#[derive(bevy_ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PhysicsStep;

/// What physics remembers about bodies beside the simulation state. Not saved: rebuilt on
/// load, and on restore (with `adopt` set).
#[derive(Resource, Default)]
pub(crate) struct BodyTracking {
    /// Spawned entities whose body could not be built (reported once as a `physics_error`
    /// event).
    pub failed: std::collections::BTreeSet<u64>,
    /// Bodies whose collider is built from a game mesh: the mesh version it was built at.
    pub meshes: BTreeMap<u64, u64>,
    /// Restored from a snapshot: take the game meshes' current versions as built.
    pub adopt: bool,
    /// The `Joint` each built joint was built from (a changed component rebuilds it).
    pub joints: BTreeMap<u64, Joint>,
    /// Restored from a snapshot: take the saved joints as built.
    pub adopt_joints: bool,
}

/// Forces and impulses to apply this step, read from components in scene id order.
#[derive(Default)]
pub struct Pushes {
    pub forces: Vec<(u64, [f32; 3], [f32; 3])>,
    pub impulses: Vec<(u64, [f32; 3], [f32; 3])>,
}

fn v3(v: marionette_core::spatial::Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

/// Joints, forces and impulses of the scene, each in scene id order. Impulses are set back
/// to zero here: they apply once.
fn gather(world: &mut World) -> (Vec<(u64, Joint)>, Pushes) {
    let mut joints: Vec<(u64, Joint)> = world
        .query::<(&SceneId, &Joint)>()
        .iter(world)
        .map(|(id, j)| (id.0, *j))
        .collect();
    joints.sort_by_key(|(id, _)| *id);
    let mut p = Pushes {
        forces: world
            .query::<(&SceneId, &ExternalForce)>()
            .iter(world)
            .map(|(id, f)| (id.0, v3(f.force), v3(f.torque)))
            .collect(),
        impulses: Vec::new(),
    };
    p.forces.sort_by_key(|(id, ..)| *id);
    let mut q = world.query::<(&SceneId, &mut ExternalImpulse)>();
    for (id, mut i) in q.iter_mut(world) {
        if *i != ExternalImpulse::default() {
            p.impulses.push((id.0, v3(i.linear), v3(i.angular)));
            *i = ExternalImpulse::default();
        }
    }
    p.impulses.sort_by_key(|(id, ..)| *id);
    (joints, p)
}

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
        r.register_component::<CharacterInput>();
        r.register_component::<CollisionLayers>();
        r.register_component::<ContinuousCollision>();
        r.register_component::<ExternalForce>();
        r.register_component::<ExternalImpulse>();
        r.register_component::<Joint>();
        r.register_component::<planet::GravityWell>();
        r.register_component::<planet::SphereFluid>();
        r.register_component::<planet::GravityZone>();
        r.register_component::<planet::PlanetCharacter>();
        r.register_component::<planet::PlanetState>();
    }

    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(physics_tick.in_set(Stage::Physics).in_set(PhysicsStep));
    }

    fn init(&self, world: &mut World) -> Result<()> {
        world.remove_resource::<BodyTracking>();
        world.insert_resource(RayCaster(raycast));
        let Some(settings) = world.get_resource::<PhysicsSettings>().copied() else {
            world.remove_resource::<Physics>();
            return Ok(());
        };
        let dir = world
            .get_resource::<SceneDir>()
            .map(|d| d.0.clone())
            .unwrap_or_default();
        let ids = scene_ids(world);
        let mut track = BodyTracking::default();
        let physics = match settings.mode {
            PhysicsMode::Dim3 => Physics::D3(Box::new(dims::d3::build(world, &ids, &dir, &mut track)?)),
            PhysicsMode::Dim2 => Physics::D2(Box::new(dims::d2::build(world, &ids, &dir, &mut track)?)),
        };
        world.insert_resource(physics);
        world.insert_resource(track);
        Ok(())
    }

    fn save(&self, world: &World) -> Option<Vec<u8>> {
        let p = world.get_resource::<Physics>()?;
        // The world first, then joints and game bodies: a snapshot from before they existed
        // is the first part alone and still loads.
        let mut bytes = postcard::to_allocvec(p).ok()?;
        let extra = match p {
            Physics::D3(s) => postcard::to_allocvec(&s.extra),
            Physics::D2(s) => postcard::to_allocvec(&s.extra),
        };
        bytes.extend(extra.ok()?);
        Some(bytes)
    }

    fn load(&self, world: &mut World, bytes: &[u8]) -> Result<()> {
        let bad = |e: postcard::Error| {
            Errors::from(Diagnostic::error(
                codes::SNAPSHOT,
                format!("cannot restore physics state: {e}"),
            ))
        };
        let (mut p, rest): (Physics, &[u8]) = postcard::take_from_bytes(bytes).map_err(bad)?;
        if !rest.is_empty() {
            match &mut p {
                Physics::D3(s) => s.extra = postcard::from_bytes(rest).map_err(bad)?,
                Physics::D2(s) => s.extra = postcard::from_bytes(rest).map_err(bad)?,
            }
        }
        world.insert_resource(p);
        world.insert_resource(BodyTracking {
            adopt: true,
            adopt_joints: true,
            ..Default::default()
        });
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
    let mut track = world.remove_resource::<BodyTracking>().unwrap_or_default();
    let errors = match &mut physics {
        Physics::D3(s) => dims::d3::sync(s, world, &ids, &dir, &mut track),
        Physics::D2(s) => dims::d2::sync(s, world, &ids, &dir, &mut track),
    };
    let (joints, pushes) = gather(world);
    match &mut physics {
        Physics::D3(s) => {
            dims::d3::sync_joints(s, &joints, &mut track);
            s.extra.pushes = pushes;
        }
        Physics::D2(s) => {
            dims::d2::sync_joints(s, &joints, &mut track);
            s.extra.pushes = pushes;
        }
    }
    world.insert_resource(track);
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

/// Casts a ray in the world's physics scene (2D ignores z). `None` if nothing is hit or the
/// scene has no physics.
pub fn raycast(
    world: &World,
    origin: [f32; 3],
    direction: [f32; 3],
    max_distance: f32,
    filter: RayFilter,
) -> Option<RayHit> {
    let hit = match world.get_resource::<Physics>()? {
        Physics::D3(s) => dims::d3::raycast(s, origin, direction, max_distance, filter),
        Physics::D2(s) => dims::d2::raycast(s, origin, direction, max_distance, filter),
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
