//! Reads what to draw out of the ECS world. Takes `&World`: rendering can never change
//! simulation state. Everything is sorted by stable scene id so frames are reproducible.

use glam::{Mat4, Quat, Vec2, Vec3};
use marionette_core::Kind;
use marionette_core::builtin::Position;
use marionette_core::scene::SceneId;
use marionette_core::spatial::{
    AmbientLight, Bloom, Camera, CastShadows, DirectionalLight, Emissive, Environment, Fog, LookAt, Material,
    Model, ModelPlacement, PointLight, Projection, Shadows, SkyLight, SpotLight, Sprite, SpriteRegion,
    ToneMapping, Transform,
};
use marionette_core::spatial::{Color, Vec3 as CVec3};

/// Everything the renderer needs for one frame, independent of the GPU.
#[derive(Debug, Clone, Default)]
pub struct FrameData {
    pub cameras: Vec<CameraData>,
    pub meshes: Vec<MeshDraw>,
    pub sprites: Vec<SpriteDraw>,
    pub lights: Vec<LightData>,
    /// Ambient light color times intensity (linear).
    pub ambient: Vec3,
    /// UI elements, sorted by layer then entity id (drawn last, in screen pixels).
    pub ui: Vec<UiDraw>,
    /// Copies drawn from modules' arrays (see `BulkDrawHooks`), in hook order.
    pub bulk: Vec<marionette_core::spatial::BulkDraw>,
    /// Point and spot lights, sorted by entity id (decision 0063).
    pub local_lights: Vec<LocalLightData>,
    /// Opt-in looks; `None` (the default) keeps the plain look.
    pub shadows: Option<Shadows>,
    /// Exposure, when tone mapping is on.
    pub exposure: Option<f32>,
    /// Fog color (linear RGB) and density.
    pub fog: Option<(Vec3, f32)>,
    /// Sky and ground ambient (color times intensity), replacing `ambient`.
    pub sky: Option<(Vec3, Vec3)>,
    /// Bloom threshold and intensity (decision 0064).
    pub bloom: Option<(f32, f32)>,
    /// Environment lighting and reflections (decision 0066).
    pub environment: Option<EnvironmentData>,
}

/// The `Environment` resource plus the sky colors a generated environment is made from.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentData {
    /// Equirectangular image path; empty for a sky made from `sky` and `ground`.
    pub image: String,
    pub intensity: f32,
    pub background: bool,
    /// `SkyLight` colors (without its intensity), or a default sky.
    pub sky: Vec3,
    pub ground: Vec3,
}

/// A point light (`cos_outer` below -1) or a spot light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalLightData {
    pub position: Vec3,
    /// Color times intensity (linear).
    pub radiance: Vec3,
    pub range: f32,
    /// Direction the light travels (spot lights; normalized).
    pub direction: Vec3,
    pub cos_inner: f32,
    pub cos_outer: f32,
    /// Shadow map size and softness, from `CastShadows` (decision 0065).
    pub shadow: Option<(u32, f32)>,
}

/// What a UI node looks like.
#[derive(Debug, Clone)]
pub enum UiLook {
    Rect {
        color: [f32; 4],
    },
    Text {
        text: String,
        size: f32,
        color: [f32; 4],
    },
    Image {
        image: String,
        color: [f32; 4],
    },
}

#[derive(Debug, Clone)]
pub struct UiDraw {
    pub entity: u64,
    pub node: marionette_core::ui::UiNode,
    pub look: UiLook,
}

