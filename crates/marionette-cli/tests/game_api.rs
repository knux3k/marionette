//! Engine features added while building the proof scenes (decision 0050), each checked
//! through the operation table: scene resources as operations, batch recipes, `engine.raycast`
//! in scripts, characters that face where a script turns them and that scripts drive
//! (`CharacterInput`), `engine.emit` without a detail, scenario assertions on script state and
//! event counts, and observe operations that replay input.

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::PathBuf;

fn project(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-game-api-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

fn transform(x: f32, y: f32, z: f32) -> Value {
    json!({ "translation": { "x": x, "y": y, "z": z }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "scale": { "x": 1, "y": 1, "z": 1 } })
}

fn cube(half: [f32; 3], sensor: bool) -> Value {
    json!({ "shape": { "Cuboid": { "half_extents": { "x": half[0], "y": half[1], "z": half[2] } } },
            "density": 1, "friction": 0.5, "restitution": 0, "sensor": sensor })
}

/// A 3D physics scene built only with operations: a floor, a wall at z = -6, a sensor
/// between, and a scripted character at the origin.
fn arena(a: &mut Agent, script: &str) {
    let scene = "arena.scene.json";
    call(
        a,
        "batch_run",
        json!({ "ops": [
            { "op": "scene_create", "input": { "scene": scene } },
            { "op": "resource_set", "input": { "scene": scene, "resource": "PhysicsSettings",
                "value": { "mode": "Dim3", "gravity": { "x": 0, "y": -9.81, "z": 0 } } } },
            { "op": "entity_create", "input": { "scene": scene, "id": 1, "name": "floor", "components": {
                "Transform": transform(0.0, -0.5, 0.0), "Collider": cube([20.0, 0.5, 20.0], false) } } },
            { "op": "entity_create", "input": { "scene": scene, "id": 2, "name": "wall", "components": {
                "Transform": transform(0.0, 1.0, -6.0), "Collider": cube([5.0, 1.0, 0.5], false) } } },
            { "op": "entity_create", "input": { "scene": scene, "id": 3, "name": "sensor", "components": {
                "Transform": transform(0.0, 1.0, -3.0), "Collider": cube([5.0, 1.0, 0.2], true) } } },
            { "op": "entity_create", "input": { "scene": scene, "id": 4, "name": "hero", "components": {
                "Transform": transform(0.0, 1.0, 0.0),
                "RigidBody": { "kind": "Kinematic", "linear_damping": 0, "angular_damping": 0,
                               "gravity_scale": 1, "lock_rotations": true },
                "Collider": { "shape": { "Capsule": { "half_height": 0.5, "radius": 0.4 } },
                              "density": 1, "friction": 0, "restitution": 0, "sensor": false },
                "CharacterController": { "speed": 3, "jump_speed": 5, "gravity": 9.81, "mass": 1,
                                         "move_x": "x", "move_forward": "f", "jump": "j" },
                "Script": { "path": "hero.ts" } } } }
        ] }),
    );
    std::fs::write(a.root().join("hero.ts"), script).unwrap();
}

fn state(a: &mut Agent, ticks: u64) -> Value {
    let e = call(
        a,
        "observe_entity",
        json!({ "scene": "arena.scene.json", "seed": 1, "ticks": ticks, "id": 4 }),
    );
    let text = e["components"]["ScriptState"]["data"]
        .as_str()
        .unwrap()
        .to_string();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn resources_are_set_and_removed_through_operations() {
    let d = project("resources");
    let mut a = Agent::new(&d, false);
    call(&mut a, "scene_create", json!({ "scene": "r.scene.json" }));
    let out = call(
        &mut a,
        "resource_set",
        json!({ "scene": "r.scene.json", "resource": "FixedStep", "value": { "dt": 0.02 } }),
    );
    assert_eq!(out["replaced"], false);
    let text = std::fs::read_to_string(d.join("r.scene.json")).unwrap();
    assert!(text.contains("\"FixedStep\""), "{text}");
    // A wrong value is refused and leaves the file alone.
    let bad = a.call(
        "resource_set",
        json!({ "scene": "r.scene.json", "resource": "FixedStep", "value": { "dt": "fast" } }),
    );
    assert!(bad.is_err());
    assert_eq!(std::fs::read_to_string(d.join("r.scene.json")).unwrap(), text);
    call(
        &mut a,
        "resource_remove",
        json!({ "scene": "r.scene.json", "resource": "FixedStep" }),
    );
    assert!(
        !std::fs::read_to_string(d.join("r.scene.json"))
            .unwrap()
            .contains("FixedStep")
    );
    let missing = a
        .call(
            "resource_remove",
            json!({ "scene": "r.scene.json", "resource": "FixedStep" }),
        )
        .unwrap_err();
    assert_eq!(missing.0[0].code, "agent.not_found");
}

#[test]
fn batches_stop_at_the_failing_step_and_do_not_nest() {
    let d = project("batch");
    let mut a = Agent::new(&d, false);
    let err = a
        .call(
            "batch_run",
            json!({ "ops": [
                { "op": "scene_create", "input": { "scene": "b.scene.json" } },
                { "op": "entity_create", "input": { "scene": "b.scene.json", "id": 1 } },
                { "op": "entity_create", "input": { "scene": "b.scene.json", "id": 1 } },
                { "op": "entity_create", "input": { "scene": "b.scene.json", "id": 2 } }
            ] }),
        )
        .unwrap_err();
    assert!(err.0[0].message.starts_with("step 2 (`entity_create`)"), "{err}");
    let scene = std::fs::read_to_string(d.join("b.scene.json")).unwrap();
    assert!(
        scene.contains("\"id\": 1") && !scene.contains("\"id\": 2"),
        "{scene}"
    );
    let nested = a
        .call(
            "batch_run",
            json!({ "ops": [{ "op": "batch_run", "input": { "ops": [] } }] }),
        )
        .unwrap_err();
    assert!(nested.0[0].message.contains("do not nest"));
    std::fs::write(
        d.join("x.ops.json"),
        r#"{"format": "other", "version": 1, "ops": []}"#,
    )
    .unwrap();
    assert!(a.call("batch_run", json!({ "file": "x.ops.json" })).is_err());
    assert!(a.call("batch_run", json!({})).is_err());
}

#[test]
fn scripts_cast_rays_that_skip_sensors_and_the_caster() {
    let d = project("ray");
    let mut a = Agent::new(&d, false);
    arena(
        &mut a,
        r#"export function update(self: Entity): void {
  const o = { x: 0, y: 1, z: 0 };
  const ahead = engine.raycast(o, { x: 0, y: 0, z: -1 }, 50, self);
  const short = engine.raycast(o, { x: 0, y: 0, z: -2 }, 2, self);
  const own = engine.raycast(o, { x: 0, y: 0, z: -1 }, 50);
  self.state.ahead = ahead ? { id: ahead.entity.id, d: Math.round(ahead.distance * 100) / 100, nz: ahead.normal.z } : null;
  self.state.short = short;
  self.state.own = own ? own.entity.id : null;
}
"#,
    );
    let s = state(&mut a, 2);
    assert_eq!(
        s["ahead"]["id"], 2,
        "the sensor at z = -3 is skipped, the wall at z = -5.5 is hit: {s}"
    );
    assert_eq!(s["ahead"]["d"], 5.5);
    assert_eq!(s["ahead"]["nz"], 1.0);
    assert_eq!(s["short"], Value::Null, "nothing within 2 m");
    assert_eq!(
        s["own"], 4,
        "without `exclude` the ray starts inside the caster's capsule"
    );
}

#[test]
fn script_driven_characters_face_turn_and_jump_once() {
    let d = project("drive");
    let mut a = Agent::new(&d, false);
    // Turn to face +X (90 degrees right), walk "forward" for 60 ticks, ask for one jump.
    arena(
        &mut a,
        r#"export function update(self: Entity): void {
  const t = self.get("Transform")!;
  if (engine.tick === 0) {
    t.rotation = { x: 0, y: -Math.SQRT1_2, z: 0, w: Math.SQRT1_2 };
    self.set("Transform", t);
  }
  self.set("CharacterInput", { move_x: 0, move_forward: engine.tick < 60 ? 1 : 0, jump: engine.tick === 30 });
  if (engine.tick === 31) self.state.jumpCleared = !self.get("CharacterInput")!.jump;
  self.state.x = t.translation.x;
  self.state.z = t.translation.z;
  self.state.ry = t.rotation.y;
}
"#,
    );
    let s = state(&mut a, 90);
    assert!(s["x"].as_f64().unwrap() > 2.5, "moved toward +X: {s}");
    assert!(s["z"].as_f64().unwrap().abs() < 0.01, "not toward -Z: {s}");
    assert!(
        (s["ry"].as_f64().unwrap() + std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-4,
        "kept its facing: {s}"
    );
    assert_eq!(s["jumpCleared"], true);
    let jumps = call(
        &mut a,
        "observe_events",
        json!({ "scene": "arena.scene.json", "seed": 1, "ticks": 90, "kind": "jump" }),
    );
    assert_eq!(jumps["total"], 1, "{jumps}");
}

#[test]
fn emit_without_detail_assertions_on_script_state_and_event_counts() {
    let d = project("emit");
    let mut a = Agent::new(&d, false);
    arena(
        &mut a,
        r#"export function update(self: Entity): void {
  if (engine.tick % 10 === 0) engine.emit("tock");
  self.state.ticks = (self.state.ticks ?? 0) + 1;
  self.state.nested = { list: [1, 2, 3] };
}
"#,
    );
    let r = call(
        &mut a,
        "test_run",
        json!({ "scenario": { "scene": "arena.scene.json", "seed": 1, "ticks": 30, "assertions": [
            { "event": { "kind": "tock", "at_least": 3, "at_most": 3 } },
            { "event": { "kind": "script_error", "at_most": 0 } },
            { "event": { "kind": "tock", "at_least": 4 } },
            { "field": { "entity": 4, "path": "ScriptState.data.ticks", "op": "eq", "value": 30 } },
            { "field": { "entity": 4, "path": "ScriptState.data.nested.list.2", "op": "eq", "value": 3 } },
            { "field": { "entity": 4, "path": "ScriptState.data.missing", "op": "exists" } }
        ] } }),
    );
    let passed: Vec<bool> = r["results"][0]["assertions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["passed"].as_bool().unwrap())
        .collect();
    assert_eq!(passed, [true, true, false, true, true, false], "{r}");
}

#[test]
fn observe_operations_replay_scripted_input() {
    let d = project("observe-input");
    let mut a = Agent::new(&d, false);
    arena(&mut a, "export function update(self: Entity): void {}\n");
    call(
        &mut a,
        "resource_set",
        json!({ "scene": "arena.scene.json", "resource": "InputConfig", "value": { "action_map": "map.json" } }),
    );
    std::fs::write(
        d.join("map.json"),
        r#"{"format": "marionette.input-map", "version": 1, "actions": {
  "x": {"bindings": [{"input": "key:KeyD", "scale": 1}]},
  "f": {"bindings": []}, "j": {"bindings": [{"input": "key:Space", "scale": 1}]}}}"#,
    )
    .unwrap();
    std::fs::write(
        d.join("walk.inputs.json"),
        r#"{"format": "marionette.input-events", "version": 1, "events": [{"tick": 0, "input": "key:KeyD", "value": 1}]}"#,
    )
    .unwrap();
    let x = |a: &mut Agent, input: Value| {
        call(
            a,
            "observe_entity",
            json!({ "scene": "arena.scene.json", "seed": 1, "ticks": 60, "id": 4, "input": input }),
        )["components"]["Transform"]["translation"]["x"]
            .as_f64()
            .unwrap()
    };
    let still = x(&mut a, Value::Null);
    assert!(still.abs() < 0.01, "{still}");
    assert!(x(&mut a, json!("walk.inputs.json")) > 2.5);
}

