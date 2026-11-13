//! Round-world movement (decision 0230): characters walk all the way round a planet with
//! up following their position, dynamic bodies fall toward the centre, a character on a
//! moon stands on the moon, and runs repeat bit for bit.

use marionette_core::Sim;
use marionette_core::scene::SceneId;
use marionette_core::spatial::Transform;
use marionette_physics::{PhysicsModule, PlanetState};
use serde_json::{Value, json};
use std::sync::Arc;

fn t(p: [f32; 3]) -> Value {
    json!({ "translation": { "x": p[0], "y": p[1], "z": p[2] }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "scale": { "x": 1, "y": 1, "z": 1 } })
}

fn collider(shape: Value) -> Value {
    json!({ "shape": shape, "friction": 0.7, "restitution": 0, "density": 1, "sensor": false })
}

fn world(r: f32, walker: Value) -> String {
    json!({
        "format": "marionette.scene", "version": 1,
        "resources": { "PhysicsSettings": { "mode": "Dim3", "gravity": { "x": 0, "y": 0, "z": 0 } } },
        "entities": [
            { "id": 1, "name": "planet", "components": {
                "Transform": t([0.0, 0.0, 0.0]), "Collider": collider(json!({ "Ball": { "radius": r } })),
                "GravityWell": { "surface_gravity": 9.81, "radius": r, "range": 0.0 } } },
            { "id": 2, "name": "moon", "components": {
                "Transform": t([0.0, 200.0, 0.0]), "Collider": collider(json!({ "Ball": { "radius": 10.0 } })),
                "GravityWell": { "surface_gravity": 3.0, "radius": 10.0, "range": 60.0 } } },
            { "id": 3, "name": "walker", "components": walker },
            { "id": 4, "name": "crate", "components": {
                "Transform": t([r + 8.0, 0.0, 0.0]),
                "RigidBody": { "kind": "Dynamic", "linear_damping": 0.1, "angular_damping": 0.1, "gravity_scale": 1, "lock_rotations": false },
                "Collider": collider(json!({ "Cuboid": { "half_extents": { "x": 0.5, "y": 0.5, "z": 0.5 } } })),
                "Velocity": { "x": 0, "y": 0, "z": 0 }, "AngularVelocity": { "x": 0, "y": 0, "z": 0 } } }
        ]
    })
    .to_string()
}

fn walker(at: [f32; 3], forward: f32) -> Value {
    json!({
        "Transform": t(at),
        "RigidBody": { "kind": "Kinematic", "linear_damping": 0, "angular_damping": 0, "gravity_scale": 1, "lock_rotations": true },
        "Collider": collider(json!({ "Capsule": { "half_height": 0.5, "radius": 0.3 } })),
        "CharacterController": { "speed": 4.0, "jump_speed": 5.0, "gravity": 9.81, "mass": 70.0,
                                 "move_x": "", "move_forward": "", "jump": "" },
        "CharacterInput": { "move_x": 0.0, "move_forward": forward, "jump": false },
        "PlanetCharacter": { "align_speed": 0.0, "thrust": 0.0, "thrust_action": "", "face_movement": false }
    })
}

fn sim(text: &str) -> Sim {
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn marionette_core::Module>]);
    Sim::with_modules(text, None, 1, modules, Default::default()).unwrap()
}

fn pos(sim: &mut Sim, id: u64) -> [f32; 3] {
    let mut q = sim.world.query::<(&SceneId, &Transform)>();
    let t = q.iter(&sim.world).find(|(i, _)| i.0 == id).unwrap().1.translation;
    [t.x, t.y, t.z]
}

fn dist(p: [f32; 3], c: [f32; 3]) -> f32 {
    ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt()
}

#[test]
fn a_character_walks_round_the_planet_standing_up() {
    let r = 20.0;
    let mut s = sim(&world(r, walker([0.0, r + 0.81, 0.0], 1.0)));
    let start = pos(&mut s, 3);
    let mut far = 0.0f32;
    // Circumference 2π·20 ≈ 126 m at 4 m/s: about 31 s. Walk 40 s.
    for tick in 0..2400 {
        s.step();
        let p = pos(&mut s, 3);
        let d = dist(p, [0.0; 3]);
        if tick > 60 {
            assert!((d - (r + 0.8)).abs() < 0.15, "tick {tick}: off the ground at {d}");
        }
        far = far.max(dist(p, start));
        let st = {
            let mut q = s.world.query::<(&SceneId, &PlanetState)>();
            *q.iter(&s.world).find(|(i, _)| i.0 == 3).unwrap().1
        };
        let up = [st.up.x, st.up.y, st.up.z];
        let radial = [p[0] / d, p[1] / d, p[2] / d];
        let cos = up[0] * radial[0] + up[1] * radial[1] + up[2] * radial[2];
        assert!(cos > 0.999, "tick {tick}: up {up:?} is not radial {radial:?}");
    }
    assert!(far > 2.0 * r, "went to the far side: {far}");
    // Dynamic bodies fall to the planet and rest on it.
    let c = pos(&mut s, 4);
    let d = dist(c, [0.0; 3]);
    assert!(d > r + 0.4 && d < r + 0.95, "crate rests on the ground: {d}");
}

#[test]
fn a_character_on_a_moon_stands_on_the_moon() {
    let mut s = sim(&world(20.0, walker([0.0, 200.0 + 10.85, 0.0], 0.5)));
    for _ in 0..600 {
        s.step();
    }
    let p = pos(&mut s, 3);
    let d = dist(p, [0.0, 200.0, 0.0]);
    assert!((d - 10.8).abs() < 0.2, "on the moon: {d}");
}

#[test]
fn planet_runs_repeat_bit_for_bit() {
    let text = world(20.0, walker([0.0, 20.81, 0.0], 1.0));
    let hash = || {
        let mut s = sim(&text);
        for _ in 0..300 {
            s.step();
        }
        s.state_hash()
    };
    assert_eq!(hash(), hash());
}
