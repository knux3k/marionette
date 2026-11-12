//! CC0 pool acceptance tests (workstream W3, decision 0041). No network: online libraries are
//! answered by a scripted transport whose JSON follows the shapes their APIs returned when the
//! adapters were written. Declarations and dedications are written by the tests standing in for
//! a person.

use marionette_import::library::{ConfiguredFrom, Library};
use marionette_pool::admit::{Candidate, add};
use marionette_pool::net::{Scripted, md5_hex};
use marionette_pool::search::{SearchRequest, search};
use marionette_pool::sources::{ambientcg, polyhaven};
use marionette_pool::zip::write_stored;
use marionette_pool::{Origin, Pool, PoolFrom, PoolKind, Provenance, codes, seed};
use serde_json::json;
use std::path::{Path, PathBuf};

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-pool-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn png(rgba: [u8; 4]) -> Vec<u8> {
    let px: Vec<u8> = (0..16).flat_map(|_| rgba).collect();
    marionette_assets::encode_png(4, 4, &px)
}

fn jpg() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(4, 4, image::Rgb([120, 60, 30]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

fn made(name: &str, files: Vec<(&str, Vec<u8>)>) -> Candidate {
    Candidate {
        kind: PoolKind::Texture,
        name: name.into(),
        id: None,
        tags: vec!["Test".into(), "test".into()],
        source: Provenance {
            origin: Origin::Made,
            library: None,
            page: None,
            source_id: None,
            author: None,
            retrieved: None,
            downloads: Vec::new(),
            method: Some("drawn in a test".into()),
            generator: None,
        },
        proof: Vec::new(),
        files: files.into_iter().map(|(p, b)| (p.to_string(), b)).collect(),
        main: None,
        notes: Vec::new(),
        origin_path: None,
        generated_dirs: Vec::new(),
    }
}

fn code(r: Result<marionette_pool::admit::AddReport, marionette_core::Errors>) -> String {
    match r {
        Ok(r) => panic!("expected a refusal, got {r:?}"),
        Err(e) => e.0[0].code.to_string(),
    }
}

fn new_pool(base: &Path) -> Pool {
    Pool::at(&base.join("pool"), PoolFrom::Input).unwrap()
}

#[test]
fn the_committed_seed_pool_verifies() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../pool");
    let pool = Pool::at(&repo, PoolFrom::Input).unwrap();
    let r = marionette_pool::verify::verify(&pool);
    assert_eq!(r.assets, 5);
    assert!(r.problems.is_empty(), "{:?}", r.problems);
}

#[test]
fn seed_pool_matches_its_generator() {
    let base = temp("seed");
    let pool = new_pool(&base);
    let reports = seed::build(&pool).unwrap();
    assert_eq!(reports.len(), 5);
    assert!(
        reports.iter().all(|r| r.status == "added" && r.engine_ready),
        "{reports:?}"
    );
    let index = serde_json::to_string_pretty(&pool.index()).unwrap() + "\n";
    std::fs::write(pool.root().join("index.json"), index).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../pool");
    let mut files = Vec::new();
    let mut stack = vec![repo.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                files.push(e.path());
            }
        }
    }
    files.retain(|f| f.file_name().is_none_or(|n| n != "README.md"));
    files.sort();
    let built: usize = {
        let mut n = 0;
        let mut stack = vec![pool.root().to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else {
                    n += 1;
                }
            }
        }
        n
    };
    assert_eq!(files.len(), built, "pool/ and the generator differ in file count");
    for f in files {
        let rel = f.strip_prefix(&repo).unwrap();
        let committed = std::fs::read(&f).unwrap();
        let rebuilt = std::fs::read(pool.root().join(rel)).unwrap();
        assert!(
            committed == rebuilt,
            "{} differs from its generator",
            rel.display()
        );
    }
    // Every seed asset carries its proof.
    for r in pool.records() {
        assert_eq!(r.license.spdx, "CC0-1.0");
        assert_eq!(r.license.proof[0].kind, "made_for_project");
        assert_eq!(
            r.source.generator.as_deref(),
            Some("crates/marionette-pool/src/seed.rs")
        );
    }
}

