//! Colliders built from game meshes (`mesh:<name>`, decision 0180): a body waits until the
//! mesh exists, stands on it, follows it when the mesh is stored again, and a restored
//! snapshot continues exactly.

use marionette_core::Sim;
use marionette_core::mesh::{GameMesh, GameMeshes};
use marionette_core::scene::SceneId;
use marionette_core::spatial::Transform;
use marionette_physics::PhysicsModule;
use serde_json::{Value, json};
use std::sync::Arc;

fn t(p: [f32; 3]) -> Value {
    json!({ "translation": { "x": p[0], "y": p[1], "z": p[2] }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "scale": { "x": 1, "y": 1, "z": 1 } })
}

fn collider(shape: Value) -> Value {
    json!({ "shape": shape, "friction": 0.7, "restitution": 0, "density": 1, "sensor": false })
}

fn scene(ground: &str) -> String {
    json!({
        "format": "marionette.scene", "version": 1,
        "resources": { "PhysicsSettings": { "mode": "Dim3", "gravity": { "x": 0, "y": -9.81, "z": 0 } } },
        "entities": [
            { "id": 1, "name": "ground", "components": {
                "Transform": t([0.0, 0.0, 0.0]), "Collider": collider(json!({ "TriMesh": { "model": ground } })) } },
            { "id": 2, "name": "ball", "components": {
                "Transform": t([0.0, 3.0, 0.0]),
                "RigidBody": { "kind": "Dynamic", "linear_damping": 0.5, "angular_damping": 0.5, "gravity_scale": 1, "lock_rotations": true },
                "Collider": collider(json!({ "Ball": { "radius": 0.5 } })),
                "Velocity": { "x": 0, "y": 0, "z": 0 }, "AngularVelocity": { "x": 0, "y": 0, "z": 0 } } }
        ]
    })
    .to_string()
}

fn sim(text: &str) -> Sim {
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn marionette_core::Module>]);
    Sim::with_modules(text, None, 1, modules, Default::default()).unwrap()
}

fn height(sim: &mut Sim, id: u64) -> f32 {
    let mut q = sim.world.query::<(&SceneId, &Transform)>();
    q.iter(&sim.world)
        .find(|(i, _)| i.0 == id)
        .unwrap()
        .1
        .translation
        .y
}

fn floor(y: f32) -> GameMesh {
    GameMesh::grid(4, 4, 20.0, 20.0, |_, _| y)
}

#[test]
fn a_body_falls_through_until_the_mesh_exists_then_stands_on_it() {
    let mut s = sim(&scene("mesh:floor"));
    // No mesh yet: the ground has no body, so the ball falls past y = 0.
    // Store it before the ball gets there and the ground appears in time.
    for _ in 0..10 {
        s.step();
    }
    assert!(height(&mut s, 2) < 3.0, "the ball falls");
    GameMeshes::set_in(&mut s.world, "floor", floor(0.0)).unwrap();
    for _ in 0..240 {
        s.step();
    }
    let y = height(&mut s, 2);
    assert!((y - 0.5).abs() < 0.05, "rests on the game mesh: {y}");
}

#[test]
fn storing_the_mesh_again_rebuilds_the_collider() {
    let mut s = sim(&scene("mesh:floor"));
    GameMeshes::set_in(&mut s.world, "floor", floor(0.0)).unwrap();
    for _ in 0..240 {
        s.step();
    }
    // Raise the ground under the sleeping ball by less than its radius: the shape follows
    // and pushes the ball up.
    GameMeshes::set_in(&mut s.world, "floor", floor(0.3)).unwrap();
    for _ in 0..240 {
        s.step();
    }
    let y = height(&mut s, 2);
    assert!((y - 0.8).abs() < 0.05, "pushed up by the new shape: {y}");
    // Lower it far below: the ball (woken) falls to it.
    GameMeshes::set_in(&mut s.world, "floor", floor(-2.0)).unwrap();
    for _ in 0..240 {
        s.step();
    }
    let y = height(&mut s, 2);
    assert!((y + 1.5).abs() < 0.05, "fell to the lowered shape: {y}");
}

#[test]
fn a_restored_snapshot_continues_exactly() {
    let mut a = sim(&scene("mesh:floor"));
    GameMeshes::set_in(&mut a.world, "floor", floor(0.0)).unwrap();
    for _ in 0..30 {
        a.step();
    }
    let snap = a.save_state();
    let mut b = sim(&scene("mesh:floor"));
    b.restore(&snap).unwrap();
    // Meshes are render data, rebuilt by the game after a load.
    GameMeshes::set_in(&mut b.world, "floor", floor(0.0)).unwrap();
    for _ in 0..120 {
        a.step();
        b.step();
    }
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn a_missing_model_file_is_still_an_error_and_shapes_name_their_mesh() {
    use marionette_physics::components::ColliderShape;
    let s = ColliderShape::TriMesh {
        model: "mesh:terrain".into(),
    };
    assert_eq!(s.game_mesh(), Some("terrain"));
    let s = ColliderShape::ConvexHull {
        model: "primitive:cube".into(),
    };
    assert_eq!(s.game_mesh(), None);
    let modules = Arc::new(vec![Box::new(PhysicsModule) as Box<dyn marionette_core::Module>]);
    assert!(Sim::with_modules(&scene("missing.glb"), None, 1, modules, Default::default()).is_err());
}
