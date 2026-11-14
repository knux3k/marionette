//! What physics does with the numbers in a scene, checked against what they mean: gravity
//! and its scale, restitution, friction, sensors, kinematic bodies and locked rotation.

use marionette_core::scene::SceneId;
use marionette_core::spatial::{Quat, Transform, Vec3};
use marionette_core::{EventLog, Module, Sim};
use marionette_physics::PhysicsModule;
use serde_json::{Value, json};
use std::sync::Arc;

const G: f32 = 9.81;
const DT: f32 = 1.0 / 60.0;

fn at(p: [f32; 3], rot: [f32; 4]) -> Value {
    json!({ "translation": { "x": p[0], "y": p[1], "z": p[2] },
            "rotation": { "x": rot[0], "y": rot[1], "z": rot[2], "w": rot[3] },
            "scale": { "x": 1, "y": 1, "z": 1 } })
}

const UPRIGHT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

fn body(kind: &str, gravity_scale: f32, lock: bool) -> Value {
    json!({ "kind": kind, "linear_damping": 0, "angular_damping": 0, "gravity_scale": gravity_scale,
            "lock_rotations": lock })
}

fn ball(friction: f32, restitution: f32) -> Value {
    json!({ "shape": { "Ball": { "radius": 0.5 } }, "friction": friction, "restitution": restitution,
            "density": 1, "sensor": false })
}

fn slab(half: [f32; 3], friction: f32, restitution: f32, sensor: bool) -> Value {
    json!({ "shape": { "Cuboid": { "half_extents": { "x": half[0], "y": half[1], "z": half[2] } } },
            "friction": friction, "restitution": restitution, "density": 1, "sensor": sensor })
}

/// A ground slab whose top is at y = 0.
fn ground(friction: f32, restitution: f32) -> Value {
    json!({ "id": 1, "components": { "Transform": at([0.0, -0.5, 0.0], UPRIGHT),
        "Collider": slab([20.0, 0.5, 20.0], friction, restitution, false) } })
}

fn sim(gravity: f32, entities: Vec<Value>) -> Sim {
    let text = json!({
        "format": "marionette.scene",
        "version": 1,
        "resources": { "PhysicsSettings": { "mode": "Dim3", "gravity": { "x": 0, "y": -gravity, "z": 0 } } },
        "entities": entities,
    })
    .to_string();
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn Module>]);
    Sim::with_modules(&text, None, 0, modules, Default::default()).unwrap()
}

fn place(sim: &mut Sim, id: u64) -> Transform {
    let mut q = sim.world.query::<(&SceneId, &Transform)>();
    *q.iter(&sim.world).find(|(i, _)| i.0 == id).expect("entity").1
}

fn falling(id: u64, y: f32, gravity_scale: f32) -> Value {
    json!({ "id": id, "components": { "Transform": at([id as f32 * 3.0, y, 0.0], UPRIGHT),
        "RigidBody": body("Dynamic", gravity_scale, false), "Collider": ball(0.5, 0.0) } })
}

#[test]
fn free_fall_follows_gravity_and_its_scale() {
    let mut sim = sim(
        G,
        vec![
            falling(2, 100.0, 1.0),
            falling(3, 100.0, 2.0),
            falling(4, 100.0, 0.0),
        ],
    );
    let n = 90;
    sim.run(n);
    // g * t^2 / 2 after a second and a half; stepping in ticks may be off by one tick's worth.
    let t = n as f32 * DT;
    let fall = G * t * t / 2.0;
    let dropped = |sim: &mut Sim, id| 100.0 - place(sim, id).translation.y;
    let one = dropped(&mut sim, 2);
    assert!((one - fall).abs() < G * t * DT, "fell {one} m, expected {fall}");
    let two = dropped(&mut sim, 3);
    assert!(
        (two - 2.0 * one).abs() < 0.001,
        "double gravity scale fell {two} m, not twice {one}"
    );
    assert_eq!(dropped(&mut sim, 4), 0.0, "gravity scale 0 does not fall");
    // Nothing pushes sideways.
    assert_eq!(place(&mut sim, 2).translation.x, 6.0);
}