#[test]
fn made_assets_need_a_method_and_cc0_license_files() {
    let base = temp("made");
    let pool = new_pool(&base);
    let ok = add(
        &pool,
        made("Orange", vec![("texture.png", png([255, 128, 0, 255]))]),
        None,
        false,
    )
    .unwrap();
    assert_eq!(ok.status, "added");
    assert!(ok.id.starts_with("orange-"));
    let rec = pool.get(&ok.id).unwrap();
    assert_eq!(rec.tags, vec!["test".to_string()]);
    assert_eq!(rec.main.as_deref(), Some("texture.png"));
    // Adding it again is a no-op; the same id with other files is refused.
    let again = add(
        &pool,
        made("Orange", vec![("texture.png", png([255, 128, 0, 255]))]),
        None,
        false,
    )
    .unwrap();
    assert_eq!(again.status, "already_in_pool");
    let mut other = made("Orange", vec![("texture.png", png([0, 0, 0, 255]))]);
    other.id = Some(ok.id.clone());
    assert_eq!(code(add(&pool, other, None, false)), codes::EXISTS);

    let mut no_method = made("A", vec![("texture.png", png([1, 2, 3, 255]))]);
    no_method.source.method = None;
    assert_eq!(code(add(&pool, no_method, None, false)), codes::NO_PROOF);

    let by = made(
        "B",
        vec![
            ("texture.png", png([1, 2, 3, 255])),
            (
                "LICENSE.txt",
                b"Creative Commons Attribution 4.0 (CC BY 4.0)".to_vec(),
            ),
        ],
    );
    assert_eq!(code(add(&pool, by, None, false)), codes::NOT_CC0);
    let unclear = made(
        "C",
        vec![
            ("texture.png", png([1, 2, 3, 255])),
            ("License.txt", b"Thanks!".to_vec()),
        ],
    );
    assert_eq!(code(add(&pool, unclear, None, false)), codes::NOT_CC0);
    let cc0 = made(
        "D",
        vec![
            ("texture.png", png([1, 2, 3, 255])),
            ("License.txt", b"CC0 1.0 Universal".to_vec()),
        ],
    );
    let r = add(&pool, cc0, None, false).unwrap();
    assert!(r.proof.iter().any(|p| p.kind == "license_file"));

    let decl = made(
        "E",
        vec![
            ("texture.png", png([1, 2, 3, 255])),
            ("pack.license.json", b"{}".to_vec()),
        ],
    );
    assert_eq!(code(add(&pool, decl, None, false)), codes::NOT_CC0);
    let broken = made("F", vec![("texture.png", b"not a png".to_vec())]);
    assert_eq!(code(add(&pool, broken, None, false)), codes::BAD_FILE);
    let unsafe_path = made("G", vec![("../texture.png", png([1, 2, 3, 255]))]);
    assert_eq!(code(add(&pool, unsafe_path, None, false)), codes::BAD_INPUT);

    // Dry runs write nothing.
    let before = pool.records().len();
    let dry = add(
        &pool,
        made("H", vec![("texture.png", png([9, 9, 9, 255]))]),
        None,
        true,
    )
    .unwrap();
    assert_eq!(dry.status, "checked");
    assert_eq!(pool.records().len(), before);
    // Tools never write dedications or declarations.
    assert!(marionette_pool::write_file(&base.join("x.cc0.json"), b"{}").is_err());
    assert!(marionette_pool::write_file(&base.join("x.license.json"), b"{}").is_err());
}

