//! Controllers and hands: one OpenXR action set with the engine's fixed controller inputs
//! (`xr:<hand>/<input>`, `marionette_input::XR_INPUTS`), suggested bindings for the common
//! controller families, tracked poses, hand tracking and haptics.
//!
//! Games never see OpenXR paths: they bind their actions to `xr:` inputs in the action map,
//! and the runtime (SteamVR and others let players rebind) maps controllers onto these.

use crate::{XrError, codes};
use marionette_input::InputEvent;
use marionette_xr::pose::{self, Pose, XrSource};
use marionette_xr::{Hand, HapticPulse};
use openxr as xr;
use std::collections::BTreeMap;

const HANDS: [&str; 2] = ["left", "right"];

pub struct Controls {
    set: xr::ActionSet,
    hand_paths: [xr::Path; 2],
    trigger: xr::Action<f32>,
    squeeze: xr::Action<f32>,
    stick: xr::Action<xr::Vector2f>,
    stick_click: xr::Action<bool>,
    primary: xr::Action<bool>,
    secondary: xr::Action<bool>,
    menu: xr::Action<bool>,
    grip: xr::Action<xr::Posef>,
    aim: xr::Action<xr::Posef>,
    haptic: xr::Action<xr::Haptic>,
    /// Controller families whose suggested bindings the runtime refused (reported).
    pub refused_profiles: Vec<String>,
}

/// Per-session objects made after the session exists.
pub struct Spaces {
    grip: [xr::Space; 2],
    aim: [xr::Space; 2],
    view: xr::Space,
    hands: Option<[xr::HandTracker; 2]>,
}

/// One hand's joints this frame (play space), for drawing hands. 26 joints in OpenXR order
/// (`XR_EXT_hand_tracking`): palm, wrist, then thumb to little finger, base to tip.
#[derive(Debug, Clone)]
pub struct HandJoints {
    pub positions: Vec<[f32; 3]>,
    pub radii: Vec<f32>,
}

/// What [`Controls::read`] found this frame.
#[derive(Default)]
pub struct Reading {
    pub events: Vec<InputEvent>,
    pub joints: [Option<HandJoints>; 2],
    /// The interaction profile in each hand, if any (`/interaction_profiles/...`).
    pub tracked_sources: usize,
}

fn path(i: &xr::Instance, s: &str) -> Result<xr::Path, XrError> {
    i.string_to_path(s)
        .map_err(|e| XrError::new(codes::SESSION, format!("bad OpenXR path {s}: {e}")))
}

fn act_err(e: xr::sys::Result) -> XrError {
    XrError::new(codes::SESSION, format!("cannot create input actions: {e}"))
}

/// Suggested bindings: (profile, [(action, path suffix after /user/hand/<hand>/)]).
/// `*` binds both hands; `L:`/`R:` one hand.
const PROFILES: &[(&str, &[(&str, &str)])] = &[
    (
        "/interaction_profiles/khr/simple_controller",
        &[
            ("trigger", "*input/select/click"),
            ("menu", "*input/menu/click"),
            ("grip", "*input/grip/pose"),
            ("aim", "*input/aim/pose"),
            ("haptic", "*output/haptic"),
        ],
    ),
    (
        "/interaction_profiles/oculus/touch_controller",
        &[
            ("trigger", "*input/trigger/value"),
            ("squeeze", "*input/squeeze/value"),
            ("stick", "*input/thumbstick"),
            ("stick_click", "*input/thumbstick/click"),
            ("primary", "L:input/x/click"),
            ("secondary", "L:input/y/click"),
            ("primary", "R:input/a/click"),
            ("secondary", "R:input/b/click"),
            ("menu", "L:input/menu/click"),
            ("grip", "*input/grip/pose"),
            ("aim", "*input/aim/pose"),
            ("haptic", "*output/haptic"),
        ],
    ),
    (
        "/interaction_profiles/valve/index_controller",
        &[
            ("trigger", "*input/trigger/value"),
            ("squeeze", "*input/squeeze/value"),
            ("stick", "*input/thumbstick"),
            ("stick_click", "*input/thumbstick/click"),
            ("primary", "*input/a/click"),
            ("secondary", "*input/b/click"),
            ("grip", "*input/grip/pose"),
            ("aim", "*input/aim/pose"),
            ("haptic", "*output/haptic"),
        ],
    ),
    (
        "/interaction_profiles/htc/vive_controller",
        &[
            ("trigger", "*input/trigger/value"),
            ("squeeze", "*input/squeeze/click"),
            ("stick", "*input/trackpad"),
            ("stick_click", "*input/trackpad/click"),
            ("menu", "*input/menu/click"),
            ("grip", "*input/grip/pose"),
            ("aim", "*input/aim/pose"),
            ("haptic", "*output/haptic"),
        ],
    ),
    (
        "/interaction_profiles/microsoft/motion_controller",
        &[
            ("trigger", "*input/trigger/value"),
            ("squeeze", "*input/squeeze/click"),
            ("stick", "*input/thumbstick"),
            ("stick_click", "*input/thumbstick/click"),
            ("menu", "*input/menu/click"),
            ("grip", "*input/grip/pose"),
            ("aim", "*input/aim/pose"),
            ("haptic", "*output/haptic"),
        ],
    ),
];

