//! Small vector and quaternion maths on plain `f32` operations only (`+ - * /`, `sqrt`), so
//! every tick computes the same bits on every platform (no SIMD fused multiply-add, no
//! platform `sin`/`cos`; angles go through `libm`, which is pure Rust).

use marionette_core::spatial::{Quat, Transform, Vec3};

pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x + b.x, a.y + b.y, a.z + b.z)
}

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}

pub fn scale(a: Vec3, k: f32) -> Vec3 {
    Vec3::new(a.x * k, a.y * k, a.z * k)
}

pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

pub fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

/// `a` scaled to length 1, or zero if `a` is (almost) zero.
pub fn normalize(a: Vec3) -> Vec3 {
    let l = length(a);
    if l > 1e-6 {
        scale(a, 1.0 / l)
    } else {
        Vec3::default()
    }
}

pub fn qmul(a: Quat, b: Quat) -> Quat {
    Quat {
        w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
        x: a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
        y: a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
        z: a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
    }
}

pub fn conj(q: Quat) -> Quat {
    Quat {
        x: -q.x,
        y: -q.y,
        z: -q.z,
        w: q.w,
    }
}

/// `q` scaled to unit length; identity if `q` is (almost) zero.
pub fn qnormalize(q: Quat) -> Quat {
    let l = (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w).sqrt();
    if l > 1e-6 {
        Quat {
            x: q.x / l,
            y: q.y / l,
            z: q.z / l,
            w: q.w / l,
        }
    } else {
        Quat::default()
    }
}

/// Rotates `v` by the unit quaternion `q`.
pub fn rotate(q: Quat, v: Vec3) -> Vec3 {
    // v + 2w(u x v) + 2u x (u x v), with u = q.xyz.
    let u = Vec3::new(q.x, q.y, q.z);
    let t = scale(cross(u, v), 2.0);
    add(add(v, scale(t, q.w)), cross(u, t))
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    )
}

/// Rotation of `radians` around +Y.
pub fn yaw(radians: f32) -> Quat {
    let h = radians * 0.5;
    Quat {
        x: 0.0,
        y: libm::sinf(h),
        z: 0.0,
        w: libm::cosf(h),
    }
}

/// `parent` applied to `local` (translation and rotation; scale is ignored).
pub fn compose(parent: &Transform, local_t: Vec3, local_r: Quat) -> (Vec3, Quat) {
    (
        add(parent.translation, rotate(parent.rotation, local_t)),
        qnormalize(qmul(parent.rotation, local_r)),
    )
}

/// The pose of `child` relative to `parent` (inverse of [`compose`]).
pub fn relative(parent_t: Vec3, parent_r: Quat, child_t: Vec3, child_r: Quat) -> (Vec3, Quat) {
    let inv = conj(parent_r);
    (
        rotate(inv, sub(child_t, parent_t)),
        qnormalize(qmul(inv, child_r)),
    )
}

/// Angular velocity (radians per second, world axes) that turns `from` into `to` in `dt`
/// seconds. Uses the small-angle form `2 * xyz(to * from^-1) / dt`, exact enough for one tick.
pub fn angular_velocity(from: Quat, to: Quat, dt: f32) -> Vec3 {
    let mut d = qmul(to, conj(from));
    if d.w < 0.0 {
        d = Quat {
            x: -d.x,
            y: -d.y,
            z: -d.z,
            w: -d.w,
        };
    }
    scale(Vec3::new(d.x, d.y, d.z), 2.0 / dt)
}

/// The direction an entity faces (its local -Z) flattened onto the XZ plane, unit length.
/// Falls back to the local +Y projected (looking straight down or up) and then to -Z.
pub fn flat_forward(q: Quat) -> Vec3 {
    let f = rotate(q, Vec3::new(0.0, 0.0, -1.0));
    let flat = normalize(Vec3::new(f.x, 0.0, f.z));
    if length(flat) > 0.5 {
        return flat;
    }
    let up = rotate(q, Vec3::new(0.0, 1.0, 0.0));
    let flat = normalize(Vec3::new(up.x, 0.0, up.z));
    if length(flat) > 0.5 {
        flat
    } else {
        Vec3::new(0.0, 0.0, -1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        length(sub(a, b)) < 1e-5
    }

    #[test]
    fn yaw_turns_forward_left() {
        let q = yaw(std::f32::consts::FRAC_PI_2);
        assert!(close(
            rotate(q, Vec3::new(0.0, 0.0, -1.0)),
            Vec3::new(-1.0, 0.0, 0.0)
        ));
    }

    #[test]
    fn relative_undoes_compose() {
        let parent = Transform {
            translation: Vec3::new(1.0, 2.0, 3.0),
            rotation: yaw(0.7),
            ..Default::default()
        };
        let (t, r) = compose(&parent, Vec3::new(0.5, 0.0, -1.0), yaw(0.2));
        let (lt, lr) = relative(parent.translation, parent.rotation, t, r);
        assert!(close(lt, Vec3::new(0.5, 0.0, -1.0)));
        assert!((lr.y - yaw(0.2).y).abs() < 1e-5);
    }

    #[test]
    fn angular_velocity_of_a_small_yaw() {
        let w = angular_velocity(Quat::default(), yaw(0.01), 0.01);
        assert!((w.y - 1.0).abs() < 1e-3, "{w:?}");
    }
}
