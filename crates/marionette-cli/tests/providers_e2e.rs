//! Providers through the CLI (workstream W5, plan line 7 proof): with no provider configured
//! and no network everything runs on the mock; one command swaps a provider; swapping back
//! gives the same output again; keys and the spend cap never come from files an operation
//! writes.
#![cfg(feature = "generate")]

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_marionette");

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-prov-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Runs the CLI with no provider keys, no spend cap and an empty user config folder.
fn cli(root: &Path, args: &[&str]) -> (bool, Value) {
    let config_home = root.join("user-config");
    let out = Command::new(BIN)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("DASHSCOPE_API_KEY")
        .env_remove("TRIPO_API_KEY")
        .env_remove("MARIONETTE_SPEND_CAP_USD")
        .env("XDG_CONFIG_HOME", &config_home)
        .env("APPDATA", &config_home)
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
    (v["ok"] == json!(true), v)
}

fn op(root: &Path, name: &str, input: Value) -> (bool, Value) {
    cli(root, &["op", name, &input.to_string()])
}

#[test]
fn one_command_swaps_a_provider_and_mock_needs_nothing() {
    let root = temp_dir("swap");

    // Nothing configured: every kind is the mock and ready; no spending, zero cap.
    let (ok, status) = cli(&root, &["provider", "status"]);
    assert!(ok, "{status}");
    for k in status["kinds"].as_array().unwrap() {
        assert_eq!(k["provider"], "mock");
        assert_eq!(k["readiness"], "ready");
    }
    assert_eq!(status["config_file"], false);
    assert_eq!(status["spent_usd"], 0.0);
    assert_eq!(status["cap"], json!({ "usd": 0.0, "source": "default" }));

    let image = json!({ "kind": "image", "prompt": "mossy stone", "width": 64, "height": 64, "out": "gen/stone.png" });
    let (ok, first) = op(&root, "provider_generate", image.clone());
    assert!(ok, "{first}");
    assert_eq!(first["result"]["provider"], "mock");
    let first_hash = first["result"]["sha256"].clone();
    assert!(root.join("gen/stone.png").exists());

    // One command swaps the image provider to Qwen.
    let (ok, swapped) = cli(&root, &["provider", "use", "image", "qwen"]);
    assert!(ok, "{swapped}");
    assert_eq!(swapped["provider"], "qwen");
    assert_eq!(swapped["model"], "qwen-image");
    assert_eq!(swapped["previous"]["provider"], "mock");
    assert_eq!(swapped["readiness"], "key_missing");
    let text = std::fs::read_to_string(root.join("providers.json")).unwrap();
    assert_eq!(
        text,
        "{\n  \"format\": \"marionette.providers/1\",\n  \"image\": {\n    \"provider\": \"qwen\"\n  }\n}\n"
    );

    // Generation now goes to Qwen and stops at the missing key, before any network or money.
    let (ok, refused) = op(&root, "provider_generate", image.clone());
    assert!(!ok);
    assert_eq!(refused["diagnostics"][0]["code"], "provider.key_missing");
    // Other kinds are untouched.
    let (ok, text_reply) = op(
        &root,
        "provider_generate",
        json!({ "kind": "language", "prompt": "hello" }),
    );
    assert!(ok, "{text_reply}");
    assert_eq!(text_reply["result"]["provider"], "mock");

    // Swap back: the same request gives the same bytes as before.
    let (ok, _) = cli(&root, &["provider", "use", "image", "mock"]);
    assert!(ok);
    let (ok, again) = op(&root, "provider_generate", image);
    assert!(ok, "{again}");
    assert_eq!(again["result"]["sha256"], first_hash);

    // A model name is part of the choice and changes mock output.
    let (ok, _) = cli(
        &root,
        &["provider", "use", "model3d", "mock", "--model", "blocky"],
    );
    assert!(ok);
    let (ok, glb) = op(
        &root,
        "provider_generate",
        json!({ "kind": "model3d", "prompt": "crate", "out": "gen/crate.glb" }),
    );
    assert!(ok, "{glb}");
    assert_eq!(glb["result"]["model"], "blocky");
    assert!(
        std::fs::read(root.join("gen/crate.glb"))
            .unwrap()
            .starts_with(b"glTF")
    );

    // No spending happened, so no ledger exists.
    assert!(!root.join(".marionette/spend.jsonl").exists());
}

#[test]
fn bad_switches_are_refused_and_leave_the_file_alone() {
    let root = temp_dir("refuse");
    let (ok, _) = cli(&root, &["provider", "use", "language", "claude"]);
    assert!(ok);
    let before = std::fs::read(root.join("providers.json")).unwrap();
    for (args, code) in [
        (
            vec!["provider", "use", "language", "tripo"],
            "provider.kind_unsupported",
        ),
        (vec!["provider", "use", "image", "nobody"], "provider.unknown"),
        (vec!["provider", "use", "sound", "mock"], "agent.bad_input"),
    ] {
        let (ok, v) = cli(&root, &args);
        assert!(!ok, "{args:?}");
        assert_eq!(v["diagnostics"][0]["code"], code, "{args:?}");
    }
    assert_eq!(std::fs::read(root.join("providers.json")).unwrap(), before);

    // Read-only mode refuses the switch.
    let (ok, v) = cli(&root, &["--read-only", "provider", "use", "language", "mock"]);
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "agent.read_only");

    // A key written into the file by hand makes the config invalid instead of being used.
    std::fs::write(
        root.join("providers.json"),
        r#"{"format":"marionette.providers/1","language":{"provider":"claude","api_key":"x"}}"#,
    )
    .unwrap();
    let (ok, v) = cli(&root, &["provider", "status"]);
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "provider.bad_config");

    // Image and model3d need an output file.
    std::fs::remove_file(root.join("providers.json")).unwrap();
    let (ok, v) = op(
        &root,
        "provider_generate",
        json!({ "kind": "image", "prompt": "x" }),
    );
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "provider.bad_request");
}

#[test]
fn provider_list_names_every_plugin_without_showing_keys() {
    let root = temp_dir("list");
    let (ok, v) = cli(&root, &["provider", "list"]);
    assert!(ok, "{v}");
    let names: Vec<&str> = v["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["mock", "claude", "qwen", "tripo"]);
    for p in v["providers"].as_array().unwrap() {
        assert_eq!(p["key_present"], false);
        if p["name"] != "mock" {
            assert_eq!(p["readiness"], "key_missing");
            assert_eq!(p["verified"], false);
        }
    }
}
