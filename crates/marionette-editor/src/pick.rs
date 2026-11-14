//! Clicking things: screen rays, what each entity occupies, and hits sorted front to back.
//!
//! Everything here reads a [`FrameData`] and the world; nothing writes. Models are tested
//! against their rest-pose bounding box in model space (an oriented box in the world),
//! sprites against their quad, and entities with a place but nothing to draw (cameras,
//! lights, spawn points) against a marker a few pixels wide on screen.

use bevy_ecs::entity::Entity;
use bevy_ecs::world::World;
use glam::{Mat4, Vec2, Vec3, Vec4};
use marionette_core::builtin::Position;
use marionette_core::scene::SceneId;
use marionette_core::spatial::{
    Camera, DirectionalLight, LookAt, Model, PointLight, SpotLight, Sprite, Transform,
};
use marionette_core::ui::UiNode;
use marionette_render::{FrameData, View};

/// Screen radius in pixels within which a marker counts as clicked.
pub const MARKER_PIXELS: f32 = 10.0;

/// Half size of the box assumed for a model whose bounds are not known yet.
const UNKNOWN_HALF: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    /// Unit length.
    pub dir: Vec3,
}

/// What an entity occupies for picking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A box (`min`, `max`) in the space of `world`.
    Box { world: Mat4, min: Vec3, max: Vec3 },
    /// A point drawn as a marker (camera, light, empty).
    Marker { position: Vec3, letter: char },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pickable {
    pub entity: u64,
    pub shape: Shape,
    /// Sprite layer (0 for everything else): at equal distance the higher layer, then the
    /// higher id, is drawn on top and is hit first.
    pub layer: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub entity: u64,
    /// Distance along the ray in world units.
    pub distance: f32,
    layer: i32,
}

/// The ray through pixel (`x`, `y`) of a `w` x `h` image (y down) seen through `view`.
pub fn screen_ray(view: &View, x: f32, y: f32, w: f32, h: f32) -> Ray {
    let ndc = Vec2::new(2.0 * x / w.max(1.0) - 1.0, 1.0 - 2.0 * y / h.max(1.0));
    let inv = (view.proj * view.view).inverse();
    let near = inv.project_point3(ndc.extend(0.0));
    let far = inv.project_point3(ndc.extend(1.0));
    Ray {
        origin: near,
        dir: (far - near).normalize_or(Vec3::NEG_Z),
    }
}

/// Pixel position (y down) and depth (0..1) of a world point, or `None` when it is behind
/// the camera or outside the depth range.
pub fn project(view: &View, p: Vec3, w: f32, h: f32) -> Option<(Vec2, f32)> {
    let clip = view.proj * view.view * Vec4::new(p.x, p.y, p.z, 1.0);
    if clip.w <= 1e-6 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    if !(0.0..=1.0).contains(&ndc.z) {
        return None;
    }
    Some((Vec2::new((ndc.x + 1.0) * 0.5 * w, (1.0 - ndc.y) * 0.5 * h), ndc.z))
}

/// Distance along `ray` to where it enters the box (`min`, `max`) in the space of `world`
/// (0 when it starts inside), or `None` when it misses.
pub fn ray_box(ray: &Ray, world: &Mat4, min: Vec3, max: Vec3) -> Option<f32> {
    if world.determinant().abs() < 1e-12 {
        return None;
    }
    let inv = world.inverse();
    // A point at distance t along the world ray maps to o + t * d in box space.
    let o = inv.transform_point3(ray.origin);
    let d = inv.transform_vector3(ray.dir);
    let (mut t0, mut t1) = (f32::NEG_INFINITY, f32::INFINITY);
    for i in 0..3 {
        if d[i].abs() < 1e-12 {
            if o[i] < min[i] || o[i] > max[i] {
                return None;
            }
        } else {
            let (a, b) = ((min[i] - o[i]) / d[i], (max[i] - o[i]) / d[i]);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
        }
    }
    (t1 >= t0 && t1 >= 0.0).then_some(t0.max(0.0))
}

/// The letter a marker shows for an entity with no model or sprite.
fn marker_letter(world: &World, e: Entity) -> char {
    if world.get::<Camera>(e).is_some() {
        'C'
    } else if world.get::<PointLight>(e).is_some()
        || world.get::<SpotLight>(e).is_some()
        || world.get::<DirectionalLight>(e).is_some()
    {
        'L'
    } else {
        '+'
    }
}

