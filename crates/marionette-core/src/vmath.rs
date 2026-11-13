//! Small vector and quaternion helpers on plain arrays (`[f32; 3]`, quaternions as
//! `[x, y, z, w]`) for simulation code. Only `+ - * /` and `sqrt`, so results are the same on
//! every platform (determinism rules in ARCHITECTURE.md).

pub fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn len(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
pub fn norm(a: [f32; 3]) -> Option<[f32; 3]> {
    let l = len(a);
    (l > 1e-9).then(|| scale(a, 1.0 / l))
}

/// Quaternion product `a * b` (x, y, z, w).
pub fn qmul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}
pub fn qnorm(q: [f32; 4]) -> [f32; 4] {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l < 1e-9 {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [q[0] / l, q[1] / l, q[2] / l, q[3] / l]
    }
}
/// Rotates `v` by unit quaternion `q`.
pub fn qrot(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let t = scale(cross(u, v), 2.0);
    add(add(v, scale(t, q[3])), cross(u, t))
}
/// The shortest rotation taking unit `a` to unit `b`, with `sqrt` only. Opposite vectors
/// turn half way round an axis perpendicular to `a`.
pub fn arc(a: [f32; 3], b: [f32; 3]) -> [f32; 4] {
    let d = dot(a, b);
    if d < -0.999_999 {
        let axis = if a[0].abs() < 0.9 {
            cross(a, [1.0, 0.0, 0.0])
        } else {
            cross(a, [0.0, 1.0, 0.0])
        };
        let n = norm(axis).unwrap_or([0.0, 0.0, 1.0]);
        return [n[0], n[1], n[2], 0.0];
    }
    let c = cross(a, b);
    qnorm([c[0], c[1], c[2], 1.0 + d])
}
/// Rotation whose local +Y is `up` and local -Z is as close as possible to `forward`.
pub fn basis(up: [f32; 3], forward: [f32; 3]) -> Option<[f32; 4]> {
    let f = norm(sub(forward, scale(up, dot(forward, up))))?;
    let z = scale(f, -1.0);
    let x = cross(up, z);
    let (m00, m11, m22) = (x[0], up[1], z[2]);
    let tr = m00 + m11 + m22;
    // Columns are x, up, z: m[row][col].
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [
            (up[2] - z[1]) / s,
            (z[0] - x[2]) / s,
            (x[1] - up[0]) / s,
            0.25 * s,
        ]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        [
            0.25 * s,
            (up[0] + x[1]) / s,
            (z[0] + x[2]) / s,
            (up[2] - z[1]) / s,
        ]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        [
            (up[0] + x[1]) / s,
            0.25 * s,
            (z[1] + up[2]) / s,
            (z[0] - x[2]) / s,
        ]
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        [
            (z[0] + x[2]) / s,
            (z[1] + up[2]) / s,
            0.25 * s,
            (x[1] - up[0]) / s,
        ]
    };
    Some(qnorm(q))
}

/// The inverse of unit quaternion `q`.
pub fn qconj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}
