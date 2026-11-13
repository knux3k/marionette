//! The editor's own camera. It never touches the simulation: the scene's cameras stay where
//! the scene put them, and the editor camera only decides what the window shows.

use glam::Vec3;
use marionette_core::spatial::Projection;
use marionette_render::View;
use marionette_render::extract::CameraData;

/// Radians of turn per pixel of mouse movement.
pub const LOOK_PER_PIXEL: f32 = 0.0035;
const MAX_PITCH: f32 = 89.0 * std::f32::consts::PI / 180.0;

/// A fly camera (perspective) or a panning camera (orthographic, for 2D scenes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlyCamera {
    pub position: Vec3,
    /// Turn around +Y in radians; 0 looks down -Z.
    pub yaw: f32,
    /// Up/down in radians, clamped to +-89 degrees.
    pub pitch: f32,
    /// Flying speed in meters per second (perspective) .
    pub speed: f32,
    pub fov_y_degrees: f32,
    /// Visible height in world units when orthographic.
    pub ortho_height: Option<f32>,
    pub near: f32,
    pub far: f32,
    pub clear: [f32; 4],
}

impl Default for FlyCamera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 3.0, 8.0),
            yaw: 0.0,
            pitch: -0.3,
            speed: 6.0,
            fov_y_degrees: 60.0,
            ortho_height: None,
            near: 0.05,
            far: 2000.0,
            clear: [0.11, 0.12, 0.14, 1.0],
        }
    }
}

