//! Marionette XR gameplay: tracked head and hands, grab and throw, and comfortable movement
//! (snap or smooth turning, smooth movement, teleport, a comfort vignette), all in the fixed
//! tick and deterministic.
//!
//! This crate has no headset code. Tracked poses arrive as input actions ([`pose`]), so a
//! headset (`marionette-openxr`), a scripted test or a replayed recording drive it the same
//! way, and every run replays bit for bit. Buttons and sticks come through the normal action
//! map with `xr:` inputs (`xr:right/squeeze`, `xr:left/thumbstick_y`, ...; list in
//! `marionette-input`).
//!
//! Scene setup (decision 0190):
//! - one entity with [`XrRig`] (its `Transform` places the play space in the world) and,
//!   optionally, [`XrLocomotion`];
//! - entities with [`XrTracked`] follow a tracked pose inside that rig: the head (put the
//!   `Camera` and `AudioListener` here) and the two grips;
//! - a grip entity with [`XrHand`] grabs entities with [`Grabbable`] and throws them on
//!   release;
//! - with [`XrCharacter`] the rig rides a character body (flat or round worlds) and moves by
//!   physics (decision 0191).
//!
//! Outputs for the presentation side (not simulation state, not hashed): [`XrHaptics`]
//! (vibration requests) and [`XrComfort`] (vignette strength this tick).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![deny(clippy::clone_on_ref_ptr)]

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::Registry;
use marionette_core::builtin::SceneRef;
use marionette_core::module::{Module, Stage};
use marionette_core::spatial::{Quat, Vec3};

pub mod math;
pub mod pose;
mod systems;

pub use pose::{Pose, XrSource};

/// The play space: where the tracked area sits in the world. Tracked poses are relative to
/// this entity's `Transform` (translation and rotation; scale is ignored). Locomotion moves
/// and turns it.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct XrRig {
    /// Added to every tracked height: 0 for a standing play space (floor at 0); a seated
    /// player on a space whose origin is at eye level uses about 1.2.
    pub height_offset: f32,
}

/// Follows a tracked pose inside a rig every tick (its `Transform` is overwritten). While
/// the source is not tracked the entity keeps its last pose.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct XrTracked {
    /// The entity with [`XrRig`].
    pub rig: SceneRef,
    pub source: XrSource,
}

/// A hand that grabs: put it on a grip entity (`XrTracked` with `LeftGrip` or `RightGrip`).
/// Pressing `grab_action` picks up the nearest [`Grabbable`] whose centre is within
/// `grab_radius`; releasing throws it with the hand's velocity times `throw_scale`.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct XrHand {
    pub grab_action: String,
    /// Metres from the grip to an object's centre.
    pub grab_radius: f32,
    /// Multiplies the release velocity (1 = as moved).
    pub throw_scale: f32,
}

/// Hand motion over the last few ticks. Written by the XR module.
#[derive(Component, Reflect, Debug, Clone, PartialEq, Default)]
pub struct XrHandMotion {
    /// Recent grip positions (world), oldest first, at most [`MOTION_TICKS`].
    pub positions: Vec<Vec3>,
    /// Grip rotation last tick.
    pub last_rotation: Quat,
    /// Average velocity over `positions` (m/s): a held object leaves the hand with this.
    pub velocity: Vec3,
    /// Rotation speed over the last tick (radians per second, world axes).
    pub angular_velocity: Vec3,
}

/// Ticks of hand history averaged into the throw velocity (5 ticks = 83 ms at 60 Hz;
/// one tick alone makes throws jittery, many make them sluggish).
pub const MOTION_TICKS: usize = 5;

/// Can be picked up by an [`XrHand`]. With a dynamic `RigidBody` the object is moved by
/// velocity while held, so it still collides (it cannot pass through walls); without one its
/// `Transform` follows the hand exactly.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct Grabbable {
    /// Snap the object's origin and rotation to the grip (tools, guns); otherwise keep the
    /// offset it had when grabbed (boxes, loose items).
    pub snap: bool,
}

/// On an object while a hand holds it. Written by the XR module.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct Held {
    pub hand: SceneRef,
    /// The object's pose relative to the grip.
    pub offset_translation: Vec3,
    pub offset_rotation: Quat,
}

/// How turning works.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TurnMode {
    /// Turn in steps of `snap_degrees` (most comfortable).
    #[default]
    Snap,
    /// Turn continuously at `smooth_degrees_per_second`.
    Smooth,
    /// No stick turning (the player turns their body).
    Off,
}

/// Stick movement, turning and teleport for the rig it is on. An empty action name turns
/// that part off.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct XrLocomotion {
    /// Strafe action (-1 left .. 1 right).
    pub move_x: String,
    /// Forward action (-1 back .. 1 forward).
    pub move_y: String,
    /// Metres per second at full stick.
    pub speed: f32,
    /// Forward is where this source points, flattened (`Head`, or `LeftAim` for hand-relative).
    pub direction: XrSource,
    /// Turn action (-1 left .. 1 right).
    pub turn: String,
    pub turn_mode: TurnMode,
    pub snap_degrees: f32,
    pub smooth_degrees_per_second: f32,
    /// Hold to aim, release to teleport (needs physics: the aim ray must hit ground).
    pub teleport: String,
    /// The ray used to aim a teleport.
    pub teleport_aim: XrSource,
    /// Longest teleport in metres.
    pub teleport_range: f32,
    /// Comfort vignette strength while moving or turning smoothly (0 off .. 1 strong).
    pub vignette: f32,
}

