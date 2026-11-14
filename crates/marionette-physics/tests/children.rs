//! Decision 0272: a collider on a child follows its parent; a child cannot also be moved by
//! physics.

use marionette_core::module::{RayCaster, RayFilter};
use marionette_core::{Module, Sim};
use marionette_physics::PhysicsModule;
use serde_json::{Value, json};
use std::sync::Arc;

fn place(parent: Option<u64>, x: f32) -> Value {
    let mut v = json!({ "translation": { "x": x, "y": 0, "z": 0 }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
                        "scale": { "x": 1, "y": 1, "z": 1 } });
    if let Some(p) = parent {
        v["parent"] = json!(p);
    }
    v
}

fn load(child: Value) -> Result<Sim, marionette_core::Errors> {
    let text = json!({
        "format": "marionette.scene",
        "version": 1,
        "resources": { "PhysicsSettings": { "mode": "Dim3", "gravity": { "x": 0, "y": 0, "z": 0 } } },
        "entities": [
            { "id": 1, "components": { "Transform": place(None, 0.0),
                "Position": { "x": 0, "y": 0, "z": 0 }, "Velocity": { "x": 6, "y": 0, "z": 0 } } },
            { "id": 2, "components": child },
        ]
    })
    .to_string();
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn Module>]);
    Sim::with_modules(&text, None, 0, modules, Default::default())
}

fn collider() -> Value {
    json!({ "shape": { "Ball": { "radius": 0.5 } }, "friction": 0.5, "restitution": 0, "density": 1, "sensor": false })
}

#[test]
fn a_collider_on_a_child_moves_with_the_parent() {
    let mut sim = load(json!({ "ChildOf": place(Some(1), 2.0), "Collider": collider() })).unwrap();
    let hit = |sim: &Sim, x: f32| {
        let cast = sim.world.resource::<RayCaster>().0;
        cast(
            &sim.world,
            [x, 5.0, 0.0],
            [0.0, -1.0, 0.0],
            10.0,
            RayFilter::default(),
        )
        .map(|h| h.entity)
    };
    sim.run(2);
    assert_eq!(
        hit(&sim, 2.2),
        Some(2),
        "the collider starts 2 m beside its parent"
    );
    // The parent moves 6 m/s: after two seconds the child is 12 m further on.
    sim.run(120);
    assert_eq!(hit(&sim, 2.2), None);
    assert_eq!(hit(&sim, 14.2), Some(2));
}

#[test]
fn a_child_moved_by_physics_is_refused() {
    let body = json!({ "kind": "Dynamic", "linear_damping": 0, "angular_damping": 0, "gravity_scale": 1,
                       "lock_rotations": false });
    let errs = load(json!({ "ChildOf": place(Some(1), 2.0), "Collider": collider(), "RigidBody": body }))
        .err()
        .expect("refused");
    assert_eq!(errs.0[0].code, "physics.child_body");
    assert_eq!(errs.0[0].entity, Some(2));
}

#[test]
fn a_child_of_a_falling_body_is_on_it_in_the_same_tick() {
    use marionette_core::scene::SceneId;
    use marionette_core::spatial::Transform;
    let body = json!({ "kind": "Dynamic", "linear_damping": 0, "angular_damping": 0, "gravity_scale": 1,
                       "lock_rotations": true });
    let text = json!({
        "format": "marionette.scene",
        "version": 1,
        "resources": { "PhysicsSettings": { "mode": "Dim3", "gravity": { "x": 0, "y": -9.81, "z": 0 } } },
        "entities": [
            { "id": 1, "components": { "Transform": place(None, 0.0), "Collider": collider(), "RigidBody": body } },
            { "id": 2, "components": { "ChildOf": place(Some(1), 2.0) } },
        ]
    })
    .to_string();
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn Module>]);
    let mut sim = Sim::with_modules(&text, None, 0, modules, Default::default()).unwrap();
    for _ in 0..30 {
        sim.step();
        let mut q = sim.world.query::<(&SceneId, &Transform)>();
        let mut at = |id| q.iter(&sim.world).find(|(i, _)| i.0 == id).unwrap().1.translation;
        let (body, child) = (at(1), at(2));
        assert_eq!((child.x, child.y, child.z), (body.x + 2.0, body.y, body.z));
    }
    let mut q = sim.world.query::<(&SceneId, &Transform)>();
    assert!(q.iter(&sim.world).all(|(_, t)| t.translation.y < -0.5), "it fell");
}
