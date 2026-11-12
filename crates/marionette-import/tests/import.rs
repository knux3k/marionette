//! Acceptance tests for the owned library and the importers (workstream W2, plan line 8a).
//! Every fixture is built at run time: no third-party files are committed. Declaration files
//! are written by the tests standing in for the person who bought the asset.

use marionette_import::codes;
use marionette_import::library::{AssetKind, ConfiguredFrom, Library};
use marionette_import::{ImportOptions, ImportReport, import, pool, tar};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-import-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A fresh (library, project) pair in a temp folder.
fn setup(tag: &str) -> (PathBuf, Library) {
    let base = temp(tag);
    let project = base.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let lib = Library::at(&base.join("owned"), &project, ConfiguredFrom::Input).unwrap();
    (base, lib)
}

fn declare(path: &Path, allowed: &str, extra: Value) {
    let mut d = json!({
        "format": "marionette.license_declaration/1",
        "store": "Example Store",
        "product": "Example Pack",
        "license_type": "Standard single-user license",
        "other_engines_allowed": allowed,
    });
    for (k, v) in extra.as_object().unwrap() {
        d[k] = v.clone();
    }
    std::fs::write(path, serde_json::to_vec_pretty(&d).unwrap()).unwrap();
}

fn png(rgba: [u8; 4]) -> Vec<u8> {
    let px: Vec<u8> = (0..16).flat_map(|_| rgba).collect();
    marionette_assets::encode_png(4, 4, &px)
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn tex_meta(guid: &str, normal: bool) -> String {
    format!(
        "fileFormatVersion: 2\nguid: {guid}\nTextureImporter:\n  sRGBTexture: {}\n  textureType: {}\n",
        if normal { 0 } else { 1 },
        if normal { 1 } else { 0 }
    )
}

/// A minimal glTF (one triangle) with an external buffer.
fn gltf_pair(bin_uri: &str) -> (Vec<u8>, Vec<u8>) {
    let pos: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let bin: Vec<u8> = pos.iter().flat_map(|f| f.to_le_bytes()).collect();
    let doc = json!({
        "asset": { "version": "2.0" },
        "scene": 0, "scenes": [{ "nodes": [0] }], "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 } }] }],
        "buffers": [{ "uri": bin_uri, "byteLength": 36 }],
        "bufferViews": [{ "buffer": 0, "byteLength": 36 }],
        "accessors": [{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                        "min": [0, 0, 0], "max": [1, 1, 0] }]
    });
    (serde_json::to_vec(&doc).unwrap(), bin)
}

/// Builds a .unitypackage from (guid, pathname, asset bytes, meta).
fn unitypackage(path: &Path, items: &[(&str, &str, Vec<u8>, Option<String>)]) {
    let mut owned: Vec<(String, Vec<u8>)> = Vec::new();
    for (guid, pathname, bytes, meta) in items {
        owned.push((format!("{guid}/pathname"), format!("{pathname}\n00").into_bytes()));
        if !bytes.is_empty() {
            owned.push((format!("{guid}/asset"), bytes.clone()));
        }
        if let Some(m) = meta {
            owned.push((format!("{guid}/asset.meta"), m.clone().into_bytes()));
        }
    }
    let refs: Vec<(&str, &[u8])> = owned.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let tarball = tar::write_ustar(&refs);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tarball).unwrap();
    write(path, &gz.finish().unwrap());
}

fn run(lib: &Library, source: &Path, decls: &[&Path]) -> ImportReport {
    let mut o = ImportOptions::new(source);
    o.declarations = decls.iter().map(|p| p.to_path_buf()).collect();
    import(lib, &o).unwrap()
}

fn refused_codes(r: &ImportReport) -> Vec<(String, String)> {
    r.refused
        .iter()
        .map(|x| (x.path.clone(), x.code.clone()))
        .collect()
}

fn library_is_empty(lib: &Library) -> bool {
    lib.records().is_empty()
}