#[test]
fn characters_pass_through_sensors_trigger_them_and_can_be_teleported() {
    let d = project("sensor");
    let mut a = Agent::new(&d, false);
    // Walk toward -Z through the sensor at z = -3; at tick 100 teleport back to z = 2.
    arena(
        &mut a,
        r#"export function onTriggerEnter(self: Entity, other: Entity): void {
  self.state.entered = [...(self.state.entered ?? []), other.name];
}
export function update(self: Entity): void {
  self.set("CharacterInput", { move_x: 0, move_forward: engine.tick < 80 ? 1 : 0, jump: false });
  const t = self.get("Transform")!;
  if (engine.tick === 79) self.state.before = t.translation.z;
  if (engine.tick === 100) {
    t.translation.z = 2;
    self.set("Transform", t);
  }
  self.state.z = t.translation.z;
  self.state.me = self.name;
}
"#,
    );
    let s = state(&mut a, 120);
    assert_eq!(s["entered"], json!(["sensor"]), "{s}");
    assert!(
        s["before"].as_f64().unwrap() < -3.5,
        "walked through the sensor: {s}"
    );
    assert!(
        (s["z"].as_f64().unwrap() - 2.0).abs() < 1e-4,
        "teleported and stayed: {s}"
    );
    assert_eq!(s["me"], "hero");
}

