//! Leak checks (decision 0032): repeated work must give its memory back. Every scenario runs
//! a warm-up (caches, lazily built tables), takes a baseline of live heap bytes from the
//! counting allocator, repeats the work many more times and requires the growth to stay
//! under a small fixed allowance. A leak of even a few bytes per round fails, because the
//! measured rounds outnumber the allowance.
//!
//! This binary has exactly one test so no other test allocates while it measures.

use marionette_agent::Agent;
use marionette_host::alloc::{Counting, stats};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[global_allocator]
static ALLOC: Counting = Counting;

/// Heap growth tolerated across the measured rounds of one scenario.
const ALLOWANCE: u64 = 64 << 10;

fn live() -> u64 {
    stats().live_bytes
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input}: {}", serde_json::to_string(&e).unwrap()))
}

/// Runs `round` `warm` times, then `rounds` times, and checks the live heap did not grow.
fn steady(name: &str, warm: usize, rounds: usize, mut round: impl FnMut(usize)) {
    for i in 0..warm {
        round(i);
    }
    let before = live();
    for i in 0..rounds {
        round(warm + i);
    }
    let after = live();
    let grew = after.saturating_sub(before);
    eprintln!("{name}: {rounds} rounds, heap {before} -> {after} bytes");
    assert!(
        grew <= ALLOWANCE,
        "{name}: heap grew by {grew} bytes over {rounds} rounds ({} bytes per round)",
        grew / rounds as u64
    );
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn scratch() -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-leaks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("scripts")).unwrap();
    d
}

/// A scene whose script emits ten events per tick, so the event log must trim to stay flat.
fn chatty_project(dir: &Path) {
    std::fs::write(
        dir.join("scripts/chatty.ts"),
        "export function update(self: Entity): void {\n  for (let i = 0; i < 10; i++) engine.emit(\"chat\", { i, t: engine.tick });\n  self.state.n = (self.state.n ?? 0) + 1;\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("chatty.scene.json"),
        r#"{"format":"marionette.scene","version":1,"resources":{},"entities":[
{"id":1,"name":"talker","components":{"Position":{"x":0,"y":0,"z":0},"Script":{"path":"scripts/chatty.ts"}}}]}"#,
    )
    .unwrap();
}

#[test]
fn memory_stays_flat_under_repeated_work() {
    assert!(live() > 0, "the counting allocator is installed");

    // 1. Scene load and unload: sessions of every example kind, stepped, snapshotted,
    //    restored and stopped (core, physics, input, audio, scripts).
    let mut agent = Agent::new(examples(), false);
    let scenes = [
        ("drift.scene.json", None),
        ("scripted.scene.json", None),
        ("playground3d.scene.json", Some("playground3d.inputs.json")),
        ("playground2d.scene.json", Some("playground2d.inputs.json")),
    ];
    steady("session load/unload", 3, 40, |i| {
        let (scene, input) = scenes[i % scenes.len()];
        let s = call(
            &mut agent,
            "session_start",
            json!({ "scene": scene, "seed": i, "input": input, "ticks": 30 }),
        );
        let id = s["session"].as_str().unwrap().to_string();
        call(
            &mut agent,
            "session_snapshot",
            json!({ "session": id, "name": "a" }),
        );
        call(&mut agent, "session_step", json!({ "session": id, "ticks": 30 }));
        call(
            &mut agent,
            "session_restore",
            json!({ "session": id, "snapshot": "a" }),
        );
        call(&mut agent, "session_stop", json!({ "session": id }));
    });

    // 2. A long run that fills the event log past its bound (10 events per tick): the Rust
    //    heap and the script heap must level off.
    let dir = scratch();
    chatty_project(&dir);
    let mut agent = Agent::new(&dir, false);
    let s = call(
        &mut agent,
        "session_start",
        json!({ "scene": "chatty.scene.json", "seed": 1 }),
    );
    let id = s["session"].as_str().unwrap().to_string();
    let heap = |agent: &mut Agent| {
        let v = call(agent, "observe_stats", json!({ "session": id }));
        v["script_heap_bytes"].as_u64().unwrap()
    };
    // Warm-up: 20,000 events, more than the event log keeps, and more session log entries
    // than the session log keeps.
    call(
        &mut agent,
        "session_step",
        json!({ "session": id, "ticks": 2_000 }),
    );
    for _ in 0..1_100 {
        call(&mut agent, "session_step", json!({ "session": id, "ticks": 0 }));
    }
    let js_before = heap(&mut agent);
    steady("long run with a chatty script", 1, 20, |_| {
        call(&mut agent, "session_step", json!({ "session": id, "ticks": 500 }));
    });
    let js_after = heap(&mut agent);
    eprintln!("script heap: {js_before} -> {js_after} bytes");
    assert!(
        js_after <= js_before + (256 << 10),
        "script heap grew: {js_before} -> {js_after}"
    );
    let ev = call(&mut agent, "observe_events", json!({ "session": id, "limit": 1 }));
    assert!(ev["dropped"].as_u64().unwrap() > 0, "the event log trimmed");

    // 3. Script hot reload, with the file changing between reloads.
    let script = dir.join("scripts/chatty.ts");
    let original = std::fs::read_to_string(&script).unwrap();
    steady("script hot reload", 5, 60, |i| {
        let changed = original.replace("i < 10", if i % 2 == 0 { "i < 10 && i >= 0" } else { "i < 10" });
        std::fs::write(&script, changed).unwrap();
        call(&mut agent, "script_reload", json!({ "session": id }));
        call(&mut agent, "session_step", json!({ "session": id, "ticks": 2 }));
    });

    // 4. Entity churn through live edits (spawn and destroy), and the session log.
    steady("entity create/delete", 20, 400, |i| {
        let id_n = 1_000 + (i % 7) as u64;
        call(
            &mut agent,
            "entity_create",
            json!({ "session": id, "id": id_n, "name": "temp",
                    "components": { "Position": { "x": i as f64, "y": 0.0, "z": 0.0 } } }),
        );
        call(&mut agent, "session_step", json!({ "session": id, "ticks": 1 }));
        call(&mut agent, "entity_delete", json!({ "session": id, "id": id_n }));
    });
    call(&mut agent, "session_stop", json!({ "session": id }));

    // 5. Screenshots: GPU setup and teardown per call, PNG encoding (skipped without a GPU).
    let mut agent = Agent::new(examples(), false);
    let shot = dir.join("shot.png");
    let probe = agent.call(
        "observe_screenshot",
        json!({ "scene": "showcase.scene.json", "size": "64x36", "out": shot.to_string_lossy() }),
    );
    if probe.is_ok() {
        steady("screenshots", 3, 15, |i| {
            let scene = ["showcase.scene.json", "sprites.scene.json", "ui.scene.json"][i % 3];
            call(
                &mut agent,
                "observe_screenshot",
                json!({ "scene": scene, "size": "64x36", "out": shot.to_string_lossy() }),
            );
        });
    } else if std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() == Ok("1") {
        panic!("GPU required but screenshots fail: {probe:?}");
    } else {
        eprintln!("screenshots skipped: no GPU");
    }
    drop(agent);
    let _ = std::fs::remove_dir_all(&dir);
}
