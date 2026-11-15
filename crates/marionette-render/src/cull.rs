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

use glam::{Mat4, Vec2, Vec3, Vec4};

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
    /// Their view-projections.
    shadow_mats: Vec<Mat4>,
    /// Ray-traced frames (decision 0093) keep everything: rays see behind the camera.
    keep_all: bool,
    /// Shadow-casting point and spot lights: (position, range).
    lamps: Vec<(Vec3, f32)>,
    /// Screen-size measure from the first view: (view-projection, vertical scale).
    first_view: Option<(Mat4, f32)>,
    lod: Option<Lod>,
    /// Per sun shadow map, where the views' pixels can read it (decision 0109); `None` tests
    /// only the map's volume.
    receivers: Vec<Option<Receivers>>,
    /// Per sun map, the rows taking the first map's clip position to this map's clip x and y
    /// (both maps orthographic): one matrix product per caster, then two dot products per map.
    from_first: Vec<Option<(Vec4, Vec4)>>,
}

/// The part of an orthographic sun shadow map that the views' pixels can read (decision
/// 0109): the views' frustums seen along the light, as a convex polygon in the map's clip xy,
/// widened by `pad` for the filter taps and the normal offset. A caster whose circle misses it
/// writes only texels no pixel reads.
#[derive(Debug, Clone)]
pub struct Receivers {
    /// Edge lines `n.x * x + n.y * y + d >= 0` inside, `n` of unit length.
    edges: Vec<Vec3>,
    /// The map's view-projection: clip xy of a world point, and world radius to clip.
    map: Mat4,
    radius_scale: f32,
    pad: f32,
}