#[test]
fn generated_assets_need_a_persons_dedication() {
    let base = temp("generated");
    let pool = new_pool(&base);
    let prov = serde_json::to_vec(&json!({
        "format": "marionette.generated_asset/1", "provider": "tripo", "pool": "blocked"
    }))
    .unwrap();
    let gen_files = || {
        vec![
            ("texture.png", png([7, 7, 7, 255])),
            ("texture.provenance.json", prov.clone()),
        ]
    };
    assert_eq!(
        code(add(&pool, made("Gen", gen_files()), None, false)),
        codes::GENERATED
    );
    let mut files = gen_files();
    files.push((
        "texture.cc0.json",
        serde_json::to_vec(&json!({
            "format": "marionette.cc0_dedication/1",
            "dedicated_by": "the person who generated it",
            "statement": "I dedicate this image to the public domain under CC0 1.0."
        }))
        .unwrap(),
    ));
    let r = add(&pool, made("Gen", files), None, false).unwrap();
    assert!(r.proof.iter().any(|p| p.kind == "person_dedication"), "{r:?}");
    // Provenance and dedication become proof, not pooled files.
    assert_eq!(r.files, vec!["texture.png".to_string()]);
}

#[test]
fn owned_assets_and_copies_of_them_are_refused() {
    let base = temp("owned");
    let project = base.join("game");
    std::fs::create_dir_all(&project).unwrap();
    let lib = Library::at(&base.join("owned"), &project, ConfiguredFrom::Input).unwrap();
    let bought = base.join("bought");
    std::fs::create_dir_all(&bought).unwrap();
    let owned_png = png([200, 10, 10, 255]);
    std::fs::write(bought.join("tile.png"), &owned_png).unwrap();
    std::fs::write(
        base.join("bought.license.json"),
        json!({ "format": "marionette.license_declaration/1", "store": "Example Store", "product": "Tiles",
                "license_type": "Standard", "other_engines_allowed": "yes" })
        .to_string(),
    )
    .unwrap();
    let mut o = marionette_import::ImportOptions::new(&bought);
    o.declarations = vec![base.join("bought.license.json")];
    marionette_import::import(&lib, &o).unwrap();
    let pool = new_pool(&base);

    // A renamed copy of an owned file.
    let copy = made("Copy", vec![("texture.png", owned_png.clone())]);
    assert_eq!(
        code(add(&pool, copy, Some(&lib), false)),
        marionette_import::codes::POOL_OWNED_CONTENT
    );
    // Anything picked up from inside the owned library.
    let rec = lib.records().into_iter().next().unwrap();
    let mut inside = made("Inside", vec![("texture.png", png([1, 1, 1, 255]))]);
    inside.origin_path = Some(lib.asset_dir(&rec.id).join("texture.png"));
    assert_eq!(
        code(add(&pool, inside, Some(&lib), false)),
        marionette_import::codes::POOL_OWNED_ASSET
    );
    // A pool inside the owned library is refused outright.
    assert!(Pool::at(&lib.root().join("pool"), PoolFrom::Input).is_err());
}