#[test]
fn owned_file_with_declaration_is_imported_with_its_license() {
    let (base, lib) = setup("owned-file");
    let src = base.join("bought/gem.glb");
    write(
        &src,
        &std::fs::read(repo().join("examples/assets/gem.glb")).unwrap(),
    );
    declare(
        &base.join("bought/gem.glb.license.json"),
        "yes",
        json!({ "publisher": "Some Studio" }),
    );

    let r = run(&lib, &src, &[]);
    assert_eq!(r.importer, "files");
    assert_eq!(r.counts.imported, 1, "{r:#?}");
    assert!(r.refused.is_empty());
    let a = &r.imported[0];
    assert_eq!(a.kind, AssetKind::Model);
    assert!(a.engine_ready, "{a:#?}");

    let rec = lib.get(&a.id).unwrap();
    assert_eq!(rec.license.status, "declared");
    assert_eq!(rec.license.product, "Example Pack");
    assert!(rec.license.other_engines_allowed);
    assert_eq!(rec.license.declaration_file, "gem.glb.license.json");
    assert_eq!(rec.license.declaration_sha256.len(), 64);
    assert!(rec.private);
    assert_eq!(rec.pool, "blocked");
    assert_eq!(rec.source.source_name, "gem.glb");
    assert!(lib.asset_dir(&a.id).join("model.glb").is_file());
    // The record holds no machine paths.
    let text = std::fs::read_to_string(lib.asset_dir(&a.id).join("record.json")).unwrap();
    assert!(!text.contains(&base.to_string_lossy().replace('\\', "\\\\")));

    // Importing again changes nothing.
    let again = run(&lib, &src, &[]);
    assert_eq!(again.counts.imported, 0);
    assert_eq!(again.counts.already_in_library, 1);
}

#[test]
fn undeclared_assets_are_refused_with_reasons() {
    let (base, lib) = setup("undeclared");
    write(&base.join("loose/a.png"), &png([255, 0, 0, 255]));
    write(&base.join("loose/b.png"), &png([0, 255, 0, 255]));
    let r = run(&lib, &base.join("loose"), &[]);
    assert_eq!(r.counts.imported, 0);
    assert_eq!(r.counts.refused, 2);
    for x in &r.refused {
        assert_eq!(x.code, codes::REFUSED_LICENSE_UNKNOWN);
        assert!(x.reason.contains("person"), "{}", x.reason);
    }
    assert!(library_is_empty(&lib));
}

#[test]
fn engine_locked_licenses_are_refused() {
    let (base, lib) = setup("locked");
    write(&base.join("no/a.png"), &png([1, 2, 3, 255]));
    declare(&base.join("no.license.json"), "no", json!({}));
    let r = run(&lib, &base.join("no"), &[]);
    assert_eq!(
        refused_codes(&r),
        vec![("a.png".into(), codes::REFUSED_OTHER_ENGINES_NO.into())]
    );

    // A person said yes, but the license text they pasted limits use to one engine.
    write(&base.join("text/a.png"), &png([1, 2, 3, 255]));
    declare(
        &base.join("text.license.json"),
        "yes",
        json!({ "license_text": "Licensed content may only be used with the Unreal Engine." }),
    );
    let r = run(&lib, &base.join("text"), &[]);
    assert_eq!(
        refused_codes(&r),
        vec![("a.png".into(), codes::REFUSED_ENGINE_LOCKED_LICENSE.into())]
    );
    assert!(
        r.refused[0].reason.contains("only be used with the unreal"),
        "{}",
        r.refused[0].reason
    );

    // Content published by an engine maker is refused even with a yes declaration.
    write(&base.join("pub/a.png"), &png([1, 2, 3, 255]));
    declare(
        &base.join("pub.license.json"),
        "yes",
        json!({ "publisher": "Unity Technologies" }),
    );
    let r = run(&lib, &base.join("pub"), &[]);
    assert_eq!(
        refused_codes(&r),
        vec![("a.png".into(), codes::REFUSED_ENGINE_CONTENT.into())]
    );
    assert!(library_is_empty(&lib));
}