impl Receivers {
    /// `None` when the map is not orthographic or the views give no area.
    pub fn new(map: &Mat4, views: &[Mat4], pad: f32) -> Option<Self> {
        if map.row(3) != Vec4::W {
            return None;
        }
        let mut points = Vec::with_capacity(views.len() * 8);
        for vp in views {
            let inv = vp.inverse();
            for &z in &[0.0, 1.0] {
                for &(x, y) in &[(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let world = inv.project_point3(Vec3::new(x, y, z));
                    if !world.is_finite() {
                        return None;
                    }
                    points.push(map.transform_point3(world).truncate());
                }
            }
        }
        let hull = convex_hull(points);
        if hull.len() < 3 {
            return None;
        }
        let edges = (0..hull.len())
            .map(|i| {
                let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
                let n = Vec2::new(a.y - b.y, b.x - a.x).normalize_or_zero();
                n.extend(-n.dot(a))
            })
            .collect();
        Some(Self {
            edges,
            map: *map,
            radius_scale: map.row(0).truncate().length().max(map.row(1).truncate().length()),
            pad,
        })
    }

    /// The sphere's shadow can land where a pixel reads the map (conservative).
    pub fn touches(&self, s: &Sphere) -> bool {
        self.touches_at(self.map.transform_point3(s.center).truncate(), s.radius)
    }

    /// [`Self::touches`] with the sphere's center already in the map's clip xy.
    fn touches_at(&self, c: Vec2, radius: f32) -> bool {
        let reach = radius * self.radius_scale + self.pad;
        self.edges.iter().all(|e| e.x * c.x + e.y * c.y + e.z >= -reach)
    }
}

/// Convex hull, counter-clockwise, of a few points (Andrew's monotone chain).
fn convex_hull(mut points: Vec<Vec2>) -> Vec<Vec2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let mut hull: Vec<Vec2> = Vec::with_capacity(points.len() * 2);
    // Lower half left to right, then upper half right to left; each drops its last point,
    // which starts the other half.
    let add = |hull: &mut Vec<Vec2>, start: usize, p: Vec2| {
        while hull.len() >= start + 2
            && (hull[hull.len() - 1] - hull[hull.len() - 2]).perp_dot(p - hull[hull.len() - 2]) <= 0.0
        {
            hull.pop();
        }
        hull.push(p);
    };
    for &p in &points {
        add(&mut hull, 0, p);
    }
    hull.pop();
    let start = hull.len();
    for &p in points.iter().rev() {
        add(&mut hull, start, p);
    }
    hull.pop();
    hull
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
            shadow_mats: shadow.to_vec(),
            keep_all: false,
            lamps,
            first_view: views.first().map(|vp| (*vp, 1.0)),
            lod,
            receivers: Vec::new(),
            from_first: Vec::new(),
        }
    }

    /// Leaves casters out of a sun shadow map when no pixel of `views` can read their shadow
    /// there (decision 0109). `pad` is in the maps' clip units.
    pub fn with_shadow_receivers(mut self, views: &[Mat4], shadow: &[Mat4], pad: f32) -> Self {
        self.receivers = shadow.iter().map(|m| Receivers::new(m, views, pad)).collect();
        self.from_first = match shadow.first() {
            Some(first) => {
                let inv = first.inverse();
                let orthographic = |m: &Mat4| m.row(3) == Vec4::W;
                shadow
                    .iter()
                    .map(|m| {
                        let rel = *m * inv;
                        (orthographic(first) && orthographic(m)).then(|| (rel.row(0), rel.row(1)))
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        self
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

    /// Which sun shadow maps draw a world-space sphere: bit `k` is set when it touches the
    /// volume of map `k` (cascade `k`). The test is the same conservative one as
    /// [`Self::classify`] against the map's own clip volume, so leaving a caster out of a map it
    /// does not touch never changes a texel. Every bit when culling is off or the frame keeps
    /// everything.
    ///
    /// `seen` says the sphere is in a view ([`Class::Seen`]): its shadow then falls where that
    /// view reads the map, so the receiver test (decision 0109) is skipped. Skipping it only
    /// ever keeps a caster.
    pub fn shadow_maps(&self, s: &Sphere, seen: bool) -> u8 {
        if !self.culls_shadow_maps() {
            return u8::MAX;
        }
        // The volume test first: it rejects most maps for most casters, and the receiver test
        // reuses one projection of the center.
        let mut first: Option<Vec4> = None;
        self.shadow.iter().enumerate().fold(0, |mask, (k, f)| {
            if !f.touches(s) {
                return mask;
            }
            let read = match (
                self.receivers.get(k).and_then(Option::as_ref),
                self.from_first.get(k),
            ) {
                _ if seen => true,
                (None, _) => true,
                (Some(r), Some(Some((rx, ry)))) => {
                    let c0 = *first.get_or_insert_with(|| self.shadow_first_clip(s.center));
                    r.touches_at(Vec2::new(rx.dot(c0), ry.dot(c0)), s.radius)
                }
                (Some(r), _) => r.touches(s),
            };
            if read { mask | 1 << k } else { mask }
        })
    }

    /// Clip position (w = 1) of a world point in the first sun map.
    fn shadow_first_clip(&self, p: Vec3) -> Vec4 {
        self.shadow_mats
            .first()
            .map_or(Vec4::W, |m| m.transform_point3(p).extend(1.0))
    }

    /// Whether [`Self::shadow_maps`] can leave casters out (culling on, not keeping all).
    pub fn culls_shadow_maps(&self) -> bool {
        self.enabled && !self.keep_all
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
        out.push(meshopt::optimize_vertex_cache(&reduced, positions.len()));
    }
    out
}

#[cfg(not(feature = "lod"))]
pub fn reduced_levels(_indices: &[u32], _positions: &[[f32; 3]]) -> Vec<Vec<u32>> {
    Vec::new()
}

/// The same triangles reordered so the GPU reuses transformed vertices (meshoptimizer's
/// vertex cache order, decision 0301): every pass runs the vertex shader fewer times. Depth
/// is the same whatever the order, so shadow maps are unchanged; colors could differ only
/// where two triangles of one mesh are exactly coplanar and overlap. Unchanged when the `lod`
/// feature is off.
#[cfg(feature = "lod")]
pub fn cache_order(indices: &mut Vec<u32>, vertex_count: usize) {
    if indices.len() >= 3 && indices.iter().all(|&i| (i as usize) < vertex_count) {
        *indices = meshopt::optimize_vertex_cache(indices, vertex_count);
    }
}

#[cfg(not(feature = "lod"))]
pub fn cache_order(_indices: &mut Vec<u32>, _vertex_count: usize) {}

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
    fn hull_keeps_only_the_outside_points_counter_clockwise() {
        let pts = [
            (0.0, 0.0),
            (2.0, 0.0),
            (1.0, 1.0),
            (2.0, 2.0),
            (0.0, 2.0),
            (1.0, 0.0),
        ];
        let hull = convex_hull(pts.iter().map(|&(x, y)| Vec2::new(x, y)).collect());
        assert_eq!(hull.len(), 4, "{hull:?}");
        let area: f32 = (0..4).map(|i| hull[i].perp_dot(hull[(i + 1) % 4])).sum::<f32>() / 2.0;
        assert_eq!(area, 4.0, "counter-clockwise, full square");
    }

    #[test]
    fn sun_maps_skip_casters_whose_shadow_no_pixel_reads() {
        // Sun straight down over a camera looking along -z: pixels read the map only under the
        // view's wedge.
        let shadow = glam::camera::rh::proj::directx::orthographic(-200.0, 200.0, -200.0, 200.0, -50.0, 50.0)
            * glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO, Vec3::NEG_Z);
        let c = Culler::new(true, &[persp()], &[shadow], vec![], None).with_shadow_receivers(
            &[persp()],
            &[shadow],
            0.01,
        );
        assert_eq!(
            c.shadow_maps(&sphere(0.0, 0.0, -20.0, 1.0), false),
            1,
            "under the view"
        );
        assert_eq!(
            c.shadow_maps(&sphere(150.0, 0.0, 50.0, 1.0), false),
            0,
            "in the map, beside the view"
        );
        // Just outside the wedge's edge but within the radius: kept.
        let edge = 20.0 * 30f32.to_radians().tan();
        assert_eq!(c.shadow_maps(&sphere(edge + 0.5, 0.0, -20.0, 1.0), false), 1);
        let plain = Culler::new(true, &[persp()], &[shadow], vec![], None);
        assert_eq!(
            plain.shadow_maps(&sphere(150.0, 0.0, 50.0, 1.0), false),
            1,
            "volume test only"
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
