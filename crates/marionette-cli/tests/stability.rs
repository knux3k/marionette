//! Stability suite (decision 0027, docs/ai/STABILITY.md): the fast suite passes for every game
//! mode, the harness catches each kind of failure it promises to catch, crash reports replay,
//! and every scene in the repository has a stability mode.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn marionette(args: &[&str]) -> (bool, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_marionette"))
        .current_dir(root())
        .arg("--json")
        .args(args)
        .output()
        .expect("run marionette");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "not JSON ({e}): {stdout}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (out.status.success(), value)
}

fn out_dir(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("marionette-stability-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.to_string_lossy().replace('\\', "/")
}

fn require_gpu() -> bool {
    std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() == Ok("1")
}

fn suite() -> Value {
    let text = std::fs::read_to_string(root().join("stability/suite.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn failures(result: &Value) -> Vec<String> {
    result["failures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["kind"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn fast_suite_passes_for_every_mode() {
    let out = out_dir("fast");
    let (ok, v) = marionette(&["stability", "--out", &out]);
    let modes: Vec<String> = suite()["modes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap().to_string())
        .collect();
    let ran: Vec<String> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["mode"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ran, modes, "every mode runs once in the fast suite");
    for r in v["results"].as_array().unwrap() {
        assert!(
            r["passed"] == true,
            "mode {} failed: {}",
            r["mode"],
            serde_json::to_string_pretty(&r["failures"]).unwrap()
        );
        if r["skipped"].is_string() {
            assert!(
                !require_gpu(),
                "{} skipped with MARIONETTE_REQUIRE_GPU=1",
                r["mode"]
            );
            continue;
        }
        assert_eq!(r["ticks_done"], r["ticks"], "{}", r["mode"]);
        assert_eq!(r["state_hash"].as_str().unwrap().len(), 64, "{}", r["mode"]);
    }
    assert!(ok && v["passed"] == true);
    assert!(Path::new(&out).join("report.json").is_file());
}

#[test]
fn drift_mode_reaches_the_pinned_drift_hash() {
    // The stability runner steps the same simulation as `marionette run`: the core-headless
    // mode at 1,000 ticks and seed 1 must land on the drift hash in CLAUDE.md.
    let (_, v) = marionette(&[
        "op",
        "stability_run",
        r#"{"modes":["core-headless"],"ticks":1000,"seed":1}"#,
    ]);
    assert_eq!(
        v["result"]["results"][0]["state_hash"],
        "7441258fdf64141911d44e0608c48498897564168684fdd86cd244e149926558"
    );
}

#[test]
fn injected_panic_is_reported_with_tick_and_backtrace() {
    let input = json!({ "modes": ["physics-3d"], "ticks": 60, "inject": { "panic_at": 20 } }).to_string();
    let (_, v) = marionette(&["op", "stability_run", &input]);
    let r = &v["result"]["results"][0];
    assert_eq!(v["result"]["passed"], false);
    assert_eq!(failures(r), ["panic"]);
    let f = &r["failures"][0];
    assert_eq!(f["tick"], 20);
    assert!(
        f["message"]
            .as_str()
            .unwrap()
            .contains("injected panic at tick 20")
    );
    assert!(
        f["backtrace"].as_str().unwrap().contains("stability"),
        "{}",
        f["backtrace"]
    );
    assert!(
        r["reproduce"]
            .as_str()
            .unwrap()
            .contains("--mode physics-3d --seed 1 --ticks 60")
    );
}

#[test]
fn injected_hang_is_detected() {
    let input = json!({ "modes": ["core-headless"], "ticks": 100, "hang_s": 1.0,
                        "inject": { "hang_at": 40 } })
    .to_string();
    let (_, v) = marionette(&["op", "stability_run", &input]);
    let r = &v["result"]["results"][0];
    assert_eq!(failures(r), ["hang"]);
    assert_eq!(r["failures"][0]["tick"], 40);
}

#[test]
fn nondeterminism_is_caught_and_the_crash_report_replays() {
    let out = out_dir("nondet");
    // The injected fault only touches the replay run, so the first run's hash must be what
    // `marionette run` gives with the crash report's input file.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_marionette"));
    let status = cmd
        .current_dir(root())
        .args(["--json", "op", "stability_run"])
        .arg(
            json!({ "modes": ["physics-3d"], "ticks": 120, "seed": 7,
                     "inject": { "nondeterminism": true } })
            .to_string(),
        )
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&status.stdout).unwrap();
    let r = &v["result"]["results"][0];
    assert_eq!(failures(r), ["nondeterminism"]);
    let events = r["input_events"]
        .as_array()
        .expect("failed runs carry their input");
    assert!(!events.is_empty(), "physics-3d gets random input");
    std::fs::create_dir_all(&out).unwrap();
    let file = Path::new(&out).join("replay.inputs.json");
    std::fs::write(
        &file,
        json!({ "format": "marionette.input-events", "version": 1, "events": events }).to_string(),
    )
    .unwrap();
    let (ok, run) = marionette(&[
        "run",
        "examples/playground3d.scene.json",
        "--headless",
        "--ticks",
        "120",
        "--seed",
        "7",
        "--input",
        file.to_str().unwrap(),
    ]);
    assert!(ok, "{run}");
    assert_eq!(
        run["state_hash"], r["state_hash"],
        "the crash report reproduces the run"
    );
}

#[test]
fn budget_miss_is_caught() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_marionette"));
    let o = cmd
        .current_dir(root())
        .args(["--json", "op", "stability_run"])
        .arg(
            json!({ "modes": ["core-headless"], "ticks": 30, "enforce_timing": true,
                     "budget_scale": 1.0, "inject": { "slow_ms": 5.0 } })
            .to_string(),
        )
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let r = &v["result"]["results"][0];
    assert_eq!(failures(r), ["budget"]);
    assert!(
        r["failures"][0]["message"]
            .as_str()
            .unwrap()
            .contains("over the budget")
    );
}

#[test]
fn the_cli_writes_crash_reports_and_exits_1() {
    // A project of its own whose only mode cannot meet its budget.
    let project = PathBuf::from(out_dir("project"));
    std::fs::create_dir_all(project.join("stability")).unwrap();
    std::fs::copy(
        root().join("examples/drift.scene.json"),
        project.join("drift.scene.json"),
    )
    .unwrap();
    let suite = json!({ "format": "marionette.stability-suite", "version": 1, "seed": 3, "modes": [
        { "name": "tight", "kind": "sim", "scene": "drift.scene.json", "random_input": true,
          "fast_ticks": 20, "long_ticks": 20, "tick_budget_ms": 0.000001 } ] });
    std::fs::write(project.join("stability/suite.json"), suite.to_string()).unwrap();
    let out = project.join("reports");
    let o = Command::new(env!("CARGO_BIN_EXE_marionette"))
        .current_dir(&project)
        .args(["--json", "stability", "--enforce-timing", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["passed"], false);
    let crash: Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("tight-seed3.crash.json")).unwrap()).unwrap();
    assert_eq!(crash["format"], "marionette.stability-report");
    assert_eq!(crash["mode"], "tight");
    assert_eq!(crash["seed"], 3);
    assert_eq!(crash["failures"][0]["kind"], "budget");
    assert!(
        crash["reproduce"]
            .as_str()
            .unwrap()
            .contains("--mode tight --seed 3 --ticks 20")
    );
    assert!(out.join("tight-seed3.inputs.json").is_file());
    let report: Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    assert!(
        report["results"][0].get("input_events").is_none(),
        "the summary leaves inputs out"
    );
}

#[test]
fn memory_growth_is_caught() {
    let input = json!({ "modes": ["core-headless"], "ticks": 100,
                        "inject": { "leak_kib": 1024 } })
    .to_string();
    let (_, v) = marionette(&["op", "stability_run", &input]);
    let r = &v["result"]["results"][0];
    if r["memory"].is_null() {
        // No memory probe on this platform (resident size is read on Linux).
        return;
    }
    assert_eq!(failures(r), ["memory"], "{r}");
}

#[test]
fn every_scene_has_a_stability_mode() {
    // Rule (docs/ai/STABILITY.md): a new game mode, example or proof scene registers a mode in
    // stability/suite.json, or an `exempt` entry with the reason.
    let suite = suite();
    let modes = suite["modes"].as_array().unwrap();
    let covered: Vec<&str> = modes.iter().map(|m| m["scene"].as_str().unwrap()).collect();
    let exempt = suite["exempt"].as_object().unwrap();
    let dirs: Vec<String> = std::fs::read_dir(root())
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !matches!(n.as_str(), "crates" | "spikes" | "target" | "docs") && !n.starts_with('.'))
        .collect();
    let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    let scenes = marionette_agent::stability::scene_files(&root(), &dirs);
    assert!(scenes.contains(&"examples/drift.scene.json".to_string()));
    let missing: Vec<&String> = scenes
        .iter()
        .filter(|s| !covered.contains(&s.as_str()) && !exempt.contains_key(s.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "scenes without a stability mode (add one to stability/suite.json, see docs/ai/STABILITY.md): {missing:?}"
    );
    for m in modes {
        let scene = m["scene"].as_str().unwrap();
        let generated = m.get("setup").is_some_and(|s| !s.as_array().unwrap().is_empty());
        assert!(
            generated || root().join(scene).is_file(),
            "mode {} names a missing scene {scene}",
            m["name"]
        );
    }
    for (scene, reason) in exempt {
        assert!(
            root().join(scene).is_file(),
            "exempt scene {scene} does not exist"
        );
        assert!(
            reason.as_str().is_some_and(|r| r.len() > 10),
            "exempt {scene} needs a reason"
        );
    }
    for kind in ["sim", "render", "agent"] {
        assert!(
            modes.iter().any(|m| m["kind"] == kind),
            "no `{kind}` mode in the suite"
        );
    }
}

#[test]
fn bad_suites_and_modes_are_refused_as_data() {
    let (ok, v) = marionette(&["op", "stability_run", r#"{"modes":["nope"]}"#]);
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "stability.unknown_mode");
    let (ok, v) = marionette(&["op", "stability_run", r#"{"suite":"examples/drift.scene.json"}"#]);
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "stability.suite");
}