/// Drops a ball from 3 m (center) onto the ground and returns the height of its center at
/// the top of the first bounce and at the end.
fn bounce(restitution: f32) -> (f32, f32) {
    let mut sim = sim(
        G,
        vec![
            ground(0.5, restitution),
            json!({ "id": 2, "components": { "Transform": at([0.0, 3.0, 0.0], UPRIGHT),
                "RigidBody": body("Dynamic", 1.0, false), "Collider": ball(0.5, restitution) } }),
        ],
    );
    let (mut hit, mut top) = (false, 0.0f32);
    let mut last = 3.0;
    for _ in 0..150 {
        sim.step();
        let y = place(&mut sim, 2).translation.y;
        if !hit && y > last {
            hit = true;
        }
        if hit {
            top = top.max(y);
        }
        last = y;
    }
    sim.run(600);
    (top, place(&mut sim, 2).translation.y)
}

#[test]
fn restitution_sets_how_high_a_ball_bounces() {
    let (top, end) = bounce(0.0);
    assert!(top < 0.56, "no restitution, no bounce: the center rose to {top}");
    assert!((end - 0.5).abs() < 0.02, "the ball rests on the ground: {end}");
    // It fell 2.5 m; a bounce keeps restitution squared of that height.
    let (top, end) = bounce(0.8);
    let rise = top - 0.5;
    assert!(
        (rise - 2.5 * 0.64).abs() < 0.25,
        "restitution 0.8 bounces to about 1.6 m above rest, got {rise}"
    );
    assert!((end - 0.5).abs() < 0.05, "and it comes to rest: {end}");
    let (half, _) = bounce(0.5);
    assert!(
        (half - 0.5 - 2.5 * 0.25).abs() < 0.2,
        "restitution 0.5 bounces to about 0.6 m, got {}",
        half - 0.5
    );
}

/// A box with locked rotation on a slope of 20 degrees; returns how far it moved along x
/// between the first and the third second.
fn slide(friction: f32) -> f32 {
    let (s, c) = (10f32.to_radians().sin(), 10f32.to_radians().cos());
    let tilt = [0.0, 0.0, s, c];
    let mut sim = sim(
        G,
        vec![
            json!({ "id": 1, "components": { "Transform": at([0.0, 0.0, 0.0], tilt),
                "Collider": slab([20.0, 0.5, 20.0], friction, 0.0, false) } }),
            // On the slope's surface normal, 1 m from its middle plane: just above the top.
            json!({ "id": 2, "components": {
                "Transform": at([-(20f32.to_radians().sin()), 20f32.to_radians().cos(), 0.0], tilt),
                "RigidBody": body("Dynamic", 1.0, true),
                "Collider": slab([0.5, 0.45, 0.5], friction, 0.0, false) } }),
        ],
    );
    sim.run(60);
    let x0 = place(&mut sim, 2).translation.x;
    sim.run(120);
    (place(&mut sim, 2).translation.x - x0).abs()
}

#[test]
fn friction_holds_a_box_on_a_slope_or_lets_it_slide() {
    // tan(20 degrees) is 0.36: friction above it holds, friction below it does not.
    let held = slide(1.0);
    assert!(held < 0.02, "friction 1.0 holds the box: it moved {held} m");
    let slid = slide(0.05);
    // Without friction it would cover g * sin(20) * (3^2 - 1^2) / 2 * cos(20) = 12.6 m along x.
    assert!(slid > 8.0 && slid < 13.0, "friction 0.05 lets it slide: {slid} m");
    let grip = slide(0.6);
    assert!(grip < 0.02, "friction 0.6 still holds: {grip} m");
    let slow = slide(0.2);
    assert!(
        slow > 3.0 && slow < slid,
        "friction 0.2 slides, slower: {slow} m and {slid} m"
    );
}

fn platform(sensor: bool) -> Sim {
    sim(
        G,
        vec![
            ground(0.5, 0.0),
            json!({ "id": 5, "components": { "Transform": at([0.0, 3.0, 0.0], UPRIGHT),
                "Collider": slab([2.0, 0.25, 2.0], 0.5, 0.0, sensor) } }),
            json!({ "id": 2, "components": { "Transform": at([0.0, 6.0, 0.0], UPRIGHT),
                "RigidBody": body("Dynamic", 1.0, false), "Collider": ball(0.5, 0.0) } }),
        ],
    )
}

fn events(sim: &Sim, kind: &str, a: u64, b: u64) -> usize {
    sim.world
        .resource::<EventLog>()
        .events
        .iter()
        .filter(|e| e.kind == kind && e.entity == Some(a.min(b)) && e.other == Some(a.max(b)))
        .count()
}

