//! Decision 0272: an entity with `ChildOf` is placed relative to its parent, at load and
//! every tick, parents first; cycles are refused.

use marionette_core::builtin::SceneRef;
use marionette_core::error::codes;
use marionette_core::hierarchy::{ChildOf, compose, local};
use marionette_core::scene::{self, SceneId};
use marionette_core::spatial::{Quat, Transform, Vec3};
use marionette_core::{Sim, default_registry};
use serde_json::{Value, json};

const HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;

fn t(x: f32, y: f32, z: f32) -> Value {
    json!({ "translation": { "x": x, "y": y, "z": z },
            "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 }, "scale": { "x": 1, "y": 1, "z": 1 } })
}

fn child(parent: u64, x: f32, y: f32, z: f32) -> Value {
    let mut v = t(x, y, z);
    v["parent"] = json!(parent);
    v
}

fn text(entities: Value) -> String {
    json!({ "version": 1, "format": "marionette.scene", "resources": {}, "entities": entities }).to_string()
}

fn sim(entities: Value) -> Sim {
    Sim::from_text(&text(entities), None, default_registry(), 1).unwrap()
}

fn at(sim: &mut Sim, id: u64) -> Transform {
    let mut q = sim.world.query::<(&SceneId, &Transform)>();
    *q.iter(&sim.world).find(|(i, _)| i.0 == id).expect("placed").1
}

fn near(a: Vec3, b: [f32; 3]) {
    let d = (a.x - b[0]).abs() + (a.y - b[1]).abs() + (a.z - b[2]).abs();
    assert!(d < 1e-5, "{a:?} is not {b:?}");
}

#[test]
fn children_are_placed_at_load_and_follow_every_tick() {
    // The parent (id 9) moves by its Position; ids are chosen so children sort before parents.
    let mut sim = sim(json!([
        { "id": 9, "components": { "Position": { "x": 1, "y": 0, "z": 0 }, "Velocity": { "x": 60, "y": 0, "z": 0 },
            "Transform": { "translation": { "x": 0, "y": 0, "z": 0 },
                "rotation": { "x": 0, "y": HALF, "z": 0, "w": HALF }, "scale": { "x": 2, "y": 2, "z": 2 } } } },
        { "id": 5, "components": { "ChildOf": child(9, 0.0, 1.0, -1.0) } },
        { "id": 2, "components": { "ChildOf": child(5, 1.0, 0.0, 0.0), "Transform": t(99.0, 99.0, 99.0) } },
    ]));
    // A quarter turn about +Y takes -Z to -X and +X to -Z; the parent's scale is 2.
    near(at(&mut sim, 5).translation, [1.0 - 2.0, 2.0, 0.0]);
    near(at(&mut sim, 2).translation, [-1.0, 2.0, -2.0]);
    assert_eq!(at(&mut sim, 2).scale, Vec3::new(2.0, 2.0, 2.0));
    sim.step();
    near(at(&mut sim, 5).translation, [2.0 - 2.0, 2.0, 0.0]);
    near(at(&mut sim, 2).translation, [0.0, 2.0, -2.0]);
    let r = at(&mut sim, 2).rotation;
    assert!((r.y - HALF).abs() < 1e-6 && (r.w - HALF).abs() < 1e-6);
}

#[test]
fn links_changed_while_running_are_followed() {
    let mut sim = sim(json!([
        { "id": 1, "components": { "Transform": t(10.0, 0.0, 0.0) } },
        { "id": 2, "components": { "Transform": t(0.0, 5.0, 0.0) } },
        { "id": 3, "components": { "ChildOf": child(1, 1.0, 0.0, 0.0) } },
        { "id": 4, "components": { "ChildOf": child(3, 1.0, 0.0, 0.0) } },
    ]));
    near(at(&mut sim, 4).translation, [12.0, 0.0, 0.0]);
    let entity = |sim: &mut Sim, id: u64| {
        let mut q = sim.world.query::<(bevy_ecs::entity::Entity, &SceneId)>();
        q.iter(&sim.world).find(|(_, i)| i.0 == id).unwrap().0
    };
    // Move entity 3 under entity 2: its child follows.
    let e3 = entity(&mut sim, 3);
    sim.world.get_mut::<ChildOf>(e3).unwrap().parent = SceneRef(2);
    sim.step();
    near(at(&mut sim, 3).translation, [1.0, 5.0, 0.0]);
    near(at(&mut sim, 4).translation, [2.0, 5.0, 0.0]);
    // A cycle made while running, and a parent that is gone, leave the entities where they
    // are and do not stop the tick.
    sim.world.entity_mut(e3).get_mut::<ChildOf>().unwrap().parent = SceneRef(4);
    sim.step();
    near(at(&mut sim, 4).translation, [2.0, 5.0, 0.0]);
    sim.world.entity_mut(e3).get_mut::<ChildOf>().unwrap().parent = SceneRef(77);
    sim.step();
    near(at(&mut sim, 3).translation, [1.0, 5.0, 0.0]);
    // Detached: the Transform stays where it was and is the entity's own again.
    sim.world.entity_mut(e3).remove::<ChildOf>();
    sim.world.get_mut::<Transform>(e3).unwrap().translation = Vec3::new(0.0, 0.0, 0.0);
    sim.step();
    near(at(&mut sim, 3).translation, [0.0, 0.0, 0.0]);
    near(at(&mut sim, 4).translation, [1.0, 0.0, 0.0]);
}

#[test]
fn a_cycle_is_refused_with_its_own_code() {
    let registry = default_registry();
    for entities in [
        json!([{ "id": 1, "components": { "ChildOf": child(1, 0.0, 0.0, 0.0) } }]),
        json!([
            { "id": 1, "components": { "ChildOf": child(3, 0.0, 0.0, 0.0) } },
            { "id": 2, "components": { "ChildOf": child(1, 0.0, 0.0, 0.0) } },
            { "id": 3, "components": { "ChildOf": child(2, 0.0, 0.0, 0.0) } },
            { "id": 4, "components": { "ChildOf": child(3, 0.0, 0.0, 0.0) } },
            { "id": 5, "components": {} },
        ]),
    ] {
        let text = text(entities);
        let errs = scene::format_text(&text, &registry).unwrap_err();
        assert!(
            errs.0.iter().all(|d| d.code == codes::SCENE_PARENT_CYCLE),
            "{errs:?}"
        );
        // Only the entities on the cycle are named, each with its line.
        let named: Vec<u64> = errs.0.iter().filter_map(|d| d.entity).collect();
        assert!(named == [1] || named == [1, 2, 3], "{named:?}");
        assert!(errs.0.iter().all(|d| d.pointer.is_some()));
        assert!(Sim::from_text(&text, None, default_registry(), 1).is_err());
    }
}

#[test]
fn local_is_the_inverse_of_compose() {
    let parent = Transform {
        translation: Vec3::new(3.0, -2.0, 0.5),
        rotation: Quat {
            x: 0.0,
            y: HALF,
            z: 0.0,
            w: HALF,
        },
        scale: Vec3::new(2.0, 2.0, 2.0),
    };
    let world = Transform {
        translation: Vec3::new(-4.0, 1.0, 7.0),
        rotation: Quat {
            x: HALF,
            y: 0.0,
            z: 0.0,
            w: HALF,
        },
        scale: Vec3::new(0.5, 3.0, 1.0),
    };
    let c = local(&parent, &world, SceneRef(1)).unwrap();
    let back = compose(&parent, &c);
    near(back.translation, [-4.0, 1.0, 7.0]);
    near(back.scale, [0.5, 3.0, 1.0]);
    assert!((back.rotation.x - HALF).abs() < 1e-6 && (back.rotation.w - HALF).abs() < 1e-6);
    let flat = Transform {
        scale: Vec3::new(1.0, 0.0, 1.0),
        ..parent
    };
    assert!(local(&flat, &world, SceneRef(1)).is_none());
}

#[test]
fn snapshots_and_reruns_give_the_same_hash() {
    let entities = json!([
        { "id": 1, "components": { "Position": { "x": 0, "y": 0, "z": 0 }, "Velocity": { "x": 1, "y": 2, "z": 3 },
            "RandomImpulse": { "strength": 0.5 } } },
        { "id": 2, "components": { "ChildOf": child(1, 0.0, 1.0, 0.0) } },
        { "id": 3, "components": { "ChildOf": child(2, 0.5, 0.0, 0.0) } },
    ]);
    let run = |ticks| {
        let mut s = sim(entities.clone());
        s.run(ticks);
        s
    };
    let mut a = run(200);
    let mut b = run(120);
    let snap = b.save_state();
    b.run(40);
    b.restore(&snap).unwrap();
    b.run(80);
    assert_eq!(a.state_hash(), b.state_hash());
    near(at(&mut a, 3).translation, {
        let p = at(&mut a, 2).translation;
        [p.x + 0.5, p.y, p.z]
    });
}
