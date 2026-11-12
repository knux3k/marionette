//! Coordinate conversion from Unity and Unreal to Marionette.
//!
//! Marionette is right-handed, +Y up, -Z forward, +X right, meters (glTF convention).
//! Unity is left-handed, +Y up, +Z forward, +X right, meters.
//! Unreal is left-handed, +Z up, +X forward, +Y right, centimeters.
//!
//! Unity -> Marionette: (x, y, z) -> (x, y, -z); quaternion (x, y, z, w) -> (-x, -y, z, w).
//! Unreal -> Marionette: (x, y, z) -> (y, z, -x) / 100 for positions (directions are not
//! scaled); quaternion (x, y, z, w) -> (-y, -z, x, w); scale (x, y, z) -> (y, z, x).
//!
//! These run while porting, never per tick, so the trigonometry used for Euler angles does
//! not affect determinism.

use crate::codes;
use crate::map::Engine;
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a value is; it decides how axes, signs and units change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    /// A point or offset (Unreal: centimeters, divided by 100).
    Position,
    /// A direction, normal or axis (never scaled).
    Direction,
    /// A velocity or acceleration (Unreal: cm/s, divided by 100).
    Velocity,
    /// A quaternion (x, y, z, w).
    Quaternion,
    /// Euler angles in degrees: Unity (x, y, z) as in the inspector; Unreal FRotator as
    /// (pitch, yaw, roll).
    Euler,
    /// A scale (axes swap, signs and units do not change).
    Scale,
    /// A length or distance (Unreal: divided by 100).
    Length,
}

fn bad(msg: String) -> Errors {
    Diagnostic::error(codes::BAD_VALUE, msg)
        .fix("position/direction/velocity/scale take 3 numbers, quaternion 4 (x, y, z, w), euler 3 (degrees), length 1")
        .into()
}

fn qmul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn axis(axis: usize, degrees: f64) -> [f64; 4] {
    let h = degrees.to_radians() / 2.0;
    let mut q = [0.0, 0.0, 0.0, h.cos()];
    q[axis] = h.sin();
    q
}

/// Rounds away float noise so results read cleanly (1e-9).
fn clean(v: f64) -> f64 {
    let r = (v * 1e9).round() / 1e9;
    if r == 0.0 { 0.0 } else { r }
}

