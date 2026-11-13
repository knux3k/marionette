//! Frustum culling and level of detail (decision 0080).
//!
//! Every model part gets a bounding sphere when it is uploaded. Each frame, before instances
//! are grouped into draws, every instance is put in one of three classes:
//!
//! - [`Class::Seen`]: inside at least one view's frustum. Drawn in every view and in shadow
//!   maps.
//! - [`Class::ShadowOnly`]: outside every view but inside a shadow map's volume (the
//!   directional shadow box, or the range of a shadow-casting point or spot light). Drawn
//!   into shadow maps only, so shadows from things behind the camera stay.
//! - [`Class::Hidden`]: neither. Not written to the instance buffer at all.
//!
//! The tests are conservative (a sphere that touches a plane counts as inside), so culling
//! never changes a pixel; the reference images prove that.

use glam::{Mat4, Vec3, Vec4};

/// A bounding sphere in the space of the vertices it was made from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sphere {
    pub center: Vec3,
    pub radius: f32,
}

impl Sphere {
    /// Sphere around `positions`: the box center and the farthest point from it. Not the
    /// smallest sphere, but cheap, stable and never too small. Empty input gives a point.
    pub fn around(positions: &[[f32; 3]]) -> Self {
        let Some(first) = positions.first() else {
            return Self {
                center: Vec3::ZERO,
                radius: 0.0,
            };
        };
        let (mut lo, mut hi) = (Vec3::from(*first), Vec3::from(*first));
        for p in positions {
            lo = lo.min(Vec3::from(*p));
            hi = hi.max(Vec3::from(*p));
        }
        let center = (lo + hi) * 0.5;
        let radius = positions
            .iter()
            .map(|p| Vec3::from(*p).distance_squared(center))
            .fold(0.0f32, f32::max)
            .sqrt();
        Self { center, radius }
    }

    /// The sphere after `m` (center moved, radius scaled by the largest axis scale).
    pub fn transformed(&self, m: &Mat4) -> Self {
        let scale = m
            .x_axis
            .truncate()
            .length_squared()
            .max(m.y_axis.truncate().length_squared())
            .max(m.z_axis.truncate().length_squared())
            .sqrt();
        Self {
            center: m.transform_point3(self.center),
            radius: self.radius * scale,
        }
    }
}

/// Six planes (`n·p + d >= 0` inside) taken from a view-projection matrix with depth 0..1
/// (wgpu's clip space). Works for perspective and orthographic projections.
#[derive(Debug, Clone, Copy)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    pub fn from_view_proj(m: &Mat4) -> Self {
        let r = |i: usize| m.row(i);
        let raw = [
            r(3) + r(0), // left
            r(3) - r(0), // right
            r(3) + r(1), // bottom
            r(3) - r(1), // top
            r(2),        // near (z >= 0)
            r(3) - r(2), // far (z <= w)
        ];
        let planes = raw.map(|p| {
            let len = p.truncate().length();
            if len > 0.0 { p / len } else { p }
        });
        Self { planes }
    }

    /// The sphere touches or is inside the frustum.
    pub fn touches(&self, s: &Sphere) -> bool {
        self.planes
            .iter()
            .all(|p| p.truncate().dot(s.center) + p.w >= -s.radius)
    }
}

/// Which passes draw an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Seen = 0,
    ShadowOnly = 1,
    Hidden = 2,
}

/// Level of detail settings for one frame (the `LevelOfDetail` resource).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lod {
    /// Below this screen fraction a part draws its first reduced level.
    pub medium_below: f32,
    /// Below this, its second reduced level.
    pub low_below: f32,
    /// Below this it is not drawn in views (shadows keep it).
    pub hide_below: f32,
}

/// Triangle share of each reduced level (level 1, level 2).
pub const LOD_RATIOS: [f32; 2] = [0.5, 0.25];

/// Parts with fewer triangles than this keep one level: reducing them saves nothing.
pub const LOD_MIN_TRIANGLES: usize = 64;

/// Everything one frame tests instances against.
pub struct Culler {
    /// Off: every instance is `Seen` at level 0 (the old behavior, for comparisons).
    pub enabled: bool,
    views: Vec<Frustum>,
    /// Sun shadow volumes (one per cascade).
    shadow: Vec<Frustum>,
    /// Ray-traced frames (decision 0093) keep everything: rays see behind the camera.
    keep_all: bool,
    /// Shadow-casting point and spot lights: (position, range).
    lamps: Vec<(Vec3, f32)>,
    /// Screen-size measure from the first view: (view-projection, vertical scale).
    first_view: Option<(Mat4, f32)>,
    lod: Option<Lod>,
}

