//! The two expansions of the backend and their dimension-specific conversions.

/// Rapier 3D.
pub mod d3 {
    use rapier3d::control::KinematicCharacterController;
    use rapier3d::prelude::*;

    pub(crate) mod conv {
        use crate::components::ColliderShape;
        use rapier3d::prelude::*;
        use std::path::Path;

        pub const DIM: u32 = 3;

        pub fn vector(v: [f32; 3]) -> Vector {
            Vector::new(v[0], v[1], v[2])
        }
        pub fn to3(v: Vector) -> [f32; 3] {
            [v.x, v.y, v.z]
        }
        pub fn angular(v: [f32; 3]) -> AngVector {
            Vector::new(v[0], v[1], v[2])
        }
        pub fn angular_to3(b: &RigidBody) -> [f32; 3] {
            to3(b.angvel())
        }
        pub fn pose(t: [f32; 3], q: [f32; 4]) -> Pose {
            Pose::from_parts(vector(t), Rotation::from_xyzw(q[0], q[1], q[2], q[3]).normalize())
        }
        pub fn from_pose(p: &Pose) -> ([f32; 3], [f32; 4]) {
            let r = p.rotation;
            (to3(p.translation), [r.x, r.y, r.z, r.w])
        }
        /// Desired character movement: x sideways, forward toward -Z, dy up.
        pub fn walk(dx: f32, dforward: f32, dy: f32) -> Vector {
            Vector::new(dx, dy, -dforward)
        }
        /// Turns horizontal movement by the character's facing (its rotation, normally a yaw
        /// set by a script), keeping its length; vertical movement is unchanged.
        pub fn face(q: [f32; 4], v: Vector) -> Vector {
            if q == [0.0, 0.0, 0.0, 1.0] {
                return v;
            }
            let h = Rotation::from_xyzw(q[0], q[1], q[2], q[3]).normalize() * Vector::new(v.x, 0.0, v.z);
            let (len_in, len) = ((v.x * v.x + v.z * v.z).sqrt(), (h.x * h.x + h.z * h.z).sqrt());
            if len < 1e-6 {
                return v;
            }
            Vector::new(h.x / len * len_in, v.y, h.z / len * len_in)
        }
        /// A character's body takes its rotation from `Transform` (scripts turn it).
        pub fn turn(body: &mut RigidBody, q: [f32; 4]) {
            body.set_next_kinematic_rotation(Rotation::from_xyzw(q[0], q[1], q[2], q[3]).normalize());
        }

        pub fn shape(s: &ColliderShape, k: [f32; 3], dir: &Path) -> Result<ColliderBuilder, String> {
            Ok(match s {
                ColliderShape::Cuboid { half_extents: h } => {
                    ColliderBuilder::cuboid(h.x * k[0], h.y * k[1], h.z * k[2])
                }
                ColliderShape::Ball { radius } => ColliderBuilder::ball(radius * k[0].max(k[1]).max(k[2])),
                ColliderShape::Capsule { half_height, radius } => {
                    ColliderBuilder::capsule_y(half_height * k[1], radius * k[0].max(k[2]))
                }
                ColliderShape::Cylinder { half_height, radius } => {
                    ColliderBuilder::cylinder(half_height * k[1], radius * k[0].max(k[2]))
                }
                ColliderShape::ConvexHull { model } => {
                    let (points, _) = mesh(model, k, dir)?;
                    ColliderBuilder::convex_hull(&points)
                        .ok_or_else(|| format!("cannot build a convex hull of `{model}`"))?
                }
                ColliderShape::TriMesh { model } => {
                    let (points, tris) = mesh(model, k, dir)?;
                    ColliderBuilder::trimesh(points, tris)
                        .map_err(|e| format!("bad triangle mesh `{model}`: {e:?}"))?
                }
            })
        }

        /// All triangles of a model, scaled, in model space.
        fn mesh(model: &str, k: [f32; 3], dir: &Path) -> Result<(Vec<Vector>, Vec<[u32; 3]>), String> {
            let m = marionette_assets::load_model(model, dir)?;
            let (mut points, mut tris) = (Vec::new(), Vec::new());
            for part in &m.parts {
                let base = points.len() as u32;
                for p in &part.mesh.positions {
                    let q = part
                        .transform
                        .transform_point3(marionette_assets::glam::Vec3::from(*p));
                    points.push(Vector::new(q.x * k[0], q.y * k[1], q.z * k[2]));
                }
                for t in part.mesh.indices.as_chunks::<3>().0 {
                    tris.push([base + t[0], base + t[1], base + t[2]]);
                }
            }
            Ok((points, tris))
        }
    }

    crate::backend::backend!();
}

/// Rapier 2D in the XY plane.
pub mod d2 {
    use rapier2d::control::KinematicCharacterController;
    use rapier2d::prelude::*;

    pub(crate) mod conv {
        use crate::components::ColliderShape;
        use rapier2d::prelude::*;
        use std::path::Path;

        pub const DIM: u32 = 2;

        pub fn vector(v: [f32; 3]) -> Vector {
            Vector::new(v[0], v[1])
        }
        pub fn to3(v: Vector) -> [f32; 3] {
            [v.x, v.y, 0.0]
        }
        pub fn angular(v: [f32; 3]) -> AngVector {
            v[2]
        }
        pub fn angular_to3(b: &RigidBody) -> [f32; 3] {
            [0.0, 0.0, b.angvel()]
        }
        /// The rotation about Z of a quaternion (only Z rotation is meaningful in 2D).
        pub fn pose(t: [f32; 3], q: [f32; 4]) -> Pose {
            let angle = 2.0 * q[2].atan2(q[3]);
            Pose::from_parts(vector(t), Rotation::new(angle))
        }
        /// Half-angle quaternion from cos/sin without trigonometry.
        pub fn from_pose(p: &Pose) -> ([f32; 3], [f32; 4]) {
            let (c, s) = (p.rotation.cos(), p.rotation.sin());
            let w = ((1.0 + c) * 0.5).max(0.0).sqrt();
            let z = ((1.0 - c) * 0.5).max(0.0).sqrt();
            let z = if s < 0.0 { -z } else { z };
            (to3(p.translation), [0.0, 0.0, z, w])
        }
        pub fn walk(dx: f32, _dforward: f32, dy: f32) -> Vector {
            Vector::new(dx, dy)
        }
        /// 2D characters move in X only; facing does not turn movement.
        pub fn face(_q: [f32; 4], v: Vector) -> Vector {
            v
        }
        /// 2D character bodies keep their rotation.
        pub fn turn(_body: &mut RigidBody, _q: [f32; 4]) {}

        pub fn shape(s: &ColliderShape, k: [f32; 3], _dir: &Path) -> Result<ColliderBuilder, String> {
            Ok(match s {
                ColliderShape::Cuboid { half_extents: h } => ColliderBuilder::cuboid(h.x * k[0], h.y * k[1]),
                ColliderShape::Ball { radius } => ColliderBuilder::ball(radius * k[0].max(k[1])),
                ColliderShape::Capsule { half_height, radius } => {
                    ColliderBuilder::capsule_y(half_height * k[1], radius * k[0])
                }
                other => {
                    return Err(format!(
                        "{} is not available in 2D physics; use Cuboid, Ball or Capsule",
                        match other {
                            ColliderShape::Cylinder { .. } => "Cylinder",
                            ColliderShape::ConvexHull { .. } => "ConvexHull",
                            _ => "TriMesh",
                        }
                    ));
                }
            })
        }
    }

    crate::backend::backend!();
}
