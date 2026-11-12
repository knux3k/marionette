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
    let (ok, v) = marionette(&[
        "screenshot",
        EXAMPLE,
        "--size",
        "0x10",
        "--out",
        "x.png",
        "--json",
    ]);
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "cli.bad_size");
}

const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

fn gpu_required() -> bool {
    std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() == Ok("1")
}

#[test]
fn every_example_is_valid_and_canonical() {
    for name in ["drift", "showcase", "sprites"] {
        let path = format!("{EXAMPLES}/{name}.scene.json");
        let (ok, v) = marionette(&["scene", "fmt", "--check", &path, "--json"]);
        assert!(ok && v["changed"] == false, "{name}: {v}");
    }
}

#[test]
fn screenshot_is_deterministic_and_leaves_state_alone() {
    let dir = std::env::temp_dir().join(format!("marionette-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["showcase", "sprites"] {
        let scene = format!("{EXAMPLES}/{name}.scene.json");
        let mut images = Vec::new();
        let mut hashes = Vec::new();
        for i in 0..2 {
            let out = dir.join(format!("{name}-{i}.png"));
            let out = out.to_str().unwrap();
            let args = [
                "screenshot",
                &scene,
                "--ticks",
                "30",
                "--seed",
                "3",
                "--size",
                "320x180",
                "--out",
                out,
                "--json",
            ];
            let (ok, v) = marionette(&args);
            if !ok && v["diagnostics"][0]["code"] == "render.no_adapter" && !gpu_required() {
                eprintln!("skipping: no GPU adapter");
                return;
            }
            assert!(ok, "{v}");
            images.push(std::fs::read(out).unwrap());
            hashes.push(v["state_hash"].as_str().unwrap().to_string());
        }
        assert!(images[0] == images[1], "{name}: PNG bytes differ between runs");
        let (_, v) = marionette(&[
            "run",
            &scene,
            "--headless",
            "--ticks",
            "30",
            "--seed",
            "3",
            "--json",
        ]);
        assert_eq!(
            hashes[0],
            v["state_hash"].as_str().unwrap(),
            "{name}: rendering changed the state hash"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drift_hash_is_unchanged_from_plan_line_2() {
    let (ok, v) = marionette(&[
        "run",
        EXAMPLE,
        "--headless",
        "--ticks",
        "1000",
        "--seed",
        "1",
        "--json",
    ]);
    assert!(ok);
    assert_eq!(
        v["state_hash"],
        "7441258fdf64141911d44e0608c48498897564168684fdd86cd244e149926558"
    );
}