/// Converts one value. Returns the Marionette value (3 or 4 numbers, or 1 for a length).
pub fn convert(engine: Engine, kind: ValueKind, v: &[f64]) -> Result<Vec<f64>, Errors> {
    let want = match kind {
        ValueKind::Quaternion => 4,
        ValueKind::Length => 1,
        _ => 3,
    };
    if v.len() != want {
        return Err(bad(format!("{kind:?} needs {want} numbers, got {}", v.len())));
    }
    if v.iter().any(|x| !x.is_finite()) {
        return Err(bad("values must be finite numbers".into()));
    }
    let out: Vec<f64> = match (engine, kind) {
        (Engine::Unity, ValueKind::Position | ValueKind::Direction | ValueKind::Velocity) => {
            vec![v[0], v[1], -v[2]]
        }
        (Engine::Unity, ValueKind::Scale) => v.to_vec(),
        (Engine::Unity, ValueKind::Length) => v.to_vec(),
        (Engine::Unity, ValueKind::Quaternion) => vec![-v[0], -v[1], v[2], v[3]],
        (Engine::Unity, ValueKind::Euler) => {
            // Unity applies Z, then X, then Y: q = qy * qx * qz.
            let q = qmul(qmul(axis(1, v[1]), axis(0, v[0])), axis(2, v[2]));
            return convert(engine, ValueKind::Quaternion, &q);
        }
        (Engine::Unreal, ValueKind::Position | ValueKind::Velocity) => {
            vec![v[1] / 100.0, v[2] / 100.0, -v[0] / 100.0]
        }
        (Engine::Unreal, ValueKind::Direction) => vec![v[1], v[2], -v[0]],
        (Engine::Unreal, ValueKind::Scale) => vec![v[1], v[2], v[0]],
        (Engine::Unreal, ValueKind::Length) => vec![v[0] / 100.0],
        (Engine::Unreal, ValueKind::Quaternion) => vec![-v[1], -v[2], v[0], v[3]],
        (Engine::Unreal, ValueKind::Euler) => {
            // FRotator(Pitch, Yaw, Roll) to FQuat, as Unreal computes it.
            let (sp, cp) = (v[0].to_radians() / 2.0).sin_cos();
            let (sy, cy) = (v[1].to_radians() / 2.0).sin_cos();
            let (sr, cr) = (v[2].to_radians() / 2.0).sin_cos();
            let q = [
                cr * sp * sy - sr * cp * cy,
                -cr * sp * cy - sr * cp * sy,
                cr * cp * sy - sr * sp * cy,
                cr * cp * cy + sr * sp * sy,
            ];
            return convert(engine, ValueKind::Quaternion, &q);
        }
    };
    Ok(out.into_iter().map(clean).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rotates v by unit quaternion q.
    fn rotate(q: &[f64], v: [f64; 3]) -> [f64; 3] {
        let p = qmul(
            qmul([q[0], q[1], q[2], q[3]], [v[0], v[1], v[2], 0.0]),
            [-q[0], -q[1], -q[2], q[3]],
        );
        let r = |v: f64| (v * 1e6).round() / 1e6 + 0.0;
        [r(p[0]), r(p[1]), r(p[2])]
    }

    #[test]
    fn unity_axes() {
        assert_eq!(
            convert(Engine::Unity, ValueKind::Position, &[1.0, 2.0, 3.0]).unwrap(),
            vec![1.0, 2.0, -3.0]
        );
        // Unity yaw 90 turns forward (+Z) to right (+X); in Marionette forward is -Z and
        // must also turn to +X.
        let q = convert(Engine::Unity, ValueKind::Euler, &[0.0, 90.0, 0.0]).unwrap();
        assert_eq!(rotate(&q, [0.0, 0.0, -1.0]), [1.0, 0.0, 0.0]);
        // Pitch 90 in Unity tilts forward down (-Y).
        let q = convert(Engine::Unity, ValueKind::Euler, &[90.0, 0.0, 0.0]).unwrap();
        assert_eq!(rotate(&q, [0.0, 0.0, -1.0]), [0.0, -1.0, 0.0]);
    }

    #[test]
    fn unreal_axes_and_units() {
        // 100 cm forward, 200 cm right, 300 cm up.
        assert_eq!(
            convert(Engine::Unreal, ValueKind::Position, &[100.0, 200.0, 300.0]).unwrap(),
            vec![2.0, 3.0, -1.0]
        );
        assert_eq!(
            convert(Engine::Unreal, ValueKind::Scale, &[1.0, 2.0, 3.0]).unwrap(),
            vec![2.0, 3.0, 1.0]
        );
        // Yaw 90 turns forward (+X) to right (+Y): Marionette forward (-Z) to right (+X).
        let q = convert(Engine::Unreal, ValueKind::Euler, &[0.0, 90.0, 0.0]).unwrap();
        assert_eq!(rotate(&q, [0.0, 0.0, -1.0]), [1.0, 0.0, 0.0]);
        // Pitch 90 points forward up (+Z in Unreal, +Y in Marionette).
        let q = convert(Engine::Unreal, ValueKind::Euler, &[90.0, 0.0, 0.0]).unwrap();
        assert_eq!(rotate(&q, [0.0, 0.0, -1.0]), [0.0, 1.0, 0.0]);
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(convert(Engine::Unity, ValueKind::Quaternion, &[1.0]).is_err());
        assert!(convert(Engine::Unreal, ValueKind::Position, &[f64::NAN, 0.0, 0.0]).is_err());
    }
}
