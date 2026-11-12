//! Plan line 4 proofs through the operation table (the code the CLI and MCP server run):
//! the scripted example is deterministic, a script change shows in a live session in under
//! one second, typings are written, and physics contacts reach script hooks.
#![cfg(feature = "script")]

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// `examples/scripted.scene.json`, 1,000 ticks, seed 1. A change here changes script
/// results: say so in the commit and in docs/ai/PROOFS.md.
const SCRIPTED_HASH: &str = "4450bc3cb97805f59584f11c13b999a13f8ac42ca67752dc7d1cc247575251d3";

/// `examples/spawner.scene.json`, 1,000 ticks, seed 1: a script drops physics crates with a
/// repeating timer; each crate removes itself after three seconds.
const SPAWNER_HASH: &str = "a04911705b0d9f1ce6b8ebc66858c7054c72aa081cff416e227d881650c4aaa8";

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

/// A temporary project holding copies of example files (keeping relative paths).
fn project(tag: &str, files: &[&str]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-script-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for f in files {
        let to = d.join(f);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(examples().join(f), to).unwrap();
    }
    d
}

#[test]
fn scripted_example_gives_the_same_hash_twice_and_matches_the_recorded_hash() {
    let mut a = Agent::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."), false);
    let run = |a: &mut Agent| {
        call(
            a,
            "scene_run",
            json!({ "scene": "examples/scripted.scene.json", "seed": 1, "ticks": 1000 }),
        )["state_hash"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let first = run(&mut a);
    assert_eq!(first, run(&mut a));
    assert_eq!(first, SCRIPTED_HASH);
}

#[test]
fn script_change_shows_in_a_live_session_in_under_a_second() {
    let d = project("reload", &["scripted.scene.json", "scripts/bouncer.ts"]);
    let mut a = Agent::new(&d, false);
    let s = call(
        &mut a,
        "session_start",
        json!({ "scene": "scripted.scene.json", "seed": 1, "ticks": 60 }),
    );
    let id = s["session"].as_str().unwrap().to_string();

    // Same session continued without a change, for comparison.
    let mut b = Agent::new(&d, false);
    let base = call(
        &mut b,
        "session_start",
        json!({ "scene": "scripted.scene.json", "seed": 1, "ticks": 61 }),
    );

    let script = d.join("scripts/bouncer.ts");
    let text = std::fs::read_to_string(&script).unwrap();
    std::fs::write(
        &script,
        text.replace("const GRAVITY = 9.8;", "const GRAVITY = -3;"),
    )
    .unwrap();
    let wall = std::time::Instant::now();
    let r = call(&mut a, "script_reload", json!({ "session": id }));
    let stepped = call(&mut a, "session_step", json!({ "session": id, "ticks": 1 }));
    let elapsed = wall.elapsed().as_secs_f64();
    assert!(elapsed < 1.0, "reload + one tick took {elapsed} s");
    assert!(r["ms"].as_f64().unwrap() < 1000.0);
    assert_eq!(r["scripts"], json!(["scripts/bouncer.ts"]));
    assert_eq!(r["tick"], 60, "reload keeps the tick");
    assert_ne!(
        stepped["state_hash"], base["state_hash"],
        "the change shows on the next tick"
    );
    let ball = call(&mut a, "observe_entity", json!({ "session": id, "id": 12 }));
    assert!(
        ball["components"]["Velocity"]["y"].as_f64().unwrap() > 0.0,
        "{ball}"
    );
}

#[test]
fn script_types_writes_declarations_from_the_schema() {
    let d = project("types", &[]);
    std::fs::create_dir_all(&d).unwrap();
    let mut a = Agent::new(&d, false);
    let r = call(&mut a, "script_types", json!({}));
    assert_eq!(r["path"], "marionette.d.ts");
    let text = std::fs::read_to_string(d.join("marionette.d.ts")).unwrap();
    assert!(text.contains("interface Components"));
    assert!(text.contains("  CharacterController: CharacterController;"));
    assert!(
        Agent::new(&d, true).call("script_types", json!({})).is_err(),
        "read-only refuses"
    );
}

#[test]
fn physics_contacts_reach_collision_hooks_one_tick_later() {
    let d = project(
        "contacts",
        &[
            "playground3d.scene.json",
            "playground3d.actions.json",
            "playground3d.inputs.json",
        ],
    );
    std::fs::write(
        d.join("hits.ts"),
        r#"export function update(self: Entity): void {}
export function onCollisionEnter(self: Entity, other: Entity): void {
  engine.emit("hit", { other: other.id });
}
"#,
    )
    .unwrap();
    // Attach the script to the player (entity 10).
    let path = d.join("playground3d.scene.json");
    let mut scene: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let player = scene["entities"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["id"] == 10)
        .unwrap();
    player["components"]["Script"] = json!({ "path": "hits.ts" });
    std::fs::write(&path, serde_json::to_string_pretty(&scene).unwrap()).unwrap();

    let mut a = Agent::new(&d, false);
    let s = call(
        &mut a,
        "session_start",
        json!({ "scene": "playground3d.scene.json", "seed": 1, "ticks": 1000, "input": "playground3d.inputs.json" }),
    );
    let id = s["session"].as_str().unwrap().to_string();
    let list = |a: &mut Agent, kind: &str| -> Vec<Value> {
        call(
            a,
            "observe_events",
            json!({ "session": id, "kind": kind, "limit": 100000 }),
        )["events"]
            .as_array()
            .unwrap()
            .clone()
    };
    let started: Vec<(u64, u64)> = list(&mut a, "collision_started")
        .iter()
        .filter_map(|e| {
            let (x, y) = (e["entity"].as_u64()?, e["other"].as_u64()?);
            let other = if x == 10 {
                y
            } else if y == 10 {
                x
            } else {
                return None;
            };
            Some((e["tick"].as_u64()? + 1, other))
        })
        .filter(|(t, _)| *t < 1000)
        .collect();
    let hits: Vec<(u64, u64)> = list(&mut a, "hit")
        .iter()
        .map(|e| {
            (
                e["tick"].as_u64().unwrap(),
                e["detail"]["other"].as_u64().unwrap(),
            )
        })
        .collect();
    assert!(!started.is_empty(), "the player touches something");
    assert_eq!(hits, started);
}

#[test]
fn spawned_crates_get_bodies_land_and_leave_physics_when_destroyed() {
    let mut a = Agent::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."), false);
    let run = |a: &mut Agent| {
        call(
            a,
            "scene_run",
            json!({ "scene": "examples/spawner.scene.json", "seed": 1, "ticks": 1000 }),
        )["state_hash"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let first = run(&mut a);
    assert_eq!(first, run(&mut a));
    assert_eq!(first, SPAWNER_HASH);

    let s = call(
        &mut a,
        "session_start",
        json!({ "scene": "examples/spawner.scene.json", "seed": 1, "ticks": 300 }),
    );
    let id = s["session"].as_str().unwrap().to_string();
    let count = |a: &mut Agent, kind: &str| -> usize {
        call(
            a,
            "observe_events",
            json!({ "session": id, "kind": kind, "limit": 100000 }),
        )["events"]
            .as_array()
            .unwrap()
            .len()
    };
    let tree = |a: &mut Agent| -> usize {
        call(a, "observe_tree", json!({ "session": id, "limit": 500 }))["total"]
            .as_u64()
            .unwrap() as usize
    };
    // Mid-run: crates are falling and landing (they got physics bodies when spawned).
    let alive = tree(&mut a) - 4;
    assert!(alive > 5, "{alive} crates alive at tick 300");
    assert!(count(&mut a, "landed") > 5);
    assert_eq!(count(&mut a, "physics_error"), 0);
    assert_eq!(count(&mut a, "script_error"), 0);

    call(&mut a, "session_step", json!({ "session": id, "ticks": 700 }));
    // 40 drops plus 4 copies, each removed after three seconds; nothing is left but the
    // camera, sun, ground and spawner, and rays hit the ground, not a removed crate's body.
    assert_eq!(count(&mut a, "crate_gone"), 44);
    assert_eq!(count(&mut a, "destroyed"), 44);
    assert_eq!(tree(&mut a), 4);
    for x in [-2.0, 0.0, 2.0] {
        let hit = call(
            &mut a,
            "observe_raycast",
            json!({ "session": id, "origin": [x, 5.0, -3.0], "direction": [0.0, -1.0, 0.0] }),
        );
        assert_eq!(hit["hit"]["entity"], 3, "{hit}");
    }
    assert_eq!(count(&mut a, "physics_error"), 0);
    assert_eq!(count(&mut a, "script_error"), 0);

    // Snapshot with crates in flight, bodies spawned by scripts and pending timers; the
    // restored session continues exactly like the uninterrupted one. (The event log is
    // not rewound by a restore, so this uses its own session.)
    let s = call(
        &mut a,
        "session_start",
        json!({ "scene": "examples/spawner.scene.json", "seed": 1, "ticks": 300 }),
    );
    let id = s["session"].as_str().unwrap().to_string();
    call(
        &mut a,
        "session_snapshot",
        json!({ "session": id, "name": "mid" }),
    );
    let straight = call(&mut a, "session_step", json!({ "session": id, "ticks": 200 }))["state_hash"].clone();
    call(
        &mut a,
        "session_restore",
        json!({ "session": id, "snapshot": "mid" }),
    );
    let again = call(&mut a, "session_step", json!({ "session": id, "ticks": 200 }))["state_hash"].clone();
    assert!(straight.is_string());
    assert_eq!(straight, again);
}
