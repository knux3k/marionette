//! Script host behavior with only the core and the script module loaded.

use marionette_core::module::{Modules, registry_with};
use marionette_core::{Errors, Registry, Sim};
use marionette_script::{ScriptModule, codes};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

fn modules() -> Modules {
    static M: OnceLock<Modules> = OnceLock::new();
    M.get_or_init(|| Arc::new(vec![Box::new(ScriptModule::new(registry))]))
        .clone()
}

fn registry() -> Registry {
    registry_with(&modules())
}

/// A fresh directory with `files` (name, text) in it.
fn dir(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-script-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for (name, text) in files {
        std::fs::write(d.join(name), text).unwrap();
    }
    d
}

/// One scripted entity (id 1) with Position and Velocity, plus entity 2 named "target".
fn scene(script: &str) -> String {
    format!(
        r#"{{"format":"marionette.scene","version":1,"resources":{{}},"entities":[
        {{"id":1,"name":"mover","components":{{"Position":{{"x":0,"y":0,"z":0}},"Velocity":{{"x":0,"y":0,"z":0}},"Script":{{"path":"{script}"}}}}}},
        {{"id":2,"name":"target","components":{{"Position":{{"x":10,"y":0,"z":0}}}}}}]}}"#
    )
}

fn load(d: &std::path::Path, script: &str, seed: u64) -> Result<Sim, Errors> {
    Sim::with_modules(
        &scene(script),
        Some("test.scene.json"),
        seed,
        modules(),
        d.to_path_buf(),
    )
}

fn events(sim: &Sim, kind: &str) -> Vec<serde_json::Value> {
    sim.events()
        .events
        .iter()
        .filter(|e| e.kind == kind)
        .map(|e| e.detail.clone())
        .collect()
}

const MOVER: &str = r#"
export function start(self: Entity): void {
  self.state.started = (self.state.started ?? 0) + 1;
}
export function update(self: Entity): void {
  const target = engine.find("target")!.get("Position")!;
  const p = self.get("Position")!;
  const v = self.get("Velocity")!;
  v.x = (target.x - p.x) * 0.5 + (engine.rand() - 0.5);
  v.y = Math.sin(engine.tick * 0.1) + Math.atan2(1, 2) * 0.01 + 2 ** 0.5 * 0.001;
  self.set("Velocity", v);
  self.state.n = (self.state.n ?? 0) + 1;
}
"#;

#[test]
fn same_seed_same_hash_and_seed_matters() {
    let d = dir("hash", &[("m.ts", MOVER)]);
    let run = |seed| {
        let mut s = load(&d, "m.ts", seed).unwrap();
        s.run(200);
        s.state_hash()
    };
    assert_eq!(run(1), run(1));
    assert_ne!(run(1), run(2));
}

#[test]
fn scripts_change_components_and_keep_state() {
    let d = dir("state", &[("m.ts", MOVER)]);
    let mut s = load(&d, "m.ts", 1).unwrap();
    s.run(100);
    let scene = s.snapshot().to_text();
    let v: serde_json::Value = serde_json::from_str(&scene).unwrap();
    let mover = &v["entities"][0]["components"];
    assert!(mover["Position"]["x"].as_f64().unwrap() > 1.0, "{mover}");
    let state: serde_json::Value =
        serde_json::from_str(mover["ScriptState"]["data"].as_str().unwrap()).unwrap();
    assert_eq!(state["n"], 100);
    assert_eq!(state["started"], 1, "start runs once");
}

#[test]
fn snapshot_then_restore_continues_exactly() {
    let d = dir("snap", &[("m.ts", MOVER)]);
    let mut a = load(&d, "m.ts", 3).unwrap();
    a.run(300);
    let expected = a.state_hash();

    let mut b = load(&d, "m.ts", 3).unwrap();
    b.run(150);
    let snap = b.save_state();
    b.run(50); // wander off, then come back
    b.restore(&snap).unwrap();
    b.run(150);
    assert_eq!(b.state_hash(), expected);
}