/// Every entity that can be clicked, sorted by id. `bounds` gives a model's model-space box
/// (`Renderer::model_bounds`); models without one count as a 1 m cube.
pub fn pickables(
    frame: &FrameData,
    world: &World,
    bounds: &dyn Fn(&str) -> Option<(Vec3, Vec3)>,
) -> Vec<Pickable> {
    let mut out: Vec<Pickable> = Vec::with_capacity(frame.meshes.len() + frame.sprites.len());
    for m in &frame.meshes {
        let (min, max) = bounds(&m.asset).unwrap_or((Vec3::splat(-UNKNOWN_HALF), Vec3::splat(UNKNOWN_HALF)));
        out.push(Pickable {
            entity: m.entity,
            shape: Shape::Box {
                world: m.world,
                min,
                max,
            },
            layer: 0,
        });
    }
    for s in &frame.sprites {
        let half = (s.size * 0.5).extend(0.005);
        out.push(Pickable {
            entity: s.entity,
            shape: Shape::Box {
                world: s.world,
                min: -half,
                max: half,
            },
            layer: s.layer,
        });
    }
    if let Some(mut q) = world.try_query::<(
        Entity,
        &SceneId,
        Option<&Position>,
        Option<&Transform>,
        Option<&LookAt>,
    )>() {
        for (e, id, p, t, l) in q.iter(world) {
            if p.is_none() && t.is_none() {
                continue;
            }
            if world.get::<Model>(e).is_some()
                || world.get::<Sprite>(e).is_some()
                || world.get::<UiNode>(e).is_some()
            {
                continue;
            }
            let position = marionette_render::extract::world_matrix(t, p, l)
                .w_axis
                .truncate();
            out.push(Pickable {
                entity: id.0,
                shape: Shape::Marker {
                    position,
                    letter: marker_letter(world, e),
                },
                layer: 0,
            });
        }
    }
    out.sort_by_key(|p| p.entity);
    out
}

/// Everything under pixel (`x`, `y`), nearest first.
pub fn hits(items: &[Pickable], view: &View, x: f32, y: f32, w: f32, h: f32) -> Vec<Hit> {
    let ray = screen_ray(view, x, y, w, h);
    let mut out: Vec<Hit> = items
        .iter()
        .filter_map(|item| {
            let distance = match item.shape {
                Shape::Box { world, min, max } => ray_box(&ray, &world, min, max)?,
                Shape::Marker { position, .. } => {
                    let (screen, _) = project(view, position, w, h)?;
                    if screen.distance(Vec2::new(x, y)) > MARKER_PIXELS {
                        return None;
                    }
                    (position - ray.origin).dot(ray.dir).max(0.0)
                }
            };
            Some(Hit {
                entity: item.entity,
                distance,
                layer: item.layer,
            })
        })
        .collect();
    // Nearest first (to the millimeter); among equals, what is drawn on top.
    let mm = |d: f32| (d as f64 * 1000.0).round() as i64;
    out.sort_by(|a, b| {
        mm(a.distance)
            .cmp(&mm(b.distance))
            .then(b.layer.cmp(&a.layer))
            .then(b.entity.cmp(&a.entity))
    });
    out
}

/// The eight corners of a shape in world space (a marker is a point).
pub fn corners(shape: &Shape) -> Vec<Vec3> {
    match *shape {
        Shape::Box { world, min, max } => (0..8)
            .map(|i| {
                let c = Vec3::new(
                    if i & 1 == 0 { min.x } else { max.x },
                    if i & 2 == 0 { min.y } else { max.y },
                    if i & 4 == 0 { min.z } else { max.z },
                );
                world.transform_point3(c)
            })
            .collect(),
        Shape::Marker { position, .. } => vec![position],
    }
}

/// World-space box around the given items (for framing), or `None` when empty.
pub fn world_bounds<'a>(items: impl IntoIterator<Item = &'a Pickable>) -> Option<(Vec3, Vec3)> {
    let mut b: Option<(Vec3, Vec3)> = None;
    for item in items {
        for c in corners(&item.shape) {
            b = Some(b.map_or((c, c), |(lo, hi)| (lo.min(c), hi.max(c))));
        }
    }
    b
}

