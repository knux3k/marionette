//! Misbehaving scripts fail as data and never take the engine down (decision 0029).

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
    let d = std::env::temp_dir().join(format!("marionette-script-bad-{tag}-{}", std::process::id()));
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

fn first_error(sim: &Sim) -> String {
    let errs = events(sim, "script_error");
    assert!(!errs.is_empty(), "expected a script_error event");
    errs[0]["message"].as_str().unwrap().to_string()
}

#[test]
fn an_endless_loop_is_stopped_and_the_next_tick_runs() {
    let src = r#"
export function update(self: Entity): void {
  if (engine.tick === 1) { while (true) {} }
  self.state.t = engine.tick;
}
"#;
    let d = dir("loop", &[("l.ts", src)]);
    let mut s = load(&d, "l.ts", 1).unwrap();
    let t = std::time::Instant::now();
    s.run(3);
    assert!(t.elapsed().as_secs() < 30, "the loop was stopped by the budget");
    assert!(first_error(&s).contains("script.budget"), "{}", first_error(&s));
    assert_eq!(events(&s, "script_error").len(), 1);
    assert_eq!(s.tick(), 3);
}

#[test]
fn an_endless_loop_at_load_is_a_load_error() {
    let d = dir(
        "loadloop",
        &[(
            "l.ts",
            "for (;;) {}\nexport function update(self: Entity): void {}",
        )],
    );
    let e = load(&d, "l.ts", 1).err().expect("load fails");
    let text = serde_json::to_string(&e).unwrap();
    assert!(
        text.contains(codes::LOAD) && text.contains("script.budget"),
        "{text}"
    );
}

#[test]
fn running_out_of_script_memory_is_an_event_and_memory_comes_back() {
    let src = r#"
export function update(self: Entity): void {
  if (engine.tick === 1) {
    const hog: number[][] = [];
    for (;;) hog.push(new Array(100000).fill(1));
  }
  self.state.t = engine.tick;
}
"#;
    let d = dir("oom", &[("o.ts", src)]);
    let mut s = load(&d, "o.ts", 1).unwrap();
    s.run(4);
    let msg = first_error(&s);
    assert!(msg.contains("memory") || msg.contains("budget"), "{msg}");
    assert_eq!(s.tick(), 4);
    let used = marionette_script::heap_bytes(&s.world).unwrap();
    assert!(used < 16 << 20, "the hog was collected: {used} bytes in use");
}

#[test]
fn deep_recursion_is_an_event() {
    let src = r#"
function down(n: number): number { return down(n + 1) + 1; }
export function update(self: Entity): void {
  if (engine.tick === 1) down(0);
  self.state.t = engine.tick;
}
"#;
    let d = dir("deep", &[("r.ts", src)]);
    let mut s = load(&d, "r.ts", 1).unwrap();
    s.run(3);
    assert!(
        first_error(&s).to_lowercase().contains("stack"),
        "{}",
        first_error(&s)
    );
    assert_eq!(s.tick(), 3);
}

#[test]
fn engine_calls_outside_a_tick_are_errors_not_crashes() {
    let src = "const t = engine.tick;\nexport function update(self: Entity): void {}";
    let d = dir("toplevel", &[("t.ts", src)]);
    let e = load(&d, "t.ts", 1).err().expect("load fails");
    let text = serde_json::to_string(&e).unwrap();
    assert!(text.contains(codes::LOAD), "{text}");
    // The engine still loads and runs other scripts afterwards.
    let d2 = dir(
        "toplevel2",
        &[(
            "ok.ts",
            "export function update(self: Entity): void { self.state.ok = true; }",
        )],
    );
    let mut s = load(&d2, "ok.ts", 1).unwrap();
    s.run(2);
    assert!(events(&s, "script_error").is_empty());
}