impl Controls {
    pub fn new(instance: &xr::Instance) -> Result<Self, XrError> {
        let set = instance
            .create_action_set("gameplay", "Gameplay", 0)
            .map_err(act_err)?;
        let hand_paths = [
            path(instance, "/user/hand/left")?,
            path(instance, "/user/hand/right")?,
        ];
        let hp = &hand_paths;
        let c = Controls {
            trigger: set.create_action("trigger", "Trigger", hp).map_err(act_err)?,
            squeeze: set.create_action("squeeze", "Grip", hp).map_err(act_err)?,
            stick: set
                .create_action("thumbstick", "Thumbstick", hp)
                .map_err(act_err)?,
            stick_click: set
                .create_action("thumbstick_click", "Thumbstick click", hp)
                .map_err(act_err)?,
            primary: set
                .create_action("primary", "Primary button", hp)
                .map_err(act_err)?,
            secondary: set
                .create_action("secondary", "Secondary button", hp)
                .map_err(act_err)?,
            menu: set.create_action("menu", "Menu", hp).map_err(act_err)?,
            grip: set
                .create_action("grip_pose", "Hand grip pose", hp)
                .map_err(act_err)?,
            aim: set
                .create_action("aim_pose", "Hand aim pose", hp)
                .map_err(act_err)?,
            haptic: set.create_action("haptic", "Vibration", hp).map_err(act_err)?,
            set,
            hand_paths,
            refused_profiles: Vec::new(),
        };
        Ok(c)
    }

    /// Suggests bindings for every known controller family. A family the runtime does not
    /// know is skipped and reported, never fatal.
    pub fn suggest(&mut self, instance: &xr::Instance) -> Result<(), XrError> {
        for (profile, list) in PROFILES {
            let mut bindings = Vec::new();
            for (action, spec) in *list {
                let (hands, suffix): (&[&str], &str) = if let Some(s) = spec.strip_prefix('*') {
                    (&HANDS, s)
                } else if let Some(s) = spec.strip_prefix("L:") {
                    (&HANDS[..1], s)
                } else if let Some(s) = spec.strip_prefix("R:") {
                    (&HANDS[1..], s)
                } else {
                    continue;
                };
                for hand in hands {
                    let p = path(instance, &format!("/user/hand/{hand}/{suffix}"))?;
                    bindings.push(match *action {
                        "trigger" => xr::Binding::new(&self.trigger, p),
                        "squeeze" => xr::Binding::new(&self.squeeze, p),
                        "stick" => xr::Binding::new(&self.stick, p),
                        "stick_click" => xr::Binding::new(&self.stick_click, p),
                        "primary" => xr::Binding::new(&self.primary, p),
                        "secondary" => xr::Binding::new(&self.secondary, p),
                        "menu" => xr::Binding::new(&self.menu, p),
                        "grip" => xr::Binding::new(&self.grip, p),
                        "aim" => xr::Binding::new(&self.aim, p),
                        _ => xr::Binding::new(&self.haptic, p),
                    });
                }
            }
            let pp = path(instance, profile)?;
            if instance
                .suggest_interaction_profile_bindings(pp, &bindings)
                .is_err()
            {
                self.refused_profiles.push((*profile).to_string());
            }
        }
        Ok(())
    }

