//! Asset generation through the CLI and operations (workstream W4, decision 0045): recipes
//! written by an operation build into engine-ready assets on the mock with no key or
//! network; a paid provider with no cap is refused with a clear reason; only a person can
//! approve an image, never an operation or a piped command.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_marionette");

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-gen-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Runs the CLI with a fake image key, no spend cap and an empty user config folder.
fn cli(root: &Path, args: &[&str]) -> (bool, Value) {
    let config_home = root.join("user-config");
    let out = Command::new(BIN)
        .env("DASHSCOPE_API_KEY", "test-key-not-real")
        .env_remove("TRIPO_API_KEY")
        .env_remove("MARIONETTE_SPEND_CAP_USD")
        .env("XDG_CONFIG_HOME", &config_home)
        .env("APPDATA", &config_home)
        .stdin(Stdio::null())
        .arg("--project")
        .arg(root)
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{args:?}: bad JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_eq!(out.status.success(), v["ok"] == json!(true));
    (v["ok"] == json!(true), v)
}

fn op(root: &Path, name: &str, input: Value) -> (bool, Value) {
    cli(root, &["op", name, &input.to_string()])
}

fn first_code(v: &Value) -> String {
    v["diagnostics"][0]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn recipes_build_rebuild_and_list_through_the_cli() {
    let root = temp_dir("build");
    let (ok, v) = op(
        &root,
        "generate_recipe",
        json!({"name": "barrel_ref", "output": "image", "prompt": "a barrel", "size": [128, 128]}),
    );
    assert!(ok, "{v}");
    assert_eq!(v["result"]["recipe"], "assets/generated/barrel_ref.recipe.json");
    assert_eq!(v["result"]["plan"]["steps"][0]["status"], "call");
    assert!(
        v["result"]["plan"]["before_generating"][0]
            .as_str()
            .unwrap()
            .contains("project")
    );
    let (ok, v) = op(
        &root,
        "generate_recipe",
        json!({"name": "barrel", "output": "model", "from": "barrel_ref", "model": {"height_m": 1.2}}),
    );
    assert!(ok, "{v}");

    let (ok, plan) = cli(&root, &["generate", "plan"]);
    assert!(ok, "{plan}");
    assert_eq!(plan["total_estimate_usd"], 0.0);

    let (ok, run) = cli(&root, &["generate", "run"]);
    assert!(ok, "{run}");
    assert_eq!(run["calls"], 2);
    let glb = root.join("assets/generated/barrel.glb");
    let first = std::fs::read(&glb).unwrap();

    // Rebuild offline from the cache after losing the outputs: identical bytes.
    std::fs::remove_file(&glb).unwrap();
    std::fs::remove_file(root.join("assets/generated/barrel_ref.png")).unwrap();
    let (ok, run) = cli(&root, &["generate", "run", "--offline"]);
    assert!(ok, "{run}");
    assert_eq!(run["calls"], 0);
    assert_eq!(std::fs::read(&glb).unwrap(), first);

    let (ok, list) = cli(&root, &["generate", "list"]);
    assert!(ok, "{list}");
    for r in list["recipes"].as_array().unwrap() {
        assert_eq!(r["state"], "built");
        assert_eq!(r["pool"], "blocked");
    }
}

#[test]
fn paid_generation_without_a_cap_is_refused_clearly() {
    let root = temp_dir("cap");
    let (ok, _) = cli(&root, &["provider", "use", "image", "qwen"]);
    assert!(ok);
    let (ok, v) = op(
        &root,
        "generate_recipe",
        json!({"name": "sky", "output": "texture", "prompt": "blue sky"}),
    );
    assert!(ok, "{v}");
    assert_eq!(v["result"]["plan"]["steps"][0]["status"], "blocked");
    let (ok, v) = cli(&root, &["generate", "run", "sky"]);
    assert!(!ok);
    assert_eq!(first_code(&v), "provider.spend_cap");
    let fix = v["diagnostics"][0]["fix"].as_str().unwrap();
    assert!(fix.contains("MARIONETTE_SPEND_CAP_USD"), "{fix}");
    assert!(!root.join(".marionette/spend.jsonl").exists());
    assert!(!root.join("assets/generated/sky.png").exists());
}

#[test]
fn only_a_person_approves_an_image() {
    let root = temp_dir("approve");
    let (ok, _) = op(
        &root,
        "generate_recipe",
        json!({"name": "ref", "output": "image", "prompt": "a lamp", "size": [64, 64]}),
    );
    assert!(ok);
    let (ok, _) = cli(&root, &["generate", "run"]);
    assert!(ok);
    // No terminal (stdin is not one): refused.
    let (ok, v) = cli(&root, &["generate", "approve", "ref"]);
    assert!(!ok);
    assert_eq!(first_code(&v), "generate.approval_needs_person");
    // No operation can write an approval file either.
    let (ok, v) = op(
        &root,
        "provider_generate",
        json!({"kind": "image", "prompt": "x", "width": 16, "height": 16, "out": "assets/generated/ref.approval.json"}),
    );
    assert!(!ok);
    assert_eq!(first_code(&v), "generate.approval_write_refused");
    assert!(!root.join("assets/generated/ref.approval.json").exists());
    // Recipes cannot be written outside the project.
    let (ok, v) = op(
        &root,
        "generate_recipe",
        json!({"name": "x", "output": "image", "prompt": "p", "dir": "../elsewhere"}),
    );
    assert!(!ok);
    assert_eq!(first_code(&v), "agent.path");
}
