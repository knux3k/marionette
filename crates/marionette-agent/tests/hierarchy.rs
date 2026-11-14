//! Decision 0272 through the operations: `entity_attach` keeps an entity where it is, a child
//! follows its parent in files, in sessions and under a script, and re-parenting keeps place.

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::PathBuf;

const SCENE: &str = "scenes/h.scene.json";

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-hierarchy-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent.call(op, input).unwrap_or_else(|e| panic!("{op}: {e}"))
}

/// A quarter turn about +Y.
fn turned(x: f64, scale: f64) -> Value {
    let h = std::f32::consts::FRAC_1_SQRT_2;
    json!({ "translation": { "x": x, "y": 0, "z": 0 }, "rotation": { "x": 0, "y": h, "z": 0, "w": h },
            "scale": { "x": scale, "y": scale, "z": scale } })
}

fn plain(x: f64, y: f64, z: f64) -> Value {
    json!({ "translation": { "x": x, "y": y, "z": z }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "scale": { "x": 1, "y": 1, "z": 1 } })
}

fn near(v: &Value, want: [f64; 3]) {
    let got = [
        v["x"].as_f64().unwrap(),
        v["y"].as_f64().unwrap(),
        v["z"].as_f64().unwrap(),
    ];
    assert!(
        got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-4),
        "{got:?} is not {want:?}"
    );
}

fn build(agent: &mut Agent) {
    call(
        agent,
        "scene_create",
        json!({ "scene": SCENE, "template": "basic" }),
    );
    for (id, t) in [
        (10, turned(1.0, 2.0)),
        (11, plain(3.0, 1.0, 0.0)),
        (12, plain(0.0, 4.0, 0.0)),
    ] {
        call(
            agent,
            "entity_create",
            json!({ "scene": SCENE, "id": id, "components": { "Transform": t } }),
        );
    }
}

#[test]
fn attach_keeps_the_place_and_the_child_follows_in_a_file() {
    let d = dir("file");
    let mut agent = Agent::new(&d, false);
    build(&mut agent);
    let see = |agent: &mut Agent, id: u64| {
        call(agent, "observe_entity", json!({ "scene": SCENE, "id": id }))["components"].clone()
    };
    call(
        &mut agent,
        "entity_attach",
        json!({ "scene": SCENE, "id": 11, "parent": 10 }),
    );
    let c = see(&mut agent, 11);
    near(&c["Transform"]["translation"], [3.0, 1.0, 0.0]);
    // 2 m along world +X is 1 unit along the local +Z of a parent turned a quarter and doubled.
    near(&c["ChildOf"]["translation"], [0.0, 0.5, 1.0]);
    near(&c["ChildOf"]["scale"], [0.5, 0.5, 0.5]);
    // Move the parent: the child's world place follows without touching the child.
    call(
        &mut agent,
        "component_set",
        json!({ "scene": SCENE, "id": 10, "component": "Transform", "value": turned(-4.0, 2.0) }),
    );
    near(&see(&mut agent, 11)["Transform"]["translation"], [-2.0, 1.0, 0.0]);
    // Attach to another parent: the file still holds the old Transform, the place is kept.
    call(
        &mut agent,
        "entity_attach",
        json!({ "scene": SCENE, "id": 11, "parent": 12 }),
    );
    let c = see(&mut agent, 11);
    near(&c["Transform"]["translation"], [-2.0, 1.0, 0.0]);
    near(&c["ChildOf"]["translation"], [-2.0, -3.0, 0.0]);
    // Detach: it stays where it is.
    call(
        &mut agent,
        "component_remove",
        json!({ "scene": SCENE, "id": 11, "component": "ChildOf" }),
    );
    call(
        &mut agent,
        "component_set",
        json!({ "scene": SCENE, "id": 12, "component": "Transform", "value": plain(50.0, 0.0, 0.0) }),
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
#[cfg(feature = "script")]
fn a_child_follows_a_parent_that_a_script_moves() {
    let d = dir("session");
    let mut agent = Agent::new(&d, false);
    build(&mut agent);
    std::fs::write(
        d.join("scenes/slide.ts"),
        "export function update(self: Entity): void {\n  const t = self.get(\"Transform\");\n  \
         t.translation.y += 1;\n  self.set(\"Transform\", t);\n}\n",
    )
    .unwrap();
    call(
        &mut agent,
        "component_set",
        json!({ "scene": SCENE, "id": 12, "component": "Script", "value": { "path": "slide.ts" } }),
    );
    let s = call(&mut agent, "session_start", json!({ "scene": SCENE }));
    let id = s["session"].as_str().unwrap().to_string();
    call(
        &mut agent,
        "entity_attach",
        json!({ "session": id, "id": 11, "parent": 12 }),
    );
    call(
        &mut agent,
        "entity_attach",
        json!({ "session": id, "id": 10, "parent": 11 }),
    );
    call(&mut agent, "session_step", json!({ "session": id, "ticks": 5 }));
    let see = |agent: &mut Agent, e: u64| {
        call(agent, "observe_entity", json!({ "session": id, "id": e }))["components"]["Transform"]["translation"]
            .clone()
    };
    near(&see(&mut agent, 12), [0.0, 9.0, 0.0]);
    near(&see(&mut agent, 11), [3.0, 6.0, 0.0]);
    near(&see(&mut agent, 10), [1.0, 5.0, 0.0]);
    let _ = std::fs::remove_dir_all(&d);
}