#[test]
fn engine_content_in_a_unity_project_is_refused_without_override() {
    let (base, lib) = setup("project");
    let p = base.join("Game");
    std::fs::create_dir_all(p.join("ProjectSettings")).unwrap();
    for (rel, guid) in [
        (
            "Assets/Standard Assets/Env/rock.png",
            "11111111111111111111111111111111",
        ),
        (
            "Assets/Samples/Universal RP/14.0/lit.png",
            "22222222222222222222222222222222",
        ),
        ("Assets/StarterAssets/arm.png", "33333333333333333333333333333333"),
        (
            "Assets/MyPack/Textures/wood.png",
            "44444444444444444444444444444444",
        ),
    ] {
        write(&p.join(rel), &png([9, 9, 9, 255]));
        write(&p.join(format!("{rel}.meta")), tex_meta(guid, false).as_bytes());
    }
    write(&p.join("Assets/MyPack/Scripts/Spin.cs"), b"class Spin {}");
    write(&p.join("Assets/.hidden/x.png"), &png([0; 4]));
    write(&p.join("Assets/Docs~/x.png"), &png([0; 4]));
    write(&p.join("Packages/com.unity.thing/x.png"), &png([0; 4]));
    declare(&base.join("Game.license.json"), "yes", json!({}));

    let r = run(&lib, &p, &[]);
    assert_eq!(r.importer, "unity_project");
    assert_eq!(r.counts.imported, 1, "{r:#?}");
    assert_eq!(r.imported[0].path, "Assets/MyPack/Textures/wood.png");
    let refused: Vec<_> = refused_codes(&r)
        .into_iter()
        .map(|(p, c)| (p, c == codes::REFUSED_ENGINE_CONTENT))
        .collect();
    assert_eq!(refused.len(), 3);
    assert!(refused.iter().all(|(_, engine)| *engine), "{refused:?}");
    assert_eq!(r.skipped.len(), 1);
    assert_eq!(r.skipped[0].code, codes::SKIPPED_UNSUPPORTED);
    let rec = lib.get(&r.imported[0].id).unwrap();
    assert_eq!(
        rec.source.unity_guid.as_deref(),
        Some("44444444444444444444444444444444")
    );

    // A package that the store cache filed under an engine publisher is refused as a whole.
    let pkg = base.join("cache/Asset Store-5.x/Unity Technologies/3D/Kit.unitypackage");
    unitypackage(
        &pkg,
        &[(
            "55555555555555555555555555555555",
            "Assets/Kit/a.png",
            png([1; 4]),
            None,
        )],
    );
    declare(
        &base.join("cache/Asset Store-5.x/Unity Technologies/3D/Kit.unitypackage.license.json"),
        "yes",
        json!({}),
    );
    let r = run(&lib, &pkg, &[]);
    assert_eq!(
        refused_codes(&r),
        vec![("Assets/Kit/a.png".into(), codes::REFUSED_ENGINE_CONTENT.into())]
    );
}

#[test]
fn the_most_specific_declaration_applies() {
    let (base, lib) = setup("covers");
    let p = base.join("Game");
    std::fs::create_dir_all(p.join("ProjectSettings")).unwrap();
    for rel in [
        "Assets/A/a.png",
        "Assets/B/b.png",
        "Assets/C/c.png",
        "Assets/A/Sub/s.png",
    ] {
        write(&p.join(rel), &png([3, 3, 3, 255]));
    }
    // Different colors so the two A textures get different ids.
    write(&p.join("Assets/A/Sub/s.png"), &png([4, 4, 4, 255]));
    let decl_dir = base.join("declarations");
    std::fs::create_dir_all(&decl_dir).unwrap();
    declare(
        &decl_dir.join("a.license.json"),
        "yes",
        json!({ "covers": ["Assets/A/"] }),
    );
    declare(
        &decl_dir.join("b.license.json"),
        "no",
        json!({ "covers": ["Assets/B"] }),
    );
    declare(
        &decl_dir.join("asub.license.json"),
        "no",
        json!({ "covers": ["Assets/A/Sub"], "product": "Other" }),
    );
    let r = run(
        &lib,
        &p,
        &[
            &decl_dir.join("a.license.json"),
            &decl_dir.join("b.license.json"),
            &decl_dir.join("asub.license.json"),
        ],
    );
    let imported: Vec<&str> = r.imported.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(imported, vec!["Assets/A/a.png"]);
    let mut refused = refused_codes(&r);
    refused.sort();
    assert_eq!(
        refused,
        vec![
            (
                "Assets/A/Sub/s.png".into(),
                codes::REFUSED_OTHER_ENGINES_NO.into()
            ),
            ("Assets/B/b.png".into(), codes::REFUSED_OTHER_ENGINES_NO.into()),
            ("Assets/C/c.png".into(), codes::REFUSED_LICENSE_UNKNOWN.into()),
        ]
    );
}