#[test]
fn a_sensor_reports_and_does_not_block_and_a_solid_does_both() {
    let mut solid = platform(false);
    solid.run(240);
    let y = place(&mut solid, 2).translation.y;
    assert!((y - 3.75).abs() < 0.02, "the ball rests on the platform: {y}");
    assert_eq!(events(&solid, "collision_started", 2, 5), 1);
    assert_eq!(events(&solid, "collision_stopped", 2, 5), 0);
    assert_eq!(
        events(&solid, "collision_started", 1, 2),
        0,
        "it never reaches the ground"
    );

    let mut sensor = platform(true);
    sensor.run(240);
    let y = place(&mut sensor, 2).translation.y;
    assert!(
        (y - 0.5).abs() < 0.02,
        "the ball falls through a sensor to the ground: {y}"
    );
    assert_eq!(
        events(&sensor, "collision_started", 2, 5),
        1,
        "entering is reported once"
    );
    assert_eq!(events(&sensor, "collision_stopped", 2, 5), 1, "and so is leaving");
    assert_eq!(events(&sensor, "collision_started", 1, 2), 1);
}

#[test]
fn a_kinematic_body_goes_where_it_is_put_and_pushes() {
    let mut sim = sim(
        0.0,
        vec![
            json!({ "id": 2, "components": { "Transform": at([0.0, 0.0, 0.0], UPRIGHT),
                "RigidBody": body("Kinematic", 1.0, false), "Collider": slab([0.5, 0.5, 0.5], 0.5, 0.0, false) } }),
            json!({ "id": 3, "components": { "Transform": at([1.5, 0.0, 0.0], UPRIGHT),
                "RigidBody": body("Dynamic", 1.0, true), "Collider": ball(0.0, 0.0) } }),
            json!({ "id": 4, "components": { "Transform": at([0.0, 5.0, 0.0], UPRIGHT),
                "RigidBody": body("Dynamic", 1.0, true), "Collider": ball(0.0, 0.0) } }),
        ],
    );
    let entity = {
        let mut q = sim.world.query::<(bevy_ecs::entity::Entity, &SceneId)>();
        q.iter(&sim.world).find(|(_, i)| i.0 == 2).unwrap().0
    };
    for tick in 1..=100 {
        sim.world.get_mut::<Transform>(entity).unwrap().translation.x = 0.02 * tick as f32;
        sim.step();
    }
    let (pusher, pushed, far) = (place(&mut sim, 2), place(&mut sim, 3), place(&mut sim, 4));
    assert_eq!(
        pusher.translation,
        Vec3::new(2.0, 0.0, 0.0),
        "it is exactly where it was put"
    );
    assert!(
        pushed.translation.x >= 2.0 + 1.0 - 0.02,
        "the ball stays ahead of the moving face: {:?}",
        pushed.translation
    );
    assert_eq!(
        far.translation,
        Vec3::new(0.0, 5.0, 0.0),
        "a ball out of the way is not moved"
    );
}

/// A box dropped onto the ground turned 30 degrees about z; returns its rotation at rest.
fn dropped_on_an_edge(lock: bool) -> Quat {
    let (s, c) = (15f32.to_radians().sin(), 15f32.to_radians().cos());
    let mut sim = sim(
        G,
        vec![
            ground(0.5, 0.0),
            json!({ "id": 2, "components": { "Transform": at([0.0, 2.0, 0.0], [0.0, 0.0, s, c]),
                "RigidBody": body("Dynamic", 1.0, lock), "Collider": slab([0.5, 0.5, 0.5], 0.5, 0.0, false) } }),
        ],
    );
    sim.run(300);
    place(&mut sim, 2).rotation
}

#[test]
fn locked_rotation_keeps_a_body_from_turning() {
    let locked = dropped_on_an_edge(true);
    assert!(
        (locked.z - 15f32.to_radians().sin()).abs() < 1e-5,
        "a locked box keeps its 30 degrees: {locked:?}"
    );
    let free = dropped_on_an_edge(false);
    // It falls onto a face: a quarter turn about z from upright, or upright.
    let flat = free.z.abs() < 0.02 || (free.z.abs() - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02;
    assert!(flat, "a free box comes to rest on a face: {free:?}");
}
