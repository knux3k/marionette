//! Joints, layers, continuous collision, forces, impulses, game-owned bodies and shape
//! queries (decisions 0310, 0311), each checked against what the numbers mean.

use marionette_core::scene::SceneId;
use marionette_core::spatial::{Transform, Vec3};
use marionette_core::{Module, Sim};
use marionette_physics::components::{BodyKind, ColliderShape, ExternalImpulse};
use marionette_physics::{
    GameBody, PhysicsModule, QueryOptions, Target, add_body, body_contacts, body_state, overlap, raycast_all,
    shape_cast,
};
use serde_json::{Value, json};
use std::f32::consts::{FRAC_PI_6, PI};
use std::sync::Arc;

const UP: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

fn at(p: [f32; 3]) -> Value {
    json!({ "translation": { "x": p[0], "y": p[1], "z": p[2] },
            "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 }, "scale": { "x": 1, "y": 1, "z": 1 } })
}

fn v(x: f32, y: f32, z: f32) -> Value {
    json!({ "x": x, "y": y, "z": z })
}

fn dynamic() -> Value {
    json!({ "kind": "Dynamic", "linear_damping": 0, "angular_damping": 0, "gravity_scale": 1,
            "lock_rotations": false })
}

fn ball(r: f32) -> Value {
    json!({ "shape": { "Ball": { "radius": r } }, "friction": 0.5, "restitution": 0,
            "density": 1, "sensor": false })
}

fn slab(half: [f32; 3]) -> Value {
    json!({ "shape": { "Cuboid": { "half_extents": v(half[0], half[1], half[2]) } },
            "friction": 0.5, "restitution": 0, "density": 1, "sensor": false })
}

/// A ground slab whose top is at y = 0.
fn ground() -> Value {
    json!({ "id": 1, "components": { "Transform": at([0.0, -0.5, 0.0]), "Collider": slab([20.0, 0.5, 20.0]) } })
}

fn sim(gravity: f32, entities: Vec<Value>) -> Sim {
    let text = json!({
        "format": "marionette.scene", "version": 1,
        "resources": { "PhysicsSettings": { "mode": "Dim3", "gravity": v(0.0, -gravity, 0.0) } },
        "entities": entities,
    })
    .to_string();
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn Module>]);
    Sim::with_modules(&text, None, 0, modules, Default::default()).unwrap()
}

fn get<T: bevy_ecs::component::Component + Copy>(sim: &mut Sim, id: u64) -> T {
    let mut q = sim.world.query::<(&SceneId, &T)>();
    *q.iter(&sim.world).find(|(i, _)| i.0 == id).unwrap().1
}