impl UiLook {
    /// The texture this look samples.
    pub fn image(&self) -> &str {
        match self {
            UiLook::Rect { .. } => "builtin:white",
            UiLook::Text { .. } => "builtin:font",
            UiLook::Image { image, .. } => image,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CameraData {
    pub entity: u64,
    pub camera: Camera,
    pub world: Mat4,
}

/// Per-view data. One frame can have several views (stereo for VR later, split screen),
/// so nothing about the camera is global renderer state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub view: Mat4,
    pub proj: Mat4,
    pub eye: Vec3,
    pub clear: [f32; 4],
}

impl CameraData {
    /// The view for an image with the given aspect ratio (width / height).
    pub fn view(&self, aspect: f32) -> View {
        let proj = match self.camera.projection {
            Projection::Perspective {
                fov_y_degrees,
                near,
                far,
            } => glam::camera::rh::proj::directx::perspective(fov_y_degrees.to_radians(), aspect, near, far),
            Projection::Orthographic { height, near, far } => {
                let (hw, hh) = (height * aspect * 0.5, height * 0.5);
                glam::camera::rh::proj::directx::orthographic(-hw, hw, -hh, hh, near, far)
            }
        };
        let c = self.camera.clear_color;
        View {
            view: self.world.inverse(),
            proj,
            eye: self.world.w_axis.truncate(),
            clear: [c.r, c.g, c.b, c.a],
        }
    }
}

#[derive(Debug, Clone)]
pub struct MeshDraw {
    pub entity: u64,
    /// Model asset path. Shared between draws of the same asset (one allocation per asset
    /// per frame, not per entity).
    pub asset: std::sync::Arc<str>,
    pub world: Mat4,
    pub material: Option<Material>,
    /// Animated node matrices (from the animation module); `None` draws the rest pose.
    pub pose: Option<std::sync::Arc<marionette_core::spatial::Pose>>,
    /// Light given off (color times intensity), from `Emissive`; zero for most models.
    pub emissive: Vec3,
    /// The entity also has `CastShadows` (a lamp): the model does not block its own light.
    pub lamp: bool,
}

#[derive(Debug, Clone)]
pub struct SpriteDraw {
    pub entity: u64,
    pub image: String,
    pub world: Mat4,
    pub size: Vec2,
    pub color: [f32; 4],
    pub layer: i32,
    pub region: Option<SpriteRegion>,
}

#[derive(Debug, Clone, Copy)]
pub struct LightData {
    /// Direction the light travels (normalized).
    pub direction: Vec3,
    /// Color times illuminance (linear).
    pub radiance: Vec3,
}

fn v3(v: CVec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn rgb(c: Color) -> Vec3 {
    Vec3::new(c.r, c.g, c.b)
}

fn rgba(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// World matrix from `Position`/`Transform`/`LookAt`.
fn world_matrix(t: Option<&Transform>, p: Option<&Position>, look: Option<&LookAt>) -> Mat4 {
    let t = t.copied().unwrap_or_default();
    let translation = p.map_or(v3(t.translation), |p| Vec3::new(p.x, p.y, p.z));
    let mut rotation = Quat::from_xyzw(t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w).normalize();
    if let Some(look) = look {
        let target = v3(look.target);
        if (target - translation).length_squared() > 1e-12 {
            let up = v3(look.up).normalize_or(Vec3::Y);
            let view = glam::camera::rh::view::look_at_mat4(translation, target, up);
            rotation = Quat::from_mat4(&view.inverse());
        }
    }
    Mat4::from_scale_rotation_translation(v3(t.scale), rotation, translation)
}

/// The entity's translation and rotation (not its scale), then the placement's offset,
/// rotation and scale.
fn placed_matrix(
    t: Option<&Transform>,
    p: Option<&Position>,
    look: Option<&LookAt>,
    m: &ModelPlacement,
) -> Mat4 {
    let (_, rotation, translation) = world_matrix(t, p, look).to_scale_rotation_translation();
    let local = Quat::from_xyzw(m.rotation.x, m.rotation.y, m.rotation.z, m.rotation.w).normalize();
    Mat4::from_scale_rotation_translation(
        v3(m.scale),
        rotation * local,
        translation + rotation * v3(m.offset),
    )
}

/// Extracts the frame from the world at its latest tick.
pub fn extract(world: &bevy_ecs::world::World) -> FrameData {
    extract_at(world, 1.0)
}

/// Extracts the frame `alpha` (0..1) of the way from the latest tick toward the next one
/// (passed to bulk draw hooks for smooth motion between ticks).
///
/// One query per kind of drawable (each visits only the archetypes that have it), then a
/// sort by stable id, so the cost is proportional to what is drawn, and the order does not
/// depend on storage layout.
pub fn extract_at(world: &bevy_ecs::world::World, alpha: f32) -> FrameData {
    use marionette_core::ui::{UiImage, UiNode, UiRect, UiText};
    let mut frame = FrameData::default();
    if let Some(a) = world.get_resource::<AmbientLight>() {
        frame.ambient = Vec3::new(a.color.r, a.color.g, a.color.b) * a.intensity;
    }
    frame.shadows = world.get_resource::<Shadows>().copied();
    frame.exposure = world.get_resource::<ToneMapping>().map(|t| t.exposure);
    frame.fog = world
        .get_resource::<Fog>()
        .map(|f| (rgb(f.color), f.density.max(0.0)));
    frame.sky = world
        .get_resource::<SkyLight>()
        .map(|s| (rgb(s.sky) * s.intensity, rgb(s.ground) * s.intensity));
    frame.bloom = world
        .get_resource::<Bloom>()
        .map(|b| (b.threshold.max(0.0), b.intensity.max(0.0)));
    frame.environment = world.get_resource::<Environment>().map(|e| {
        let (sky, ground) = world
            .get_resource::<SkyLight>()
            .map_or((Vec3::new(0.45, 0.62, 0.9), Vec3::new(0.32, 0.29, 0.26)), |s| {
                (rgb(s.sky), rgb(s.ground))
            });
        EnvironmentData {
            image: e.image.clone(),
            intensity: e.intensity.max(0.0),
            background: e.background,
            sky,
            ground,
        }
    });
    type Place<'a> = (Option<&'a Transform>, Option<&'a Position>, Option<&'a LookAt>);

    if let Some(mut q) = world.try_query::<(&SceneId, &Camera, Place)>() {
        for (id, camera, (t, p, l)) in q.iter(world) {
            frame.cameras.push(CameraData {
                entity: id.0,
                camera: *camera,
                world: world_matrix(t, p, l),
            });
        }
        frame.cameras.sort_by_key(|c| c.entity);
    }
    // `Pose` is looked up separately: a query naming a component type the world has never
    // seen (no animation module) matches nothing, even inside `Option`.
    let poses = world
        .components()
        .component_id::<marionette_core::spatial::Pose>()
        .is_some();
    if let Some(mut q) = world.try_query::<(
        bevy_ecs::entity::Entity,
        &SceneId,
        &Model,
        Option<&Material>,
        Place,
    )>() {
        // Share one allocation per distinct asset path.
        let mut names: std::collections::BTreeMap<&str, std::sync::Arc<str>> = Default::default();
        frame.meshes.reserve(q.iter(world).len());
        let emissives = world.components().component_id::<Emissive>().is_some();
        let placements = world.components().component_id::<ModelPlacement>().is_some();
        let lamps = world.components().component_id::<CastShadows>().is_some();
        for (e, id, model, material, (t, p, l)) in q.iter(world) {
            let emissive = if emissives {
                world
                    .get::<Emissive>(e)
                    .map_or(Vec3::ZERO, |m| rgb(m.color) * m.intensity.max(0.0))
            } else {
                Vec3::ZERO
            };
            let pose = if poses {
                world.get::<marionette_core::spatial::Pose>(e)
            } else {
                None
            };
            let asset = names
                .entry(model.asset.as_str())
                .or_insert_with(|| model.asset.as_str().into())
                .clone();
            let placement = if placements {
                world.get::<ModelPlacement>(e)
            } else {
                None
            };
            frame.meshes.push(MeshDraw {
                entity: id.0,
                asset,
                world: match placement {
                    Some(m) => placed_matrix(t, p, l, m),
                    None => world_matrix(t, p, l),
                },
                material: material.copied(),
                pose: pose.map(|p| std::sync::Arc::new(p.clone())),
                emissive,
                lamp: lamps && world.get::<CastShadows>(e).is_some(),
            });
        }
        frame.meshes.sort_unstable_by_key(|m| m.entity);
    }
    if let Some(mut q) = world.try_query::<(&SceneId, &Sprite, Option<&SpriteRegion>, Place)>() {
        for (id, sprite, region, (t, p, l)) in q.iter(world) {
            frame.sprites.push(SpriteDraw {
                entity: id.0,
                image: sprite.image.clone(),
                world: world_matrix(t, p, l),
                size: Vec2::new(sprite.size.x, sprite.size.y),
                color: rgba(sprite.color),
                layer: sprite.layer,
                region: region.copied(),
            });
        }
    }
    if let Some(mut q) = world.try_query::<(
        &SceneId,
        &UiNode,
        Option<&UiRect>,
        Option<&UiText>,
        Option<&UiImage>,
    )>() {
        for (id, node, r, t, i) in q.iter(world) {
            let look = if let Some(r) = r {
                Some(UiLook::Rect { color: rgba(r.color) })
            } else if let Some(t) = t {
                Some(UiLook::Text {
                    text: t.text.clone(),
                    size: t.size,
                    color: rgba(t.color),
                })
            } else {
                i.map(|i| UiLook::Image {
                    image: i.image.clone(),
                    color: rgba(i.color),
                })
            };
            if let Some(look) = look {
                frame.ui.push(UiDraw {
                    entity: id.0,
                    node: *node,
                    look,
                });
            }
        }
    }
    if let Some(mut q) = world.try_query::<(&SceneId, &DirectionalLight)>() {
        let mut lights: Vec<(u64, LightData)> = q
            .iter(world)
            .map(|(id, light)| {
                (
                    id.0,
                    LightData {
                        direction: v3(light.direction).normalize_or(Vec3::NEG_Y),
                        radiance: Vec3::new(light.color.r, light.color.g, light.color.b) * light.illuminance,
                    },
                )
            })
            .collect();
        lights.sort_by_key(|(id, _)| *id);
        frame.lights = lights.into_iter().map(|(_, l)| l).collect();
    }
    let mut local: Vec<(u64, LocalLightData)> = Vec::new();
    let casters = world.components().component_id::<CastShadows>().is_some();
    let shadow_of = |e: bevy_ecs::entity::Entity| {
        if !casters {
            return None;
        }
        world
            .get::<CastShadows>(e)
            .map(|c| (c.resolution.clamp(128, 1024), c.softness.clamp(0.0, 4.0)))
    };
    if let Some(mut q) = world.try_query::<(bevy_ecs::entity::Entity, &SceneId, &PointLight, Place)>() {
        for (e, id, light, (t, p, l)) in q.iter(world) {
            local.push((
                id.0,
                LocalLightData {
                    position: world_matrix(t, p, l).w_axis.truncate(),
                    radiance: rgb(light.color) * light.intensity,
                    range: light.range.max(1e-3),
                    direction: Vec3::NEG_Y,
                    cos_inner: -2.0,
                    cos_outer: -3.0,
                    shadow: shadow_of(e),
                },
            ));
        }
    }
    if let Some(mut q) = world.try_query::<(bevy_ecs::entity::Entity, &SceneId, &SpotLight, Place)>() {
        for (e, id, light, (t, p, l)) in q.iter(world) {
            let outer = light.outer_angle_degrees.clamp(0.1, 89.9);
            let inner = light.inner_angle_degrees.clamp(0.0, outer - 0.05);
            local.push((
                id.0,
                LocalLightData {
                    position: world_matrix(t, p, l).w_axis.truncate(),
                    radiance: rgb(light.color) * light.intensity,
                    range: light.range.max(1e-3),
                    direction: v3(light.direction).normalize_or(Vec3::NEG_Y),
                    cos_inner: inner.to_radians().cos(),
                    cos_outer: outer.to_radians().cos(),
                    shadow: shadow_of(e),
                },
            ));
        }
    }
    local.sort_by_key(|(id, _)| *id);
    frame.local_lights = local.into_iter().map(|(_, l)| l).collect();
    if let Some(hooks) = world.get_resource::<marionette_core::spatial::BulkDrawHooks>() {
        for hook in &hooks.0 {
            hook(world, alpha.clamp(0.0, 1.0), &mut frame.bulk);
        }
    }
    // Stable draw order for sprites: layer, then entity id.
    frame.sprites.sort_by_key(|s| (s.layer, s.entity));
    frame.ui.sort_by_key(|u| (u.node.layer, u.entity));
    frame
}

/// Names of the core types this module reads, for documentation and tests.
pub const READS: [(&str, Kind); 20] = [
    ("Transform", Kind::Component),
    ("LookAt", Kind::Component),
    ("Camera", Kind::Component),
    ("Model", Kind::Component),
    ("Material", Kind::Component),
    ("ModelPlacement", Kind::Component),
    ("Sprite", Kind::Component),
    ("SpriteRegion", Kind::Component),
    ("DirectionalLight", Kind::Component),
    ("AmbientLight", Kind::Resource),
    ("PointLight", Kind::Component),
    ("SpotLight", Kind::Component),
    ("Emissive", Kind::Component),
    ("Shadows", Kind::Resource),
    ("ToneMapping", Kind::Resource),
    ("Fog", Kind::Resource),
    ("SkyLight", Kind::Resource),
    ("Bloom", Kind::Resource),
    ("CastShadows", Kind::Component),
    ("Environment", Kind::Resource),
];
