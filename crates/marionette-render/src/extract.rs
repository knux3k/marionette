//! Reads what to draw out of the ECS world. Takes `&World`: rendering can never change
//! simulation state. Everything is sorted by stable scene id so frames are reproducible.

use glam::{Mat4, Quat, Vec2, Vec3};
use marionette_core::Kind;
use marionette_core::builtin::Position;
use marionette_core::scene::SceneId;
use marionette_core::spatial::{
    AmbientLight, Camera, DirectionalLight, LookAt, Material, Model, Projection, Sprite, SpriteRegion,
    Transform,
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
    pub asset: String,
    pub world: Mat4,
    pub material: Option<Material>,
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

fn rgba(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// World matrix of an entity from `Position`/`Transform`/`LookAt`.
fn world_matrix(world: &bevy_ecs::world::World, e: bevy_ecs::entity::Entity) -> Mat4 {
    let t = world.get::<Transform>(e).copied().unwrap_or_default();
    let translation = world
        .get::<Position>(e)
        .map_or(v3(t.translation), |p| Vec3::new(p.x, p.y, p.z));
    let mut rotation = Quat::from_xyzw(t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w).normalize();
    if let Some(look) = world.get::<LookAt>(e) {
        let target = v3(look.target);
        if (target - translation).length_squared() > 1e-12 {
            let up = v3(look.up).normalize_or(Vec3::Y);
            let view = glam::camera::rh::view::look_at_mat4(translation, target, up);
            rotation = Quat::from_mat4(&view.inverse());
        }
    }
    Mat4::from_scale_rotation_translation(v3(t.scale), rotation, translation)
}

/// Extracts the frame from the world.
pub fn extract(world: &bevy_ecs::world::World) -> FrameData {
    let mut frame = FrameData::default();
    if let Some(a) = world.get_resource::<AmbientLight>() {
        frame.ambient = Vec3::new(a.color.r, a.color.g, a.color.b) * a.intensity;
    }
    let Some(mut q) = world.try_query::<(bevy_ecs::entity::Entity, &SceneId)>() else {
        return frame;
    };
    let mut entities: Vec<(u64, bevy_ecs::entity::Entity)> = q.iter(world).map(|(e, id)| (id.0, e)).collect();
    entities.sort_by_key(|(id, _)| *id);

    for (id, e) in entities {
        let mat = world_matrix(world, e);
        if let Some(camera) = world.get::<Camera>(e) {
            frame.cameras.push(CameraData {
                entity: id,
                camera: *camera,
                world: mat,
            });
        }
        if let Some(model) = world.get::<Model>(e) {
            frame.meshes.push(MeshDraw {
                entity: id,
                asset: model.asset.clone(),
                world: mat,
                material: world.get::<Material>(e).copied(),
            });
        }
        if let Some(sprite) = world.get::<Sprite>(e) {
            frame.sprites.push(SpriteDraw {
                entity: id,
                image: sprite.image.clone(),
                world: mat,
                size: Vec2::new(sprite.size.x, sprite.size.y),
                color: rgba(sprite.color),
                layer: sprite.layer,
                region: world.get::<SpriteRegion>(e).copied(),
            });
        }
        if let Some(node) = world.get::<marionette_core::ui::UiNode>(e) {
            use marionette_core::ui::{UiImage, UiRect, UiText};
            let look = if let Some(r) = world.get::<UiRect>(e) {
                Some(UiLook::Rect { color: rgba(r.color) })
            } else if let Some(t) = world.get::<UiText>(e) {
                Some(UiLook::Text {
                    text: t.text.clone(),
                    size: t.size,
                    color: rgba(t.color),
                })
            } else {
                world.get::<UiImage>(e).map(|i| UiLook::Image {
                    image: i.image.clone(),
                    color: rgba(i.color),
                })
            };
            if let Some(look) = look {
                frame.ui.push(UiDraw {
                    entity: id,
                    node: *node,
                    look,
                });
            }
        }
        if let Some(light) = world.get::<DirectionalLight>(e) {
            frame.lights.push(LightData {
                direction: v3(light.direction).normalize_or(Vec3::NEG_Y),
                radiance: Vec3::new(light.color.r, light.color.g, light.color.b) * light.illuminance,
            });
        }
    }
    // Stable draw order for sprites: layer, then entity id.
    frame.sprites.sort_by_key(|s| (s.layer, s.entity));
    frame.ui.sort_by_key(|u| (u.node.layer, u.entity));
    frame
}

/// Names of the core types this module reads, for documentation and tests.
pub const READS: [(&str, Kind); 9] = [
    ("Transform", Kind::Component),
    ("LookAt", Kind::Component),
    ("Camera", Kind::Component),
    ("Model", Kind::Component),
    ("Material", Kind::Component),
    ("Sprite", Kind::Component),
    ("SpriteRegion", Kind::Component),
    ("DirectionalLight", Kind::Component),
    ("AmbientLight", Kind::Resource),
];