impl Culler {
    pub fn new(
        enabled: bool,
        views: &[Mat4],
        shadow: &[Mat4],
        lamps: Vec<(Vec3, f32)>,
        lod: Option<Lod>,
    ) -> Self {
        Self {
            enabled,
            views: views.iter().map(Frustum::from_view_proj).collect(),
            shadow: shadow.iter().map(Frustum::from_view_proj).collect(),
            keep_all: false,
            lamps,
            first_view: views.first().map(|vp| (*vp, 1.0)),
            lod,
        }
    }

    /// Keeps every instance at least for shadows (never `Hidden`): ray-traced shadows and
    /// reflections need the whole scene in the acceleration structure.
    pub fn keeping_all(mut self, keep: bool) -> Self {
        self.keep_all = keep;
        self
    }

    /// Sets the vertical projection scale of the first view (`proj.y_axis.y`), which turns a
    /// radius into a share of the screen height.
    pub fn with_projection_scale(mut self, proj_y: f32) -> Self {
        if let Some((_, s)) = self.first_view.as_mut() {
            *s = proj_y.abs();
        }
        self
    }

    /// The class of a world-space sphere and the level to draw it at (0 = full detail).
    pub fn classify(&self, s: &Sphere, levels: usize) -> (Class, usize) {
        if !self.enabled {
            return (Class::Seen, 0);
        }
        let seen = self.views.iter().any(|f| f.touches(s));
        let mut level = 0;
        let mut hidden_by_size = false;
        if seen && let (Some(lod), Some((vp, scale))) = (self.lod, self.first_view) {
            // Diameter over screen height: radius * scale / w covers half the height per 1.
            let w = vp.row(3).dot(s.center.extend(1.0)).max(1e-4);
            let size = s.radius * scale / w;
            if size < lod.hide_below {
                hidden_by_size = true;
            } else if levels > 2 && size < lod.low_below {
                level = 2;
            } else if levels > 1 && size < lod.medium_below {
                level = 1;
            }
        }
        if seen && !hidden_by_size {
            return (Class::Seen, level);
        }
        let shadowed = self.keep_all
            || self.shadow.iter().any(|f| f.touches(s))
            || self
                .lamps
                .iter()
                .any(|(p, r)| p.distance_squared(s.center) <= (r + s.radius) * (r + s.radius));
        // Shadow maps always use full detail: a reduced caster would move shadow edges.
        if shadowed {
            (Class::ShadowOnly, 0)
        } else {
            (Class::Hidden, 0)
        }
    }
}