#[test]
fn online_assets_need_a_known_library_its_domain_and_license_file() {
    let base = temp("online");
    let pool = new_pool(&base);
    let online = |lib: &str, page: &str, files: Vec<(&str, Vec<u8>)>| {
        let mut c = made("Pack", files);
        c.kind = PoolKind::Model;
        c.source.origin = Origin::Online;
        c.source.method = None;
        c.source.library = Some(lib.into());
        c.source.page = Some(page.into());
        c
    };
    let glb = marionette_providers::glb::box_glb([0.5; 3], [1.0; 4], "box");
    let lic =
        b"License: (Creative Commons Zero, CC0)\nhttp://creativecommons.org/publicdomain/zero/1.0/".to_vec();
    assert_eq!(
        code(add(
            &pool,
            online("somewhere", "https://example.com/x", vec![("m.glb", glb.clone())]),
            None,
            false
        )),
        codes::UNKNOWN_SOURCE
    );
    assert_eq!(
        code(add(
            &pool,
            online(
                "kenney",
                "https://kenney.nl.example.com/x",
                vec![("m.glb", glb.clone()), ("License.txt", lic.clone())]
            ),
            None,
            false
        )),
        codes::NO_PROOF
    );
    assert_eq!(
        code(add(
            &pool,
            online(
                "kenney",
                "https://kenney.nl/assets/box",
                vec![("m.glb", glb.clone())]
            ),
            None,
            false
        )),
        codes::NO_PROOF
    );
    let r = add(
        &pool,
        online(
            "kenney",
            "https://kenney.nl/assets/box",
            vec![("Models/m.glb", glb), ("License.txt", lic)],
        ),
        None,
        false,
    )
    .unwrap();
    assert_eq!(r.proof[0].kind, "library_policy");
    assert!(r.proof.iter().any(|p| p.kind == "license_file"));
    let rec = pool.get(&r.id).unwrap();
    assert_eq!(rec.main.as_deref(), Some("Models/m.glb"));
    assert!(rec.source.retrieved.is_some());

    // The archive route: a Kenney-style zip with its license file.
    let zip = write_stored(&[
        ("Pack/License.txt", b"Creative Commons Zero, CC0"),
        (
            "Pack/Models/a.glb",
            &marionette_providers::glb::box_glb([0.25; 3], [1.0; 4], "a"),
        ),
        (
            "Pack/Models/b.glb",
            &marionette_providers::glb::box_glb([0.75; 3], [1.0; 4], "b"),
        ),
    ]);
    let t = Scripted::default().with("https://kenney.nl/media/pack.zip", 200, zip);
    let c = marionette_pool::sources::archive::ingest(
        &t,
        marionette_pool::sources::archive::ArchiveRequest {
            library: "kenney",
            page: "https://kenney.nl/assets/pack",
            from: marionette_pool::sources::archive::ArchiveFrom::Url("https://kenney.nl/media/pack.zip"),
            kind: PoolKind::Model,
            name: "Box A",
            tags: vec![],
            include: vec!["Models/a".into()],
            main: None,
        },
    )
    .unwrap();
    let names: Vec<&str> = c.files.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(names, vec!["License.txt", "Models/a.glb"]);
    let r = add(&pool, c, None, false).unwrap();
    assert_eq!(
        pool.get(&r.id).unwrap().source.downloads,
        vec!["https://kenney.nl/media/pack.zip".to_string()]
    );
    // An archive URL off the library's site is refused before any download.
    let off = marionette_pool::sources::archive::ingest(
        &t,
        marionette_pool::sources::archive::ArchiveRequest {
            library: "quaternius",
            page: "https://quaternius.com/packs/x.html",
            from: marionette_pool::sources::archive::ArchiveFrom::Url("https://files.example.com/x.zip"),
            kind: PoolKind::Model,
            name: "X",
            tags: vec![],
            include: vec![],
            main: None,
        },
    );
    assert!(off.is_err());
}

fn entry(url: &str, bytes: &[u8]) -> serde_json::Value {
    json!({ "url": url, "size": bytes.len(), "md5": md5_hex(bytes) })
}

