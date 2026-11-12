//! Spawning, destroying (`onDestroy`) and timers, with only the core and the script module
//! loaded. Physics bodies of spawned entities are covered in
//! `crates/marionette-cli/tests/script_e2e.rs`.

use marionette_core::module::{Modules, registry_with};
use marionette_core::{Errors, Registry, Sim};
use marionette_script::ScriptModule;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

fn modules() -> Modules {
    static M: OnceLock<Modules> = OnceLock::new();
    M.get_or_init(|| Arc::new(vec![Box::new(ScriptModule::new(registry))]))
        .clone()
}

fn registry() -> Registry {
    registry_with(&modules())
}

fn dir(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-life-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for (name, text) in files {
        std::fs::write(d.join(name), text).unwrap();
    }
    d
}

/// Entity 1 runs `script`; entity 2 is a plain template named "template".
fn scene(script: &str) -> String {
    format!(
        r#"{{"format":"marionette.scene","version":1,"resources":{{"FixedStep":{{"dt":0.1}}}},"entities":[
        {{"id":1,"name":"boss","components":{{"Position":{{"x":0,"y":0,"z":0}},"Script":{{"path":"{script}"}}}}}},
        {{"id":2,"name":"template","components":{{"Position":{{"x":5,"y":0,"z":0}},"Velocity":{{"x":1,"y":0,"z":0}}}}}}]}}"#
    )
}

fn load(d: &Path, script: &str, seed: u64) -> Result<Sim, Errors> {
    Sim::with_modules(
        &scene(script),
        Some("t.scene.json"),
        seed,
        modules(),
        d.to_path_buf(),
    )
}

/// (tick, entity, detail) of every event of `kind`.
fn events(sim: &Sim, kind: &str) -> Vec<(u64, Option<u64>, Value)> {
    sim.events()
        .events
        .iter()
        .filter(|e| e.kind == kind)
        .map(|e| (e.tick, e.entity, e.detail.clone()))
        .collect()
}

fn scene_json(sim: &mut Sim) -> Value {
    serde_json::from_str(&sim.snapshot().to_text()).unwrap()
}