#[test]
fn unitypackage_brings_in_textures_models_and_materials() {
    let (base, lib) = setup("package");
    let (ga, gb, gc, gm) = (
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "cccccccccccccccccccccccccccccccc",
        "dddddddddddddddddddddddddddddddd",
    );
    let mat = format!(
        "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!21 &2100000\nMaterial:\n  m_Name: Crate\n  \
         m_Shader: {{fileID: 0, guid: 933532a4fcc9baf4fa0491de14d08ed7, type: 3}}\n  m_SavedProperties:\n    \
         m_TexEnvs:\n    - _BaseMap:\n        m_Texture: {{fileID: 2800000, guid: {ga}, type: 3}}\n        \
         m_Scale: {{x: 1, y: 1}}\n        m_Offset: {{x: 0, y: 0}}\n    - _BumpMap:\n        \
         m_Texture: {{fileID: 2800000, guid: {gb}, type: 3}}\n    - _MetallicGlossMap:\n        \
         m_Texture: {{fileID: 2800000, guid: {gc}, type: 3}}\n    - _DetailMask:\n        \
         m_Texture: {{fileID: 2800000, guid: eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee, type: 3}}\n    \
         m_Floats:\n    - _Smoothness: 0.5\n    - _Surface: 0\n    m_Colors:\n    \
         - _BaseColor: {{r: 1, g: 1, b: 1, a: 1}}\n--- !u!114 &1\nMonoBehaviour:\n  m_Name: \n"
    );
    let (gltf, bin) = gltf_pair("tri.bin");
    let pkg = base.join("Crates.unitypackage");
    unitypackage(
        &pkg,
        &[
            (
                "ffffffffffffffffffffffffffffffff",
                "Assets/Crates",
                Vec::new(),
                None,
            ),
            (
                ga,
                "Assets/Crates/crate_albedo.png",
                png([200, 100, 50, 255]),
                Some(tex_meta(ga, false)),
            ),
            (
                gb,
                "Assets/Crates/crate_normal.png",
                png([128, 128, 255, 255]),
                Some(tex_meta(gb, true)),
            ),
            (
                gc,
                "Assets/Crates/crate_metal.png",
                png([255, 0, 0, 128]),
                Some(tex_meta(gc, false)),
            ),
            (gm, "Assets/Crates/Crate.mat", mat.into_bytes(), None),
            (
                "10000000000000000000000000000001",
                "Assets/Crates/crate.fbx",
                b"Kaydara FBX Binary  \0".to_vec(),
                None,
            ),
            (
                "10000000000000000000000000000002",
                "Assets/Crates/tri.gltf",
                gltf,
                None,
            ),
            (
                "10000000000000000000000000000003",
                "Assets/Crates/tri.bin",
                bin,
                None,
            ),
            (
                "10000000000000000000000000000004",
                "Assets/Crates/Spin.cs",
                b"class Spin {}".to_vec(),
                None,
            ),
            (
                "10000000000000000000000000000005",
                "Assets/../../escape.png",
                png([0; 4]),
                None,
            ),
        ],
    );
    declare(&base.join("Crates.unitypackage.license.json"), "yes", json!({}));

    let r = run(&lib, &pkg, &[]);
    assert_eq!(r.importer, "unitypackage");
    assert_eq!(r.counts.imported, 6, "{r:#?}");
    assert_eq!(
        refused_codes(&r),
        vec![("Assets/../../escape.png".into(), codes::REFUSED_BAD_PATH.into())]
    );
    let mut skipped: Vec<_> = r
        .skipped
        .iter()
        .map(|s| (s.path.as_str(), s.code.as_str()))
        .collect();
    skipped.sort();
    assert_eq!(
        skipped,
        vec![
            ("Assets/Crates/Spin.cs", codes::SKIPPED_UNSUPPORTED),
            ("Assets/Crates/tri.bin", codes::SKIPPED_COMPANION)
        ]
    );
    let id = |path: &str| r.imported.iter().find(|a| a.path == path).unwrap().id.clone();

    // Texture usage from the .meta.
    let normal = lib.get(&id("Assets/Crates/crate_normal.png")).unwrap();
    assert!(
        normal.notes.iter().any(|n| n.contains("normal map")),
        "{:?}",
        normal.notes
    );

    // FBX is kept but not engine-ready; glTF comes with its buffer and loads.
    let fbx = lib.get(&id("Assets/Crates/crate.fbx")).unwrap();
    assert!(!fbx.engine_ready);
    let tri = lib.get(&id("Assets/Crates/tri.gltf")).unwrap();
    assert!(tri.engine_ready, "{:?}", tri.notes);
    assert!(lib.asset_dir(&tri.id).join("deps/tri.bin").is_file());

    // The material references the imported textures by library id.
    let m = lib.get(&id("Assets/Crates/Crate.mat")).unwrap();
    assert_eq!(m.kind, AssetKind::Material);
    let doc: Value =
        serde_json::from_slice(&std::fs::read(lib.asset_dir(&m.id).join("material.json")).unwrap()).unwrap();
    assert_eq!(
        doc["base_color_texture"]["asset"],
        json!(id("Assets/Crates/crate_albedo.png"))
    );
    assert_eq!(
        doc["normal_texture"]["asset"],
        json!(id("Assets/Crates/crate_normal.png"))
    );
    assert_eq!(
        doc["metallic_roughness_texture"]["file"],
        json!("metallic_roughness.png")
    );
    assert_eq!(doc["unity"]["shader"], json!("Universal Render Pipeline/Lit"));
    assert_eq!(doc["unity"]["unmapped_textures"], json!(["_DetailMask"]));
    assert_eq!(doc["alpha_mode"], json!("OPAQUE"));
    assert_eq!(m.uses.len(), 3);
    // Metallic R=255 -> B=255; smoothness A=128 -> roughness G=127.
    let mr = marionette_assets::decode_png(
        &std::fs::read(lib.asset_dir(&m.id).join("metallic_roughness.png")).unwrap(),
    )
    .unwrap();
    assert_eq!(&mr.rgba[..4], &[255, 127, 255, 255]);
}

