//! Physics components and resources (plain data, part of the scene and the state hash).
//!
//! Physics owns the placement of entities with a `RigidBody`: it reads `Transform` when the
//! scene loads and writes `Transform` (translation and rotation), `Velocity` and
//! `AngularVelocity` back after every step. Such entities must not also have `Position`.

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::spatial::Vec3;

/// Turns physics on for the scene and picks 3D or 2D.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct PhysicsSettings {
    pub mode: PhysicsMode,
    /// Acceleration in m/s² (2D uses x and y).
    pub gravity: Vec3,
}

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicsMode {
    /// Rapier 3D.
    Dim3,
    /// Rapier 2D in the XY plane; z is kept as authored.
    Dim2,
}

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// Moved by forces, gravity and contacts.
    Dynamic,
    /// Never moves.
    Fixed,
    /// Moved by setting its Transform (or by a CharacterController); pushes dynamic bodies.
    Kinematic,
}

/// A body simulated by physics. Needs a `Collider` and a `Transform`.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct RigidBody {
    pub kind: BodyKind,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub gravity_scale: f32,
    /// Keep the body upright (no rotation from contacts).
    pub lock_rotations: bool,
}

/// Collision shape. Sizes are before `Transform.scale` is applied.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub enum ColliderShape {
    /// Box with half sizes per axis (2D uses x and y).
    Cuboid {
        half_extents: Vec3,
    },
    /// Box with rounded edges: the same outer half sizes as `Cuboid`, with edges and corners
    /// rounded by `border` metres (after scale). Character controllers slide over flat ground
    /// with it where a sharp `Cuboid` snags; use it for box-shaped characters such as cars.
    RoundCuboid {
        half_extents: Vec3,
        border: f32,
    },
    Ball {
        radius: f32,
    },
    /// Capsule along Y: a cylinder of `half_height` with rounded ends of `radius`.
    Capsule {
        half_height: f32,
        radius: f32,
    },
    /// Cylinder along Y (3D only).
    Cylinder {
        half_height: f32,
        radius: f32,
    },
    /// Convex hull of a model's vertices (`primitive:...`, a glTF path, or `mesh:<name>` for
    /// a game mesh made in code; 3D only).
    ConvexHull {
        model: String,
    },
    /// Exact triangle mesh of a model (for fixed geometry; 3D only). `mesh:<name>` builds it
    /// from a game mesh (`GameMeshes`, decision 0180): the body waits until the mesh exists
    /// and its shape is rebuilt whenever the mesh is stored again (terrain edits).
    TriMesh {
        model: String,
    },
}

impl ColliderShape {
    /// The game mesh (`mesh:<name>`) a `ConvexHull` or `TriMesh` is built from, if any.
    pub fn game_mesh(&self) -> Option<&str> {
        match self {
            ColliderShape::ConvexHull { model } | ColliderShape::TriMesh { model } => {
                marionette_core::mesh::mesh_name(model)
            }
            _ => None,
        }
    }
}

/// The shape and surface of a body. An entity with a `Collider` but no `RigidBody` is fixed.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct Collider {
    pub shape: ColliderShape,
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
    /// Detects overlaps (collision events) without pushing.
    pub sensor: bool,
}

/// Rotation speed in radians per second around each axis (2D uses z). Written by physics.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct AngularVelocity {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Walks a kinematic body with input actions: slides along walls, climbs small slopes,
/// stands on ground, jumps, and pushes dynamic bodies. 3D moves in X/Z relative to the
/// entity's facing (`Transform.rotation`; a script turns the character by setting it), 2D in X.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct CharacterController {
    /// Horizontal speed in m/s at full input.
    pub speed: f32,
    /// Upward speed when a jump starts.
    pub jump_speed: f32,
    /// Downward acceleration in m/s².
    pub gravity: f32,
    /// Mass used when pushing dynamic bodies.
    pub mass: f32,
    /// Action for left (-1) / right (+1).
    pub move_x: String,
    /// Action for back (-1) / forward (+1, toward -Z). Ignored in 2D.
    pub move_forward: String,
    pub jump: String,
}

