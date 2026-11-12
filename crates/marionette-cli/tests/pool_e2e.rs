//! The CC0 pool through the CLI (workstream W3, decision 0041): info, list, search in the
//! required order, fetch into a project, add a made asset, refuse an owned one, write the index.
//! No network: online steps are not requested.
#![cfg(feature = "pool")]

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_marionette");

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-pool-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn op(root: &Path, name: &str, input: &Value) -> (bool, Value) {
    let out = Command::new(BIN)
        .env_remove("MARIONETTE_LIBRARY")
        .env_remove("MARIONETTE_POOL")
        .env("XDG_CONFIG_HOME", root.join(".no-config"))
        .env("APPDATA", root.join(".no-config"))
        .args([
            "--project",
            root.to_str().unwrap(),
            "op",
            name,
            &input.to_string(),
            "--json",
        ])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{name}: bad JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)));
    (v["ok"] == json!(true), v)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

#[test]
fn pool_operations_search_fetch_add_and_refuse() {
    let base = temp_dir("ops");
    let project = base.join("game");
    std::fs::create_dir_all(&project).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pool_dir = base.join("pool");
    copy_dir(&repo.join("pool"), &pool_dir);
    let pool = pool_dir.to_str().unwrap();

    let (ok, v) = op(&project, "pool_info", &json!({}));
    assert!(ok, "{v:#}");
    assert!(v["result"]["problem"].as_str().unwrap().contains("no CC0 pool"));
    assert_eq!(v["result"]["libraries"].as_array().unwrap().len(), 4);
    let (ok, v) = op(&project, "pool_info", &json!({ "pool": pool }));
    assert!(ok);
    assert_eq!(v["result"]["assets"], 5);

    let (ok, v) = op(&project, "pool_list", &json!({ "pool": pool, "kind": "texture" }));
    assert!(ok);
    assert_eq!(v["result"]["total"], 3);

    // Search: nothing in the project, so the pool answers.
    let (ok, v) = op(
        &project,
        "asset_search",
        &json!({ "query": "crate", "pool": pool }),
    );
    assert!(ok, "{v:#}");
    assert_eq!(v["result"]["found_in"], "cc0_pool");
    assert_eq!(v["result"]["steps"][2]["results"][0]["id"], "wooden-crate");

    // Fetch it into the project; then the project itself answers first.
    let (ok, v) = op(
        &project,
        "pool_fetch",
        &json!({ "id": "wooden-crate", "pool": pool }),
    );
    assert!(ok, "{v:#}");
    assert_eq!(v["result"]["main"], "assets/pool/wooden-crate/model.glb");
    assert!(project.join("assets/pool/wooden-crate/model.glb").is_file());
    let (ok, v) = op(
        &project,
        "asset_search",
        &json!({ "query": "crate", "pool": pool }),
    );
    assert!(ok);
    assert_eq!(v["result"]["found_in"], "project_files");

    // A made asset from a folder.
    let made = base.join("made");
    std::fs::create_dir_all(&made).unwrap();
    // A 1x1 green PNG.
    const LEAF: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
        31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 224, 58, 33, 247, 31, 0, 3, 192, 1, 240,
        207, 39, 181, 168, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    std::fs::write(made.join("leaf.png"), LEAF).unwrap();
    let (ok, v) = op(
        &project,
        "pool_add",
        &json!({ "pool": pool, "path": made.to_str().unwrap(), "kind": "texture", "name": "Leaf green",
                 "tags": ["leaf", "green"], "origin": "made", "method": "flat color drawn in a test" }),
    );
    assert!(ok, "{v:#}");
    assert_eq!(v["result"]["status"], "added");
    let id = v["result"]["id"].as_str().unwrap().to_string();
    let (ok, v) = op(&project, "pool_get", &json!({ "pool": pool, "id": id }));
    assert!(ok);
    assert_eq!(v["result"]["license"]["spdx"], "CC0-1.0");

    // An asset carrying an owned-asset declaration never enters.
    std::fs::write(made.join("leaf.license.json"), "{}").unwrap();
    let (ok, v) = op(
        &project,
        "pool_add",
        &json!({ "pool": pool, "path": made.to_str().unwrap(), "kind": "texture", "name": "Leaf again",
                 "origin": "made", "method": "copy" }),
    );
    assert!(!ok, "{v:#}");
    let code = v["diagnostics"][0]["code"].as_str().unwrap();
    assert!(code.starts_with("pool."), "{code}");

    // Online origin without a known library is refused.
    std::fs::remove_file(made.join("leaf.license.json")).unwrap();
    let (ok, v) = op(
        &project,
        "pool_add",
        &json!({ "pool": pool, "path": made.to_str().unwrap(), "kind": "texture", "name": "Leaf web",
                 "origin": "online", "library": "somewhere", "page": "https://example.com/leaf" }),
    );
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "pool.unknown_source");

    // A copy of a generated asset (bytes match a provenance record in the project's
    // assets/generated/) is refused even without its record (decision 0045).
    let generated = project.join("assets/generated");
    std::fs::create_dir_all(&generated).unwrap();
    std::fs::write(generated.join("moss.png"), LEAF).unwrap();
    // SHA-256 of LEAF.
    let sha = "ac5d59c07e56ce5c075672d90e4c439fba9a4f8f200f9d1a8a43ccbde81ed9c5";
    let record = json!({
        "format": "marionette.generated_asset/1", "name": "moss", "output": "texture",
        "file": "moss.png", "sha256": sha, "recipe": "moss.recipe.json", "recipe_sha256": "0",
        "source": { "provider": "mock", "model": "mock", "provider_verified": false, "kind": "image",
                    "cache_key": "0", "raw_sha256": sha, "cost_usd": 0.0 },
        "postprocess": {}, "license": { "status": "generated", "terms": "test" },
        "pool": "blocked", "pool_reason": "test"
    });
    std::fs::write(generated.join("moss.provenance.json"), record.to_string()).unwrap();
    let (ok, v) = op(
        &project,
        "pool_add",
        &json!({ "pool": pool, "path": made.to_str().unwrap(), "kind": "texture", "name": "Leaf copy",
                 "origin": "made", "method": "copy" }),
    );
    assert!(!ok, "{v:#}");
    assert_eq!(v["diagnostics"][0]["code"], "pool.generated");
    std::fs::remove_dir_all(&generated).unwrap();

    let (ok, v) = op(&project, "pool_index", &json!({ "pool": pool }));
    assert!(ok, "{v:#}");
    assert_eq!(v["result"]["assets"], 6);
}
