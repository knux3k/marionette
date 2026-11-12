//! Factory module: items flow, results never depend on the thread count, snapshots and
//! scene round trips keep the state, and bad authoring data is refused with a code.

use marionette_core::Sim;
use marionette_factory::{FactoryModule, FactoryState};
use marionette_jobs::Jobs;
use serde_json::{Value, json};
use std::sync::Arc;

fn modules() -> marionette_core::Modules {
    Arc::new(vec![Box::new(FactoryModule)])
}

fn load(scene: &Value) -> Sim {
    Sim::with_modules(&scene.to_string(), None, 1, modules(), Default::default()).expect("scene loads")
}

fn recipes() -> Value {
    json!({
        "recipes": [
            { "id": "mine", "inputs": [], "output": { "item": "ore", "count": 1 }, "ticks": 5 },
            { "id": "smelt", "inputs": [{ "item": "ore", "count": 2 }], "output": { "item": "plate", "count": 1 }, "ticks": 10 },
            { "id": "sink", "inputs": [{ "item": "plate", "count": 1 }], "output": { "item": "done", "count": 1 }, "ticks": 3 },
        ],
        "looks": [],
    })
}

fn machine(id: u64, recipe: &str, x: f32) -> Value {
    json!({ "id": id, "components": {
        "Transform": { "translation": { "x": x, "y": 0.0, "z": 0.0 },
                       "rotation": { "x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0 },
                       "scale": { "x": 1.0, "y": 1.0, "z": 1.0 } },
        "FactoryMachine": { "recipe": recipe, "buffer": 4 },
    }})
}

fn belt(id: u64, from: u64, to: u64, slots: u32) -> Value {
    json!({ "id": id, "components": {
        "FactoryBelt": { "from": from, "to": to, "slots": slots, "step_milliticks": 1000 },
    }})
}

/// `lines` copies of miner -> smelter -> sink, with belts of `slots` slots.
fn chain(lines: u64, slots: u32) -> Value {
    let mut entities = Vec::new();
    for l in 0..lines {
        let b = 10 + l * 10;
        entities.push(machine(b, "mine", 0.0));
        entities.push(machine(b + 1, "smelt", 4.0));
        entities.push(machine(b + 2, "sink", 8.0));
        entities.push(belt(b + 3, b, b + 1, slots));
        entities.push(belt(b + 4, b + 1, b + 2, slots));
    }
    json!({ "format": "marionette.scene", "version": 1,
            "resources": { "FactoryRecipes": recipes() }, "entities": entities })
}

fn state(sim: &Sim) -> &FactoryState {
    sim.world.resource::<FactoryState>()
}

#[test]
fn items_flow_from_miner_to_sink_then_back_pressure_stops_the_line() {
    let mut sim = load(&chain(1, 8));
    sim.run(200);
    // In 200 ticks the miner (1 ore / 5 ticks) makes about 40 ore; the smelter turns
    // 2 ore into a plate every 10 ticks; plates reach the sink after two 8-slot belts.
    let s = state(&sim);
    assert!(s.made >= 50, "made only {} items", s.made);
    assert!(
        s.machines[2].output > 0 || s.machines[2].running == 1,
        "nothing reached the sink"
    );

    // The sink's product has nowhere to go: its output fills, then everything upstream
    // backs up and stops, and nothing ever exceeds its buffer.
    sim.run(2000);
    let before = state(&sim).made;
    sim.run(500);
    let s = state(&sim);
    assert_eq!(s.made, before, "a fully backed-up line makes nothing");
    assert_eq!(s.machines[2].output, 4, "the sink's output is full");
    assert!(
        s.slots.iter().filter(|x| x.item >= 0).count() >= 14,
        "belts filled up"
    );
    assert!(
        s.machines
            .iter()
            .all(|m| m.output <= m.buffer && m.have.iter().all(|&h| h <= m.buffer))
    );
}

#[test]
fn results_are_identical_for_any_thread_count() {
    // 300 lines x 2 belts x 128 slots = 76,800 slots: large enough that the pool really
    // runs the belt phase on several threads.
    let scene = chain(300, 128);
    let mut hashes = Vec::new();
    for threads in [1, 2, 4] {
        let mut sim = load(&scene);
        sim.world.insert_resource(Jobs::with_threads(threads));
        sim.run(400);
        hashes.push(sim.state_hash());
    }
    assert_eq!(hashes[0], hashes[1]);
    assert_eq!(hashes[0], hashes[2]);
}

#[test]
fn snapshot_restore_and_scene_round_trip_continue_exactly() {
    let mut a = load(&chain(3, 6));
    a.run(500);
    let full = {
        let mut b = load(&chain(3, 6));
        b.run(250);
        let snap = b.save_state();
        b.run(100); // wander off, then come back
        b.restore(&snap).expect("restore");
        b.run(250);
        b.state_hash()
    };
    assert_eq!(a.state_hash(), full);

    // Saving the scene keeps the factory state; loading it continues the same way.
    let mut c = load(&chain(3, 6));
    c.run(250);
    let text = c.snapshot().to_text();
    let mut d = Sim::with_modules(&text, None, 1, modules(), Default::default()).expect("saved scene loads");
    assert_eq!(state(&d).now, 250, "state came back from the scene");
    c.run(250);
    d.run(250);
    assert_eq!(state(&c), state(&d));
}

#[test]
fn changed_layout_gets_fresh_arrays() {
    let mut sim = load(&chain(1, 8));
    sim.run(100);
    let mut scene: Value = serde_json::from_str(&sim.snapshot().to_text()).unwrap();
    // Lengthen a belt: the saved arrays no longer fit, so the module rebuilds them.
    for e in scene["entities"].as_array_mut().unwrap() {
        if let Some(b) = e["components"].get_mut("FactoryBelt") {
            b["slots"] = json!(12);
        }
    }
    let sim2 = load(&scene);
    assert_eq!(state(&sim2).now, 0);
    assert_eq!(state(&sim2).slots.len(), 24);
}

#[test]
fn bad_authoring_data_is_refused_with_codes() {
    let mut scene = chain(1, 4);
    scene["entities"][1]["components"]["FactoryMachine"]["recipe"] = json!("nope");
    let err = Sim::with_modules(&scene.to_string(), None, 1, modules(), Default::default())
        .err()
        .unwrap();
    assert_eq!(err.0[0].code, "factory.unknown_recipe");

    let mut scene = chain(1, 4);
    scene["entities"][3]["components"]["FactoryBelt"]["to"] = json!(999);
    let err = Sim::with_modules(&scene.to_string(), None, 1, modules(), Default::default())
        .err()
        .unwrap();
    assert!(
        err.0
            .iter()
            .any(|d| d.code == "factory.bad_belt" || d.code.contains("dangling")),
        "{err}"
    );
}