/// Drives a `CharacterController` from a script instead of input actions (AI characters,
/// vehicles): when present, its values replace the controller's actions. Values are like
/// action values (-1..1). `jump` asks for one jump: physics starts it if the character
/// stands on ground and sets `jump` back to false either way.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct CharacterInput {
    pub move_x: f32,
    pub move_forward: f32,
    pub jump: bool,
}

/// Runtime state of a character controller. Written by physics.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct CharacterState {
    pub vertical_velocity: f32,
    pub grounded: bool,
}

/// Which collision layers a body is on and which it touches (Unity's layers and collision
/// matrix, PhysX filter groups). Two colliders touch only when each one's `member` shares a
/// bit with the other's `collides_with`. Queries take a layer mask too. Without this
/// component a collider is on layer 1 (bit 0, Unity's Default layer) and touches every layer.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionLayers {
    /// Bits of the layers this collider is on (32 layers).
    pub member: u32,
    /// Bits of the layers it touches.
    pub collides_with: u32,
}

/// Continuous collision detection for a fast dynamic body (Unity's `ContinuousDynamic`):
/// it sweeps from its old to its new pose each step, so it cannot pass through a thin wall
/// between two steps. Costs a little per body, so use it for thrown or shot things only.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContinuousCollision {
    pub enabled: bool,
}

/// A force and torque applied to a dynamic body every tick while the component is there
/// (Unity's `AddForce` in `FixedUpdate`, `ForceMode.Force`). World space, newtons and
/// newton-metres (2D uses x and y for the force and z for the torque).
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct ExternalForce {
    pub force: Vec3,
    pub torque: Vec3,
}

/// A one-off push (Unity's `ForceMode.Impulse`): applied at the next step, then physics sets
/// both values back to zero. World space, newton-seconds.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct ExternalImpulse {
    pub linear: Vec3,
    pub angular: Vec3,
}

/// What a [`Joint`] allows between its two bodies.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub enum JointKind {
    /// Welded: no relative motion (Unity's FixedJoint).
    Fixed,
    /// Turns freely about the anchor (CharacterJoint without limits; 2D: same as `Hinge`).
    Ball,
    /// Turns about `axis` (in this body's space; ignored in 2D, which turns about z). Angles
    /// are this body's turn relative to the other body, 0 where they were when the joint was
    /// built. Within
    /// `min` to `max` radians when `min < max`. A motor drives it toward `motor_speed`
    /// radians per second with at most `motor_force` (0 = no motor). Unity's HingeJoint.
    Hinge {
        axis: Vec3,
        min: f32,
        max: f32,
        motor_speed: f32,
        motor_force: f32,
    },
    /// Slides along `axis` (this body's space), within `min` to `max` metres when
    /// `min < max`, with an optional motor as `Hinge` (metres per second, newtons).
    /// Unity's ConfigurableJoint with one free linear axis.
    Slider {
        axis: Vec3,
        min: f32,
        max: f32,
        motor_speed: f32,
        motor_force: f32,
    },
    /// The anchors stay at most `max_length` apart (a rope or chain link).
    Rope { max_length: f32 },
    /// The anchors are pulled toward `rest_length` apart (Unity's SpringJoint).
    Spring {
        rest_length: f32,
        stiffness: f32,
        damping: f32,
    },
}

/// Joins this entity's body to another body (or to a fixed point in the world). Both need
/// a `RigidBody` or `Collider`; the joint waits until both bodies exist and goes away with
/// either of them or with this component.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Joint {
    /// Scene id of the other entity; 0 joins to the world at `other_anchor` (world space).
    pub other: u64,
    pub kind: JointKind,
    /// Where the joint sits, in this body's space (before scale).
    pub anchor: Vec3,
    /// Where it sits in the other body's space (world space when `other` is 0).
    pub other_anchor: Vec3,
    /// Let the two joined bodies collide with each other (Unity's `enableCollision`).
    pub collide_connected: bool,
}