#[test]
fn clock_and_unseeded_randomness_are_gone_and_math_is_libm() {
    let probe = r#"
export function update(self: Entity): void {
  const g = globalThis as any;
  engine.emit("probe", {
    date: typeof g.Date,
    random: typeof Math.random,
    perf: typeof g.performance,
    weakref: typeof g.WeakRef,
    sin: Math.sin(1),
    pow: 3 ** 0.7,
    order: Object.keys({ b: 1, a: 2, 10: 3, 2: 4 }).join(","),
  });
}
"#;
    let d = dir("sandbox", &[("p.ts", probe)]);
    let mut s = load(&d, "p.ts", 1).unwrap();
    s.step();
    let e = &events(&s, "probe")[0];
    assert_eq!(e["date"], "undefined");
    assert_eq!(e["random"], "undefined");
    assert_eq!(e["perf"], "undefined");
    assert_eq!(e["weakref"], "undefined");
    assert_eq!(e["sin"].as_f64().unwrap().to_bits(), libm::sin(1.0).to_bits());
    assert_eq!(
        e["pow"].as_f64().unwrap().to_bits(),
        libm::pow(3.0, 0.7).to_bits()
    );
    assert_eq!(e["order"], "2,10,b,a", "property order is fixed by the language");
}

#[test]
fn load_errors_are_data() {
    let d = dir(
        "errors",
        &[
            (
                "syntax.ts",
                "export function update(self: Entity): void {\n  let x = ;\n}\n",
            ),
            ("noupdate.ts", "export function start(self: Entity): void {}\n"),
            (
                "throws.ts",
                "throw new Error('boom');\nexport function update(): void {}\n",
            ),
        ],
    );
    let err = |name: &str| {
        load(&d, name, 1)
            .err()
            .unwrap_or_else(|| panic!("{name} loaded"))
            .0[0]
            .clone()
    };
    let e = err("syntax.ts");
    assert_eq!(e.code, codes::SYNTAX);
    assert_eq!(e.line, Some(2));
    assert_eq!(e.file.as_deref(), Some("syntax.ts"));
    assert_eq!(err("noupdate.ts").code, codes::NO_UPDATE);
    let e = err("throws.ts");
    assert_eq!(e.code, codes::LOAD);
    assert!(e.message.contains("boom"), "{}", e.message);
    assert_eq!(err("missing.ts").code, codes::READ);
}

#[test]
fn runtime_errors_become_events_and_the_tick_goes_on() {
    let bad = r#"
export function update(self: Entity): void {
  if (engine.tick === 2) throw new Error("tick two");
  try {
    self.set("Velocity", { x: "fast" } as any);
  } catch (e) {
    engine.emit("caught", { message: String(e) });
  }
  self.state.ok = engine.tick;
}
"#;
    let d = dir("runtime", &[("b.ts", bad)]);
    let mut s = load(&d, "b.ts", 1).unwrap();
    s.run(4);
    let errs = events(&s, "script_error");
    assert_eq!(errs.len(), 1);
    assert!(errs[0]["message"].as_str().unwrap().contains("tick two"));
    let caught = events(&s, "caught");
    assert_eq!(caught.len(), 3);
    assert!(
        caught[0]["message"]
            .as_str()
            .unwrap()
            .contains("script.bad_value"),
        "{}",
        caught[0]
    );
    assert_eq!(s.tick(), 4);
}

#[test]
fn reload_swaps_behavior_fast_and_keeps_state() {
    let v1 = "export function update(self: Entity): void { self.state.v = 1; self.state.n = (self.state.n ?? 0) + 1; }";
    let v2 = "export function update(self: Entity): void { self.state.v = 2; self.state.n = (self.state.n ?? 0) + 1; }";
    let d = dir("reload", &[("r.ts", v1)]);
    let mut s = load(&d, "r.ts", 1).unwrap();
    s.run(10);
    std::fs::write(d.join("r.ts"), v2).unwrap();
    let t = std::time::Instant::now();
    let loaded = marionette_script::reload(&mut s.world).unwrap();
    let elapsed = t.elapsed();
    assert_eq!(loaded, vec!["r.ts".to_string()]);
    assert!(elapsed.as_secs_f64() < 1.0, "reload took {elapsed:?}");
    s.step();
    let text = s.snapshot().to_text();
    assert!(text.contains(r#"{\"v\":2,\"n\":11}"#), "{text}");

    // A broken edit keeps the old scripts running.
    std::fs::write(d.join("r.ts"), "export function update( {").unwrap();
    assert_eq!(
        marionette_script::reload(&mut s.world).unwrap_err().0[0].code,
        codes::SYNTAX
    );
    s.step();
    assert!(s.snapshot().to_text().contains(r#"{\"v\":2,\"n\":12}"#));
}

#[test]
fn typings_cover_every_component() {
    let r = registry();
    let text = marionette_script::typings::typings(&r);
    for e in r.of_kind(marionette_core::Kind::Component) {
        assert!(
            text.contains(&format!("  {}: {};", e.name, e.name)),
            "{} missing",
            e.name
        );
    }
    assert!(text.contains("interface ScriptHooks"));
}
