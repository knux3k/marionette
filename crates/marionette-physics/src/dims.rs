//! The two expansions of the backend and their dimension-specific conversions.

/// How hard a joint motor pulls toward its target speed per unit of speed error (Rapier's
/// velocity "factor"); `motor_force` caps it. High enough to reach the speed within a few
/// ticks, as Unity's motors do.
pub(crate) const MOTOR_GAIN: f32 = 100.0;

/// Rapier 3D.
pub mod d3 {
    use rapier3d::control::KinematicCharacterController;
    use rapier3d::prelude::*;

    pub(crate) mod conv {
        use crate::components::{ColliderShape, JointKind};
        use marionette_core::mesh::GameMeshes;
        use rapier3d::prelude::*;
        use std::path::Path;

        pub use rapier3d::parry::query::ShapeCastOptions;

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

        /// Adds a torque (newton-metres) to the body's accumulated forces.
        pub fn add_torque(b: &mut RigidBody, t: [f32; 3]) {
            b.add_torque(vector(t), true);
        }
        pub fn torque_impulse(b: &mut RigidBody, t: [f32; 3]) {
            if t != [0.0; 3] {
                b.apply_torque_impulse(vector(t), true);
            }
        }

        /// A unit axis, or +Y when the given one has no length.
        fn axis(a: [f32; 3]) -> Vector {
            let v = vector(a).normalize_or_zero();
            if v == Vector::ZERO { Vector::Y } else { v }
        }

        /// The Rapier joint of a `JointKind` (decision 0310).
        pub fn joint(
            k: &JointKind,
            a1: [f32; 3],
            a2: [f32; 3],
            contacts: bool,
            rel: Rotation,
        ) -> GenericJoint {
            let (a1, a2) = (vector(a1), vector(a2));
            let mut j: GenericJoint = match *k {
                JointKind::Fixed => FixedJointBuilder::new()
                    .local_anchor1(a1)
                    .local_anchor2(a2)
                    .build()
                    .into(),
                JointKind::Ball => SphericalJointBuilder::new()
                    .local_anchor1(a1)
                    .local_anchor2(a2)
                    .build()
                    .into(),
                JointKind::Hinge {
                    axis: x,
                    min,
                    max,
                    motor_speed,
                    motor_force,
                } => {
                    let mut b = RevoluteJointBuilder::new(axis([x.x, x.y, x.z]))
                        .local_anchor1(a1)
                        .local_anchor2(a2);
                    if min < max {
                        b = b.limits([min, max]);
                    }
                    if motor_force > 0.0 {
                        b = b
                            .motor_velocity(motor_speed, crate::dims::MOTOR_GAIN)
                            .motor_max_force(motor_force);
                    }
                    b.build().into()
                }
                JointKind::Slider {
                    axis: x,
                    min,
                    max,
                    motor_speed,
                    motor_force,
                } => {
                    let mut b = PrismaticJointBuilder::new(axis([x.x, x.y, x.z]))
                        .local_anchor1(a1)
                        .local_anchor2(a2);
                    if min < max {
                        b = b.limits([min, max]);
                    }
                    if motor_force > 0.0 {
                        b = b
                            .motor_velocity(motor_speed, crate::dims::MOTOR_GAIN)
                            .motor_max_force(motor_force);
                    }
                    b.build().into()
                }
                JointKind::Rope { max_length } => RopeJointBuilder::new(max_length)
                    .local_anchor1(a1)
                    .local_anchor2(a2)
                    .build()
                    .into(),
                JointKind::Spring {
                    rest_length,
                    stiffness,
                    damping,
                } => SpringJointBuilder::new(rest_length, stiffness, damping)
                    .local_anchor1(a1)
                    .local_anchor2(a2)
                    .build()
                    .into(),
            };
            j.set_contacts_enabled(contacts);
            // The kind's frame on the second body is in that body's space; the first body's
            // frame is the same frame seen from the first body at build time.
            let mut f1 = j.local_frame1;
            f1.rotation = rel * j.local_frame2.rotation;
            j.set_local_frame1(f1);
            j
        }

        pub fn shape(
            s: &ColliderShape,
            k: [f32; 3],
            dir: &Path,
            meshes: Option<&GameMeshes>,
        ) -> Result<ColliderBuilder, String> {
            Ok(match s {
                ColliderShape::Cuboid { half_extents: h } => {
                    ColliderBuilder::cuboid(h.x * k[0], h.y * k[1], h.z * k[2])
                }
                ColliderShape::RoundCuboid {
                    half_extents: h,
                    border,
                } => {
                    let b = border.max(0.0);
                    let inner = |v: f32| (v - b).max(0.0);
                    ColliderBuilder::round_cuboid(inner(h.x * k[0]), inner(h.y * k[1]), inner(h.z * k[2]), b)
                }
                ColliderShape::Ball { radius } => ColliderBuilder::ball(radius * k[0].max(k[1]).max(k[2])),
                ColliderShape::Capsule { half_height, radius } => {
                    ColliderBuilder::capsule_y(half_height * k[1], radius * k[0].max(k[2]))
                }
                ColliderShape::Cylinder { half_height, radius } => {
                    ColliderBuilder::cylinder(half_height * k[1], radius * k[0].max(k[2]))
                }
                ColliderShape::ConvexHull { model } => {
                    let (points, _) = mesh(model, k, dir, meshes)?;
                    ColliderBuilder::convex_hull(&points)
                        .ok_or_else(|| format!("cannot build a convex hull of `{model}`"))?
                }
                ColliderShape::TriMesh { model } => {
                    let (points, tris) = mesh(model, k, dir, meshes)?;
                    ColliderBuilder::trimesh(points, tris)
                        .map_err(|e| format!("bad triangle mesh `{model}`: {e:?}"))?
                }
            })
        }

