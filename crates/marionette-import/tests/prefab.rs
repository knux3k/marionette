//! Unity prefab import (decision 0040). Every fixture is written here as Unity's text
//! serialization; no third-party files are committed. The declaration stands in for the person
//! who bought the pack.

use marionette_import::library::{AssetKind, ConfiguredFrom, Library};
use marionette_import::{ImportOptions, ImportReport, codes, import};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const CRATE: &str = "c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
const CART: &str = "ca00ca00ca00ca00ca00ca00ca00ca00";
const VARIANT: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";
const LOOP: &str = "100f100f100f100f100f100f100f100f";
const WHEEL: &str = "0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e";
const MAT: &str = "3a73a73a73a73a73a73a73a73a73a73a";
const SCRIPT: &str = "5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c";
const HEADER: &str = "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n";

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-prefab-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn asset(project: &Path, rel: &str, guid: &str, bytes: &[u8]) {
    write(&project.join(rel), bytes);
    write(
        &project.join(format!("{rel}.meta")),
        format!("fileFormatVersion: 2\nguid: {guid}\n").as_bytes(),
    );
}

fn glb() -> Vec<u8> {
    let pos: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let bin: Vec<u8> = pos.iter().flat_map(|f| f.to_le_bytes()).collect();
    let doc = json!({
        "asset": { "version": "2.0" },
        "scene": 0, "scenes": [{ "nodes": [0] }], "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 } }] }],
        "buffers": [{ "byteLength": 36 }],
        "bufferViews": [{ "buffer": 0, "byteLength": 36 }],
        "accessors": [{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                        "min": [0, 0, 0], "max": [1, 1, 0] }]
    });
    let mut json = serde_json::to_vec(&doc).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

fn crate_prefab() -> String {
    format!(
        "{HEADER}--- !u!1 &1000
GameObject:
  m_ObjectHideFlags: 0
  serializedVersion: 6
  m_Component:
  - component: {{fileID: 4000}}
  - component: {{fileID: 3300}}
  - component: {{fileID: 2300}}
  - component: {{fileID: 6500}}
  - component: {{fileID: 5400}}
  m_Layer: 0
  m_Name: Crate
  m_IsActive: 1
--- !u!4 &4000
Transform:
  m_ObjectHideFlags: 0
  m_GameObject: {{fileID: 1000}}
  m_LocalRotation: {{x: 0, y: 0, z: 0, w: 1}}
  m_LocalPosition: {{x: 5, y: 5, z: 5}}
  m_LocalScale: {{x: 1, y: 1, z: 1}}
  m_Children:
  - {{fileID: 4001}}
  m_Father: {{fileID: 0}}
  m_RootOrder: 0
--- !u!33 &3300
MeshFilter:
  m_GameObject: {{fileID: 1000}}
  m_Mesh: {{fileID: 10202, guid: 0000000000000000e000000000000000, type: 0}}
--- !u!23 &2300
MeshRenderer:
  m_GameObject: {{fileID: 1000}}
  m_Enabled: 1
  m_Materials:
  - {{fileID: 2100000, guid: {MAT}, type: 2}}
--- !u!65 &6500
BoxCollider:
  m_GameObject: {{fileID: 1000}}
  m_Material: {{fileID: 0}}
  m_IsTrigger: 0
  m_Enabled: 1
  serializedVersion: 2
  m_Size: {{x: 1, y: 1, z: 1}}
  m_Center: {{x: 0, y: 0, z: 0}}
--- !u!54 &5400
Rigidbody:
  m_GameObject: {{fileID: 1000}}
  serializedVersion: 2
  m_Mass: 8
  m_Drag: 0
  m_AngularDrag: 0.05
  m_UseGravity: 1
  m_IsKinematic: 0
  m_Constraints: 0
--- !u!1 &1001
GameObject:
  m_Component:
  - component: {{fileID: 4001}}
  - component: {{fileID: 11400}}
  - component: {{fileID: 10800}}
  - component: {{fileID: 13500}}
  m_Name: Lamp
  m_IsActive: 1
--- !u!4 &4001
Transform:
  m_GameObject: {{fileID: 1001}}
  m_LocalRotation: {{x: 0.70710677, y: 0, z: 0, w: 0.70710677}}
  m_LocalPosition: {{x: 0, y: 1, z: 2}}
  m_LocalScale: {{x: 1, y: 1, z: 1}}
  m_Children: []
  m_Father: {{fileID: 4000}}
--- !u!114 &11400
MonoBehaviour:
  m_ObjectHideFlags: 0
  m_GameObject: {{fileID: 1001}}
  m_Enabled: 1
  m_EditorHideFlags: 0
  m_Script: {{fileID: 11500000, guid: {SCRIPT}, type: 3}}
  m_Name:
  m_EditorClassIdentifier:
  speed: 2.5
  label: spinning lamp
--- !u!108 &10800
Light:
  m_GameObject: {{fileID: 1001}}
  m_Enabled: 1
  serializedVersion: 10
  m_Type: 1
  m_Color: {{r: 1, g: 1, b: 1, a: 1}}
  m_Intensity: 2
--- !u!135 &13500
SphereCollider:
  m_GameObject: {{fileID: 1001}}
  m_Enabled: 1
  m_Radius: 0.25
  m_Center: {{x: 0, y: 0, z: 0}}
"
    )
}

fn cart_prefab() -> String {
    format!(
        "{HEADER}--- !u!1 &10
GameObject:
  m_Component:
  - component: {{fileID: 40}}
  m_Name: Cart
  m_IsActive: 1
--- !u!4 &40
Transform:
  m_GameObject: {{fileID: 10}}
  m_LocalRotation: {{x: 0, y: 0, z: 0, w: 1}}
  m_LocalPosition: {{x: 0, y: 0, z: 0}}
  m_LocalScale: {{x: 2, y: 2, z: 2}}
  m_Children:
  - {{fileID: 41}}
  - {{fileID: 42}}
  m_Father: {{fileID: 0}}
--- !u!1001 &500
PrefabInstance:
  m_ObjectHideFlags: 0
  serializedVersion: 2
  m_Modification:
    m_TransformParent: {{fileID: 40}}
    m_Modifications:
    - target: {{fileID: 4000, guid: {CRATE}, type: 3}}
      propertyPath: m_LocalPosition.x
      value: 1
      objectReference: {{fileID: 0}}
    - target: {{fileID: 4000, guid: {CRATE}, type: 3}}
      propertyPath: m_LocalPosition.y
      value: 0
      objectReference: {{fileID: 0}}
    - target: {{fileID: 4000, guid: {CRATE}, type: 3}}
      propertyPath: m_LocalPosition.z
      value: 3
      objectReference: {{fileID: 0}}
    - target: {{fileID: 1000, guid: {CRATE}, type: 3}}
      propertyPath: m_Name
      value: Cargo
      objectReference: {{fileID: 0}}
    - target: {{fileID: 6500, guid: {CRATE},
        type: 3}}
      propertyPath: m_Size.y
      value: 2
      objectReference: {{fileID: 0}}
    m_RemovedComponents: []
  m_SourcePrefab: {{fileID: 100100000, guid: {CRATE}, type: 3}}
--- !u!4 &41 stripped
Transform:
  m_CorrespondingSourceObject: {{fileID: 4000, guid: {CRATE}, type: 3}}
  m_PrefabInstance: {{fileID: 500}}
  m_PrefabAsset: {{fileID: 0}}
--- !u!1 &11 stripped
GameObject:
  m_CorrespondingSourceObject: {{fileID: 1000, guid: {CRATE}, type: 3}}
  m_PrefabInstance: {{fileID: 500}}
--- !u!81 &81
AudioListener:
  m_GameObject: {{fileID: 11}}
  m_Enabled: 1
--- !u!1001 &600
PrefabInstance:
  m_Modification:
    m_TransformParent: {{fileID: 40}}
    m_Modifications:
    - target: {{fileID: 919132149155446097, guid: {WHEEL}, type: 3}}
      propertyPath: m_LocalPosition.x
      value: -1
      objectReference: {{fileID: 0}}
    - target: {{fileID: 919132149155446097, guid: {WHEEL}, type: 3}}
      propertyPath: m_Name
      value: Front Wheel
      objectReference: {{fileID: 0}}
    - target: {{fileID: -8679921383154817045, guid: {WHEEL}, type: 3}}
      propertyPath: m_LocalPosition.y
      value: 0.5
      objectReference: {{fileID: 0}}
    m_RemovedComponents: []
  m_SourcePrefab: {{fileID: 100100000, guid: {WHEEL}, type: 3}}
--- !u!4 &42 stripped
Transform:
  m_CorrespondingSourceObject: {{fileID: 919132149155446097, guid: {WHEEL}, type: 3}}
  m_PrefabInstance: {{fileID: 600}}
"
    )
}

fn variant_prefab() -> String {
    format!(
        "{HEADER}--- !u!1001 &700
PrefabInstance:
  m_Modification:
    m_TransformParent: {{fileID: 0}}
    m_Modifications:
    - target: {{fileID: 1000, guid: {CRATE}, type: 3}}
      propertyPath: m_Name
      value: Light Crate
      objectReference: {{fileID: 0}}
    - target: {{fileID: 2300, guid: {CRATE}, type: 3}}
      propertyPath: m_Materials.Array.data[0]
      value:
      objectReference: {{fileID: 10303, guid: 0000000000000000f000000000000000, type: 0}}
    m_RemovedComponents:
    - {{fileID: 5400, guid: {CRATE}, type: 3}}
  m_SourcePrefab: {{fileID: 100100000, guid: {CRATE}, type: 3}}
"
    )
}

fn loop_prefab() -> String {
    format!(
        "{HEADER}--- !u!1 &1
GameObject:
  m_Component:
  - component: {{fileID: 2}}
  m_Name: Loop
  m_IsActive: 1
--- !u!4 &2
Transform:
  m_GameObject: {{fileID: 1}}
  m_LocalRotation: {{x: 0, y: 0, z: 0, w: 1}}
  m_LocalPosition: {{x: 0, y: 0, z: 0}}
  m_LocalScale: {{x: 1, y: 1, z: 1}}
  m_Children: []
  m_Father: {{fileID: 0}}
--- !u!1001 &3
PrefabInstance:
  m_Modification:
    m_TransformParent: {{fileID: 2}}
    m_Modifications: []
  m_SourcePrefab: {{fileID: 100100000, guid: {LOOP}, type: 3}}
"
    )
}

/// A Unity project with a model, a material, a script and four prefabs, plus a declaration.
fn project(base: &Path) -> (PathBuf, PathBuf) {
    let p = base.join("Pack");
    std::fs::create_dir_all(p.join("ProjectSettings")).unwrap();
    asset(&p, "Assets/Models/Wheel.glb", WHEEL, &glb());
    let mat = format!(
        "{HEADER}--- !u!21 &2100000\nMaterial:\n  m_Name: Red\n  \
         m_Shader: {{fileID: 0, guid: 933532a4fcc9baf4fa0491de14d08ed7, type: 3}}\n  m_SavedProperties:\n    \
         m_TexEnvs: []\n    m_Floats:\n    - _Smoothness: 0.25\n    - _Metallic: 0\n    m_Colors:\n    \
         - _BaseColor: {{r: 1, g: 0, b: 0, a: 1}}\n"
    );
    asset(&p, "Assets/Materials/Red.mat", MAT, mat.as_bytes());
    asset(
        &p,
        "Assets/Scripts/Spin.cs",
        SCRIPT,
        b"public class Spin : MonoBehaviour { public float speed; }",
    );
    asset(
        &p,
        "Assets/Prefabs/Crate.prefab",
        CRATE,
        crate_prefab().as_bytes(),
    );
    asset(&p, "Assets/Prefabs/Cart.prefab", CART, cart_prefab().as_bytes());
    asset(
        &p,
        "Assets/Prefabs/Crate Variant.prefab",
        VARIANT,
        variant_prefab().as_bytes(),
    );
    asset(&p, "Assets/Prefabs/Loop.prefab", LOOP, loop_prefab().as_bytes());
    asset(
        &p,
        "Assets/Prefabs/Binary.prefab",
        "b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1",
        b"\x00\x00\x00\x01binary",
    );
    let decl = base.join("pack.license.json");
    write(
        &decl,
        &serde_json::to_vec_pretty(&json!({
            "format": "marionette.license_declaration/1",
            "store": "Example Store",
            "product": "Example Prefab Pack",
            "license_type": "Standard single-user license",
            "other_engines_allowed": "yes",
            "covers": ["Assets/"],
        }))
        .unwrap(),
    );
    (p, decl)
}

fn run(lib: &Library, source: &Path, decl: &Path, dry_run: bool) -> ImportReport {
    let mut o = ImportOptions::new(source);
    o.declarations = vec![decl.to_path_buf()];
    o.dry_run = dry_run;
    import(lib, &o).unwrap()
}

fn prefab_doc(lib: &Library, report: &ImportReport, rel: &str) -> Value {
    let a = report
        .imported
        .iter()
        .find(|a| a.path == rel)
        .unwrap_or_else(|| panic!("{rel} not imported: {report:#?}"));
    assert_eq!(a.kind, AssetKind::Prefab);
    let bytes = std::fs::read(lib.asset_dir(&a.id).join("prefab.json")).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn entity<'a>(doc: &'a Value, name: &str) -> &'a Value {
    doc["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == name)
        .unwrap_or_else(|| panic!("no entity `{name}` in {doc:#}"))
}

fn close(v: &Value, want: f64) -> bool {
    (v.as_f64().unwrap() - want).abs() < 1e-5
}

fn translation(e: &Value) -> [f64; 3] {
    let t = &e["components"]["Transform"]["translation"];
    [
        t["x"].as_f64().unwrap(),
        t["y"].as_f64().unwrap(),
        t["z"].as_f64().unwrap(),
    ]
}

fn setup(tag: &str) -> (PathBuf, Library, PathBuf, PathBuf) {
    let base = temp(tag);
    let lib = Library::at(&base.join("owned"), &base.join("game"), ConfiguredFrom::Input).unwrap();
    let (p, decl) = project(&base);
    (base, lib, p, decl)
}

#[test]
fn a_prefab_becomes_entities_with_mapped_components() {
    let (_base, lib, p, decl) = setup("basic");
    let r = run(&lib, &p, &decl, false);
    let doc = prefab_doc(&lib, &r, "Assets/Prefabs/Crate.prefab");
    assert_eq!(doc["format"], "marionette.prefab/1");
    assert_eq!(doc["name"], "Crate");

    let root = entity(&doc, "Crate");
    let c = &root["components"];
    // The root's own position is the instantiation point.
    assert_eq!(translation(root), [0.0, 0.0, 0.0]);
    assert_eq!(c["Model"]["asset"], "primitive:cube");
    assert!(close(&c["Material"]["base_color"]["r"], 1.0) && close(&c["Material"]["base_color"]["g"], 0.0));
    assert!(close(&c["Material"]["roughness"], 0.75));
    assert!(close(&c["Collider"]["shape"]["Cuboid"]["half_extents"]["x"], 0.5));
    // 8 kg in a 1 m cube.
    assert!(close(&c["Collider"]["density"], 8.0));
    assert_eq!(c["RigidBody"]["kind"], "Dynamic");
    assert!(close(&c["RigidBody"]["gravity_scale"], 1.0));

    let lamp = entity(&doc, "Crate/Lamp");
    assert_eq!(lamp["parent"], root["key"]);
    // Unity (0, 1, 2) is Marionette (0, 1, -2).
    assert_eq!(translation(lamp), [0.0, 1.0, -2.0]);
    let l = &lamp["components"]["DirectionalLight"];
    // 90 degrees about X turns Unity's forward (+Z) to straight down.
    assert!(close(&l["direction"]["y"], -1.0), "{l}");
    assert!(close(&l["illuminance"], 2.0));
    let script = &lamp["unity"]["scripts"][0];
    assert_eq!(script["script"], "Spin");
    assert_eq!(script["path"], "Assets/Scripts/Spin.cs");
    assert_eq!(script["fields"]["speed"], "2.5");
    assert_eq!(script["fields"]["label"], "spinning lamp");
    // A collider on a child of a Rigidbody is listed, not turned into a separate body.
    let unmapped = lamp["unity"]["unmapped"].to_string();
    assert!(unmapped.contains("compound"), "{unmapped}");
    assert!(lamp["components"].get("Collider").is_none());

    // The record carries the declaration and lists the material it uses.
    let rec = lib
        .records()
        .into_iter()
        .find(|x| x.source.path == "Assets/Prefabs/Crate.prefab")
        .unwrap();
    assert_eq!(rec.license.product, "Example Prefab Pack");
    assert_eq!(rec.main.as_deref(), Some("prefab.json"));
    assert_eq!(rec.pool, "blocked");
    assert!(rec.private);
    assert_eq!(rec.uses.len(), 1);
    assert_eq!(lib.get(&rec.uses[0]).unwrap().kind, AssetKind::Material);
}

#[test]
fn nested_prefabs_and_models_follow_their_overrides() {
    let (_base, lib, p, decl) = setup("nested");
    let r = run(&lib, &p, &decl, false);
    let doc = prefab_doc(&lib, &r, "Assets/Prefabs/Cart.prefab");
    let cart = entity(&doc, "Cart");
    assert!(close(&cart["components"]["Transform"]["scale"]["x"], 2.0));

    // The nested crate: renamed, moved (1, 0, 3) under a parent scaled 2, size override kept.
    let cargo = entity(&doc, "Cart/Cargo");
    assert_eq!(cargo["parent"], cart["key"]);
    assert_eq!(translation(cargo), [2.0, 0.0, -6.0]);
    let c = &cargo["components"];
    assert!(close(&c["Collider"]["shape"]["Cuboid"]["half_extents"]["y"], 1.0));
    // 8 kg in a 1 x 2 x 1 box scaled by 2: 16 cubic meters.
    assert!(close(&c["Collider"]["density"], 0.5));
    // A component the outer prefab added to the nested one.
    assert!(close(&c["AudioListener"]["volume"], 1.0));
    assert_eq!(cargo["unity"]["origin"], "Assets/Prefabs/Crate.prefab");
    let lamp = entity(&doc, "Cart/Cargo/Lamp");
    assert_eq!(translation(lamp), [2.0, 2.0, -10.0]);

    // The model instance: library model, root override applied, half turn for Unity's mirror.
    let wheel = entity(&doc, "Cart/Front Wheel");
    let model = wheel["components"]["Model"]["asset"].as_str().unwrap();
    assert!(
        model.starts_with("library:wheel-") && model.ends_with("/model.glb"),
        "{model}"
    );
    assert_eq!(translation(wheel), [-2.0, 0.0, 0.0]);
    let rot = &wheel["components"]["Transform"]["rotation"];
    assert!(close(&rot["y"], 1.0) && close(&rot["w"], 0.0), "{rot}");
    // The override of a part inside the model is reported, not lost.
    let notes = doc["notes"].to_string();
    assert!(notes.contains("model part -8679921383154817045"), "{notes}");
    assert!(notes.contains("no parent-child transforms"));
    let rec = lib
        .records()
        .into_iter()
        .find(|x| x.source.path == "Assets/Prefabs/Cart.prefab")
        .unwrap();
    assert!(rec.uses.iter().any(|u| u.starts_with("wheel-")), "{:?}", rec.uses);
}

#[test]
fn variants_remove_components_and_override_materials() {
    let (_base, lib, p, decl) = setup("variant");
    let r = run(&lib, &p, &decl, false);
    let doc = prefab_doc(&lib, &r, "Assets/Prefabs/Crate Variant.prefab");
    let root = entity(&doc, "Light Crate");
    let c = &root["components"];
    assert!(c.get("RigidBody").is_none(), "{c}");
    // Unity's default material replaced the red one: no override.
    assert!(c.get("Material").is_none(), "{c}");
    assert_eq!(c["Model"]["asset"], "primitive:cube");
    // Without a body, the lamp's sphere collider is a fixed collider of its own.
    let lamp = entity(&doc, "Light Crate/Lamp");
    assert!(close(
        &lamp["components"]["Collider"]["shape"]["Ball"]["radius"],
        0.25
    ));
}

#[test]
fn broken_recursive_and_unlicensed_prefabs_are_reported() {
    let (base, lib, p, decl) = setup("broken");
    let mut o = ImportOptions::new(&p);
    o.declarations = vec![decl.clone()];
    let r = import(&lib, &o).unwrap();
    let skipped: Vec<(&str, &str)> = r
        .skipped
        .iter()
        .map(|s| (s.path.as_str(), s.code.as_str()))
        .collect();
    assert!(
        skipped.contains(&("Assets/Prefabs/Binary.prefab", codes::SKIPPED_BINARY_SERIALIZED)),
        "{skipped:?}"
    );
    let doc = prefab_doc(&lib, &r, "Assets/Prefabs/Loop.prefab");
    assert!(doc.to_string().contains("contains itself"), "{doc:#}");
    // Scanning a Unity project only reads Assets/, so the stray file is not even seen; a plain
    // folder source shows the license rule instead.
    let loose = base.join("Loose");
    asset(
        &loose,
        "Stray.prefab",
        "abababababababababababababababab",
        crate_prefab().as_bytes(),
    );
    let mut o = ImportOptions::new(&loose);
    o.declarations = Vec::new();
    let r = import(&lib, &o).unwrap();
    assert!(
        r.refused
            .iter()
            .any(|x| x.path == "Stray.prefab" && x.code == codes::REFUSED_LICENSE_UNKNOWN),
        "{r:#?}"
    );
}

#[test]
fn conversion_is_deterministic_and_dry_runs_write_nothing() {
    let (base, lib, p, decl) = setup("determinism");
    let dry = run(&lib, &p, &decl, true);
    assert!(dry.imported.iter().any(|a| a.kind == AssetKind::Prefab));
    assert!(lib.records().is_empty());
    let a = run(&lib, &p, &decl, false);
    let first = prefab_doc(&lib, &a, "Assets/Prefabs/Cart.prefab");
    let lib2 = Library::at(&base.join("owned2"), &base.join("game"), ConfiguredFrom::Input).unwrap();
    let b = run(&lib2, &p, &decl, false);
    let second = prefab_doc(&lib2, &b, "Assets/Prefabs/Cart.prefab");
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    // Importing again is a no-op.
    let again = run(&lib, &p, &decl, false);
    assert!(again.imported.is_empty());
}
