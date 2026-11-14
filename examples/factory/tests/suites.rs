//! The engine's shared suites run the factory when the engine they run in has it (decision
//! 0290): its stability modes pass, and `perf_suite` runs the `sim/factory-*` cases that a
//! plain engine skips.

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

fn factory_agent() -> Agent {
    Agent::with_engine(repo_root(), false, marionette_example_factory::engine().unwrap())
}

/// Runs in its own `factory-bench` process, as `marionette stability` does: the memory check
/// measures the whole process, so another test running beside it would count.
#[test]
fn factory_stability_modes_pass() {
    let out = std::env::temp_dir().join(format!("marionette-factory-stability-{}", std::process::id()));
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_factory-bench"))
        .current_dir(repo_root())
        .args([
            "--json",
            "stability",
            "--suite",
            "examples/factory/stability.json",
            "--out",
        ])
        .arg(&out)
        .output()
        .expect("run factory-bench");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let r: Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "not JSON ({e}): {stdout}\n{}",
            String::from_utf8_lossy(&run.stderr)
        )
    });
    let modes: Vec<&str> = r["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["mode"].as_str().unwrap())
        .collect();
    assert_eq!(modes, ["factory-sim", "factory-vr"], "{r}");
    for m in r["results"].as_array().unwrap() {
        assert_eq!(m["passed"], json!(true), "{m}");
    }
    assert!(run.status.success());
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn perf_suite_runs_factory_cases_only_where_the_operation_exists() {
    let dir = std::env::temp_dir().join(format!("marionette-factory-suite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = |label: &str| {
        json!({ "out": dir.join(format!("{label}.json")).to_string_lossy(), "label": label,
                "length": "quick", "repeats": 1, "min_case_s": 0.0, "only": ["sim/factory-4x"],
                "no_gpu": true, "max_busy_pct": 100.0 })
    };
    let with = call(&mut factory_agent(), "perf_suite", input("with"));
    assert_eq!(with["errors"], json!(0), "{with}");
    assert_eq!(with["skipped"], json!(0), "{with}");
    let report: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("with.json")).unwrap()).unwrap();
    assert_eq!(report["cases"][0]["name"], "sim/factory-4x");
    assert_eq!(report["cases"][0]["state_hash"].as_str().unwrap().len(), 64);

    let without = call(
        &mut Agent::new(repo_root(), false),
        "perf_suite",
        input("without"),
    );
    assert_eq!(without["errors"], json!(0), "{without}");
    assert_eq!(without["skipped"], json!(1), "{without}");
    let _ = std::fs::remove_dir_all(&dir);
}
