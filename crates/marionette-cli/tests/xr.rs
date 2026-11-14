//! XR gameplay with scripted tracked poses (no headset): grab and throw, snap turn, teleport,
//! smooth movement with the comfort vignette, and replay determinism (decision 0190). The
//! last test runs the example on a real OpenXR runtime when `MARIONETTE_XR_RUNTIME=1`
//! (CI runs it on Monado's simulated headset).

use marionette_agent::engine;
use marionette_core::Sim;
use marionette_core::scene::SceneId;
use marionette_core::spatial::{Quat, Transform, Vec3};
use marionette_input::InputEvent;
use marionette_xr::pose::{self, Pose, XrSource};
use marionette_xr::{XrComfort, XrModule};
use std::path::Path;

const SCENE: &str = "../../examples/xr.scene.json";

fn load() -> Sim {
    let _ = XrModule; // the standard engine includes it
    engine::load_file(Path::new(SCENE), "xr.scene.json", 1).expect("example loads")
}

fn pose_at(tick: u64, src: XrSource, p: [f32; 3], q: Quat) -> Vec<InputEvent> {
    pose::events(
        tick,
        src,
        &Pose {
            position: Vec3::new(p[0], p[1], p[2]),
            rotation: q,
            tracked: true,
        },
    )
    .to_vec()
}

fn input(tick: u64, name: &str, value: f32) -> InputEvent {
    InputEvent {
        tick,
        input: Some(name.into()),
        action: None,
        value,
    }
}

fn feed(sim: &mut Sim, events: Vec<InputEvent>) {
    marionette_input::feed(&mut sim.world, &events).expect("events queue");
}

fn transform(sim: &mut Sim, id: u64) -> Transform {
    let mut q = sim.world.query::<(&SceneId, &Transform)>();
    *q.iter(&sim.world).find(|(s, _)| s.0 == id).expect("entity").1
}

fn run_to(sim: &mut Sim, tick: u64) {
    while sim.tick() < tick {
        sim.step();
    }
}

/// Head at standing height looking ahead; the right hand starts on block 3 (id 12), grabs
/// it, swings forward and up at about 6 m/s for a quarter second, and lets go.
fn throw_inputs() -> Vec<InputEvent> {
    throw_with(&[(5, "xr:right/squeeze", 1.0), (25, "xr:right/squeeze", 0.0)])
}

/// The same throw done with a tracked hand and no controller: the palm is the grip and the
/// pinch ramps in and out the way a real thumb and finger do (see `hand_from_joints`).
fn pinch_throw_inputs() -> Vec<InputEvent> {
    throw_with(&[
        (3, "xr:right/pinch", 0.2),
        (4, "xr:right/pinch", 0.45),
        (5, "xr:right/pinch", 0.8),
        (6, "xr:right/pinch", 1.0),
        (24, "xr:right/pinch", 0.6),
        (25, "xr:right/pinch", 0.3),
        (26, "xr:right/pinch", 0.0),
    ])
}

fn throw_with(grab: &[(u64, &str, f32)]) -> Vec<InputEvent> {
    let mut ev = pose_at(0, XrSource::Head, [0.0, 1.6, 0.0], Quat::default());
    let start = [0.3, 0.86, -0.5];
    ev.extend(pose_at(0, XrSource::RightGrip, start, Quat::default()));
    for (tick, name, v) in grab {
        ev.push(input(*tick, name, *v));
    }
    for k in 1..=15u64 {
        let p = [start[0], start[1] + 0.03 * k as f32, start[2] - 0.1 * k as f32];
        ev.extend(pose_at(10 + k, XrSource::RightGrip, p, Quat::default()));
    }
    ev
}

#[test]
fn grab_and_throw_a_block() {
    let mut sim = load();
    feed(&mut sim, throw_inputs());
    run_to(&mut sim, 20);
    let held = transform(&mut sim, 12);
    assert!(
        held.translation.z < -0.9,
        "block follows the hand while held: {held:?}"
    );
    run_to(&mut sim, 240);
    let thrown = transform(&mut sim, 12);
    let untouched = transform(&mut sim, 10);
    assert!(thrown.translation.z < -2.5, "block flew forward: {thrown:?}");
    assert!(thrown.translation.y < 0.3, "and landed on the floor: {thrown:?}");
    assert!(
        (untouched.translation.z + 0.5).abs() < 0.05,
        "other blocks stay: {untouched:?}"
    );
}

#[test]
fn a_pinching_hand_grabs_and_throws_without_a_controller() {
    let mut sim = load();
    feed(&mut sim, pinch_throw_inputs());
    run_to(&mut sim, 4);
    let before = transform(&mut sim, 12);
    assert!(
        (before.translation.z + 0.5).abs() < 0.05,
        "a light pinch (below half) does not grab: {before:?}"
    );
    run_to(&mut sim, 20);
    let held = transform(&mut sim, 12);
    assert!(held.translation.z < -0.9, "block follows the hand: {held:?}");
    run_to(&mut sim, 240);
    let thrown = transform(&mut sim, 12);
    assert!(thrown.translation.z < -2.5, "block flew forward: {thrown:?}");
    assert!(thrown.translation.y < 0.3, "and landed: {thrown:?}");
}