#[test]
fn messages_reach_on_message_in_order_and_wait_as_world_state() {
    let d = project("messages");
    let mut a = Agent::new(&d, false);
    // Entity 4 (hero) sends to entity 1 (lower id: delivered next tick) and to itself; a spawned
    // entity gets its first message before its first update.
    arena(
        &mut a,
        r#"export function onMessage(self: Entity, m: Message): void {
  self.state.got = [...(self.state.got ?? []), [engine.tick, m.kind, m.detail, m.from]];
}
export function update(self: Entity): void {
  if (engine.tick === 1) {
    engine.send(self, "note", { n: 1 });
    engine.send(self.id, "note", { n: 2 });
    const kid = engine.spawn({ Script: { path: "hero.ts" } }, "kid");
    engine.send(kid, "hello");
    self.state.kid = kid.id;
  }
  if (engine.tick === 3) {
    try {
      engine.send(1, "x");
    } catch (e) {
      self.state.refused = String(e).includes("script.no_script");
    }
  }
}
"#,
    );
    let s = state(&mut a, 5);
    assert_eq!(
        s["got"],
        json!([[2, "note", { "n": 1 }, 4], [2, "note", { "n": 2 }, 4]]),
        "{s}"
    );
    assert_eq!(s["refused"], true);
    let kid = s["kid"].as_u64().unwrap();
    let k = call(
        &mut a,
        "observe_entity",
        json!({ "scene": "arena.scene.json", "seed": 1, "ticks": 5, "id": kid }),
    );
    let ks: Value = serde_json::from_str(k["components"]["ScriptState"]["data"].as_str().unwrap()).unwrap();
    assert_eq!(ks["got"], json!([[2, "hello", null, 4]]), "{ks}");
    // Waiting messages are world state (component `ScriptInbox`), emptied on delivery.
    let sess = call(
        &mut a,
        "session_start",
        json!({ "scene": "arena.scene.json", "seed": 1 }),
    );
    let id = sess["session"].as_str().unwrap().to_string();
    let inbox = |a: &mut Agent| {
        call(a, "observe_entity", json!({ "session": id, "id": 4 }))["components"]["ScriptInbox"]["data"]
            .as_str()
            .unwrap()
            .to_string()
    };
    call(&mut a, "session_step", json!({ "session": id, "ticks": 2 }));
    assert!(inbox(&mut a).contains("\"n\":2"), "two messages wait");
    call(&mut a, "session_step", json!({ "session": id, "ticks": 1 }));
    assert_eq!(inbox(&mut a), "[]", "delivered and emptied");
}

