//! Unity prefab import and placement through the CLI (decision 0040): import a Unity project
//! holding a model and a prefab, list the prefab, place it into a scene twice, run the scene.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_marionette");
const GEM: &str = "9e909e909e909e909e909e909e909e90";
const SHIP: &str = "5b195b195b195b195b195b195b195b19";

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-prefab-e2e-{tag}-{}", std::process::id()));
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

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

const SHIP_PREFAB: &str = "%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!1 &1
GameObject:
  m_Component:
  - component: {fileID: 4}
  - component: {fileID: 65}
  - component: {fileID: 54}
  m_Name: Ship
  m_IsActive: 1
--- !u!4 &4
Transform:
  m_GameObject: {fileID: 1}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalPosition: {x: 0, y: 0, z: 0}
  m_LocalScale: {x: 1, y: 1, z: 1}
  m_Children:
  - {fileID: 40}
  - {fileID: 41}
  m_Father: {fileID: 0}
--- !u!65 &65
BoxCollider:
  m_GameObject: {fileID: 1}
  m_Enabled: 1
  m_IsTrigger: 0
  m_Size: {x: 1, y: 1, z: 1}
  m_Center: {x: 0, y: 0, z: 0}
--- !u!54 &54
Rigidbody:
  m_GameObject: {fileID: 1}
  m_Mass: 1
  m_UseGravity: 1
  m_IsKinematic: 0
--- !u!1001 &100
PrefabInstance:
  m_Modification:
    m_TransformParent: {fileID: 4}
    m_Modifications:
    - target: {fileID: 400000, guid: GEM, type: 3}
      propertyPath: m_LocalPosition.y
      value: 1
      objectReference: {fileID: 0}
  m_SourcePrefab: {fileID: 100100000, guid: GEM, type: 3}
--- !u!4 &40 stripped
Transform:
  m_CorrespondingSourceObject: {fileID: 400000, guid: GEM, type: 3}
  m_PrefabInstance: {fileID: 100}
--- !u!1 &2
GameObject:
  m_Component:
  - component: {fileID: 41}
  m_Name: Hidden
  m_IsActive: 0
--- !u!4 &41
Transform:
  m_GameObject: {fileID: 2}
  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}
  m_LocalPosition: {x: 0, y: 0, z: 0}
  m_LocalScale: {x: 1, y: 1, z: 1}
  m_Children: []
  m_Father: {fileID: 4}
";

#[test]
fn import_list_and_place_a_unity_prefab() {
    let base = temp_dir("place");
    let project = base.join("game");
    std::fs::create_dir_all(&project).unwrap();
    let lib = base.join("owned");
    let lib = lib.to_str().unwrap();
    let unity = base.join("ShipPack");
    std::fs::create_dir_all(unity.join("ProjectSettings")).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    write(
        &unity.join("Assets/Gem.glb"),
        &std::fs::read(repo.join("examples/assets/gem.glb")).unwrap(),
    );
    write(&unity.join("Assets/Gem.glb.meta"), format!("guid: {GEM}\n").as_bytes());
    write(
        &unity.join("Assets/Ship.prefab"),
        SHIP_PREFAB.replace("GEM", GEM).as_bytes(),
    );
    write(&unity.join("Assets/Ship.prefab.meta"), format!("guid: {SHIP}\n").as_bytes());
    // Written by the test standing in for the person who bought the pack.
    write(
        &base.join("ShipPack.license.json"),
        json!({ "format": "marionette.license_declaration/1", "store": "Example Store",
                "product": "Ship Pack", "license_type": "Standard", "other_engines_allowed": "yes" })
        .to_string()
        .as_bytes(),
    );

    let (ok, v) = op(
        &project,
        "library_import",
        &json!({ "source": unity.to_str().unwrap(), "library": lib }),
    );
    assert!(ok, "{v:#}");
    assert_eq!(v["result"]["counts"]["imported"], 2, "{v:#}");

    let (ok, v) = op(&project, "library_list", &json!({ "kind": "prefab", "library": lib }));
    assert!(ok, "{v:#}");
    let id = v["result"]["assets"][0]["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("ship-"));

    let (ok, v) = op(&project, "scene_create", &json!({ "scene": "level.scene.json", "template": "basic" }));
    assert!(ok, "{v:#}");

    let (ok, v) = op(
        &project,
        "prefab_instantiate",
        &json!({ "prefab": id, "scene": "level.scene.json", "library": lib,
                 "at": { "translation": { "x": 0, "y": 3, "z": 0 } } }),
    );
    assert!(ok, "{v:#}");
    let r = &v["result"];
    // Ship and the gem model; the inactive child is left out.
    assert_eq!(r["entities"].as_array().unwrap().len(), 2, "{r:#}");
    assert_eq!(r["inactive_left_out"], 1);
    let copied: Vec<&str> = r["copied"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
    assert_eq!(copied.len(), 1, "{copied:?}");
    assert!(copied[0].starts_with("library/gem-") && copied[0].ends_with("/model.glb"));
    assert!(project.join(copied[0]).is_file());

    // Placed again further along, the files are already there.
    let (ok, v) = op(
        &project,
        "prefab_instantiate",
        &json!({ "prefab": id, "scene": "level.scene.json", "library": lib, "name": "Ship 2",
                 "at": { "translation": { "x": 4, "y": 3, "z": 0 } } }),
    );
    assert!(ok, "{v:#}");
    assert!(v["result"]["copied"].as_array().unwrap().is_empty());
    assert_eq!(v["result"]["entities"][1]["name"], "Ship 2/Gem");

    // The scene holds the entities with their placement, and it runs.
    let scene: Value =
        serde_json::from_str(&std::fs::read_to_string(project.join("level.scene.json")).unwrap()).unwrap();
    let gem2 = scene["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "Ship 2/Gem")
        .unwrap();
    let t = &gem2["components"]["Transform"]["translation"];
    assert_eq!((t["x"].as_f64(), t["y"].as_f64()), (Some(4.0), Some(4.0)));
    assert!(gem2["components"]["Model"]["asset"].as_str().unwrap().starts_with("library/gem-"));
    let (ok, v) = op(&project, "scene_run", &json!({ "scene": "level.scene.json", "ticks": 60, "seed": 1 }));
    assert!(ok, "{v:#}");

    // Only prefabs can be placed.
    let (ok, v) = op(&project, "library_list", &json!({ "kind": "model", "library": lib }));
    assert!(ok);
    let model = v["result"]["assets"][0]["id"].clone();
    let (ok, v) = op(
        &project,
        "prefab_instantiate",
        &json!({ "prefab": model, "scene": "level.scene.json", "library": lib }),
    );
    assert!(!ok, "{v:#}");
}
