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

/// Where a `Model` is drawn relative to its entity, for models that do not match the entity's
/// collider: the model is moved by `offset` (in the entity's rotated frame, unscaled), turned
/// by `rotation` and sized by `scale`, which replaces the entity's `Transform.scale` for
/// drawing. Physics and scripts still see the entity's own `Transform`. Typical use: a
/// generated model whose origin is at its feet on a character whose origin is at the center of
/// its capsule.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct ModelPlacement {
    pub offset: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for ModelPlacement {
    fn default() -> Self {
        Self {
            offset: Vec3::default(),
            rotation: Quat::default(),
            scale: Vec3::new(1.0, 1.0, 1.0),
        }
    }
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

/// A light at the entity's position shining in every direction (a lamp, a torch, a glowing
/// crystal). Brightness falls off with the square of the distance and reaches zero at
/// `range`. The renderer uses the first 16 point and spot lights by entity id.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct PointLight {
    pub color: Color,
    /// Brightness at 1 m (same scale as `DirectionalLight::illuminance`).
    pub intensity: f32,
    /// Distance in meters where the light has faded to nothing.
    pub range: f32,
}

/// A cone of light from the entity's position along `direction` (a flashlight, a street
/// lamp). Full brightness inside `inner_angle_degrees` from the axis, fading to zero at
/// `outer_angle_degrees`. Shares the 16-light limit with `PointLight`.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct SpotLight {
    pub direction: Vec3,
    pub color: Color,
    /// Brightness at 1 m on the axis.
    pub intensity: f32,
    pub range: f32,
    pub inner_angle_degrees: f32,
    pub outer_angle_degrees: f32,
}

/// Light a model gives off by itself (screens, lava, lit windows), added on top of its
/// lit color. Not a light source: it does not light other objects. With `ToneMapping`,
/// intensities above 1 stay bright instead of clipping.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Emissive {
    pub color: Color,
    pub intensity: f32,
}

/// Turns on shadows cast by the first `DirectionalLight` (lowest entity id). Shadows cover
/// a square `distance` meters on each side of the camera; farther objects are unshadowed.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Shadows {
    /// Half the width of the shadowed area around the camera, in meters.
    pub distance: f32,
    /// Shadow map size in texels (256 to 4096; 2048 is a good default).
    pub resolution: u32,
    /// Edge blur in texels (0 = hard edges, 1 to 2 = soft).
    pub softness: f32,
}

/// Splits the sun's shadow (`Shadows`) into `count` cascades (2 to 4) reaching `far` meters
/// from the camera: near cascades are small and sharp, far ones large, so shadows stay crisp
/// up close and still reach the horizon (decision 0090). Each cascade is a `Shadows.resolution`
/// map; `Shadows.distance` is ignored while cascades are on. Perspective cameras only (an
/// orthographic camera keeps one map).
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct ShadowCascades {
    pub count: u32,
    /// Distance from the camera where shadows end, in meters (capped at the camera's far plane).
    pub far: f32,
}

/// Turns on filmic tone mapping: lit colors are multiplied by `exposure`, then mapped so
/// bright light rolls off smoothly instead of clipping at white. Without it, colors clamp
/// at 1.0.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct ToneMapping {
    pub exposure: f32,
}

/// Turns on distance fog for models: surfaces fade toward `color` with distance from the
/// camera (exponential squared). Set the camera's `clear_color` to the same color so the
/// horizon matches.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    pub color: Color,
    /// Higher is thicker; about 2 / density meters is where fog is nearly opaque.
    pub density: f32,
}

/// Replaces flat `AmbientLight` with sky light: surfaces facing up get `sky`, surfaces
/// facing down get `ground`, blended by the surface direction, times `intensity`.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct SkyLight {
    pub sky: Color,
    pub ground: Color,
    pub intensity: f32,
}

/// Turns on bloom: light brighter than `threshold` spills into a soft glow around it
/// (lamps, emissive screens, the sun on metal). Works best with `ToneMapping`, which keeps
/// colors above 1.0 for the glow to pick up. Renders models into a floating-point target
/// first (decision 0064).
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct Bloom {
    /// Brightness (linear, before exposure) where glow starts; 1.0 is a good default.
    pub threshold: f32,
    /// How strong the glow is; 0.05 to 0.3 is subtle to strong.
    pub intensity: f32,
}

/// Put on an entity with a `PointLight` or `SpotLight` to make that light cast shadows.
/// A spot light uses one shadow map, a point light six (one per cube face); at most 24
/// maps per frame, given to lights in entity id order (decision 0065). A model on the same
/// entity (a lantern around its light) does not block its own light.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct CastShadows {
    /// Size of each shadow map in texels (128 to 1024; 512 is a good default). All maps
    /// share the largest size asked for (24 maps of 1024 take 100 MB of GPU memory).
    pub resolution: u32,
    /// Edge blur in texels (0 = hard edges, 1 to 2 = soft).
    pub softness: f32,
}