/// Locomotion state. Written by the XR module.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct XrLocomotionState {
    /// The turn stick returned to centre since the last snap.
    pub snap_armed: bool,
    /// Where a teleport would land now (only meaningful while `teleport_valid`).
    pub teleport_point: Vec3,
    pub teleport_valid: bool,
}

/// Put on a rig with [`XrLocomotion`] to ride a character body instead of moving the play
/// space freely (decision 0191). The body is an entity with a `CharacterController` (flat
/// worlds) or a `CharacterController` and `PlanetCharacter` (round worlds, up follows gravity):
/// physics then decides where the player can go, so walls, slopes, stairs and gravity apply
/// in VR as on a flat screen.
///
/// Each tick the stick and the room-scale step (the head walking away from the body) become a
/// `MoveIntent` on the body; turning turns the rig and the body about the body's up through
/// the head; teleport moves the body. After physics the rig follows how the body moved and
/// tilted, so the play space stays upright on the body's ground wherever up points. Jumping
/// and jetpacks stay the body's own (its actions or the game's code).
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct XrCharacter {
    /// The body entity.
    pub body: SceneRef,
    /// Move the body under the head when the player walks in the room (on for standing
    /// play). Off: the body moves only by stick and teleport, and the head may leave it.
    pub follow_head: bool,
}

/// What the rig asked of its body this tick, so the rig can follow the body afterwards.
/// Written by the XR module.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct XrCharacterState {
    /// The body's centre and up when the move was asked for.
    pub body_before: Vec3,
    pub up_before: Vec3,
    /// The asked stick step and room-scale step (metres along the ground this tick).
    pub stick_step: Vec3,
    pub head_step: Vec3,
    /// The rig has been placed on the body once.
    pub placed: bool,
}

/// Which hand a haptic pulse is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    Left,
    Right,
}

/// A vibration request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HapticPulse {
    pub hand: Hand,
    /// 0..1.
    pub amplitude: f32,
    pub seconds: f32,
}

/// Vibration requests made during the ticks since the presentation side last took them
/// (grabbing makes one). Not simulation state. Game systems may push their own.
#[derive(Resource, Debug, Clone, Default)]
pub struct XrHaptics {
    pub pulses: Vec<HapticPulse>,
}

impl XrHaptics {
    /// Most pulses kept between two frames; older ones are dropped (bounded, decision 0190).
    pub const MAX: usize = 32;

    pub fn push(&mut self, pulse: HapticPulse) {
        if self.pulses.len() >= Self::MAX {
            self.pulses.remove(0);
        }
        self.pulses.push(pulse);
    }
}

/// Presentation hints from this tick. Not simulation state.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct XrComfort {
    /// How strongly to darken the edges of the view (0..1): high while moving by stick or
    /// turning smoothly, so motion in the corner of the eye causes less sickness.
    pub vignette: f32,
}

/// The XR module's systems, for games that must run before or after them (a game's hands
/// reading grip poses after the rig followed its body: `.after(XrSystems::Follow)`).
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum XrSystems {
    /// Locomotion, tracking, hand motion, grab and throw (`Stage::Logic`).
    Tick,
    /// A rig riding a body follows it; tracked entities placed again (`Stage::Effects`).
    Follow,
}

/// The XR gameplay module. Add it to the engine (`Engine::builder().module(XrModule)`).
pub struct XrModule;

impl Module for XrModule {
    fn name(&self) -> &'static str {
        "xr"
    }

    fn register(&self, r: &mut Registry) {
        r.register_component::<XrRig>();
        r.register_component::<XrTracked>();
        r.register_component::<XrHand>();
        r.register_component::<XrHandMotion>();
        r.register_component::<Grabbable>();
        r.register_component::<Held>();
        r.register_component::<XrLocomotion>();
        r.register_component::<XrLocomotionState>();
        r.register_component::<XrCharacter>();
        r.register_component::<XrCharacterState>();
    }

    fn add_systems(&self, schedule: &mut Schedule) {
        // Logic runs before physics, so a throw's velocity is used by this tick's step.
        schedule.add_systems(systems::xr_tick.in_set(Stage::Logic).in_set(XrSystems::Tick));
        // A rig riding a body follows it once physics has moved it, before anything reads the
        // head and hands for the frame.
        schedule.add_systems(
            systems::xr_follow
                .in_set(Stage::Effects)
                .in_set(XrSystems::Follow),
        );
    }

    fn init(&self, world: &mut World) -> marionette_core::Result<()> {
        world.insert_resource(XrHaptics::default());
        world.insert_resource(XrComfort::default());
        Ok(())
    }
}