    pub fn attach<G: xr::Graphics>(
        &self,
        session: &xr::Session<G>,
        hand_tracking: bool,
    ) -> Result<Spaces, XrError> {
        let e = |e: xr::sys::Result| XrError::new(codes::SESSION, format!("input setup: {e}"));
        session.attach_action_sets(&[&self.set]).map_err(e)?;
        let space = |a: &xr::Action<xr::Posef>, i: usize| {
            a.create_space(session, self.hand_paths[i], xr::Posef::IDENTITY)
                .map_err(e)
        };
        let hands = if hand_tracking {
            match (
                session.create_hand_tracker(xr::Hand::LEFT),
                session.create_hand_tracker(xr::Hand::RIGHT),
            ) {
                (Ok(l), Ok(r)) => Some([l, r]),
                _ => None,
            }
        } else {
            None
        };
        Ok(Spaces {
            grip: [space(&self.grip, 0)?, space(&self.grip, 1)?],
            aim: [space(&self.aim, 0)?, space(&self.aim, 1)?],
            view: session
                .create_reference_space(xr::ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)
                .map_err(e)?,
            hands,
        })
    }

    /// Syncs the actions and reads every input and pose at `time` (in `base` space), as
    /// events for `tick`. Only inputs that changed since `last` are returned.
    #[allow(clippy::too_many_arguments)]
    pub fn read<G: xr::Graphics>(
        &self,
        session: &xr::Session<G>,
        spaces: &Spaces,
        base: &xr::Space,
        time: xr::Time,
        tick: u64,
        last: &mut BTreeMap<String, f32>,
    ) -> Reading {
        let mut out = Reading::default();
        if session.sync_actions(&[(&self.set).into()]).is_err() {
            return out;
        }
        for (i, hand) in HANDS.iter().enumerate() {
            let sub = self.hand_paths[i];
            let f = |a: &xr::Action<f32>| a.state(session, sub).map(|s| s.current_state).unwrap_or(0.0);
            let b = |a: &xr::Action<bool>| {
                a.state(session, sub)
                    .map(|s| if s.current_state { 1.0 } else { 0.0 })
                    .unwrap_or(0.0)
            };
            let stick = self
                .stick
                .state(session, sub)
                .map(|s| s.current_state)
                .unwrap_or(xr::Vector2f { x: 0.0, y: 0.0 });
            let values = [
                ("trigger", f(&self.trigger)),
                ("squeeze", f(&self.squeeze)),
                ("thumbstick_x", stick.x),
                ("thumbstick_y", stick.y),
                ("thumbstick_click", b(&self.stick_click)),
                ("primary", b(&self.primary)),
                ("secondary", b(&self.secondary)),
                ("menu", b(&self.menu)),
            ];
            for (name, v) in values {
                put(last, &mut out.events, input(tick, format!("xr:{hand}/{name}"), v));
            }

            // Hand tracking: joints for drawing, pinch as an input, the palm as a fallback
            // grip and aim when no controller is tracked.
            let mut palm: Option<Pose> = None;
            let mut pinch = 0.0;
            if let Some(trackers) = &spaces.hands
                && let Ok(Some(joints)) = base.locate_hand_joints(&trackers[i], time)
            {
                let valid = |j: &xr::HandJointLocation| {
                    j.location_flags.contains(
                        xr::SpaceLocationFlags::POSITION_VALID | xr::SpaceLocationFlags::ORIENTATION_VALID,
                    )
                };
                if joints.iter().all(valid) {
                    let p = |k: usize| joints[k].pose.position;
                    let (thumb, index) = (
                        p(xr::HandJoint::THUMB_TIP.into_raw() as usize),
                        p(xr::HandJoint::INDEX_TIP.into_raw() as usize),
                    );
                    let d = ((thumb.x - index.x).powi(2)
                        + (thumb.y - index.y).powi(2)
                        + (thumb.z - index.z).powi(2))
                    .sqrt();
                    pinch = ((0.035 - d) / 0.02).clamp(0.0, 1.0);
                    let palm_j = &joints[xr::HandJoint::PALM.into_raw() as usize];
                    palm = Some(to_pose(&palm_j.pose, true));
                    out.joints[i] = Some(HandJoints {
                        positions: joints
                            .iter()
                            .map(|j| [j.pose.position.x, j.pose.position.y, j.pose.position.z])
                            .collect(),
                        radii: joints.iter().map(|j| j.radius).collect(),
                    });
                }
            }
            let pinch = (pinch * 100.0f32).round() / 100.0;
            put(
                last,
                &mut out.events,
                input(tick, format!("xr:{hand}/pinch"), pinch),
            );

            let (grip_src, aim_src) = if i == 0 {
                (XrSource::LeftGrip, XrSource::LeftAim)
            } else {
                (XrSource::RightGrip, XrSource::RightAim)
            };
            let mut grip = locate(&spaces.grip[i], base, time);
            let mut aim = locate(&spaces.aim[i], base, time);
            if !grip.tracked
                && let Some(p) = palm
            {
                grip = p;
                aim = p;
            }
            out.tracked_sources += usize::from(grip.tracked);
            for (src, p) in [(grip_src, grip), (aim_src, aim)] {
                for ev in pose::events(tick, src, &p) {
                    put(last, &mut out.events, ev);
                }
            }
        }
        let head = locate(&spaces.view, base, time);
        out.tracked_sources += usize::from(head.tracked);
        for ev in pose::events(tick, XrSource::Head, &head) {
            put(last, &mut out.events, ev);
        }
        out
    }

