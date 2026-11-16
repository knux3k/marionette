//! Bodies owned by game code instead of scene entities, and shape queries (decision 0311).
//!
//! A game module that keeps its objects as plain data (thousands of loose items in one
//! `Vec`, as Unity games often keep pooled objects) can still give some of them a physics
//! body without making each an entity: [`add_body`] with a key the game chooses, step as
//! usual, then read [`body_state`] back into its own data. The game owns the key space and
//! the order of calls, so the step stays deterministic. Game bodies are pulled by
//! `GravityWell`s and zones like entity bodies, collide with every scene collider, are saved
//! in snapshots, and show up in queries as [`Target::Body`].
//!
//! Queries ([`overlap`], [`shape_cast`], [`raycast_all`], [`raycast_first`]) are Unity's
//! `Physics.OverlapBox/Sphere/Capsule`, `BoxCast/SphereCast` and `RaycastAll/Raycast` with a
//! layer mask; results are in a fixed order (by distance, then target) so game code that
//! acts on them stays deterministic.

use crate::Physics;
use crate::components::{BodyKind, ColliderShape, CollisionLayers};
use bevy_ecs::world::World;

/// What a query or contact found: a scene entity (its scene id) or a game body (its key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    Entity(u64),
    Body(u64),
}

/// Rapier keeps one `u128` of user data per collider: scene ids are stored as they are, game
/// body keys with bit 64 set, so the two key spaces never meet.
pub(crate) const GAME_BIT: u128 = 1 << 64;

pub(crate) fn target(user_data: u128) -> Target {
    if user_data & GAME_BIT != 0 {
        Target::Body(user_data as u64)
    } else {
        Target::Entity(user_data as u64)
    }
}

/// A game body to add. `Default` is a 1 kg-per-m³ dynamic unit cube at the origin.
#[derive(Debug, Clone, PartialEq)]
pub struct GameBody {
    pub kind: BodyKind,
    /// Model shapes (`ConvexHull`, `TriMesh`) load from the scene folder or game meshes.
    pub shape: ColliderShape,
    pub position: [f32; 3],
    /// Quaternion x, y, z, w.
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    /// Total mass in kg; 0 takes it from `density` and the shape.
    pub mass: f32,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub gravity_scale: f32,
    /// Sweep between steps so a fast body cannot pass through thin walls.
    pub ccd: bool,
    pub sensor: bool,
    /// `None` = on every layer, touching every layer.
    pub layers: Option<CollisionLayers>,
}

impl Default for GameBody {
    fn default() -> Self {
        GameBody {
            kind: BodyKind::Dynamic,
            shape: ColliderShape::Cuboid {
                half_extents: marionette_core::spatial::Vec3::new(0.5, 0.5, 0.5),
            },
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
            mass: 0.0,
            density: 1.0,
            friction: 0.6,
            restitution: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.05,
            gravity_scale: 1.0,
            ccd: false,
            sensor: false,
            layers: None,
        }
    }
}

/// Where a game body is after the last step.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BodyState {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    /// Rapier put it to sleep: it has been still for a while (Unity's `IsSleeping`).
    pub sleeping: bool,
}

/// A game body started or stopped touching something during the last step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contact {
    pub body: u64,
    pub other: Target,
    pub started: bool,
}

/// Which colliders a query sees.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct QueryOptions {
    /// Leave out this entity or body (the asker itself).
    pub exclude: Option<Target>,
    pub skip_sensors: bool,
    /// Leave out every game body.
    pub skip_bodies: bool,
    /// Only colliders on one of these layers (`CollisionLayers.member`); 0 = every layer.
    pub layers: u32,
}

/// A query hit: what, how far along the ray or sweep, where, and the surface normal there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub target: Target,
    pub distance: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

/// Calls the 3D or 2D backend function of the same name with the state first; `None` when
/// the scene has no physics.
macro_rules! dispatch {
    ($world:expr, $f:ident($($a:expr),*)) => {
        match $world.get_resource::<Physics>() {
            Some(Physics::D3(s)) => Some(crate::dims::d3::$f(s, $($a),*)),
            Some(Physics::D2(s)) => Some(crate::dims::d2::$f(s, $($a),*)),
            None => None,
        }
    };
    (mut $world:expr, $f:ident($($a:expr),*)) => {
        match $world.get_resource_mut::<Physics>().as_deref_mut() {
            Some(Physics::D3(s)) => Some(crate::dims::d3::$f(s, $($a),*)),
            Some(Physics::D2(s)) => Some(crate::dims::d2::$f(s, $($a),*)),
            None => None,
        }
    };
}

