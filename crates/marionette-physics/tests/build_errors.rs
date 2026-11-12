//! Scenes that physics must refuse when it builds its world, with stable codes and the
//! entity involved.

use marionette_core::{Diagnostic, Sim};
use marionette_physics::PhysicsModule;
use serde_json::{Value, json};
use std::sync::Arc;

fn scene(mode: &str, components: Value) -> String {
    json!({
        "format": "marionette.scene",
        "version": 1,
        "resources": { "PhysicsSettings": { "mode": mode, "gravity": { "x": 0, "y": -9.81, "z": 0 } } },
        "entities": [ { "id": 7, "name": "body", "components": components } ]
    })
    .to_string()
}

fn transform() -> Value {
    json!({ "translation": { "x": 0, "y": 1, "z": 0 }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "scale": { "x": 1, "y": 1, "z": 1 } })
}

fn body() -> Value {
    json!({ "kind": "Dynamic", "linear_damping": 0, "angular_damping": 0, "gravity_scale": 1, "lock_rotations": false })
}

fn collider(shape: Value) -> Value {
    json!({ "shape": shape, "friction": 0.5, "restitution": 0, "density": 1, "sensor": false })
}

fn refused(mode: &str, components: Value) -> Diagnostic {
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn marionette_core::Module>]);
    let text = scene(mode, components);
    match Sim::with_modules(&text, Some("t.scene.json"), 0, modules, Default::default()) {
        Ok(_) => panic!("scene was accepted"),
        Err(e) => {
            assert_eq!(e.0.len(), 1, "{e}");
            e.0[0].clone()
        }
    }
}

#[test]
fn a_body_without_a_collider_is_refused() {
    let d = refused("Dim3", json!({ "Transform": transform(), "RigidBody": body() }));
    assert_eq!(d.code, "physics.missing_collider");
    assert_eq!(d.entity, Some(7));
    assert_eq!(d.file.as_deref(), Some("t.scene.json"));
}

#[test]
fn a_body_with_position_is_refused() {
    let d = refused(
        "Dim3",
        json!({ "Transform": transform(), "RigidBody": body(),
                "Collider": collider(json!({ "Ball": { "radius": 0.5 } })),
                "Position": { "x": 0, "y": 1, "z": 0 } }),
    );
    assert_eq!(d.code, "physics.position_conflict");
    assert_eq!(d.entity, Some(7));
}

#[test]
fn shapes_that_2d_lacks_are_refused_with_the_alternatives() {
    for (shape, name) in [
        (
            json!({ "Cylinder": { "half_height": 1, "radius": 0.5 } }),
            "Cylinder",
        ),
        (
            json!({ "ConvexHull": { "model": "primitive:cube" } }),
            "ConvexHull",
        ),
        (json!({ "TriMesh": { "model": "primitive:cube" } }), "TriMesh"),
    ] {
        let d = refused(
            "Dim2",
            json!({ "Transform": transform(), "RigidBody": body(), "Collider": collider(shape) }),
        );
        assert_eq!(d.code, "physics.shape");
        assert_eq!(d.entity, Some(7));
        assert_eq!(d.component.as_deref(), Some("Collider"));
        assert!(d.message.starts_with(name), "{}", d.message);
        assert!(d.message.contains("Cuboid, Ball or Capsule"), "{}", d.message);
    }
}

#[test]
fn a_mesh_shape_of_a_missing_model_is_refused() {
    let d = refused(
        "Dim3",
        json!({ "Transform": transform(), "RigidBody": body(),
                "Collider": collider(json!({ "ConvexHull": { "model": "primitive:teapot" } })) }),
    );
    assert_eq!(d.code, "physics.shape");
    assert!(d.message.contains("teapot"), "{}", d.message);
}

#[test]
fn the_same_scene_is_accepted_with_a_collider() {
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn marionette_core::Module>]);
    let text = scene(
        "Dim3",
        json!({ "Transform": transform(), "RigidBody": body(),
                "Collider": collider(json!({ "Ball": { "radius": 0.5 } })) }),
    );
    let mut sim = Sim::with_modules(&text, None, 0, modules, Default::default()).unwrap();
    sim.run(30);
    assert_eq!(sim.tick(), 30);
}