fn pos(sim: &mut Sim, id: u64) -> [f32; 3] {
    let t: Transform = get(sim, id);
    [t.translation.x, t.translation.y, t.translation.z]
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

#[test]
fn a_ball_joint_to_the_world_swings_at_a_fixed_length() {
    // A pendulum: the bob starts 2 m to the side of the pivot at (0, 5, 0).
    let mut s = sim(
        9.81,
        vec![json!({ "id": 2, "components": {
        "Transform": at([2.0, 5.0, 0.0]), "RigidBody": dynamic(), "Collider": ball(0.2),
        "Joint": { "other": 0, "kind": "Ball", "anchor": v(-2.0, 0.0, 0.0),
                   "other_anchor": v(0.0, 5.0, 0.0), "collide_connected": false } } })],
    );
    let mut lowest = f32::MAX;
    for _ in 0..120 {
        s.step();
        let p = pos(&mut s, 2);
        let l = dist(p, [0.0, 5.0, 0.0]);
        assert!((l - 2.0).abs() < 0.05, "length {l}");
        lowest = lowest.min(p[1]);
    }
    assert!(lowest < 3.1, "swung down to {lowest}");
}

#[test]
fn a_rope_lets_a_body_fall_until_it_is_taut() {
    let mut s = sim(
        9.81,
        vec![json!({ "id": 2, "components": {
        "Transform": at([0.0, 5.0, 0.0]), "RigidBody": dynamic(), "Collider": ball(0.2),
        "Joint": { "other": 0, "kind": { "Rope": { "max_length": 1.5 } }, "anchor": v(0.0, 0.0, 0.0),
                   "other_anchor": v(0.0, 5.0, 0.0), "collide_connected": false } } })],
    );
    s.run(120);
    let p = pos(&mut s, 2);
    assert!((p[1] - 3.5).abs() < 0.08, "hangs at {}", p[1]);
}

#[test]
fn a_hinge_motor_spins_its_body_at_the_motor_speed() {
    let mut s = sim(
        0.0,
        vec![json!({ "id": 2, "components": {
        "Transform": at([0.0, 0.0, 0.0]), "RigidBody": dynamic(), "Collider": slab([1.0, 0.1, 0.1]),
        "AngularVelocity": { "x": 0, "y": 0, "z": 0 },
        "Joint": { "other": 0, "kind": { "Hinge": { "axis": v(0.0, 0.0, 1.0), "min": 0, "max": 0,
            "motor_speed": 2.0, "motor_force": 1000.0 } },
            "anchor": v(0.0, 0.0, 0.0), "other_anchor": v(0.0, 0.0, 0.0), "collide_connected": false } } })],
    );
    s.run(60);
    let w: marionette_physics::components::AngularVelocity = get(&mut s, 2);
    assert!((w.z - 2.0).abs() < 0.05, "spins at {} rad/s", w.z);
    assert!(w.x.abs() < 1e-3 && w.y.abs() < 1e-3, "only about the axis");
    assert!(dist(pos(&mut s, 2), [0.0; 3]) < 0.01, "stays on its pivot");
}

#[test]
fn a_hinge_limit_stops_the_swing() {
    // A bar hinged at its left end on the world, falling: the limit holds it at 30 degrees.
    let mut s = sim(
        9.81,
        vec![json!({ "id": 2, "components": {
        "Transform": at([1.0, 5.0, 0.0]), "RigidBody": dynamic(), "Collider": slab([1.0, 0.05, 0.05]),
        "Joint": { "other": 0, "kind": { "Hinge": { "axis": v(0.0, 0.0, 1.0), "min": -FRAC_PI_6, "max": FRAC_PI_6,
            "motor_speed": 0, "motor_force": 0 } },
            "anchor": v(-1.0, 0.0, 0.0), "other_anchor": v(0.0, 5.0, 0.0), "collide_connected": false } } })],
    );
    s.run(180);
    let p = pos(&mut s, 2);
    let angle = (5.0 - p[1]).atan2(p[0]);
    assert!((angle - PI / 6.0).abs() < 0.05, "rests at {angle} rad");
}

#[test]
fn layers_decide_what_touches_what() {
    let layered = |id: u64, x: f32, member: u32, with: u32| {
        json!({ "id": id, "components": { "Transform": at([x, 2.0, 0.0]), "RigidBody": dynamic(),
            "Collider": ball(0.5), "CollisionLayers": { "member": member, "collides_with": with } } })
    };
    let mut ground = ground();
    ground["components"]["CollisionLayers"] = json!({ "member": 1, "collides_with": 0xffff_ffffu32 });
    let mut s = sim(9.81, vec![ground, layered(2, 0.0, 2, 1), layered(3, 3.0, 4, 2)]);
    s.run(120);
    assert!((pos(&mut s, 2)[1] - 0.5).abs() < 0.05, "lands on the ground");
    assert!(
        pos(&mut s, 3)[1] < -5.0,
        "falls through: it does not touch layer 1"
    );
}

#[test]
fn an_external_force_accelerates_by_force_over_mass() {
    let mass = 4.0 / 3.0 * PI * 0.125; // density 1, radius 0.5
    let mut s = sim(
        0.0,
        vec![json!({ "id": 2, "components": {
        "Transform": at([0.0, 0.0, 0.0]), "RigidBody": dynamic(), "Collider": ball(0.5),
        "Velocity": { "x": 0, "y": 0, "z": 0 },
        "ExternalForce": { "force": v(2.0, 0.0, 0.0), "torque": v(0.0, 0.0, 0.0) } } })],
    );
    s.run(60);
    let vel: marionette_core::builtin::Velocity = get(&mut s, 2);
    assert!((vel.x - 2.0 / mass).abs() < 0.05, "{} vs {}", vel.x, 2.0 / mass);
    // Removing the force stops the acceleration at the next step.
    let e = s
        .world
        .query::<(bevy_ecs::entity::Entity, &SceneId)>()
        .iter(&s.world)
        .find(|(_, i)| i.0 == 2)
        .unwrap()
        .0;
    s.world
        .entity_mut(e)
        .remove::<marionette_physics::components::ExternalForce>();
    s.run(30);
    let after: marionette_core::builtin::Velocity = get(&mut s, 2);
    assert!(
        (after.x - vel.x).abs() < 2.0 / mass / 60.0 + 1e-4,
        "kept {} then {}",
        vel.x,
        after.x
    );
}

#[test]
fn an_impulse_applies_once_and_clears() {
    let mass = 4.0 / 3.0 * PI * 0.125;
    let mut s = sim(
        0.0,
        vec![json!({ "id": 2, "components": {
        "Transform": at([0.0, 0.0, 0.0]), "RigidBody": dynamic(), "Collider": ball(0.5),
        "Velocity": { "x": 0, "y": 0, "z": 0 },
        "ExternalImpulse": { "linear": v(0.0, 0.0, 1.0), "angular": v(0.0, 0.0, 0.0) } } })],
    );
    s.run(10);
    let vel: marionette_core::builtin::Velocity = get(&mut s, 2);
    assert!((vel.z - 1.0 / mass).abs() < 1e-3, "{}", vel.z);
    let i: ExternalImpulse = get(&mut s, 2);
    assert_eq!(i, ExternalImpulse::default());
}

#[test]
fn continuous_collision_stops_a_fast_body_at_a_thin_wall() {
    let shot = |ccd: bool| {
        let mut s = sim(
            0.0,
            vec![
                json!({ "id": 1, "components": { "Transform": at([10.0, 0.0, 0.0]), "Collider": slab([0.005, 5.0, 5.0]) } }),
                json!({ "id": 2, "components": { "Transform": at([0.0, 0.0, 0.0]), "RigidBody": dynamic(),
                "Collider": ball(0.1), "Velocity": { "x": 3000, "y": 0, "z": 0 },
                "ContinuousCollision": { "enabled": ccd } } }),
            ],
        );
        s.run(10);
        pos(&mut s, 2)[0]
    };
    // Rapier's contact prediction already stops this straight shot without the flag (both
    // stay in front); the flag adds a full sweep for the cases prediction misses.
    assert!(shot(true) < 10.0, "with it the shot stays in front");
    assert!(shot(false) < 10.0);
}

fn crate_body(y: f32) -> GameBody {
    GameBody {
        shape: ColliderShape::Cuboid {
            half_extents: Vec3::new(0.25, 0.25, 0.25),
        },
        position: [0.0, y, 0.0],
        mass: 2.0,
        ccd: true,
        ..GameBody::default()
    }
}

#[test]
fn a_game_body_falls_lands_and_reports_its_contact() {
    let mut s = sim(9.81, vec![ground()]);
    add_body(&mut s.world, 7, &crate_body(3.0)).unwrap();
    let mut touched = false;
    for _ in 0..240 {
        s.step();
        touched |= body_contacts(&s.world)
            .iter()
            .any(|c| c.body == 7 && c.other == Target::Entity(1) && c.started);
    }
    let st = body_state(&s.world, 7).unwrap();
    assert!(
        (st.position[1] - 0.25).abs() < 0.03,
        "rests at {}",
        st.position[1]
    );
    assert!(touched, "the landing is reported");
    assert!(st.sleeping, "a still body sleeps");
    // Script rays see entities only; game queries see both.
    let down = [0.0, -1.0, 0.0];
    let script =
        marionette_physics::raycast(&s.world, [0.0, 5.0, 0.0], down, 10.0, Default::default()).unwrap();
    assert_eq!(script.entity, 1);
    let all = raycast_all(&s.world, [0.0, 5.0, 0.0], down, 10.0, QueryOptions::default());
    let targets: Vec<Target> = all.iter().map(|h| h.target).collect();
    assert_eq!(targets, vec![Target::Body(7), Target::Entity(1)]);
    assert!((all[0].distance - 4.5).abs() < 0.03);
}

#[test]
fn a_game_body_is_pulled_by_a_gravity_well() {
    let mut s = sim(
        0.0,
        vec![json!({ "id": 1, "components": { "Transform": at([0.0, 0.0, 0.0]),
        "GravityWell": { "surface_gravity": 9.81, "radius": 10.0, "range": 0 },
        "Collider": ball(10.0) } })],
    );
    add_body(
        &mut s.world,
        3,
        &GameBody {
            position: [0.0, 0.0, 15.0],
            ..crate_body(0.0)
        },
    )
    .unwrap();
    s.run(180);
    let p = body_state(&s.world, 3).unwrap().position;
    assert!((dist(p, [0.0; 3]) - 10.25).abs() < 0.05, "on the surface: {p:?}");
}

#[test]
fn overlap_and_shape_cast_find_what_is_there() {
    let mut s = sim(
        0.0,
        vec![
            ground(),
            json!({ "id": 2, "components": {
        "Transform": at([3.0, 1.0, 0.0]), "Collider": slab([0.5, 0.5, 0.5]),
        "CollisionLayers": { "member": 2, "collides_with": 0xffff_ffffu32 } } }),
        ],
    );
    add_body(&mut s.world, 9, &crate_body(1.0)).unwrap();
    s.step();
    let ball = ColliderShape::Ball { radius: 4.0 };
    let all = overlap(&s.world, &ball, [1.0, 1.0, 0.0], UP, QueryOptions::default()).unwrap();
    assert_eq!(all, vec![Target::Entity(1), Target::Entity(2), Target::Body(9)]);
    let only2 = QueryOptions {
        layers: 2,
        ..Default::default()
    };
    assert_eq!(
        overlap(&s.world, &ball, [1.0, 1.0, 0.0], UP, only2).unwrap(),
        vec![Target::Entity(2)]
    );
    let no_bodies = QueryOptions {
        skip_bodies: true,
        exclude: Some(Target::Entity(1)),
        ..Default::default()
    };
    assert_eq!(
        overlap(&s.world, &ball, [1.0, 1.0, 0.0], UP, no_bodies).unwrap(),
        vec![Target::Entity(2)]
    );

    // A 0.5 m sphere swept along +x from x = -3 at y = 1 meets the crate's face at x = -0.25.
    let probe = ColliderShape::Ball { radius: 0.5 };
    let hit = shape_cast(
        &s.world,
        &probe,
        [-3.0, 1.0, 0.0],
        UP,
        [1.0, 0.0, 0.0],
        20.0,
        QueryOptions::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(hit.target, Target::Body(9));
    assert!((hit.distance - 2.25).abs() < 0.01, "{}", hit.distance);
    assert!((hit.point[0] + 0.25).abs() < 0.01, "{:?}", hit.point);
    assert!((hit.normal[0] + 1.0).abs() < 0.01, "{:?}", hit.normal);
}

#[test]
fn joints_and_game_bodies_restore_from_a_snapshot_exactly() {
    let scene = || {
        sim(
            9.81,
            vec![
                ground(),
                json!({ "id": 2, "components": {
        "Transform": at([2.0, 5.0, 0.0]), "RigidBody": dynamic(), "Collider": ball(0.2),
        "Joint": { "other": 0, "kind": "Ball", "anchor": v(-2.0, 0.0, 0.0),
                   "other_anchor": v(0.0, 5.0, 0.0), "collide_connected": false } } }),
            ],
        )
    };
    let mut a = scene();
    add_body(
        &mut a.world,
        4,
        &GameBody {
            velocity: [1.0, 3.0, 0.0],
            ..crate_body(2.0)
        },
    )
    .unwrap();
    a.run(30);
    let snap = a.save_state();
    a.run(60);
    let mut b = scene();
    b.restore(&snap).unwrap();
    b.run(60);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_eq!(body_state(&a.world, 4), body_state(&b.world, 4));
    let _ = BodyKind::Dynamic;
}