/// Screen rectangle (min, max pixels) around a shape's visible corners, or `None` when no
/// corner is in front of the camera.
pub fn screen_rect(shape: &Shape, view: &View, w: f32, h: f32) -> Option<(Vec2, Vec2)> {
    let mut r: Option<(Vec2, Vec2)> = None;
    for c in corners(shape) {
        if let Some((p, _)) = project(view, c, w, h) {
            r = Some(r.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::FlyCamera;

    fn cam() -> View {
        FlyCamera {
            position: Vec3::new(0.0, 0.0, 10.0),
            yaw: 0.0,
            pitch: 0.0,
            ..FlyCamera::default()
        }
        .view(1.0)
    }

    fn cube(entity: u64, at: Vec3) -> Pickable {
        Pickable {
            entity,
            shape: Shape::Box {
                world: Mat4::from_translation(at),
                min: Vec3::splat(-0.5),
                max: Vec3::splat(0.5),
            },
            layer: 0,
        }
    }

    #[test]
    fn the_center_ray_looks_straight_ahead() {
        let r = screen_ray(&cam(), 50.0, 50.0, 100.0, 100.0);
        assert!((r.dir - Vec3::NEG_Z).length() < 1e-4);
        let (p, _) = project(&cam(), Vec3::ZERO, 100.0, 100.0).unwrap();
        assert!((p - Vec2::splat(50.0)).length() < 1e-3);
        assert_eq!(
            project(&cam(), Vec3::new(0.0, 0.0, 20.0), 100.0, 100.0),
            None,
            "behind"
        );
    }

    #[test]
    fn nearest_box_comes_first_and_misses_are_left_out() {
        let items = [
            cube(7, Vec3::new(0.0, 0.0, -3.0)),
            cube(2, Vec3::ZERO),
            cube(9, Vec3::new(4.0, 0.0, 0.0)),
        ];
        let hits = hits(&items, &cam(), 50.0, 50.0, 100.0, 100.0);
        let ids: Vec<u64> = hits.iter().map(|h| h.entity).collect();
        assert_eq!(ids, vec![2, 7]);
        assert!(
            (hits[0].distance - (10.0 - 0.5 - 0.05)).abs() < 1e-3,
            "{}",
            hits[0].distance
        );
    }

    #[test]
    fn rotated_and_scaled_boxes() {
        // A long thin box turned 90 degrees around Y lies along X.
        let world = Mat4::from_scale_rotation_translation(
            Vec3::new(1.0, 1.0, 1.0),
            glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::ZERO,
        );
        let item = Pickable {
            entity: 1,
            shape: Shape::Box {
                world,
                min: Vec3::new(-0.1, -0.1, -3.0),
                max: Vec3::new(0.1, 0.1, 3.0),
            },
            layer: 0,
        };
        let v = cam();
        let (right, _) = project(&v, Vec3::new(2.5, 0.0, 0.0), 100.0, 100.0).unwrap();
        assert_eq!(hits(&[item], &v, right.x, right.y, 100.0, 100.0).len(), 1);
        let (above, _) = project(&v, Vec3::new(0.0, 2.0, 0.0), 100.0, 100.0).unwrap();
        assert!(hits(&[item], &v, above.x, above.y, 100.0, 100.0).is_empty());
    }

    #[test]
    fn markers_are_hit_within_a_few_pixels() {
        let item = Pickable {
            entity: 4,
            shape: Shape::Marker {
                position: Vec3::new(1.0, 0.0, 0.0),
                letter: 'L',
            },
            layer: 0,
        };
        let v = cam();
        let (p, _) = project(&v, Vec3::new(1.0, 0.0, 0.0), 400.0, 400.0).unwrap();
        assert_eq!(hits(&[item], &v, p.x + 6.0, p.y, 400.0, 400.0).len(), 1);
        assert!(hits(&[item], &v, p.x + 30.0, p.y, 400.0, 400.0).is_empty());
    }

    #[test]
    fn at_equal_distance_the_top_sprite_wins() {
        let quad = |entity, layer| Pickable {
            entity,
            shape: Shape::Box {
                world: Mat4::IDENTITY,
                min: Vec3::new(-1.0, -1.0, -0.005),
                max: Vec3::new(1.0, 1.0, 0.005),
            },
            layer,
        };
        let items = [quad(2, -10), quad(21, 0), quad(30, 0)];
        let ids: Vec<u64> = hits(&items, &cam(), 50.0, 50.0, 100.0, 100.0)
            .iter()
            .map(|h| h.entity)
            .collect();
        assert_eq!(ids, vec![30, 21, 2]);
    }

    #[test]
    fn bounds_and_screen_rects() {
        let items = [cube(1, Vec3::ZERO), cube(2, Vec3::new(3.0, 0.0, 0.0))];
        let (lo, hi) = world_bounds(&items).unwrap();
        assert!((lo - Vec3::splat(-0.5)).length() < 1e-5 && (hi - Vec3::new(3.5, 0.5, 0.5)).length() < 1e-5);
        let (a, b) = screen_rect(&items[0].shape, &cam(), 100.0, 100.0).unwrap();
        assert!(a.x < 50.0 && b.x > 50.0 && a.y < 50.0 && b.y > 50.0);
    }
}