/// Index lists for the reduced levels of one part (meshoptimizer's simplifier). Empty when
/// the part is small or the `lod` feature is off.
#[cfg(feature = "lod")]
pub fn reduced_levels(indices: &[u32], positions: &[[f32; 3]]) -> Vec<Vec<u32>> {
    if indices.len() / 3 < LOD_MIN_TRIANGLES || positions.is_empty() {
        return Vec::new();
    }
    let adapter = match meshopt::VertexDataAdapter::new(bytemuck::cast_slice(positions), 12, 0) {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut last = indices.len();
    for ratio in LOD_RATIOS {
        let target = ((indices.len() / 3) as f32 * ratio) as usize * 3;
        // Error is relative to the mesh size; 5% keeps silhouettes at the distances used.
        let reduced = meshopt::simplify(
            indices,
            &adapter,
            target,
            0.05,
            meshopt::SimplifyOptions::None,
            None,
        );
        // A level that saved under 10% over the previous one is not worth a draw path.
        if reduced.is_empty() || reduced.len() * 10 > last * 9 {
            break;
        }
        last = reduced.len();
        out.push(reduced);
    }
    out
}

#[cfg(not(feature = "lod"))]
pub fn reduced_levels(_indices: &[u32], _positions: &[[f32; 3]]) -> Vec<Vec<u32>> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persp() -> Mat4 {
        let proj = glam::camera::rh::proj::directx::perspective(60f32.to_radians(), 1.0, 0.1, 100.0);
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::NEG_Z, Vec3::Y);
        proj * view
    }

    fn sphere(x: f32, y: f32, z: f32, r: f32) -> Sphere {
        Sphere {
            center: Vec3::new(x, y, z),
            radius: r,
        }
    }

    #[test]
    fn frustum_keeps_what_is_in_front_and_drops_what_is_behind() {
        let f = Frustum::from_view_proj(&persp());
        assert!(f.touches(&sphere(0.0, 0.0, -5.0, 0.5)));
        assert!(!f.touches(&sphere(0.0, 0.0, 5.0, 0.5)), "behind");
        assert!(!f.touches(&sphere(0.0, 0.0, -200.0, 0.5)), "past far");
        assert!(!f.touches(&sphere(50.0, 0.0, -5.0, 0.5)), "far right");
        // Straddling the right edge counts as inside.
        let edge = 5.0 * 30f32.to_radians().tan();
        assert!(f.touches(&sphere(edge + 0.3, 0.0, -5.0, 0.5)));
    }

    #[test]
    fn transformed_sphere_follows_translation_and_largest_scale() {
        let s = sphere(1.0, 0.0, 0.0, 1.0);
        let m = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 3.0, 1.0),
            glam::Quat::IDENTITY,
            Vec3::new(0.0, 10.0, 0.0),
        );
        let t = s.transformed(&m);
        assert_eq!(t.center, Vec3::new(2.0, 10.0, 0.0));
        assert!((t.radius - 3.0).abs() < 1e-6);
    }

    #[test]
    fn ray_traced_frames_keep_everything_for_rays() {
        let c = Culler::new(true, &[persp()], &[], vec![], None).keeping_all(true);
        assert_eq!(c.classify(&sphere(0.0, 0.0, 500.0, 0.5), 1).0, Class::ShadowOnly);
    }

    #[test]
    fn things_behind_the_camera_stay_in_shadow_maps_when_they_could_cast() {
        let shadow = glam::camera::rh::proj::directx::orthographic(-20.0, 20.0, -20.0, 20.0, -50.0, 50.0)
            * glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO, Vec3::Z);
        let c = Culler::new(true, &[persp()], &[shadow], vec![], None);
        assert_eq!(c.classify(&sphere(0.0, 0.0, 5.0, 0.5), 1).0, Class::ShadowOnly);
        assert_eq!(c.classify(&sphere(0.0, 0.0, 500.0, 0.5), 1).0, Class::Hidden);
        let lamp = Culler::new(
            true,
            &[persp()],
            &[],
            vec![(Vec3::new(0.0, 0.0, 600.0), 150.0)],
            None,
        );
        assert_eq!(
            lamp.classify(&sphere(0.0, 0.0, 500.0, 0.5), 1).0,
            Class::ShadowOnly
        );
    }

    #[test]
    fn far_small_things_use_reduced_levels() {
        let lod = Lod {
            medium_below: 0.2,
            low_below: 0.05,
            hide_below: 0.005,
        };
        let proj_y = 1.0 / 30f32.to_radians().tan();
        let c = Culler::new(true, &[persp()], &[], vec![], Some(lod)).with_projection_scale(proj_y);
        assert_eq!(c.classify(&sphere(0.0, 0.0, -2.0, 1.0), 3), (Class::Seen, 0));
        assert_eq!(c.classify(&sphere(0.0, 0.0, -20.0, 1.0), 3), (Class::Seen, 1));
        assert_eq!(c.classify(&sphere(0.0, 0.0, -60.0, 1.0), 3), (Class::Seen, 2));
        // A part with fewer levels uses the most reduced one it has.
        assert_eq!(c.classify(&sphere(0.0, 0.0, -60.0, 1.0), 2), (Class::Seen, 1));
        assert_eq!(c.classify(&sphere(0.0, 0.0, -60.0, 0.01), 3).0, Class::Hidden);
        let off = Culler::new(false, &[persp()], &[], vec![], Some(lod));
        assert_eq!(off.classify(&sphere(0.0, 0.0, 50.0, 1.0), 3), (Class::Seen, 0));
    }

    #[cfg(feature = "lod")]
    #[test]
    fn a_dense_grid_gets_two_smaller_levels() {
        let n = 40u32;
        let mut pos = Vec::new();
        for y in 0..=n {
            for x in 0..=n {
                let (fx, fy) = (x as f32 / n as f32, y as f32 / n as f32);
                pos.push([fx, fy, (fx * 6.0).sin() * (fy * 5.0).cos() * 0.2]);
            }
        }
        let mut idx = Vec::new();
        for y in 0..n {
            for x in 0..n {
                let a = y * (n + 1) + x;
                idx.extend_from_slice(&[a, a + 1, a + n + 2, a, a + n + 2, a + n + 1]);
            }
        }
        let levels = reduced_levels(&idx, &pos);
        assert_eq!(
            levels.len(),
            2,
            "{:?}",
            levels.iter().map(Vec::len).collect::<Vec<_>>()
        );
        assert!(levels[0].len() < idx.len() * 6 / 10);
        assert!(levels[1].len() < levels[0].len());
        assert!(
            reduced_levels(&idx[..60], &pos).is_empty(),
            "small parts keep one level"
        );
    }
}