fn ids(sim: &mut Sim) -> Vec<u64> {
    scene_json(sim)["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_u64().unwrap())
        .collect()
}

fn no_errors(sim: &Sim) {
    let errs = events(sim, "script_error");
    assert!(errs.is_empty(), "{errs:?}");
}

/// A spawner (every 0.5 s, i.e. 5 ticks) whose bullets live 0.3 s (3 ticks) and say goodbye.
const SPAWNER: &str = r#"
export function start(self: Entity): void {
  engine.every(0.5, fire, { speed: 2 });
}
export function fire(self: Entity, arg: { speed: number }): void {
  const b = engine.spawn(
    { Position: { x: 0, y: engine.rand(), z: 0 }, Velocity: { x: arg.speed, y: 0, z: 0 }, Script: { path: "bullet.ts" } },
    "bullet",
  );
  self.state.fired = (self.state.fired ?? 0) + 1;
  self.state.last = b.id;
}
export function update(self: Entity): void {}
"#;

const BULLET: &str = r#"
export function start(self: Entity): void {
  self.state.born = engine.tick;
  self.destroy(0.3);
}
export function update(self: Entity): void {
  self.state.age = engine.tick - self.state.born;
}
export function onDestroy(self: Entity): void {
  engine.emit("bye", { age: self.state.age, x: self.get("Position")!.x });
}
"#;

#[test]
fn spawned_entities_run_next_tick_and_are_destroyed_with_on_destroy() {
    let d = dir("spawn", &[("boss.ts", SPAWNER), ("bullet.ts", BULLET)]);
    let mut s = load(&d, "boss.ts", 1).unwrap();
    s.run(5); // timer set at tick 0 for tick 5
    assert_eq!(ids(&mut s), vec![1, 2]);
    s.run(1); // tick 5 fires: bullet 3 spawned during the script stage
    assert_eq!(ids(&mut s), vec![1, 2, 3]);
    let v = scene_json(&mut s);
    let bullet = &v["entities"][2];
    assert_eq!(bullet["name"], "bullet");
    // Its script has not run yet: no state, and it starts next tick.
    assert!(bullet["components"].get("ScriptState").is_none(), "{bullet}");
    s.run(1); // tick 6: bullet starts, marks itself for tick 6 + 3
    let v = scene_json(&mut s);
    assert_eq!(v["entities"][2]["components"]["DestroyAt"]["tick"], 9);
    s.run(3); // ticks 7, 8, 9: removed at the end of tick 9's script stage
    assert_eq!(ids(&mut s), vec![1, 2]);
    let bye = events(&s, "bye");
    assert_eq!(bye.len(), 1);
    assert_eq!(bye[0].0, 9);
    assert_eq!(bye[0].1, Some(3));
    assert_eq!(bye[0].2["age"], 3, "update still runs in the removal tick");
    assert_eq!(events(&s, "destroyed").len(), 1);

    // Ids are never reused: the next bullet is 4 even though 3 is gone.
    s.run(1); // tick 10 fires again
    assert_eq!(ids(&mut s), vec![1, 2, 4]);
    let v = scene_json(&mut s);
    assert_eq!(v["resources"]["SpawnIds"]["next"], 5);
    no_errors(&s);
}

#[test]
fn spawning_destroying_and_timers_replay_identically_and_follow_the_seed() {
    let d = dir("replay", &[("boss.ts", SPAWNER), ("bullet.ts", BULLET)]);
    let run = |seed| {
        let mut s = load(&d, "boss.ts", seed).unwrap();
        s.run(400);
        no_errors(&s);
        (s.state_hash(), events(&s, "bye").len())
    };
    let (a, byes) = run(1);
    assert_eq!((a.clone(), byes), run(1));
    assert_eq!(byes, 79, "one bullet per 5 ticks, each gone 4 ticks later");
    assert_ne!(a, run(2).0);
}

#[test]
fn snapshot_with_pending_timers_and_removals_continues_exactly() {
    let d = dir("snap", &[("boss.ts", SPAWNER), ("bullet.ts", BULLET)]);
    let mut a = load(&d, "boss.ts", 7).unwrap();
    a.run(300);
    let expected = a.state_hash();

    let mut b = load(&d, "boss.ts", 7).unwrap();
    b.run(152); // a bullet is alive and marked, the spawner's timer is pending
    let v = scene_json(&mut b);
    let ents = v["entities"].as_array().unwrap();
    assert!(
        ents.iter().any(|e| e["components"].get("DestroyAt").is_some()),
        "{v}"
    );
    assert!(ents[0]["components"].get("ScriptTimers").is_some(), "{v}");
    let snap = b.save_state();
    b.run(40);
    b.restore(&snap).unwrap();
    b.run(148);
    assert_eq!(b.state_hash(), expected);
}

#[test]
fn timers_after_every_cancel_and_pending() {
    let script = r#"
export function start(self: Entity): void {
  self.state.log = [];
  engine.after(0.2, note, "a");           // tick 2
  engine.afterTicks(1, "note", "b");      // tick 1, by name
  const h = engine.after(0.3, note, "never");
  engine.every(0.1, tock, null, 0.4);     // ticks 4, 5, 6, ...
  self.state.pendingAtStart = engine.pending();
  self.state.cancelled = engine.cancel(h);
  engine.after(0, note, "next");          // 0 s means the next tick
}
export function note(self: Entity, arg: string): void {
  self.state.log.push([engine.tick, arg]);
}
export function tock(self: Entity): void {
  self.state.log.push([engine.tick, "tock"]);
  if (engine.tick === 6) self.state.cancelledTock = engine.cancel(tock);
}
export function update(self: Entity): void {
  self.state.pendingNow = engine.pending();
}
"#;
    let d = dir("timers", &[("t.ts", script)]);
    let mut s = load(&d, "t.ts", 1).unwrap();
    s.run(10);
    no_errors(&s);
    let v = scene_json(&mut s);
    let state: Value = serde_json::from_str(
        v["entities"][0]["components"]["ScriptState"]["data"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        state["log"],
        serde_json::json!([
            [1, "b"],
            [1, "next"],
            [2, "a"],
            [4, "tock"],
            [5, "tock"],
            [6, "tock"]
        ])
    );
    assert_eq!(state["pendingAtStart"], 4);
    assert_eq!(state["cancelled"], 1);
    assert_eq!(state["cancelledTock"], 1);
    assert_eq!(state["pendingNow"], 0);
}

#[test]
fn timer_errors_are_events_and_fire_once() {
    let script = r#"
export function start(self: Entity): void {
  engine.after(0.1, boom);
}
export function boom(self: Entity): void {
  throw new Error("boom went the timer");
}
export function update(self: Entity): void {
  self.state.n = (self.state.n ?? 0) + 1;
}
"#;
    let d = dir("timer-err", &[("t.ts", script)]);
    let mut s = load(&d, "t.ts", 1).unwrap();
    s.run(5);
    let errs = events(&s, "script_error");
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(
        errs[0].2["message"]
            .as_str()
            .unwrap()
            .contains("boom went the timer")
    );

    // Timers name exported functions only.
    let bad = r#"
function hidden(self: Entity): void {}
export function update(self: Entity): void {
  if (engine.tick === 0) engine.after(1, hidden);
}
"#;
    let d = dir("timer-bad", &[("t.ts", bad)]);
    let mut s = load(&d, "t.ts", 1).unwrap();
    s.run(1);
    let errs = events(&s, "script_error");
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].2["message"]
            .as_str()
            .unwrap()
            .contains("script.timer_function"),
        "{errs:?}"
    );
}