#[test]
fn materials_resolve_textures_from_earlier_imports() {
    let (base, lib) = setup("cross-package");
    let g = "abababababababababababababababab";
    let tex_pkg = base.join("Textures.unitypackage");
    unitypackage(
        &tex_pkg,
        &[(
            g,
            "Assets/T/stone.png",
            png([90, 90, 90, 255]),
            Some(tex_meta(g, false)),
        )],
    );
    declare(&base.join("Textures.unitypackage.license.json"), "yes", json!({}));
    let first = run(&lib, &tex_pkg, &[]);
    let mat = format!(
        "%YAML 1.1\n--- !u!21 &2100000\nMaterial:\n  m_Name: Stone\n  m_SavedProperties:\n    m_TexEnvs:\n    \
         - _MainTex:\n        m_Texture: {{fileID: 2800000, guid: {g}, type: 3}}\n"
    );
    let mat_pkg = base.join("Materials.unitypackage");
    unitypackage(
        &mat_pkg,
        &[(
            "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
            "Assets/M/Stone.mat",
            mat.into_bytes(),
            None,
        )],
    );
    declare(
        &base.join("Materials.unitypackage.license.json"),
        "yes",
        json!({}),
    );
    let r = run(&lib, &mat_pkg, &[]);
    let m = lib.get(&r.imported[0].id).unwrap();
    assert_eq!(m.uses, vec![first.imported[0].id.clone()]);
}

