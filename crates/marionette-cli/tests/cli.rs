//! End-to-end checks of the `marionette` binary with `--json`.

use serde_json::Value;
use std::process::Command;

const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/drift.scene.json");

fn marionette(args: &[&str]) -> (bool, Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_marionette"))
        .args(args)
        .output()
        .expect("binary runs");
    let json = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (out.status.success(), json)
}

fn run_hash(seed: &str) -> String {
    let (ok, v) = marionette(&[
        "run",
        EXAMPLE,
        "--headless",
        "--ticks",
        "1000",
        "--seed",
        seed,
        "--json",
    ]);
    assert!(ok, "{v}");
    v["state_hash"].as_str().unwrap().to_string()
}

#[test]
fn run_is_deterministic() {
    assert_eq!(run_hash("7"), run_hash("7"));
    assert_ne!(run_hash("7"), run_hash("8"));
}

#[test]
fn validate_and_fmt_check_pass_on_example() {
    let (ok, v) = marionette(&["validate", EXAMPLE, "--json"]);
    assert!(ok && v["ok"] == true, "{v}");
    let (ok, v) = marionette(&["scene", "fmt", "--check", EXAMPLE, "--json"]);
    assert!(ok && v["changed"] == false, "{v}");
}

#[test]
fn schema_is_json_schema() {
    let (ok, v) = marionette(&["schema", "--json"]);
    assert!(ok);
    assert!(v["$defs"]["Velocity"].is_object());
}

#[test]
fn errors_are_json() {
    let (ok, v) = marionette(&["validate", "does/not/exist.scene.json", "--json"]);
    assert!(!ok);
    assert_eq!(v["ok"], false);
    assert_eq!(v["diagnostics"][0]["code"], "io.read");
    let (ok, v) = marionette(&["run", EXAMPLE, "--json"]);
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "cli.no_window");
}