/// The right hand squeezes `above` meters over block 3 (id 12), then rises half a meter.
/// Returns how far the block rose with it.
fn lift_from(above: f32) -> f32 {
    let mut sim = load();
    let start = [0.3, 0.86 + above, -0.5];
    let mut ev = pose_at(0, XrSource::Head, [0.0, 1.6, 0.0], Quat::default());
    ev.extend(pose_at(0, XrSource::RightGrip, start, Quat::default()));
    ev.push(input(5, "xr:right/squeeze", 1.0));
    for k in 1..=10u64 {
        let p = [start[0], start[1] + 0.05 * k as f32, start[2]];
        ev.extend(pose_at(10 + k, XrSource::RightGrip, p, Quat::default()));
    }
    feed(&mut sim, ev);
    run_to(&mut sim, 4);
    let rest = transform(&mut sim, 12).translation.y;
    run_to(&mut sim, 30);
    transform(&mut sim, 12).translation.y - rest
}

#[test]
fn a_hand_grabs_only_within_its_grab_radius() {
    // The example's hands reach 0.15 m.
    let inside = lift_from(0.12);
    assert!(
        (inside - 0.5).abs() < 0.02,
        "a block 0.12 m away is lifted with the hand: {inside}"
    );
    let edge = lift_from(0.145);
    assert!(
        (edge - 0.5).abs() < 0.02,
        "so is one just inside the radius: {edge}"
    );
    for above in [0.16, 0.2, 0.29] {
        let outside = lift_from(above);
        assert!(
            outside.abs() < 0.02,
            "a block {above} m away stays on the table: {outside}"
        );
    }
}

#[test]
fn throws_replay_bit_for_bit() {
    let hash = || {
        let mut sim = load();
        feed(&mut sim, throw_inputs());
        run_to(&mut sim, 120);
        sim.state_hash()
    };
    assert_eq!(hash(), hash());
}

#[test]
fn snap_turn_turns_once_around_the_head() {
    let mut sim = load();
    let mut ev = pose_at(0, XrSource::Head, [0.3, 1.6, 0.2], Quat::default());
    ev.push(input(2, "xr:right/thumbstick_x", 1.0));
    ev.push(input(20, "xr:right/thumbstick_x", 0.0));
    feed(&mut sim, ev);
    run_to(&mut sim, 1);
    let head_before = transform(&mut sim, 2).translation;
    run_to(&mut sim, 30);
    let rig = transform(&mut sim, 1);
    // One 30 degree turn to the right: rotation -30 degrees about +Y.
    let want = -(15f32.to_radians().sin());
    assert!((rig.rotation.y - want).abs() < 1e-4, "{:?}", rig.rotation);
    let head_after = transform(&mut sim, 2).translation;
    let moved = ((head_after.x - head_before.x).powi(2) + (head_after.z - head_before.z).powi(2)).sqrt();
    assert!(
        moved < 1e-4,
        "the head stays where it was: {head_before:?} -> {head_after:?}"
    );
}

#[test]
fn teleport_lands_where_the_hand_points() {
    let mut sim = load();
    let mut ev = pose_at(0, XrSource::Head, [0.0, 1.6, 0.0], Quat::default());
    // Right hand off to the side (clear of the table), pointing 45 degrees down.
    let down = Quat {
        x: -(22.5f32.to_radians().sin()),
        y: 0.0,
        z: 0.0,
        w: 22.5f32.to_radians().cos(),
    };
    ev.extend(pose_at(0, XrSource::RightAim, [2.0, 1.2, -0.2], down));
    ev.push(input(5, "xr:right/primary", 1.0));
    ev.push(input(10, "xr:right/primary", 0.0));
    feed(&mut sim, ev);
    run_to(&mut sim, 15);
    let rig = transform(&mut sim, 1).translation;
    assert!(
        (rig.x - 2.0).abs() < 0.02 && (rig.z + 1.4).abs() < 0.02 && rig.y.abs() < 0.02,
        "{rig:?}"
    );
}

#[test]
fn stick_movement_moves_the_rig_and_asks_for_a_vignette() {
    let mut sim = load();
    let mut ev = pose_at(0, XrSource::Head, [0.0, 1.6, 0.0], Quat::default());
    ev.push(input(1, "xr:left/thumbstick_y", 1.0));
    ev.push(input(61, "xr:left/thumbstick_y", 0.0));
    feed(&mut sim, ev);
    run_to(&mut sim, 30);
    assert!(sim.world.resource::<XrComfort>().vignette > 0.5);
    run_to(&mut sim, 70);
    let rig = transform(&mut sim, 1).translation;
    // 60 ticks at 2 m/s toward -Z.
    assert!((rig.z + 2.0).abs() < 0.05, "{rig:?}");
    assert_eq!(sim.world.resource::<XrComfort>().vignette, 0.0);
}

#[test]
fn example_runs_on_an_openxr_runtime() {
    if std::env::var("MARIONETTE_XR_RUNTIME").as_deref() != Ok("1") {
        eprintln!("skipped: set MARIONETTE_XR_RUNTIME=1 with an OpenXR runtime running");
        return;
    }
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_marionette"))
        .args(["--json", "xr", SCENE, "--frames", "90", "--mute"])
        .output()
        .expect("marionette runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(text.trim()).expect("one JSON object");
    let h = &v["headset"];
    let states: Vec<&str> = h["session_states"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s.as_str())
        .collect();
    assert!(states.contains(&"focused"), "{h}");
    assert_eq!(states.last(), Some(&"exiting"), "{h}");
    assert!(h["frames_rendered"].as_u64().unwrap() >= 60, "{h}");
    assert!(
        h["tracked_sources_max"].as_u64().unwrap() >= 1,
        "the head is tracked: {h}"
    );
}
