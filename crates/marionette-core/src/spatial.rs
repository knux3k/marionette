//! Plain-data math types and the components a renderer reads.
//!
//! These are data only (no graphics code), so the core can load, validate and simulate any
//! scene, including ones with cameras and sprites, without linking a GPU crate. The
//! renderer (`marionette-render`) reads them and never writes them.
//!
//! Conventions: right-handed, +Y up, cameras look down their local -Z. Distances in meters.
//! Colors are linear RGBA in 0..1 (the same convention as glTF material factors).

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

/// Rotation as a unit quaternion (x, y, z, w). Identity is (0, 0, 0, 1).
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        }
    }
}

/// Linear RGBA color, each channel 0..1.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for Color {
    fn default() -> Self {
        Self {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
    }
}

/// Placement of an entity: translation, rotation, scale. If the entity also has a
/// `Position` (moved by the simulation), `Position` is used as the translation.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::default(),
            rotation: Quat::default(),
            scale: Vec3::new(1.0, 1.0, 1.0),
        }
    }
}

/// Points the entity's -Z axis at `target` (world space), keeping `up` as close to its +Y
/// as possible. Overrides `Transform.rotation`. Useful for cameras.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct LookAt {
    pub target: Vec3,
    pub up: Vec3,
}

/// How a camera projects the world onto the image.
#[derive(Reflect, Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    /// Perspective projection. `fov_y_degrees` is the vertical field of view.
    Perspective { fov_y_degrees: f32, near: f32, far: f32 },
    /// Orthographic projection showing `height` world units vertically; width follows the
    /// image aspect ratio.
    Orthographic { height: f32, near: f32, far: f32 },
}

/// A viewpoint that can be rendered. Placement comes from `Transform` (and `LookAt`).
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub projection: Projection,
    /// Background color.
    pub clear_color: Color,
}

/// A 3D model. `asset` is a glTF 2.0 file (`.gltf` or `.glb`) relative to the scene file,
/// or a built-in shape: `primitive:cube` (1 m), `primitive:sphere` (1 m diameter),
/// `primitive:plane` (1 m square, facing +Y).
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct Model {
    pub asset: String,
}

/// Overrides the material of a `Model` (all of its parts) with simple PBR values.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Material {
    pub base_color: Color,
    pub metallic: f32,
    pub roughness: f32,
}

/// A textured, camera-independent quad in the XY plane at the entity's translation.
/// `image` is a PNG file relative to the scene file or a built-in image (`builtin:white`,
/// `builtin:circle`, `builtin:checker`, `builtin:atlas`). Higher `layer` draws on top;
/// equal layers draw in entity-id order.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct Sprite {
    pub image: String,
    /// Size in world units.
    pub size: Vec2,
    /// Multiplies the image color (including alpha).
    pub color: Color,
    pub layer: i32,
}

/// Uses only part of a sprite's image (a texture atlas cell), in pixels from the top-left.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct SpriteRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Sunlight: parallel rays travelling along `direction`.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct DirectionalLight {
    pub direction: Vec3,
    pub color: Color,
    /// Brightness multiplier (1.0 = a bright, unclipped sun on a white surface).
    pub illuminance: f32,
}

/// Light that reaches every surface equally, from all directions.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct AmbientLight {
    pub color: Color,
    pub intensity: f32,
}
