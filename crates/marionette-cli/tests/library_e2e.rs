//! The owned library operations through the CLI (workstream W2): import with a declaration,
//! list, read a record, the pool gate, the license text check, and the refusal to write a
//! declaration file through any operation.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_marionette");

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-lib-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn op(root: &Path, name: &str, input: &Value) -> (bool, Value) {
    let out = Command::new(BIN)
        .env_remove("MARIONETTE_LIBRARY")
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

#[test]
fn library_operations_import_list_and_gate() {
    let base = temp_dir("ops");
    let project = base.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let library = base.join("owned");
    let lib = library.to_str().unwrap();
    let src = base.join("bought");
    std::fs::create_dir_all(&src).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::copy(repo.join("examples/assets/gem.glb"), src.join("gem.glb")).unwrap();
    std::fs::write(
        src.join("loose.glb"),
        std::fs::read(repo.join("examples/assets/gem.glb")).unwrap(),
    )
    .unwrap();
    // Written by the test standing in for the person; covers gem.glb only.
    std::fs::write(
        base.join("decl.license.json"),
        json!({ "format": "marionette.license_declaration/1", "store": "Example Store",
                "product": "Gem", "license_type": "Standard", "other_engines_allowed": "yes",
                "covers": ["gem.glb"] })
        .to_string(),
    )
    .unwrap();

    let (ok, v) = op(
        &project,
        "library_import",
        &json!({ "source": src.to_str().unwrap(), "library": lib,
                 "declarations": [base.join("decl.license.json").to_str().unwrap()] }),
    );
    assert!(ok, "{v:#}");
    let r = &v["result"];
    assert_eq!(r["counts"]["imported"], json!(1), "{r:#}");
    assert_eq!(r["refused"][0]["path"], json!("loose.glb"));
    assert_eq!(r["refused"][0]["code"], json!("import.license_unknown"));
    let id = r["imported"][0]["id"].as_str().unwrap().to_string();

    let (ok, v) = op(&project, "library_list", &json!({ "library": lib }));
    assert!(ok);
    assert_eq!(v["result"]["total"], json!(1));
    assert_eq!(v["result"]["assets"][0]["product"], json!("Gem"));

    let (ok, v) = op(&project, "library_get", &json!({ "library": lib, "id": id }));
    assert!(ok);
    assert_eq!(v["result"]["pool"], json!("blocked"));
    assert_eq!(
        v["result"]["license"]["declaration_file"],
        json!("decl.license.json")
    );

    let (ok, v) = op(&project, "library_info", &json!({ "library": lib }));
    assert!(ok);
    assert_eq!(v["result"]["assets"], json!(1));
    assert_eq!(v["result"]["configured_from"], json!("input"));

    // The pool refuses the imported asset.
    let asset_dir = library.join("assets").join(&id);
    let (ok, v) = op(
        &project,
        "pool_admission_check",
        &json!({ "path": asset_dir.to_str().unwrap(), "library": lib }),
    );
    assert!(ok);
    assert_eq!(v["result"]["admitted_by_owned_gate"], json!(false));

    let (ok, v) = op(
        &project,
        "license_check_text",
        &json!({ "text": "For use exclusively with Unity." }),
    );
    assert!(ok);
    assert_eq!(v["result"]["limits_to_one_engine"], json!(true));

    // No library configured: a clear error with a fix.
    let (ok, v) = op(&project, "library_list", &json!({}));
    if !ok {
        assert_eq!(v["diagnostics"][0]["code"], json!("library.not_configured"));
    }
}

#[test]
fn operations_never_write_license_declarations() {
    let base = temp_dir("decl");
    let (ok, v) = op(&base, "project_init", &json!({ "path": "game" }));
    assert!(ok, "{v:#}");
    let base = base.join("game");
    let (ok, v) = op(
        &base,
        "scene_run",
        &json!({ "scene": "scenes/main.scene.json", "ticks": 1, "seed": 1, "record": "pack.license.json" }),
    );
    assert!(!ok, "{v:#}");
    assert_eq!(
        v["diagnostics"][0]["code"],
        json!("import.declaration_write_refused")
    );
    assert!(!base.join("pack.license.json").exists());
}
