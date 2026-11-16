//! A character set aside for game code (decision 0312): with `CharacterPlaced` the body
//! goes exactly where the entity's `Transform` puts it, turned as it says, with no walking,
//! gravity or ground; removing the component hands it back to the controller.

use marionette_core::Sim;
use marionette_core::scene::SceneId;
use marionette_core::spatial::{Quat, Transform, Vec3};
use marionette_physics::PhysicsModule;
use marionette_physics::components::CharacterPlaced;
use serde_json::{Value, json};
use std::sync::Arc;

fn t(p: [f32; 3]) -> Value {
    json!({ "translation": { "x": p[0], "y": p[1], "z": p[2] }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "scale": { "x": 1, "y": 1, "z": 1 } })
}

fn collider(shape: Value) -> Value {
    json!({ "shape": shape, "friction": 0.7, "restitution": 0, "density": 1, "sensor": false })
}

/// A character walking forward on a round world of radius 20 (round, so `PlanetState` and
/// the planet step are covered too).
fn scene() -> String {
    json!({
        "format": "marionette.scene", "version": 1,
        "resources": { "PhysicsSettings": { "mode": "Dim3", "gravity": { "x": 0, "y": 0, "z": 0 } } },
        "entities": [
            { "id": 1, "name": "planet", "components": {
                "Transform": t([0.0, 0.0, 0.0]), "Collider": collider(json!({ "Ball": { "radius": 20.0 } })),
                "GravityWell": { "surface_gravity": 9.81, "radius": 20.0, "range": 0.0 } } },
            { "id": 2, "name": "walker", "components": {
                "Transform": t([0.0, 20.81, 0.0]),
                "RigidBody": { "kind": "Kinematic", "linear_damping": 0, "angular_damping": 0, "gravity_scale": 1, "lock_rotations": true },
                "Collider": collider(json!({ "Capsule": { "half_height": 0.5, "radius": 0.3 } })),
                "CharacterController": { "speed": 4.0, "jump_speed": 5.0, "gravity": 9.81, "mass": 70.0,
                                         "move_x": "", "move_forward": "", "jump": "" },
                "CharacterInput": { "move_x": 0.0, "move_forward": 1.0, "jump": false },
                "PlanetCharacter": { "align_speed": 0.0, "thrust": 0.0, "climb_speed": 0.0, "thrust_action": "", "face_movement": false } } }
        ]
    })
    .to_string()
}

fn sim(text: &str) -> Sim {
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn marionette_core::Module>]);
    Sim::with_modules(text, None, 1, modules, Default::default()).unwrap()
}

fn walker(sim: &mut Sim) -> bevy_ecs::entity::Entity {
    let mut q = sim.world.query::<(bevy_ecs::entity::Entity, &SceneId)>();
    q.iter(&sim.world).find(|(_, i)| i.0 == 2).unwrap().0
}

fn pose(sim: &mut Sim) -> Transform {
    let e = walker(sim);
    *sim.world.get::<Transform>(e).unwrap()
}

#[test]
fn a_placed_character_stays_where_game_code_puts_it() {
    let mut s = sim(&scene());
    for _ in 0..30 {
        s.step();
    }
    let e = walker(&mut s);
    // Lift it 5 m into the air, a quarter turn about up, and set it aside.
    let r = std::f32::consts::FRAC_1_SQRT_2;
    let lifted = Transform {
        translation: Vec3::new(0.0, 26.0, 0.0),
        rotation: Quat {
            x: 0.0,
            y: r,
            z: 0.0,
            w: r,
        },
        scale: Vec3::new(1.0, 1.0, 1.0),
    };
    s.world.entity_mut(e).insert((lifted, CharacterPlaced {}));
    for tick in 0..60 {
        s.step();
        let p = pose(&mut s);
        assert_eq!(
            p.translation, lifted.translation,
            "tick {tick}: not walked, no gravity"
        );
        assert!(
            (p.rotation.y - r).abs() < 1e-5 && (p.rotation.w - r).abs() < 1e-5,
            "tick {tick}: turned as placed"
        );
    }
    // Moved again by game code: it follows.
    let moved = Transform {
        translation: Vec3::new(1.0, 26.5, 0.0),
        ..lifted
    };
    s.world.entity_mut(e).insert(moved);
    s.step();
    assert_eq!(pose(&mut s).translation, moved.translation);
    // Handed back: it falls and stands on the ground again.
    s.world.entity_mut(e).remove::<CharacterPlaced>();
    for _ in 0..240 {
        s.step();
    }
    let p = pose(&mut s).translation;
    let d = (p.x * p.x + p.y * p.y + p.z * p.z).sqrt();
    assert!((d - 20.8).abs() < 0.15, "back on the ground: {d}");
}

#[test]
fn placed_runs_repeat_bit_for_bit() {
    let hash = || {
        let mut s = sim(&scene());
        for i in 0..120 {
            if i == 20 {
                let e = walker(&mut s);
                let mut t = pose(&mut s);
                t.translation.y += 3.0;
                s.world.entity_mut(e).insert((t, CharacterPlaced {}));
            }
            if i == 60 {
                let e = walker(&mut s);
                s.world.entity_mut(e).remove::<CharacterPlaced>();
            }
            s.step();
        }
        s.state_hash()
    };
    assert_eq!(hash(), hash());
}