impl FlyCamera {
    /// Starts where a scene camera is, looking the same way, with its projection and clear
    /// color.
    pub fn from_scene_camera(cam: &CameraData) -> Self {
        let (_, rotation, position) = cam.world.to_scale_rotation_translation();
        let f = (rotation * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
        let c = cam.camera.clear_color;
        let mut out = Self {
            position,
            yaw: f.x.atan2(-f.z),
            pitch: f.y.clamp(-1.0, 1.0).asin().clamp(-MAX_PITCH, MAX_PITCH),
            clear: [c.r, c.g, c.b, c.a],
            ..Self::default()
        };
        match cam.camera.projection {
            Projection::Perspective {
                fov_y_degrees,
                near,
                far,
            } => {
                out.fov_y_degrees = fov_y_degrees;
                out.near = near.clamp(1e-3, 0.05);
                out.far = far.max(2000.0);
            }
            Projection::Orthographic { height, near, far } => {
                out.ortho_height = Some(height);
                out.near = near;
                out.far = far;
            }
        }
        out
    }

    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp)
    }

    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, sy)
    }

    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward())
    }

    /// Turns by mouse movement in pixels (right and down positive). Ignored for 2D views.
    pub fn look(&mut self, dx: f32, dy: f32) {
        if self.ortho_height.is_some() {
            return;
        }
        self.yaw = (self.yaw + dx * LOOK_PER_PIXEL).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch - dy * LOOK_PER_PIXEL).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Moves for `dt` seconds. `dir` is (right, up, forward), each -1..1. In 2D views forward
    /// moves up the screen. `fast` triples the speed.
    pub fn fly(&mut self, dir: Vec3, fast: bool, dt: f32) {
        let speed = match self.ortho_height {
            Some(h) => h * 0.8,
            None => self.speed,
        } * if fast { 3.0 } else { 1.0 };
        let step = match self.ortho_height {
            Some(_) => self.right() * dir.x + self.up() * (dir.y + dir.z),
            None => self.right() * dir.x + Vec3::Y * dir.y + self.forward() * dir.z,
        };
        self.position += step * speed * dt;
    }

    /// Drags the view by mouse movement in pixels on a viewport `height` pixels tall: the
    /// point under the cursor stays under it (at `depth` meters in perspective views).
    pub fn pan(&mut self, dx: f32, dy: f32, height: f32, depth: f32) {
        let world_per_px = match self.ortho_height {
            Some(h) => h / height.max(1.0),
            None => 2.0 * depth.max(0.1) * (self.fov_y_degrees.to_radians() * 0.5).tan() / height.max(1.0),
        };
        self.position += (-self.right() * dx + self.up() * dy) * world_per_px;
    }

    /// Mouse wheel: changes flying speed (perspective) or zooms (orthographic).
    pub fn scroll(&mut self, steps: f32) {
        match &mut self.ortho_height {
            Some(h) => *h = (*h * 0.85f32.powf(steps)).clamp(0.01, 100_000.0),
            None => self.speed = (self.speed * 1.25f32.powf(steps)).clamp(0.1, 1000.0),
        }
    }

    /// Moves back along the view direction until the box fits, keeping the direction.
    pub fn frame_box(&mut self, min: Vec3, max: Vec3) {
        let center = (min + max) * 0.5;
        let radius = ((max - min).length() * 0.5).max(0.25);
        match &mut self.ortho_height {
            Some(h) => {
                *h = radius * 2.2;
                let f = self.forward();
                self.position = center - f * (self.position - center).dot(f).abs().max(radius * 2.0);
            }
            None => {
                let half = (self.fov_y_degrees.to_radians() * 0.5).max(0.05);
                let distance = radius / half.sin() * 1.1;
                self.position = center - self.forward() * distance;
                self.speed = (distance * 0.75).clamp(0.5, 1000.0);
            }
        }
    }

    /// The view for an image of the given aspect ratio (width / height).
    pub fn view(&self, aspect: f32) -> View {
        let view = glam::camera::rh::view::look_to_mat4(self.position, self.forward(), Vec3::Y);
        let proj = match self.ortho_height {
            Some(height) => {
                let (hw, hh) = (height * aspect * 0.5, height * 0.5);
                glam::camera::rh::proj::directx::orthographic(-hw, hw, -hh, hh, self.near, self.far)
            }
            None => glam::camera::rh::proj::directx::perspective(
                self.fov_y_degrees.to_radians(),
                aspect,
                self.near,
                self.far,
            ),
        };
        View {
            view,
            proj,
            eye: self.position,
            clear: self.clear,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use marionette_core::spatial::{Camera, Color};

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn starts_where_the_scene_camera_looks() {
        let world =
            glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 5.0, 10.0), Vec3::ZERO, Vec3::Y).inverse();
        let cam = CameraData {
            entity: 1,
            camera: Camera {
                projection: Projection::Perspective {
                    fov_y_degrees: 50.0,
                    near: 0.1,
                    far: 100.0,
                },
                clear_color: Color {
                    r: 0.1,
                    g: 0.2,
                    b: 0.3,
                    a: 1.0,
                },
            },
            world,
        };
        let fly = FlyCamera::from_scene_camera(&cam);
        assert!(close(fly.position, Vec3::new(0.0, 5.0, 10.0)));
        assert!(close(fly.forward(), Vec3::new(0.0, -5.0, -10.0).normalize()));
        assert_eq!(fly.fov_y_degrees, 50.0);
        assert_eq!(fly.clear, [0.1, 0.2, 0.3, 1.0]);
        // The editor view equals the scene camera's view (same place, same direction).
        let a = fly.view(16.0 / 9.0).view;
        let b = cam.view(16.0 / 9.0).view;
        assert!(a.abs_diff_eq(b, 1e-4), "{a} vs {b}");
    }

    #[test]
    fn flying_looking_and_framing() {
        let mut c = FlyCamera {
            position: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            speed: 2.0,
            ..FlyCamera::default()
        };
        c.fly(Vec3::Z, false, 0.5);
        assert!(close(c.position, Vec3::new(0.0, 0.0, -1.0)));
        c.fly(Vec3::X, true, 0.5);
        assert!(close(c.position, Vec3::new(3.0, 0.0, -1.0)));
        c.look(0.0, -10_000.0);
        assert!((c.pitch - MAX_PITCH).abs() < 1e-6, "pitch is clamped");
        c.pitch = 0.0;
        c.look(std::f32::consts::FRAC_PI_2 / LOOK_PER_PIXEL, 0.0);
        assert!(close(c.forward(), Vec3::X), "turning right faces +X");
        c.frame_box(Vec3::splat(-1.0), Vec3::splat(1.0));
        // The box center is straight ahead and the whole box is inside the view cone.
        let to_center = -c.position;
        assert!(close(to_center.normalize(), c.forward()));
        let half = (c.fov_y_degrees.to_radians() * 0.5).sin();
        assert!(3f32.sqrt() / to_center.length() <= half);
        c.scroll(1.0);
        assert!(c.speed > 0.0);
    }

    #[test]
    fn orthographic_views_pan_and_zoom_without_turning() {
        let mut c = FlyCamera {
            position: Vec3::new(0.0, 0.0, 10.0),
            yaw: 0.0,
            pitch: 0.0,
            ortho_height: Some(10.0),
            ..FlyCamera::default()
        };
        c.look(500.0, 500.0);
        assert_eq!((c.yaw, c.pitch), (0.0, 0.0));
        c.pan(100.0, 0.0, 100.0, 1.0);
        assert!(close(c.position, Vec3::new(-10.0, 0.0, 10.0)), "{}", c.position);
        c.fly(Vec3::Z, false, 1.0);
        assert!(c.position.y > 0.0, "forward moves up the screen in 2D");
        c.scroll(2.0);
        assert!(c.ortho_height.unwrap() < 10.0);
    }
}
