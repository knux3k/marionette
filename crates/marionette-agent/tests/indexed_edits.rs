//! Decision 0270: scene edits through the indexed path (one long-lived agent, text kept
//! indexed between edits) give exactly what the whole-scene path gives (a fresh agent per
//! call, as the CLI runs): the same answers, the same refusals, the same bytes on disk.

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::PathBuf;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-indexed-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const SCENE: &str = "scenes/a.scene.json";

fn t(x: f64) -> Value {
    // Whole numbers on purpose: the canonical form writes them as floats.
    json!({ "translation": { "x": x, "y": 1, "z": 0 }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "scale": { "x": 1, "y": 1, "z": 1 } })
}

/// Edits that succeed and edits that must be refused, in an order where each depends on
/// the ones before.
fn steps() -> Vec<(&'static str, Value)> {
    let s = |mut v: Value| {
        v["scene"] = json!(SCENE);
        v
    };
    vec![
        ("scene_create", s(json!({ "template": "basic" }))),
        (
            "entity_create",
            s(json!({ "name": "a", "components": { "Transform": t(1.0) } })),
        ),
        (
            "entity_create",
            s(json!({ "components": { "Transform": t(2.5), "Model": { "asset": "primitive:cube" } } })),
        ),
        (
            "entity_create",
            s(json!({ "id": 40, "name": "far", "components": {} })),
        ),
        ("entity_create", s(json!({ "id": 40, "components": {} }))), // id in use
        ("entity_create", s(json!({ "id": 0, "components": {} }))),  // reserved id
        ("entity_create", s(json!({ "components": { "Nope": {} } }))), // unknown type
        (
            "entity_create",
            s(json!({ "components": { "Transform": { "translation": 3 } } })),
        ), // wrong value
        (
            "entity_create",
            s(
                json!({ "name": "tail", "components": { "Follow": { "target": 40, "offset": { "x": 0, "y": 1, "z": 2 } } } }),
            ),
        ),
        (
            "entity_create",
            s(json!({ "components": { "Follow": { "target": 999, "offset": { "x": 0, "y": 0, "z": 0 } } } })),
        ), // dangling
        ("entity_delete", s(json!({ "id": 40 }))), // still referred to
        (
            "component_set",
            s(json!({ "id": 40, "component": "Velocity", "value": { "x": 1, "y": 0, "z": 0 } })),
        ),
        (
            "component_set",
            s(json!({ "id": 40, "component": "Velocity", "value": { "x": 1, "y": 0, "z": 0 } })),
        ), // no change
        (
            "component_set",
            s(json!({ "id": 40, "component": "Velocity", "value": { "x": "fast" } })),
        ),
        (
            "component_set",
            s(json!({ "id": 77, "component": "Velocity", "value": { "x": 1, "y": 0, "z": 0 } })),
        ), // no entity
        (
            "entity_update",
            s(json!({ "id": 40, "name": "", "set": { "Transform": t(-4.0) }, "remove": ["Velocity"] })),
        ),
        (
            "entity_update",
            s(json!({ "id": 40, "name": "again", "remove": ["Velocity"] })),
        ), // not there
        (
            "component_remove",
            s(json!({ "id": 40, "component": "Transform" })),
        ),
        (
            "component_remove",
            s(json!({ "id": 40, "component": "Transform" })),
        ), // not there
        (
            "resource_set",
            s(json!({ "resource": "FixedStep", "value": { "dt": 0.02 } })),
        ),
        (
            "resource_set",
            s(json!({ "resource": "FixedStep", "value": { "dt": 1 } })),
        ),
        ("resource_set", s(json!({ "resource": "Nope", "value": {} }))),
        ("resource_remove", s(json!({ "resource": "FixedStep" }))),
        ("resource_remove", s(json!({ "resource": "FixedStep" }))), // not there
        ("scene_format", s(json!({}))),                             // not an edit: must see everything so far
        ("entity_delete", s(json!({ "id": 5 }))),
        ("entity_delete", s(json!({ "id": 40 }))),
        ("entity_delete", s(json!({ "id": 40 }))), // gone
        (
            "entity_create",
            s(json!({ "components": { "Transform": t(9.0) } })),
        ),
    ]
}

fn answer(r: Result<Value, marionette_core::Errors>) -> Value {
    match r {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "err": serde_json::to_value(&e.0).unwrap() }),
    }
}