    /// Sends vibration requests to the controllers.
    pub fn vibrate<G: xr::Graphics>(&self, session: &xr::Session<G>, pulses: &[HapticPulse]) {
        for p in pulses {
            let i = match p.hand {
                Hand::Left => 0,
                Hand::Right => 1,
            };
            let ns = (p.seconds.max(0.0) * 1e9) as i64;
            let event = xr::HapticVibration::new()
                .amplitude(p.amplitude.clamp(0.0, 1.0))
                .duration(xr::Duration::from_nanos(ns))
                .frequency(xr::FREQUENCY_UNSPECIFIED);
            let _ = self.haptic.apply_feedback(session, self.hand_paths[i], &event);
        }
    }
}

fn input(tick: u64, name: String, value: f32) -> InputEvent {
    InputEvent {
        tick,
        input: Some(name),
        action: None,
        value,
    }
}

/// Queues `ev` unless its input or action already has this value.
fn put(last: &mut BTreeMap<String, f32>, events: &mut Vec<InputEvent>, ev: InputEvent) {
    let name = ev
        .input
        .as_ref()
        .or(ev.action.as_ref())
        .expect("one of the two")
        .clone();
    if last.get(&name) != Some(&ev.value) {
        last.insert(name, ev.value);
        events.push(ev);
    }
}

pub fn to_pose(p: &xr::Posef, tracked: bool) -> Pose {
    use marionette_core::spatial::{Quat, Vec3};
    Pose {
        position: Vec3::new(p.position.x, p.position.y, p.position.z),
        rotation: Quat {
            x: p.orientation.x,
            y: p.orientation.y,
            z: p.orientation.z,
            w: p.orientation.w,
        },
        tracked,
    }
}

fn locate(space: &xr::Space, base: &xr::Space, time: xr::Time) -> Pose {
    match space.locate(base, time) {
        Ok(l) => {
            let ok = l
                .location_flags
                .contains(xr::SpaceLocationFlags::POSITION_VALID | xr::SpaceLocationFlags::ORIENTATION_VALID);
            to_pose(&l.pose, ok)
        }
        Err(_) => Pose::default(),
    }
}