#[test]
fn clone_copies_components_but_not_script_state_and_loads_new_scripts() {
    let boss = r#"
export function update(self: Entity): void {
  if (engine.tick !== 0) return;
  const t = engine.find("template")!;
  const a = engine.clone(t, { Position: { x: -1, y: 0, z: 0 } });
  const b = engine.clone(self, {}, "boss-copy");
  self.state.made = [a.id, b.id, a.get("Velocity")!.x];
  engine.spawn({ Script: { path: "missing.ts" } }, "broken");
  try {
    engine.spawn({ Position: { x: "nope" } } as any);
  } catch (e) {
    self.state.refused = String(e).includes("script.bad_value");
  }
}
"#;
    let d = dir("clone", &[("boss.ts", boss)]);
    let mut s = load(&d, "boss.ts", 1).unwrap();
    s.run(3);
    let v = scene_json(&mut s);
    let ents = v["entities"].as_array().unwrap();
    assert_eq!(
        ids(&mut s),
        vec![1, 2, 3, 4, 5],
        "the refused spawn added nothing: {:?}",
        events(&s, "script_error")
    );
    let state: Value =
        serde_json::from_str(ents[0]["components"]["ScriptState"]["data"].as_str().unwrap()).unwrap();
    assert_eq!(state["made"], serde_json::json!([3, 4, 1]));
    assert_eq!(state["refused"], true);
    assert_eq!(ents[2]["name"], "template");
    assert_eq!(ents[3]["name"], "boss-copy");
    // The copy of the boss ran its own `start`/`update` from tick 1 with fresh state.
    let copy: Value =
        serde_json::from_str(ents[3]["components"]["ScriptState"]["data"].as_str().unwrap()).unwrap();
    assert_eq!(copy, serde_json::json!({}));
    // A script first used by a spawned entity is loaded at once; a missing one is one event.
    let errs = events(&s, "script_error");
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!(errs[0].2["script"], "missing.ts");
    assert!(errs[0].2["message"].as_str().unwrap().contains("script.read"));
}

#[test]
fn on_destroy_can_destroy_more_in_the_same_tick() {
    let chain = r#"
export function start(self: Entity): void {
  if (self.id !== 1) return;
  let prev = self.id;
  for (let i = 0; i < 4; i++) {
    const e = engine.spawn({ Script: { path: "chain.ts" } }, "link");
    e.set("Position", { x: prev + 100, y: 0, z: 0 });
    prev = e.id;
  }
}
export function update(self: Entity): void {
  if (self.id === 1 && engine.tick === 2) engine.destroy(self);
}
export function onDestroy(self: Entity): void {
  engine.emit("gone", { tick: engine.tick });
  const next = engine.query("Position").find((e) => e.get("Position")!.x === self.id + 100);
  if (next) engine.destroy(next);
}
"#;
    let d = dir("chain", &[("chain.ts", chain)]);
    let mut s = load(&d, "chain.ts", 1).unwrap();
    s.run(3);
    no_errors(&s);
    let gone: Vec<(u64, Option<u64>)> = events(&s, "gone").iter().map(|e| (e.0, e.1)).collect();
    assert_eq!(
        gone,
        vec![
            (2, Some(1)),
            (2, Some(3)),
            (2, Some(4)),
            (2, Some(5)),
            (2, Some(6))
        ]
    );
    assert_eq!(ids(&mut s), vec![2]);
}