/// A scripted Poly Haven: one texture (PNG diffuse, JPEG normal) and one model (glTF with a
/// buffer and a JPEG texture), in the API's shapes.
fn polyhaven_fixture() -> Scripted {
    let dl = "https://dl.polyhaven.org/file/ph-assets";
    let diffuse = png([150, 60, 50, 255]);
    let normal = jpg();
    let bin: Vec<u8> = [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    let tex = jpg();
    let gltf = serde_json::to_vec(&json!({
        "asset": { "version": "2.0" }, "scene": 0, "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 }, "material": 0 }] }],
        "materials": [{ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } }],
        "textures": [{ "source": 0 }],
        "images": [{ "uri": "textures/stool_diff_1k.jpg", "mimeType": "image/jpeg" }],
        "buffers": [{ "uri": "stool.bin", "byteLength": 36 }],
        "bufferViews": [{ "buffer": 0, "byteLength": 36 }],
        "accessors": [{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                        "min": [0, 0, 0], "max": [1, 1, 0] }]
    }))
    .unwrap();
    let list_textures = json!({
        "brick_wall_001": { "name": "Brick Wall 001", "type": 1, "tags": ["red", "rough"], "categories": ["brick", "wall"], "download_count": 900 },
        "rock_face": { "name": "Rock Face", "type": 1, "tags": ["rock"], "categories": ["natural"], "download_count": 50 }
    });
    let list_models = json!({
        "wooden_stool_01": { "name": "Wooden Stool 01", "type": 2, "tags": ["stool", "wood"], "categories": ["furniture", "seating"], "download_count": 500 }
    });
    let files_brick = json!({
        "Diffuse": { "1k": { "png": entry(&format!("{dl}/Textures/png/1k/brick_wall_001/brick_wall_001_diff_1k.png"), &diffuse) } },
        "nor_gl": { "1k": { "jpg": entry(&format!("{dl}/Textures/jpg/1k/brick_wall_001/brick_wall_001_nor_gl_1k.jpg"), &normal) } },
        "blend": { "1k": { "blend": { "url": format!("{dl}/x.blend"), "size": 1, "md5": "0" } } }
    });
    let mut g = entry(
        &format!("{dl}/Models/gltf/1k/wooden_stool_01/wooden_stool_01_1k.gltf"),
        &gltf,
    );
    g["include"] = json!({
        "textures/stool_diff_1k.jpg": entry(&format!("{dl}/Models/jpg/1k/wooden_stool_01/stool_diff_1k.jpg"), &tex),
        "stool.bin": entry(&format!("{dl}/Models/gltf/8k/wooden_stool_01/stool.bin"), &bin)
    });
    let files_stool = json!({ "gltf": { "1k": { "gltf": g } } });
    let api = "https://api.polyhaven.com";
    Scripted::default()
        .with(&format!("{api}/assets?t=textures"), 200, serde_json::to_vec(&list_textures).unwrap())
        .with(&format!("{api}/assets?t=models"), 200, serde_json::to_vec(&list_models).unwrap())
        .with(
            &format!("{api}/info/brick_wall_001"),
            200,
            serde_json::to_vec(&json!({ "name": "Brick Wall 001", "type": 1, "tags": ["red"], "categories": ["brick"], "authors": { "A. Photographer": "Photography" } })).unwrap(),
        )
        .with(&format!("{api}/files/brick_wall_001"), 200, serde_json::to_vec(&files_brick).unwrap())
        .with(&format!("{dl}/Textures/png/1k/brick_wall_001/brick_wall_001_diff_1k.png"), 200, diffuse)
        .with(&format!("{dl}/Textures/jpg/1k/brick_wall_001/brick_wall_001_nor_gl_1k.jpg"), 200, normal)
        .with(
            &format!("{api}/info/wooden_stool_01"),
            200,
            serde_json::to_vec(&json!({ "name": "Wooden Stool 01", "type": 2, "tags": ["stool"], "categories": ["furniture"], "authors": { "B. Modeler": "All" } })).unwrap(),
        )
        .with(&format!("{api}/files/wooden_stool_01"), 200, serde_json::to_vec(&files_stool).unwrap())
        .with(&format!("{dl}/Models/gltf/1k/wooden_stool_01/wooden_stool_01_1k.gltf"), 200, gltf)
        .with(&format!("{dl}/Models/jpg/1k/wooden_stool_01/stool_diff_1k.jpg"), 200, tex)
        .with(&format!("{dl}/Models/gltf/8k/wooden_stool_01/stool.bin"), 200, bin)
        .with(&format!("{api}/info/hdri_sky"), 200, serde_json::to_vec(&json!({ "name": "Sky", "type": 0 })).unwrap())
}