#[test]
fn pool_refuses_owned_assets_and_copies_of_them() {
    let (base, lib) = setup("pool");
    write(&base.join("mine/wood.png"), &png([120, 80, 40, 255]));
    declare(&base.join("mine.license.json"), "yes", json!({}));
    let r = run(&lib, &base.join("mine"), &[]);
    let dir = lib.asset_dir(&r.imported[0].id);

    let v = pool::check(&dir, Some(&lib));
    assert!(!v.admitted_by_owned_gate);
    assert!(v.refusals.iter().any(|x| x.code == codes::POOL_OWNED_ASSET));

    // A renamed copy outside the library is caught by its content.
    let cand = base.join("candidate");
    write(
        &cand.join("renamed.png"),
        &std::fs::read(dir.join("texture.png")).unwrap(),
    );
    let v = pool::check(&cand, Some(&lib));
    assert_eq!(v.refusals.len(), 1, "{v:?}");
    assert_eq!(v.refusals[0].code, codes::POOL_OWNED_CONTENT);

    // A declaration file marks owned content even without a library to compare with.
    write(&cand.join("x.license.json"), b"{}");
    let v = pool::check(&cand, None);
    assert!(v.refusals.iter().any(|x| x.code == codes::POOL_NOT_CC0));

    // Unrelated content passes this gate (the pool's own CC0 rule still applies).
    let fresh = base.join("fresh");
    write(&fresh.join("new.png"), &png([1, 200, 1, 255]));
    assert!(pool::check(&fresh, Some(&lib)).admitted_by_owned_gate);
}

#[test]
fn library_location_rules_hold() {
    let base = temp("location");
    let project = base.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let code = |r: Result<Library, marionette_core::Errors>| r.unwrap_err().0[0].code;
    assert_eq!(
        code(Library::at(Path::new("rel/lib"), &project, ConfiguredFrom::Input)),
        codes::LIBRARY_RELATIVE
    );
    assert_eq!(
        code(Library::at(
            &repo().join("owned-lib"),
            &project,
            ConfiguredFrom::Input
        )),
        codes::LIBRARY_INSIDE_ENGINE
    );
    assert_eq!(
        code(Library::at(&project.join("lib"), &project, ConfiguredFrom::Input)),
        codes::LIBRARY_INSIDE_PROJECT
    );
    let busy = base.join("busy");
    write(&busy.join("other.txt"), b"data");
    let lib = Library::at(&busy, &project, ConfiguredFrom::Input).unwrap();
    assert_eq!(lib.ensure().unwrap_err().0[0].code, codes::LIBRARY_NOT_A_LIBRARY);
    // The library never writes declaration files.
    let ok = Library::at(&base.join("ok"), &project, ConfiguredFrom::Input).unwrap();
    ok.ensure().unwrap();
    assert!(ok.root().join("library.json").is_file());
}

#[test]
fn dry_run_writes_nothing_and_bad_archives_are_errors() {
    let (base, lib) = setup("dry");
    write(&base.join("p/a.png"), &png([5, 5, 5, 255]));
    declare(&base.join("p.license.json"), "yes", json!({}));
    let mut o = ImportOptions::new(base.join("p"));
    o.dry_run = true;
    let r = import(&lib, &o).unwrap();
    assert!(r.dry_run);
    assert_eq!(r.counts.imported, 1);
    assert!(!lib.root().exists());

    write(&base.join("junk.unitypackage"), b"this is not gzip");
    declare(&base.join("junk.unitypackage.license.json"), "yes", json!({}));
    let err = import(&lib, &ImportOptions::new(base.join("junk.unitypackage"))).unwrap_err();
    assert_eq!(err.0[0].code, codes::IMPORT_BAD_ARCHIVE);

    let bad = base.join("bad.license.json");
    std::fs::write(&bad, br#"{"format":"marionette.license_declaration/1","store":"","product":"x","license_type":"y","other_engines_allowed":"yes"}"#).unwrap();
    let mut o = ImportOptions::new(base.join("p"));
    o.declarations.push(bad);
    assert_eq!(
        import(&lib, &o).unwrap_err().0[0].code,
        codes::IMPORT_BAD_DECLARATION
    );
}