#[test]
fn indexed_edits_match_whole_scene_edits_step_by_step() {
    let (fast, slow) = (dir("fast"), dir("slow"));
    let mut kept = Agent::new(&fast, false);
    let mut refused = 0;
    for (i, (op, input)) in steps().into_iter().enumerate() {
        let a = answer(kept.call(op, input.clone()));
        let b = answer(Agent::new(&slow, false).call(op, input));
        assert_eq!(a, b, "step {i} (`{op}`) answered differently");
        refused += usize::from(a.get("err").is_some());
        assert_eq!(
            std::fs::read_to_string(fast.join(SCENE)).unwrap(),
            std::fs::read_to_string(slow.join(SCENE)).unwrap(),
            "step {i} (`{op}`) left different files"
        );
    }
    assert!(
        refused >= 10,
        "the refusals were meant to be exercised ({refused})"
    );
    for d in [fast, slow] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_batch_gives_the_same_file_and_refuses_the_same_way() {
    let (fast, slow) = (dir("batch-fast"), dir("batch-slow"));
    let ops: Vec<Value> = steps()
        .into_iter()
        .map(|(op, input)| json!({ "op": op, "input": input }))
        .collect();
    // The whole list stops at its first refusal; cut it into runs that each end at one.
    let mut kept = Agent::new(&fast, false);
    let mut start = 0;
    while start < ops.len() {
        let mut end = start;
        // Find how far a fresh agent gets, one call at a time.
        while end < ops.len() {
            let r =
                Agent::new(&slow, false).call(ops[end]["op"].as_str().unwrap(), ops[end]["input"].clone());
            end += 1;
            if r.is_err() {
                break;
            }
        }
        let r = kept.call("batch_run", json!({ "ops": ops[start..end] }));
        let _ = r; // refusals are compared step by step in the other test
        assert_eq!(
            std::fs::read_to_string(fast.join(SCENE)).unwrap(),
            std::fs::read_to_string(slow.join(SCENE)).unwrap(),
            "after steps {start}..{end}"
        );
        start = end;
    }
    for d in [fast, slow] {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_file_changed_by_someone_else_is_read_again() {
    let d = dir("outside");
    let mut agent = Agent::new(&d, false);
    let s = |mut v: Value| {
        v["scene"] = json!(SCENE);
        v
    };
    agent
        .call("scene_create", s(json!({ "template": "empty" })))
        .unwrap();
    agent
        .call("entity_create", s(json!({ "components": {} })))
        .unwrap();
    agent
        .call("entity_create", s(json!({ "components": {} })))
        .unwrap();
    // Another process (or a person) adds entity 9 behind the agent's back.
    Agent::new(&d, false)
        .call("entity_create", s(json!({ "id": 9, "components": {} })))
        .unwrap();
    let r = agent
        .call("entity_create", s(json!({ "components": {} })))
        .unwrap();
    assert_eq!(r["id"], 10, "the next id comes from the file as it is now");
    let text = std::fs::read_to_string(d.join(SCENE)).unwrap();
    assert!(text.contains("\"id\": 9") && text.contains("\"id\": 10"));
    let _ = std::fs::remove_dir_all(d);
}

/// Not a timing test (those are in `perf_budgets`): it only has to finish. Before decision
/// 0270 two thousand edits of one file took minutes in a debug build.
#[test]
fn two_thousand_edits_of_one_file() {
    let d = dir("many");
    let mut agent = Agent::new(&d, false);
    let mut ops = vec![json!({ "op": "scene_create", "input": { "scene": SCENE, "template": "basic" } })];
    for i in 0..2000 {
        ops.push(json!({ "op": "entity_create", "input": { "scene": SCENE,
            "components": { "Transform": t(i as f64), "Model": { "asset": "primitive:cube" } } } }));
    }
    agent.call("batch_run", json!({ "ops": ops })).unwrap();
    for i in 0..200 {
        agent
            .call(
                "component_set",
                json!({ "scene": SCENE, "id": 10 + i, "component": "Velocity",
                "value": { "x": 1, "y": 0, "z": 0 } }),
            )
            .unwrap();
    }
    let text = std::fs::read_to_string(d.join(SCENE)).unwrap();
    let again = Agent::new(&d, false)
        .call("scene_format", json!({ "scene": SCENE }))
        .unwrap();
    assert_eq!(again["changed"], false, "the file is canonical");
    assert_eq!(
        text.matches("\"Model\"").count(),
        2000 + text.matches("primitive:plane").count()
    );
    let _ = std::fs::remove_dir_all(d);
}

/// The same edits aimed at a live session, with ticks in between and one edit only the
/// script module can refuse (its file does not exist).
fn session_steps(id: &str) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    for (i, (op, mut input)) in steps().into_iter().enumerate() {
        if op.starts_with("scene_") {
            continue;
        }
        input.as_object_mut().unwrap().remove("scene");
        input["session"] = json!(id);
        out.push((op, input));
        if i == 12 {
            out.push((
                "entity_create",
                json!({ "session": id, "components": { "Script": { "path": "missing.ts" } } }),
            ));
            out.push((
                "entity_create",
                json!({ "session": id, "name": "after", "components": {} }),
            ));
        }
        if i % 7 == 3 {
            out.push(("session_step", json!({ "session": id, "ticks": 3 })));
        }
    }
    out
}

fn session_pair(tag: &str) -> (Agent, Agent, String, [PathBuf; 2]) {
    let (fast, slow) = (dir(&format!("{tag}-fast")), dir(&format!("{tag}-slow")));
    let mut a = Agent::new(&fast, false);
    let mut b = Agent::new(&slow, false);
    b.whole_scene_edits();
    let mut id = String::new();
    for agent in [&mut a, &mut b] {
        agent
            .call("scene_create", json!({ "scene": SCENE, "template": "basic" }))
            .unwrap();
        let s = agent
            .call("session_start", json!({ "scene": SCENE, "seed": 7, "ticks": 2 }))
            .unwrap();
        id = s["session"].as_str().unwrap().to_string();
    }
    (a, b, id, [fast, slow])
}

/// Everything a session shows: its hash and tick, its saved scene and its log.
fn seen(agent: &mut Agent, id: &str) -> Value {
    json!([
        answer(agent.call("session_step", json!({ "session": id, "ticks": 0 }))),
        answer(agent.call("observe_tree", json!({ "session": id }))),
        answer(agent.call("observe_logs", json!({ "session": id }))),
    ])
}

#[test]
fn session_edits_match_whole_scene_edits_step_by_step() {
    let (mut a, mut b, id, dirs) = session_pair("session");
    let mut refused = 0;
    for (i, (op, input)) in session_steps(&id).into_iter().enumerate() {
        let x = answer(a.call(op, input.clone()));
        let y = answer(b.call(op, input));
        assert_eq!(x, y, "step {i} (`{op}`) answered differently");
        refused += usize::from(x.get("err").is_some());
    }
    assert!(
        refused >= 10,
        "the refusals were meant to be exercised ({refused})"
    );
    assert_eq!(seen(&mut a, &id), seen(&mut b, &id));
    for d in dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[test]
fn a_batch_on_a_session_matches_single_edits() {
    let (mut a, mut b, id, dirs) = session_pair("session-batch");
    let ops = session_steps(&id);
    let mut start = 0;
    let mut runs = 0;
    let mut by_module = false;
    while start < ops.len() {
        // The whole-scene agent goes one call at a time up to its first refusal.
        let mut end = start;
        let mut refusal = None;
        while end < ops.len() && refusal.is_none() {
            refusal = b
                .call(ops[end].0, ops[end].1.clone())
                .err()
                .map(|e| (end - start, e));
            end += 1;
        }
        let batch: Vec<Value> = ops[start..end]
            .iter()
            .map(|(op, input)| json!({ "op": op, "input": input }))
            .collect();
        let r = a.call("batch_run", json!({ "ops": batch }));
        match (r, refusal) {
            (Ok(v), None) => assert_eq!(v["steps"], end - start),
            (Err(got), Some((at, want))) => {
                let op = ops[start + at].0;
                assert_eq!(got.0.len(), want.0.len());
                for (g, w) in got.0.iter().zip(&want.0) {
                    by_module |= w.message.contains("missing.ts");
                    assert_eq!(g.message, format!("step {at} (`{op}`): {}", w.message));
                    assert_eq!(g.code, w.code);
                }
            }
            (r, refusal) => panic!("steps {start}..{end}: batch {r:?}, single calls {refusal:?}"),
        }
        assert_eq!(seen(&mut a, &id), seen(&mut b, &id), "after steps {start}..{end}");
        start = end;
        runs += 1;
    }
    assert!(runs >= 10);
    assert!(
        by_module,
        "one refusal comes from a module, after the edits before it were held"
    );
    let tree = seen(&mut a, &id);
    assert!(tree[1]["ok"].is_object() && tree[2]["ok"].is_object(), "{tree}");
    for d in dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}