#[test]
fn poly_haven_textures_and_models_are_ingested_with_checks() {
    let base = temp("polyhaven");
    let pool = new_pool(&base);
    let t = polyhaven_fixture();

    let hits = polyhaven::search(&t, "brick wall", None, 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].source_id, "brick_wall_001");
    assert_eq!(hits[0].page, "https://polyhaven.com/a/brick_wall_001");
    let seats = polyhaven::search(&t, "seating", Some(PoolKind::Model), 10).unwrap();
    assert_eq!(seats[0].source_id, "wooden_stool_01");

    let c = polyhaven::ingest(&t, "brick_wall_001", "1k").unwrap();
    let r = add(&pool, c, None, false).unwrap();
    assert_eq!(r.files, vec!["color.png".to_string(), "normal.png".to_string()]);
    let rec = pool.get(&r.id).unwrap();
    assert_eq!(rec.source.library.as_deref(), Some("polyhaven"));
    assert_eq!(rec.source.author.as_deref(), Some("A. Photographer"));
    assert_eq!(rec.source.downloads.len(), 2);
    assert_eq!(
        rec.license.proof[0].at.as_deref(),
        Some("https://polyhaven.com/license")
    );
    assert!(rec.license.proof.iter().any(|p| p.kind == "download_integrity"));

    let c = polyhaven::ingest(&t, "wooden_stool_01", "1k").unwrap();
    let r = add(&pool, c, None, false).unwrap();
    assert!(r.engine_ready, "{r:?}");
    assert!(
        r.files.contains(&"textures/stool_diff_1k.png".to_string()),
        "{:?}",
        r.files
    );
    let gltf: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pool.asset_dir(&r.id).join("wooden_stool_01.gltf")).unwrap())
            .unwrap();
    assert_eq!(gltf["images"][0]["uri"], "textures/stool_diff_1k.png");

    // HDRIs are not pooled; tampered downloads are caught.
    assert!(
        polyhaven::ingest(&t, "hdri_sky", "1k")
            .unwrap_err()
            .contains("HDRI")
    );
    let mut bad = polyhaven_fixture();
    bad.answers.insert(
        "https://dl.polyhaven.org/file/ph-assets/Textures/png/1k/brick_wall_001/brick_wall_001_diff_1k.png"
            .into(),
        marionette_pool::net::HttpResponse {
            status: 200,
            body: png([0, 0, 0, 255]),
        },
    );
    let err = polyhaven::ingest(&bad, "brick_wall_001", "1k").unwrap_err();
    assert!(err.contains("bytes") || err.contains("MD5"), "{err}");
    assert!(polyhaven::ingest(&t, "../etc", "1k").is_err());
}

