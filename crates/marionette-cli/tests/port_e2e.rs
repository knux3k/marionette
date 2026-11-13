//! Porting operations through the CLI (decision 0020): scan, map lookup, scaffold (with the
//! no-overwrite rule and dry run), coordinate conversion, and read-only refusal.
#![cfg(feature = "port")]

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_marionette");

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-port-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn op_with(root: &Path, extra: &[&str], name: &str, input: &Value) -> (bool, Value) {
    let out = Command::new(BIN)
        .args(["--project", root.to_str().unwrap()])
        .args(extra)
        .args(["op", name, &input.to_string(), "--json"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{name}: bad JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)));
    (v["ok"] == json!(true), v)
}

fn op(root: &Path, name: &str, input: &Value) -> (bool, Value) {
    op_with(root, &[], name, input)
}

/// A fixture folder as a plain absolute path (no `canonicalize`: on Windows it returns a
/// `\\?\` verbatim path, which does not accept `/` separators).
fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("marionette-port")
        .join("tests")
        .join("fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

#[test]
fn scan_map_and_convert() {
    let root = temp_dir("scan");
    let (ok, v) = op(
        &root,
        "port_scan",
        &json!({ "source": fixture("unity"), "limit": 2 }),
    );
    assert!(ok, "{v}");
    let r = &v["result"];
    assert_eq!(r["summary"]["actions"], json!(["Horizontal", "Jump"]));
    assert_eq!(r["classes"].as_array().unwrap().len(), 2);
    assert_eq!(r["next_offset"], json!(2));
    assert!(
        r["api"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["name"] == "Mathf.Sin" && u["status"] == "hazard")
    );

    let (ok, v) = op(
        &root,
        "port_map",
        &json!({ "engine": "unreal", "section": "hooks", "query": "beginplay" }),
    );
    assert!(ok, "{v}");
    let names: Vec<&str> = v["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["BeginPlay", "ReceiveBeginPlay"]);

    let (ok, v) = op(
        &root,
        "port_convert",
        &json!({ "engine": "unreal", "kind": "position", "value": [100, 200, 300] }),
    );
    assert!(ok, "{v}");
    assert_eq!(v["result"]["value"], json!({ "x": 2.0, "y": 3.0, "z": -1.0 }));
    let (ok, v) = op(
        &root,
        "port_convert",
        &json!({ "engine": "unity", "kind": "quaternion", "value": [1, 2] }),
    );
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "port.bad_value", "{v}");
}

#[test]
fn scaffold_writes_skeletons_and_refuses_to_overwrite() {
    let root = temp_dir("scaffold");
    let input =
        json!({ "source": fixture("unreal"), "out": "scripts/ported", "scene": "scenes/main.scene.json" });
    let (ok, v) = op(
        &root,
        "port_scaffold",
        &json!({ "source": input["source"], "dry_run": true }),
    );
    assert!(ok, "{v}");
    assert!(!root.join("scripts").exists(), "dry run writes nothing");

    let (ok, v) = op(&root, "port_scaffold", &input);
    assert!(ok, "{v}");
    let files = v["result"]["files"].as_array().unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        [
            "scripts/ported/BP_Door.ts",
            "scripts/ported/ADemoCharacter.ts",
            "scripts/ported/PORT-NOTES.md"
        ]
    );
    assert_eq!(files[1]["script_path"], "../scripts/ported/ADemoCharacter.ts");
    let ts = std::fs::read_to_string(root.join("scripts/ported/ADemoCharacter.ts")).unwrap();
    assert!(ts.contains("export function update(self: Entity): void {"));
    let notes = std::fs::read_to_string(root.join("scripts/ported/PORT-NOTES.md")).unwrap();
    assert!(notes.contains("`MoveForward`") && notes.contains("FMath.Sin"));

    let (ok, v) = op(&root, "port_scaffold", &input);
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "port.exists", "{v}");
    let mut again = input.clone();
    again["overwrite"] = json!(true);
    assert!(op(&root, "port_scaffold", &again).0);

    let (ok, v) = op_with(&root, &["--read-only"], "port_scaffold", &again);
    assert!(!ok);
    assert_eq!(v["diagnostics"][0]["code"], "agent.read_only", "{v}");
    assert!(
        op_with(
            &root,
            &["--read-only"],
            "port_scan",
            &json!({ "source": input["source"] })
        )
        .0
    );
}

#[test]
fn serde_generation_writes_once_and_reports() {
    let root = temp_dir("serde");
    let src = fixture("serial");
    let (ok, v) = op(&root, "port_serde", &json!({ "source": src, "roots": ["Slot"] }));
    assert!(ok, "{v}");
    assert_eq!(v["result"]["written"], json!(false));
    assert!(v["result"]["rust"].as_str().unwrap().contains("pub struct Slot"));
    let (ok, v) = op(
        &root,
        "port_serde",
        &json!({ "source": src, "out": "src/serial.rs" }),
    );
    assert!(ok, "{v}");
    assert_eq!(v["result"]["written"], json!(true));
    assert!(root.join("src/serial.rs").exists());
    assert!(
        v["result"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["at"] == "Profile.lookup" && n["kind"] == "skipped")
    );
    let (ok, _) = op(
        &root,
        "port_serde",
        &json!({ "source": src, "out": "src/serial.rs" }),
    );
    assert!(!ok, "second write must refuse to overwrite");
}