        /// All triangles of a model or game mesh, scaled, in model space.
        fn mesh(
            model: &str,
            k: [f32; 3],
            dir: &Path,
            meshes: Option<&GameMeshes>,
        ) -> Result<(Vec<Vector>, Vec<[u32; 3]>), String> {
            if let Some(name) = marionette_core::mesh::mesh_name(model) {
                // Borrowed from the shared map: Rapier needs its own vertex arrays anyway, so
                // the only copy is the scaled one it keeps.
                let m = meshes
                    .and_then(|g| g.get(name))
                    .ok_or_else(|| format!("no game mesh `{name}` (store it in GameMeshes first)"))?;
                let points = m
                    .positions
                    .iter()
                    .map(|p| Vector::new(p[0] * k[0], p[1] * k[1], p[2] * k[2]))
                    .collect();
                let tris = m.indices.as_chunks::<3>().0.to_vec();
                return Ok((points, tris));
            }
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
        use crate::components::{ColliderShape, JointKind};
        use marionette_core::mesh::GameMeshes;
        use rapier2d::prelude::*;
        use std::path::Path;

        pub use rapier2d::parry::query::ShapeCastOptions;

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

        /// Adds a torque about z to the body's accumulated forces.
        pub fn add_torque(b: &mut RigidBody, t: [f32; 3]) {
            b.add_torque(t[2], true);
        }
        pub fn torque_impulse(b: &mut RigidBody, t: [f32; 3]) {
            if t[2] != 0.0 {
                b.apply_torque_impulse(t[2], true);
            }
        }

        /// A unit axis in the plane, or +Y when the given one has no length there.
        fn axis(a: [f32; 3]) -> Vector {
            let v = vector(a).normalize_or_zero();
            if v == Vector::ZERO { Vector::Y } else { v }
        }

        /// The Rapier joint of a `JointKind` (decision 0310). Hinges and balls turn about z.
        pub fn joint(
            k: &JointKind,
            a1: [f32; 3],
            a2: [f32; 3],
            contacts: bool,
            rel: Rotation,
        ) -> GenericJoint {
            let (a1, a2) = (vector(a1), vector(a2));
            let mut j: GenericJoint = match *k {
                JointKind::Fixed => FixedJointBuilder::new()
                    .local_anchor1(a1)
                    .local_anchor2(a2)
                    .build()
                    .into(),
                JointKind::Ball => RevoluteJointBuilder::new()
                    .local_anchor1(a1)
                    .local_anchor2(a2)
                    .build()
                    .into(),
                JointKind::Hinge {
                    min,
                    max,
                    motor_speed,
                    motor_force,
                    ..
                } => {
                    let mut b = RevoluteJointBuilder::new().local_anchor1(a1).local_anchor2(a2);
                    if min < max {
                        b = b.limits([min, max]);
                    }
                    if motor_force > 0.0 {
                        b = b
                            .motor_velocity(motor_speed, crate::dims::MOTOR_GAIN)
                            .motor_max_force(motor_force);
                    }
                    b.build().into()
                }
                JointKind::Slider {
                    axis: x,
                    min,
                    max,
                    motor_speed,
                    motor_force,
                } => {
                    let mut b = PrismaticJointBuilder::new(axis([x.x, x.y, x.z]))
                        .local_anchor1(a1)
                        .local_anchor2(a2);
                    if min < max {
                        b = b.limits([min, max]);
                    }
                    if motor_force > 0.0 {
                        b = b
                            .motor_velocity(motor_speed, crate::dims::MOTOR_GAIN)
                            .motor_max_force(motor_force);
                    }
                    b.build().into()
                }
                JointKind::Rope { max_length } => RopeJointBuilder::new(max_length)
                    .local_anchor1(a1)
                    .local_anchor2(a2)
                    .build()
                    .into(),
                JointKind::Spring {
                    rest_length,
                    stiffness,
                    damping,
                } => SpringJointBuilder::new(rest_length, stiffness, damping)
                    .local_anchor1(a1)
                    .local_anchor2(a2)
                    .build()
                    .into(),
            };
            j.set_contacts_enabled(contacts);
            // The kind's frame on the second body is in that body's space; the first body's
            // frame is the same frame seen from the first body at build time.
            let mut f1 = j.local_frame1;
            f1.rotation = rel * j.local_frame2.rotation;
            j.set_local_frame1(f1);
            j
        }

        pub fn shape(
            s: &ColliderShape,
            k: [f32; 3],
            _dir: &Path,
            _meshes: Option<&GameMeshes>,
        ) -> Result<ColliderBuilder, String> {
            Ok(match s {
                ColliderShape::Cuboid { half_extents: h } => ColliderBuilder::cuboid(h.x * k[0], h.y * k[1]),
                ColliderShape::RoundCuboid {
                    half_extents: h,
                    border,
                } => {
                    let b = border.max(0.0);
                    let inner = |v: f32| (v - b).max(0.0);
                    ColliderBuilder::round_cuboid(inner(h.x * k[0]), inner(h.y * k[1]), b)
                }
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