#[test]
fn ambientcg_materials_are_ingested_from_their_archives() {
    let base = temp("ambientcg");
    let pool = new_pool(&base);
    let zip = write_stored(&[
        ("Bricks001_1K-JPG_Color.jpg", &jpg()),
        ("Bricks001_1K-JPG_NormalGL.jpg", &jpg()),
        ("Bricks001_1K-JPG_NormalDX.jpg", &jpg()),
        ("Bricks001_1K-JPG_Roughness.jpg", &jpg()),
        ("Bricks001.usdc", b"usd"),
    ]);
    let asset = json!({
        "assetId": "Bricks001", "displayName": "Bricks 001", "dataType": "Material",
        "tags": ["bricks", "red"], "downloadCount": 25233,
        "downloadFolders": { "default": { "downloadFiletypeCategories": { "zip": { "downloads": [
            { "attribute": "1K-JPG", "fileName": "Bricks001_1K-JPG.zip", "size": zip.len(),
              "downloadLink": "https://ambientcg.com/get?file=Bricks001_1K-JPG.zip", "filetype": "zip" },
            { "attribute": "2K-JPG", "fileName": "Bricks001_2K-JPG.zip", "size": 1,
              "downloadLink": "https://ambientcg.com/get?file=Bricks001_2K-JPG.zip", "filetype": "zip" }
        ] } } } }
    });
    let answer = json!({ "numberOfResults": 1, "foundAssets": [asset] });
    let t = Scripted::default()
        .with(
            "https://ambientcg.com/api/v2/full_json?id=Bricks001&include=downloadData,tagData",
            200,
            serde_json::to_vec(&answer).unwrap(),
        )
        .with(
            "https://ambientcg.com/api/v2/full_json?q=red%20bricks&type=Material&sort=popular&limit=5&include=downloadData,tagData",
            200,
            serde_json::to_vec(&answer).unwrap(),
        )
        .with("https://ambientcg.com/get?file=Bricks001_1K-JPG.zip", 200, zip);
    let hits = ambientcg::search(&t, "red bricks", None, 5).unwrap();
    assert_eq!(hits[0].page, "https://ambientcg.com/view?id=Bricks001");
    let c = ambientcg::ingest(&t, "Bricks001", "1k").unwrap();
    let r = add(&pool, c, None, false).unwrap();
    assert_eq!(
        r.files,
        vec![
            "color.png".to_string(),
            "normal.png".to_string(),
            "roughness.png".to_string()
        ]
    );
    let rec = pool.get(&r.id).unwrap();
    assert_eq!(
        rec.license.proof[0].at.as_deref(),
        Some("https://docs.ambientcg.com/license/")
    );
    assert!(rec.notes.iter().any(|n| n.contains("converted from JPEG")));
}

#[test]
fn search_follows_the_required_order() {
    let base = temp("search");
    let project = base.join("game");
    std::fs::create_dir_all(project.join("textures")).unwrap();
    std::fs::write(project.join("textures/brick_old.png"), png([1, 1, 1, 255])).unwrap();
    let pool = new_pool(&base);
    seed::build(&pool).unwrap();
    let t = polyhaven_fixture();
    let req = |q: &'static str, online: bool, stop: bool| SearchRequest {
        query: q,
        kind: None,
        project_root: &project,
        library: None,
        library_note: None,
        pool: Some(&pool),
        pool_note: None,
        online,
        transport: &t,
        limit: 10,
        stop_at_first: stop,
    };
    // The project's own file comes first and stops the search.
    let r = search(&req("brick", true, true));
    assert_eq!(r.found_in.as_deref(), Some("project_files"));
    assert_eq!(r.steps[0].results[0].id, "textures/brick_old.png");
    assert!(!r.steps[2].searched && !r.steps[3].searched);
    // All steps: the pool and the online library also answer, in order.
    let r = search(&req("brick", true, false));
    let names: Vec<&str> = r.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "project_files",
            "owned_library",
            "cc0_pool",
            "online_cc0",
            "generate"
        ]
    );
    assert_eq!(r.steps[2].results[0].id, "red-bricks");
    assert_eq!(r.steps[3].results[0].id, "brick_wall_001");
    // Not in the project: the pool answers; online is never reached without `online`.
    let r = search(&req("crate", false, true));
    assert_eq!(r.found_in.as_deref(), Some("cc0_pool"));
    assert_eq!(r.steps[2].results[0].use_with, "pool_fetch");
    // Nothing anywhere: the generation step points at the generation operations.
    let r = search(&req("dragon", false, true));
    assert_eq!(r.found_in, None);
    assert!(r.steps[3].notes[0].contains("online: true"));
    assert!(r.steps[4].notes[0].contains("generate_plan"));
    // Searching never downloads asset files.
    assert!(
        t.asked
            .lock()
            .unwrap()
            .iter()
            .all(|u| u.starts_with("https://api.polyhaven.com/assets")
                || u.starts_with("https://ambientcg.com/api/")),
        "{:?}",
        t.asked.lock().unwrap()
    );
}
