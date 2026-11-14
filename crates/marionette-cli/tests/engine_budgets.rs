//! Timing budgets of the engine itself (decision 0023): scene edits (decision 0270), scripts
//! (decision 0271) and children (decision 0272). Wall-clock numbers depend on the machine, so these run only when
//! asked, in a release build, one test at a time:
//!
//! `cargo test --release -p marionette-cli --test engine_budgets -- --ignored --test-threads=1`
//!
//! CI runs them on every push (job `perf`). The measured value is printed so a regression
//! shows long before it fails. A game's own budgets live with the game (the factory example:
//! `examples/factory/tests/perf_budgets.rs`).

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Instant;

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-engine-budget-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} failed: {e}"))
}

/// Shared CI runners are slower and noisier than the reference desktop, so CI sets
/// `MARIONETTE_BUDGET_SCALE=2` (decision 0023); on the reference desktop it is 1.
fn scale() -> f64 {
    std::env::var("MARIONETTE_BUDGET_SCALE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|s| *s >= 1.0)
        .unwrap_or(1.0)
}

fn check(name: &str, measured: f64, budget: f64) {
    let budget = budget * scale();
    println!("budget {name}: {measured:.3} ms (budget {budget} ms)");
    assert!(
        measured <= budget,
        "{name}: {measured:.3} ms is over its {budget} ms budget"
    );
}

/// Building a 10,000 entity scene file in one batch (decision 0270).
const EDIT_BUILD_10K_MS: f64 = 2000.0;
/// One edit of that file outside a batch (the file's text is written again each time), median.
const EDIT_FILE_10K_MS: f64 = 20.0;
/// Adding 10,000 entities to a live session in one batch.
const EDIT_SESSION_BATCH_10K_MS: f64 = 2500.0;
/// One edit of that session outside a batch (the world is built again each time).
const EDIT_SESSION_10K_MS: f64 = 500.0;

#[test]
#[ignore = "timing: release builds only, see the file header"]
fn scene_edit_budgets() {
    let d = dir().join("edits");
    std::fs::create_dir_all(&d).unwrap();
    let mut agent = Agent::new(&d, false);
    let scene = "scenes/big.scene.json";
    let cube = |i: u32, target: (&str, &str)| {
        json!({ "op": "entity_create", "input": { target.0: target.1, "name": format!("cube{i}"),
            "components": {
                "Transform": { "translation": { "x": i % 100, "y": 0.5, "z": i / 100 },
                    "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 }, "scale": { "x": 1, "y": 1, "z": 1 } },
                "Model": { "asset": "primitive:cube" } } } })
    };
    let mut ops = vec![json!({ "op": "scene_create", "input": { "scene": scene, "template": "basic" } })];
    ops.extend((0..10_000).map(|i| cube(i, ("scene", scene))));
    let t = Instant::now();
    call(&mut agent, "batch_run", json!({ "ops": ops }));
    check(
        "build a 10,000 entity scene in one batch",
        t.elapsed().as_secs_f64() * 1000.0,
        EDIT_BUILD_10K_MS,
    );

    let mut times: Vec<f64> = (0..41)
        .map(|i| {
            let t = Instant::now();
            call(
                &mut agent,
                "component_set",
                json!({ "scene": scene, "id": 100 + i, "component": "Velocity", "value": { "x": 1, "y": 0, "z": i } }),
            );
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    check(
        "one edit of a 10,000 entity scene file (median)",
        times[20],
        EDIT_FILE_10K_MS,
    );

    call(
        &mut agent,
        "scene_create",
        json!({ "scene": "scenes/live.scene.json", "template": "basic" }),
    );
    let s = call(
        &mut agent,
        "session_start",
        json!({ "scene": "scenes/live.scene.json" }),
    );
    let id = s["session"].as_str().unwrap();
    let ops: Vec<Value> = (0..10_000).map(|i| cube(i, ("session", id))).collect();
    let t = Instant::now();
    call(&mut agent, "batch_run", json!({ "ops": ops }));
    check(
        "add 10,000 entities to a live session in one batch",
        t.elapsed().as_secs_f64() * 1000.0,
        EDIT_SESSION_BATCH_10K_MS,
    );
    let t = Instant::now();
    call(
        &mut agent,
        "entity_create",
        cube(1, ("session", id))["input"].clone(),
    );
    check(
        "one edit of a 10,000 entity live session",
        t.elapsed().as_secs_f64() * 1000.0,
        EDIT_SESSION_10K_MS,
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Decision 0271: a tick of 1,000 scripts whose `update` does nothing (measured 2.6 ms; 6.7 ms
/// before).
#[cfg(feature = "script")]
const SCRIPT_EMPTY_1K_MS: f64 = 5.0;
/// Decision 0271: a tick of 1,000 scripts that read, change and write their `Transform`
/// (measured 10.8 ms; 34.6 ms before).
#[cfg(feature = "script")]
const SCRIPT_MOVER_1K_MS: f64 = 18.0;

/// A scene of `n` cubes that each run `script` (written beside the scene), and its tick cost.
#[cfg(feature = "script")]
fn scripted(agent: &mut Agent, d: &std::path::Path, tag: &str, n: u32, script: &str) -> f64 {
    let scene = format!("scenes/{tag}.scene.json");
    std::fs::create_dir_all(d.join("scenes")).unwrap();
    std::fs::write(d.join(format!("scenes/{tag}.ts")), script).unwrap();
    let mut ops = vec![json!({ "op": "scene_create", "input": { "scene": scene, "template": "basic" } })];
    ops.extend((0..n).map(|i| {
        json!({ "op": "entity_create", "input": { "scene": scene, "components": {
            "Transform": { "translation": { "x": i % 100, "y": 0.5, "z": i / 100 },
                "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 }, "scale": { "x": 1, "y": 1, "z": 1 } },
            "Script": { "path": format!("{tag}.ts") } } } })
    }));
    call(agent, "batch_run", json!({ "ops": ops }));
    let r = call(
        agent,
        "perf_sim",
        json!({ "scene": scene, "ticks": 300, "warmup": 60 }),
    );
    r["avg_ms"].as_f64().unwrap()
}

/// The script skill's own example: read a component, change it, write it back.
#[cfg(feature = "script")]
const MOVER: &str = r#"export function start(self: Entity): void {
  self.state.speed = 2;
}
export function update(self: Entity): void {
  const t = self.get("Transform");
  if (!t) return;
  t.translation.x += engine.action("move_x") * self.state.speed * engine.dt;
  self.set("Transform", t);
}
"#;

#[test]
#[cfg(feature = "script")]
#[ignore = "timing: release builds only, see the file header"]
fn script_budgets() {
    let d = dir().join("scripts");
    std::fs::create_dir_all(&d).unwrap();
    let mut agent = Agent::new(&d, false);
    let empty = scripted(
        &mut agent,
        &d,
        "empty",
        1000,
        "export function update(self: Entity): void {}\n",
    );
    check("1,000 scripts with an empty update", empty, SCRIPT_EMPTY_1K_MS);
    let mover = scripted(&mut agent, &d, "mover", 1000, MOVER);
    check("1,000 scripts that move their entity", mover, SCRIPT_MOVER_1K_MS);
    let _ = std::fs::remove_dir_all(&d);
}

/// Decision 0272: a tick of 10,000 children in chains of four under moving parents (measured
/// 1.3 ms).
const CHILDREN_10K_MS: f64 = 3.0;

#[test]
#[ignore = "timing: release builds only, see the file header"]
fn hierarchy_budgets() {
    let d = dir().join("children");
    std::fs::create_dir_all(&d).unwrap();
    let mut agent = Agent::new(&d, false);
    let scene = "scenes/children.scene.json";
    let place = |parent: Option<u32>| {
        let mut v = json!({ "translation": { "x": 1, "y": 0, "z": 0 },
            "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 }, "scale": { "x": 1, "y": 1, "z": 1 } });
        if let Some(p) = parent {
            v["parent"] = json!(p);
        }
        v
    };
    let mut ops = vec![json!({ "op": "scene_create", "input": { "scene": scene, "template": "basic" } })];
    for i in 0..2500u32 {
        let root = 100 + i * 5;
        ops.push(json!({ "op": "entity_create", "input": { "scene": scene, "id": root, "components": {
            "Transform": place(None), "Position": { "x": i, "y": 0, "z": 0 }, "Velocity": { "x": 0, "y": 1, "z": 0 } } } }));
        for k in 1..5 {
            ops.push(
                json!({ "op": "entity_create", "input": { "scene": scene, "id": root + k, "components": {
                "ChildOf": place(Some(root + k - 1)) } } }),
            );
        }
    }
    call(&mut agent, "batch_run", json!({ "ops": ops }));
    let r = call(
        &mut agent,
        "perf_sim",
        json!({ "scene": scene, "ticks": 300, "warmup": 60 }),
    );
    check(
        "10,000 children following 2,500 moving parents",
        r["avg_ms"].as_f64().unwrap(),
        CHILDREN_10K_MS,
    );
    let _ = std::fs::remove_dir_all(&d);
}