/// Adds a game body under `key`, replacing one already there. Errors when the scene has no
/// physics or the shape cannot be built.
pub fn add_body(world: &mut World, key: u64, body: &GameBody) -> Result<(), String> {
    let dir = world
        .get_resource::<marionette_core::module::SceneDir>()
        .map(|d| d.0.clone())
        .unwrap_or_default();
    // The physics resource is taken out so game meshes can be read beside it (a model shape
    // may name one); it goes back before returning.
    let Some(mut p) = world.remove_resource::<Physics>() else {
        return Err("the scene has no physics (add the PhysicsSettings resource)".to_string());
    };
    let meshes = world.get_resource::<marionette_core::mesh::GameMeshes>();
    let r = match &mut p {
        Physics::D3(s) => crate::dims::d3::add_game(s, key, body, &dir, meshes),
        Physics::D2(s) => crate::dims::d2::add_game(s, key, body, &dir, meshes),
    };
    world.insert_resource(p);
    r
}

/// Removes a game body; false if there was none.
pub fn remove_body(world: &mut World, key: u64) -> bool {
    dispatch!(mut world, remove_game(key)).unwrap_or(false)
}

/// The body's pose and motion after the last step, or `None` if there is no such body.
pub fn body_state(world: &World, key: u64) -> Option<BodyState> {
    dispatch!(world, game_state(key)).flatten()
}

/// Keys of every game body, in key order.
pub fn body_keys(world: &World) -> Vec<u64> {
    dispatch!(world, game_keys()).unwrap_or_default()
}

/// Sets a body's velocity (Unity's `linearVelocity` and `angularVelocity`); `None` keeps one.
pub fn set_body_velocity(
    world: &mut World,
    key: u64,
    velocity: Option<[f32; 3]>,
    angular_velocity: Option<[f32; 3]>,
) -> bool {
    dispatch!(mut world, set_game_velocity(key, velocity, angular_velocity)).unwrap_or(false)
}

/// Moves a body (a teleport for a dynamic body; the next target for a kinematic one).
pub fn set_body_pose(world: &mut World, key: u64, position: [f32; 3], rotation: [f32; 4]) -> bool {
    dispatch!(mut world, set_game_pose(key, position, rotation)).unwrap_or(false)
}

/// Turns gravity on a body up or down (0 = Unity's `useGravity = false`).
pub fn set_body_gravity_scale(world: &mut World, key: u64, scale: f32) -> bool {
    dispatch!(mut world, set_game_gravity(key, scale)).unwrap_or(false)
}

/// Pushes a body once (Unity's `AddForce(.., ForceMode.Impulse)`), newton-seconds.
pub fn push_body(world: &mut World, key: u64, impulse: [f32; 3]) -> bool {
    dispatch!(mut world, push_game(key, impulse)).unwrap_or(false)
}

/// What game bodies started or stopped touching in the last step, in the order Rapier
/// reported them (fixed for a given world).
pub fn body_contacts(world: &World) -> Vec<Contact> {
    dispatch!(world, game_contacts()).unwrap_or_default()
}

/// Every collider overlapping `shape` placed at `position` and `rotation`, sorted. Model
/// shapes are not supported here.
pub fn overlap(
    world: &World,
    shape: &ColliderShape,
    position: [f32; 3],
    rotation: [f32; 4],
    options: QueryOptions,
) -> Result<Vec<Target>, String> {
    dispatch!(world, overlap(shape, position, rotation, options)).unwrap_or_else(|| Ok(Vec::new()))
}

/// Sweeps `shape` from `position` along `direction` up to `max_distance`: the first hit.
pub fn shape_cast(
    world: &World,
    shape: &ColliderShape,
    position: [f32; 3],
    rotation: [f32; 4],
    direction: [f32; 3],
    max_distance: f32,
    options: QueryOptions,
) -> Result<Option<Hit>, String> {
    dispatch!(
        world,
        shape_cast(shape, position, rotation, direction, max_distance, options)
    )
    .unwrap_or(Ok(None))
}

/// Every collider along a ray, nearest first (Unity's `RaycastAll`, sorted).
pub fn raycast_all(
    world: &World,
    origin: [f32; 3],
    direction: [f32; 3],
    max_distance: f32,
    options: QueryOptions,
) -> Vec<Hit> {
    dispatch!(world, ray_hits(origin, direction, max_distance, options, true)).unwrap_or_default()
}

/// The nearest collider along a ray, game bodies and layers included.
pub fn raycast_first(
    world: &World,
    origin: [f32; 3],
    direction: [f32; 3],
    max_distance: f32,
    options: QueryOptions,
) -> Option<Hit> {
    dispatch!(world, ray_hits(origin, direction, max_distance, options, false))
        .and_then(|v| v.into_iter().next())
}