#[test]
fn box_shaped_characters_with_rounded_edges_slide_over_flat_ground() {
    let d = project("roundbox");
    let mut a = Agent::new(&d, false);
    // Walk forward for 40 ticks at 3 m/s: 2 m on open, flat ground.
    arena(
        &mut a,
        r#"export function update(self: Entity): void {
  self.set("CharacterInput", { move_x: 0, move_forward: engine.tick < 40 ? 1 : 0, jump: false });
  self.state.z = self.get("Transform")!.translation.z;
}
"#,
    );
    let hero = |shape: Value| {
        json!({ "scene": "arena.scene.json", "id": 4, "component": "Collider",
                "value": { "shape": shape, "density": 1, "friction": 0, "restitution": 0, "sensor": false } })
    };
    call(
        &mut a,
        "component_set",
        hero(json!({ "RoundCuboid": { "half_extents": { "x": 0.4, "y": 0.9, "z": 0.6 }, "border": 0.1 } })),
    );
    let z = state(&mut a, 60)["z"].as_f64().unwrap();
    assert!((z + 2.0).abs() < 0.1, "a rounded box walks the full 2 m: z = {z}");
    // 2D physics builds it too.
    call(
        &mut a,
        "resource_set",
        json!({ "scene": "arena.scene.json", "resource": "PhysicsSettings",
                "value": { "mode": "Dim2", "gravity": { "x": 0, "y": -9.81, "z": 0 } } }),
    );
    let s = call(
        &mut a,
        "observe_entity",
        json!({ "scene": "arena.scene.json", "seed": 1, "ticks": 5, "id": 4 }),
    );
    assert!(s["components"]["Transform"].is_object(), "{s}");
}
