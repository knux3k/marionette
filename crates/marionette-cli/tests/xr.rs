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

/// The example with the rig riding a capsule body (decision 0191). `planet`: the floor and
/// table give way to a round world of radius 20 centred 20 m below the origin.
fn riding(planet: bool) -> Sim {
    let text = std::fs::read_to_string(SCENE).expect("example reads");
    let mut scene: serde_json::Value = serde_json::from_str(&text).expect("example parses");
    let ents = scene["entities"].as_array_mut().expect("entities");
    if planet {
        ents.retain(|e| e["name"] != "floor" && e["name"] != "table");
    }
    for e in ents.iter_mut() {
        if e["name"] == "rig" {
            e["components"]["XrCharacter"] = serde_json::json!({"body": 50, "follow_head": true});
            e["components"]["XrLocomotion"]["speed"] = serde_json::json!(4.0);
        }
    }
    let transform = |x: f32, y: f32, z: f32| {
        serde_json::json!({"translation": {"x": x, "y": y, "z": z},
            "rotation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0},
            "scale": {"x": 1.0, "y": 1.0, "z": 1.0}})
    };
    let mut body = serde_json::json!({"id": 50, "name": "body", "components": {
        "Transform": transform(0.0, 0.95, 0.8),
        "RigidBody": {"kind": "Kinematic", "linear_damping": 0.0, "angular_damping": 0.0,
            "gravity_scale": 1.0, "lock_rotations": true},
        "Collider": {"shape": {"Capsule": {"half_height": 0.6, "radius": 0.3}},
            "friction": 0.7, "restitution": 0.0, "density": 1.0, "sensor": false},
        "CharacterController": {"speed": 5.0, "jump_speed": 4.0, "gravity": 9.81, "mass": 70.0,
            "move_x": "", "move_forward": "", "jump": ""}}});
    if planet {
        body["components"]["PlanetCharacter"] = serde_json::json!({"align_speed": 0.0,
            "thrust": 0.0, "climb_speed": 0.0, "thrust_action": "", "face_movement": false});
        ents.push(serde_json::json!({"id": 51, "name": "planet", "components": {
            "Transform": transform(0.0, -20.0, 0.0),
            "Collider": {"shape": {"Ball": {"radius": 20.0}}, "friction": 0.7,
                "restitution": 0.0, "density": 1.0, "sensor": false},
            "GravityWell": {"surface_gravity": 9.81, "radius": 20.0, "range": 0.0}}}));
    }
    ents.push(body);
    let text = serde_json::to_string(&scene).expect("scene writes");
    engine::Engine::standard()
        .load_text(&text, Some("riding"), 1, Path::new("../../examples"))
        .expect("riding scene loads")
}

fn flat_gap(a: Vec3, b: Vec3) -> f32 {
    ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

#[test]
fn a_rig_riding_a_body_walks_by_physics_and_is_stopped_by_the_table() {
    let mut sim = riding(false);
    let mut ev = pose_at(0, XrSource::Head, [0.2, 1.6, 0.1], Quat::default());
    ev.push(input(5, "xr:left/thumbstick_y", 1.0));
    ev.push(input(95, "xr:left/thumbstick_y", 0.0));
    feed(&mut sim, ev);
    run_to(&mut sim, 3);
    let body = transform(&mut sim, 50).translation;
    let head = transform(&mut sim, 2).translation;
    assert!(
        flat_gap(body, head) < 0.02,
        "the head starts over the body: {body:?} {head:?}"
    );
    run_to(&mut sim, 110);
    let body = transform(&mut sim, 50).translation;
    let head = transform(&mut sim, 2).translation;
    // 1.5 s at 4 m/s would reach z = -5.2; the table's front is at z = -0.3 and the capsule
    // is 0.3 wide, so the body stops at about z = 0.
    assert!(body.z > -0.1 && body.z < 0.2, "stopped by the table: {body:?}");
    assert!(
        flat_gap(body, head) < 0.05,
        "the head stays over the body: {body:?} {head:?}"
    );
    assert!((head.y - 1.6).abs() < 0.05, "standing height: {head:?}");
}

#[test]
fn walking_in_the_room_moves_the_body_under_the_head() {
    let mut sim = riding(false);
    let mut ev = Vec::new();
    for k in 0..=30u64 {
        ev.extend(pose_at(
            k,
            XrSource::Head,
            [0.6 * k as f32 / 30.0, 1.6, 0.0],
            Quat::default(),
        ));
    }
    feed(&mut sim, ev);
    run_to(&mut sim, 2);
    let rig_before = transform(&mut sim, 1).translation;
    run_to(&mut sim, 45);
    let rig = transform(&mut sim, 1).translation;
    let body = transform(&mut sim, 50).translation;
    let head = transform(&mut sim, 2).translation;
    assert!(
        flat_gap(rig, rig_before) < 1e-3,
        "the play space stays: {rig_before:?} -> {rig:?}"
    );
    assert!(
        flat_gap(body, head) < 0.03,
        "the body followed the head: {body:?} {head:?}"
    );
    assert!((body.x - 0.6).abs() < 0.03, "{body:?}");
}

#[test]
fn a_rig_riding_a_body_stands_upright_around_a_round_world() {
    let mut sim = riding(true);
    let mut ev = pose_at(0, XrSource::Head, [0.0, 1.6, 0.0], Quat::default());
    ev.push(input(30, "xr:left/thumbstick_y", 1.0));
    ev.push(input(330, "xr:left/thumbstick_y", 0.0));
    feed(&mut sim, ev);
    run_to(&mut sim, 360);
    let centre = Vec3::new(0.0, -20.0, 0.0);
    let body = transform(&mut sim, 50).translation;
    let rig = transform(&mut sim, 1);
    let head = transform(&mut sim, 2).translation;
    let up = marionette_xr::math::normalize(marionette_xr::math::sub(body, centre));
    // 5 s at 4 m/s along a radius-20 world is about a radian of arc.
    assert!(up.y < 0.75 && up.z < -0.6, "walked a long way round: {up:?}");
    let rig_up = marionette_xr::math::rotate(rig.rotation, Vec3::new(0.0, 1.0, 0.0));
    assert!(
        marionette_xr::math::dot(rig_up, up) > 0.999,
        "upright: {rig_up:?} vs {up:?}"
    );
    let floor = marionette_xr::math::length(marionette_xr::math::sub(rig.translation, centre));
    assert!(
        (floor - 20.0).abs() < 0.05,
        "the play space floor is on the ground: {floor}"
    );
    let eye = marionette_xr::math::length(marionette_xr::math::sub(head, centre));
    assert!(
        (eye - 21.6).abs() < 0.05,
        "standing height above the ground: {eye}"
    );
}

#[test]
fn riding_replays_bit_for_bit() {
    let run = || {
        let mut sim = riding(true);
        let mut ev = pose_at(0, XrSource::Head, [0.1, 1.6, 0.0], Quat::default());
        ev.push(input(10, "xr:left/thumbstick_y", 1.0));
        ev.push(input(15, "xr:right/thumbstick_x", 1.0));
        ev.push(input(30, "xr:right/thumbstick_x", 0.0));
        ev.push(input(120, "xr:left/thumbstick_y", 0.0));
        feed(&mut sim, ev);
        run_to(&mut sim, 150);
        sim.state_hash()
    };
    assert_eq!(run(), run());
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
