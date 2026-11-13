//! Tracked poses as input.
//!
//! A headset runtime (`marionette-openxr`), a script or a recording sets each tracked pose
//! as directly set actions (`InputEvent { action: "xr.head.px", .. }`), so poses go through
//! the same queue, recording and replay as every other input and the state hash covers them
//! (they live in `ActionState`). Per source: position `px py pz` (metres, play space),
//! rotation `qx qy qz qw`, and `tracked` (1 while the runtime tracks it).
//!
//! Play space: right-handed, +Y up, -Z forward, the floor at y = 0 (standing) or the runtime's
//! seated origin. [`crate::XrRig`] places the play space in the world.

use bevy_reflect::Reflect;
use marionette_core::actions::ActionState;
use marionette_core::spatial::{Quat, Vec3};
use marionette_input::InputEvent;

/// A tracked thing.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum XrSource {
    /// The head (between the eyes).
    #[default]
    Head,
    /// Left controller grip (or the left palm with hand tracking): where a held object sits.
    LeftGrip,
    /// Right controller grip (or the right palm).
    RightGrip,
    /// Left pointing ray: origin and -Z direction for pointing and teleporting.
    LeftAim,
    /// Right pointing ray.
    RightAim,
}

/// The seven pose fields plus `tracked`, in the order [`XrSource::keys`] returns them.
pub const FIELDS: [&str; 8] = ["px", "py", "pz", "qx", "qy", "qz", "qw", "tracked"];

impl XrSource {
    pub const ALL: [XrSource; 5] = [
        XrSource::Head,
        XrSource::LeftGrip,
        XrSource::RightGrip,
        XrSource::LeftAim,
        XrSource::RightAim,
    ];

    /// Short name used in action names: `head`, `left`, `right`, `left_aim`, `right_aim`.
    pub fn name(self) -> &'static str {
        match self {
            XrSource::Head => "head",
            XrSource::LeftGrip => "left",
            XrSource::RightGrip => "right",
            XrSource::LeftAim => "left_aim",
            XrSource::RightAim => "right_aim",
        }
    }

    /// Action names of this source's fields, in [`FIELDS`] order (`xr.head.px`, ...).
    pub fn keys(self) -> &'static [&'static str; 8] {
        macro_rules! k {
            ($s:literal) => {
                &[
                    concat!("xr.", $s, ".px"),
                    concat!("xr.", $s, ".py"),
                    concat!("xr.", $s, ".pz"),
                    concat!("xr.", $s, ".qx"),
                    concat!("xr.", $s, ".qy"),
                    concat!("xr.", $s, ".qz"),
                    concat!("xr.", $s, ".qw"),
                    concat!("xr.", $s, ".tracked"),
                ]
            };
        }
        match self {
            XrSource::Head => k!("head"),
            XrSource::LeftGrip => k!("left"),
            XrSource::RightGrip => k!("right"),
            XrSource::LeftAim => k!("left_aim"),
            XrSource::RightAim => k!("right_aim"),
        }
    }
}

/// A pose in play space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose {
    pub position: Vec3,
    pub rotation: Quat,
    pub tracked: bool,
}

/// Reads a source's pose from this tick's actions. A source never set reads as untracked,
/// at the play-space origin, with identity rotation.
pub fn read(actions: &ActionState, source: XrSource) -> Pose {
    let k = source.keys();
    let v = |i: usize| actions.values.get(k[i]).copied();
    let rotation = match (v(3), v(4), v(5), v(6)) {
        (Some(x), Some(y), Some(z), Some(w)) => crate::math::qnormalize(Quat { x, y, z, w }),
        _ => Quat::default(),
    };
    Pose {
        position: Vec3::new(v(0).unwrap_or(0.0), v(1).unwrap_or(0.0), v(2).unwrap_or(0.0)),
        rotation,
        tracked: v(7).unwrap_or(0.0) > 0.5,
    }
}

/// Position step for recorded poses (0.1 mm). Quantizing keeps sensor noise below this out of
/// recordings and makes them shorter; it never changes how a replay runs.
pub const POSITION_STEP: f32 = 1e-4;
/// Rotation component step for recorded poses.
pub const ROTATION_STEP: f32 = 1e-5;

fn quantize(v: f32, step: f32) -> f32 {
    (v / step).round() * step
}

/// The input events that set `source` to `pose` at `tick`, quantized (see [`POSITION_STEP`]).
/// Feed them with `marionette_input::feed` (or the engine's feed); only changed values end
/// up in a recording.
pub fn events(tick: u64, source: XrSource, pose: &Pose) -> [InputEvent; 8] {
    let k = source.keys();
    let p = pose.position;
    let q = crate::math::qnormalize(pose.rotation);
    let values = [
        quantize(p.x, POSITION_STEP),
        quantize(p.y, POSITION_STEP),
        quantize(p.z, POSITION_STEP),
        quantize(q.x, ROTATION_STEP),
        quantize(q.y, ROTATION_STEP),
        quantize(q.z, ROTATION_STEP),
        quantize(q.w, ROTATION_STEP),
        if pose.tracked { 1.0 } else { 0.0 },
    ];
    std::array::from_fn(|i| InputEvent {
        tick,
        input: None,
        action: Some(k[i].to_string()),
        value: values[i],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_round_trip_through_actions() {
        let pose = Pose {
            position: Vec3::new(0.1, 1.6, -0.2),
            rotation: crate::math::yaw(0.3),
            tracked: true,
        };
        let mut actions = ActionState::default();
        for e in events(0, XrSource::RightAim, &pose) {
            actions.values.insert(e.action.unwrap(), e.value);
        }
        let back = read(&actions, XrSource::RightAim);
        assert!(back.tracked);
        assert!((back.position.y - 1.6).abs() < 1e-4);
        assert!((back.rotation.y - pose.rotation.y).abs() < 2e-5);
        assert!(!read(&actions, XrSource::Head).tracked);
    }
}
