//! Save operations on live sessions with physics, input and scripts (decision 0076).

#![cfg(feature = "save")]

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input}: {}", serde_json::to_string(&e).unwrap()))
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn saves_dir(tag: &str) -> String {
    let d = std::env::temp_dir().join(format!("marionette-saveops-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d.to_string_lossy().into_owned()
}

#[test]
fn saved_sessions_with_physics_input_and_scripts_continue_exactly_after_loading() {
    let dir = saves_dir("exact");
    let mut agent = Agent::new(examples(), false);
    for (scene, input) in [
        ("playground3d.scene.json", Some("playground3d.inputs.json")),
        ("playground2d.scene.json", Some("playground2d.inputs.json")),
        ("scripted.scene.json", None),
    ] {
        let start = json!({ "scene": scene, "seed": 3, "input": input, "ticks": 60 });
        let a = call(&mut agent, "session_start", start.clone())["session"]
            .as_str()
            .unwrap()
            .to_string();
        let w = call(
            &mut agent,
            "save_write",
            json!({ "session": a, "slot": "slot-1", "dir": dir, "label": scene }),
        );
        assert_eq!(w["tick"], 60);
        let a_hash =
            call(&mut agent, "session_step", json!({ "session": a, "ticks": 90 }))["state_hash"].clone();

        // A fresh session of the same scene (same input file) loads the slot.
        let fresh = json!({ "scene": scene, "seed": 3, "input": input });
        let b = call(&mut agent, "session_start", fresh)["session"]
            .as_str()
            .unwrap()
            .to_string();
        let l = call(
            &mut agent,
            "save_load",
            json!({ "session": b, "slot": "slot-1", "dir": dir }),
        );
        assert_eq!(
            (l["tick"].as_u64(), l["exact"].as_bool()),
            (Some(60), Some(true)),
            "{scene}"
        );
        let b_hash =
            call(&mut agent, "session_step", json!({ "session": b, "ticks": 90 }))["state_hash"].clone();
        assert_eq!(a_hash, b_hash, "{scene}");

        let c = call(
            &mut agent,
            "save_check",
            json!({ "session": b, "slot": "slot-1", "dir": dir }),
        );
        assert_eq!(c["ok"], true, "{c}");
        let list = call(&mut agent, "save_list", json!({ "dir": dir }));
        assert_eq!(list["slots"][0]["label"], scene);
        assert_eq!(list["slots"][0]["backup"], scene != "playground3d.scene.json");
    }
    let d = call(&mut agent, "save_delete", json!({ "slot": "slot-1", "dir": dir }));
    assert_eq!(d["deleted"], true);
    assert_eq!(
        call(&mut agent, "save_list", json!({ "dir": dir }))["slots"],
        json!([])
    );
    let e = agent
        .call(
            "save_load",
            json!({ "session": "s1", "slot": "slot-1", "dir": dir }),
        )
        .unwrap_err();
    assert_eq!(e.0[0].code, "save.not_found");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_operations_refuse_bad_slots_and_read_only_mode_refuses_writes() {
    let dir = saves_dir("refuse");
    let mut agent = Agent::new(examples(), false);
    let s = call(
        &mut agent,
        "session_start",
        json!({ "scene": "drift.scene.json" }),
    )["session"]
        .as_str()
        .unwrap()
        .to_string();
    let e = agent
        .call("save_write", json!({ "session": s, "slot": "../x", "dir": dir }))
        .unwrap_err();
    assert_eq!(e.0[0].code, "save.bad_slot");
    let mut ro = Agent::new(examples(), true);
    let e = ro
        .call("save_delete", json!({ "slot": "x", "dir": dir }))
        .unwrap_err();
    assert_eq!(e.0[0].code, "agent.read_only");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_script_asking_for_a_save_gets_its_own_slot() {
    let root = std::env::temp_dir().join(format!("marionette-saveops-script-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("gate.ts"),
        "export function update(self: Entity): void {\n  if (engine.tick === 10) engine.emit(\"save_request\", { slot: \"checkpoint\", label: \"Gate\" });\n}\n",
    )
    .unwrap();
    let scene = root.join("gate.scene.json");
    std::fs::write(
        &scene,
        r#"{"format":"marionette.scene","version":1,"resources":{"SaveFormat":{"game":"g","version":1,"autosave_every":0,"autosave_slots":2,"migrations":""}},"entities":[
{"id":1,"name":"gate","components":{"Position":{"x":0,"y":0,"z":0},"Script":{"path":"gate.ts"}}}]}"#,
    )
    .unwrap();
    let engine = marionette_agent::Engine::standard();
    let mut sim = engine.load_file(&scene, "gate", 0).unwrap();
    let store = marionette_save::SaveStore::open(root.join("saves")).unwrap();
    let registry = std::sync::Arc::new(marionette_core::module::registry_with(&engine.modules()));
    let writer = marionette_save::SaveWriter::new(store.clone(), registry).unwrap();
    let mut auto = marionette_save::Autosaver::new(&sim, &store);
    let mut queued = Vec::new();
    for _ in 0..20 {
        sim.step();
        queued.extend(auto.after_step(&mut sim, &writer).unwrap());
    }
    drop(writer);
    assert_eq!(queued, ["checkpoint"]);
    let h = store.read("checkpoint").unwrap().0.header;
    assert_eq!((h.tick, h.label.as_str()), (11, "Gate"));
    let _ = std::fs::remove_dir_all(&root);
}
