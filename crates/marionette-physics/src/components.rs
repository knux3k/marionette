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
    /// Convex hull of a model's vertices (`primitive:...` or a glTF path; 3D only).
    ConvexHull {
        model: String,
    },
    /// Exact triangle mesh of a model (for fixed geometry; 3D only).
    TriMesh {
        model: String,
    },
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