/// Turns on environment lighting and reflections: surfaces are lit by, and shiny surfaces
/// reflect, a surrounding image (decision 0066). `image` is an equirectangular image (2:1,
/// relative to the scene file): a PNG, or a Radiance `.hdr` or OpenEXR `.exr` file whose
/// values above 1.0 keep real sun and sky brightness (decision 0091). Empty makes one from
/// `SkyLight` colors (or a default sky). Replaces `AmbientLight` and `SkyLight` ambient for
/// models.
#[derive(Resource, Reflect, Debug, Clone, PartialEq)]
pub struct Environment {
    pub image: String,
    /// Strength of the light and reflections from the environment.
    pub intensity: f32,
    /// Draw the environment behind everything instead of the camera's clear color.
    pub background: bool,
}

/// A box where shiny surfaces reflect a picture of the scene taken from the entity's position
/// instead of the environment alone: rooms, halls, a car park (decision 0092). The picture is
/// taken when the probe first appears and again when it moves or changes; objects that move
/// later are not in it (`RayTracing` reflections show those). Up to four probes; a point
/// inside several uses the one with the lowest entity id.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct ReflectionProbe {
    /// Half size of the box in meters along x, y and z, centered on the entity. Reflections
    /// are corrected for the box's walls, so match it to the room.
    pub extent: Vec3,
    /// Size of each of the six captured faces in texels (32 to 512; 128 is a good default).
    pub resolution: u32,
}

/// Turns on ray-traced lighting on GPUs that support ray queries (decision 0093): exact
/// shadows from every light, and reflections of other objects on smooth surfaces. Elsewhere
/// the scene keeps its other looks (`Shadows`, `CastShadows`, `ReflectionProbe`,
/// `Environment`), so add those as the fallback. Skinned models are not in the ray-traced
/// scene yet.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct RayTracing {
    /// Ray-traced shadows for every directional, point and spot light, replacing shadow maps.
    pub shadows: bool,
    /// Ray-traced reflections of other objects on surfaces smoother than `max_roughness`.
    pub reflections: bool,
    /// Roughness (0 to 1) above which reflections fade back to the environment; 0.5 is a
    /// good default.
    pub max_roughness: f32,
    /// Size of the sun's disc in degrees for soft sun shadows (0 = hard; the real sun is 0.5).
    pub sun_size_degrees: f32,
}

/// Many copies of one model drawn from a module's own arrays (belt items, crowds,
/// particles), without one entity per copy. Built at render time by a [`BulkDrawHooks`]
/// function, so the simulation pays nothing for it.
#[derive(Debug, Clone, PartialEq)]
pub struct BulkDraw {
    /// Model asset, as in [`Model::asset`].
    pub asset: String,
    /// One material for every copy (`None` uses the model's own).
    pub material: Option<Material>,
    /// One affine transform per copy: the first three rows of the column-major world
    /// matrix, as `[x_axis.xyz, y_axis.xyz, z_axis.xyz, translation.xyz]`.
    pub transforms: Vec<[f32; 12]>,
}

/// A function that adds bulk draws for the current world. `alpha` (0..1) is how far the
/// frame is between the last tick and the next, for smooth motion at display rate.
pub type BulkDrawHook = fn(&bevy_ecs::world::World, alpha: f32, &mut Vec<BulkDraw>);

/// Bulk draw hooks registered by modules in `Module::init`. Not simulation state: not in
/// the scene, the schema or the state hash.
#[derive(bevy_ecs::prelude::Resource, Default, Clone)]
pub struct BulkDrawHooks(pub Vec<BulkDrawHook>);

impl BulkDrawHooks {
    /// Adds `hook` to the world's hooks (once).
    pub fn add(world: &mut bevy_ecs::world::World, hook: BulkDrawHook) {
        let mut hooks = world.get_resource_or_insert_with(BulkDrawHooks::default);
        if !hooks.0.iter().any(|h| std::ptr::fn_addr_eq(*h, hook)) {
            hooks.0.push(hook);
        }
    }
}

/// Animated node matrices of a `Model`: one model-space, column-major matrix per glTF node,
/// plus morph target weights per node. Written by the animation module every tick and read
/// by the renderer. Derived data: not saved in scenes and not part of the state hash (it
/// follows from the `Animator` component, which is).
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct Pose {
    pub nodes: Vec<[f32; 16]>,
    /// Morph target (blend shape) weights per node; empty for nodes without morph targets.
    pub morph_weights: Vec<Vec<f32>>,
}
