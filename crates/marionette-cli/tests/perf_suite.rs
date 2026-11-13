//! The shared performance suite (decision 0100): a quick run writes a report with outputs
//! that repeat exactly, and `perf_compare` reports identical outputs for two runs of the same
//! build and flags a changed state hash. Timings are not judged here (`perf_budgets.rs`).

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn out_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("marionette-perf-suite-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn quick_suite_repeats_outputs_and_compares_identical() {
    let dir = out_dir("repeat");
    let mut agent = Agent::new(repo_root(), false);
    let mut run = |label: &str| {
        let out = dir.join(format!("{label}.json"));
        let r = call(
            &mut agent,
            "perf_suite",
            json!({ "out": out.to_string_lossy(), "label": label, "length": "quick",
                    "repeats": 2, "only": ["sim/drift", "sim/factory-4x"], "no_gpu": true,
                    "max_busy_pct": 100.0 }),
        );
        assert_eq!(r["errors"], json!(0), "{r}");
        assert_eq!(r["cases"].as_array().unwrap().len(), 2, "{r}");
        out
    };
    let a = run("a");
    let b = run("b");
    let report: Value = serde_json::from_str(&std::fs::read_to_string(&a).unwrap()).unwrap();
    assert_eq!(report["format"], "marionette-perf/1");
    let drift = &report["cases"][0];
    assert_eq!(drift["name"], "sim/drift");
    assert_eq!(
        drift["tick"]["count"],
        json!(200),
        "100 ticks x 2 repeats: {drift}"
    );
    assert!(drift["state_hash"].as_str().unwrap().len() == 64);

    let cmp = call(
        &mut agent,
        "perf_compare",
        json!({ "base": a.to_string_lossy(), "new": b.to_string_lossy() }),
    );
    assert_eq!(cmp["outputs_identical"], json!(true), "{cmp}");
    assert_eq!(cmp["same_machine"], json!(true), "{cmp}");

    // A changed simulation result must be reported before any timing.
    let mut changed = report.clone();
    changed["cases"][0]["state_hash"] = json!("0".repeat(64));
    let c = dir.join("changed.json");
    std::fs::write(&c, changed.to_string()).unwrap();
    let cmp = call(
        &mut agent,
        "perf_compare",
        json!({ "base": a.to_string_lossy(), "new": c.to_string_lossy() }),
    );
    assert_eq!(cmp["outputs_identical"], json!(false), "{cmp}");
    assert!(
        cmp["output_mismatches"][0]
            .as_str()
            .unwrap()
            .starts_with("sim/drift")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Several agents share one benchmark machine: a held lock refuses the run, and a run that
/// takes the lock removes it afterwards.
#[test]
fn bench_lock_is_respected_and_released() {
    let dir = out_dir("lock");
    let lock = dir.join("pc-bench.lock");
    let mut agent = Agent::new(repo_root(), false);
    let input = |label: &str| {
        json!({ "out": dir.join(format!("{label}.json")).to_string_lossy(), "label": label,
                "length": "quick", "only": ["sim/drift"], "no_gpu": true, "max_busy_pct": 100.0,
                "lock": lock.to_string_lossy(), "lock_owner": "test" })
    };
    std::fs::write(&lock, "raytracing 1 pid 1\n").unwrap();
    let err = agent.call("perf_suite", input("held")).unwrap_err();
    assert!(format!("{err}").contains("raytracing"), "{err}");
    assert!(lock.exists(), "someone else's lock is never removed");
    std::fs::remove_file(&lock).unwrap();
    let r = call(&mut agent, "perf_suite", input("free"));
    assert!(r["conditions"].as_str().unwrap().contains("pc-bench.lock"), "{r}");
    assert!(!lock.exists(), "the run removes its own lock");
    let _ = std::fs::remove_dir_all(&dir);
}
