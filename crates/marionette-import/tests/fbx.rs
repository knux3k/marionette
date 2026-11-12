//! FBX to glTF conversion (decision 0027).
//!
//! The fixtures in `tests/fixtures/fbx/` were made for the project with Blender and Assimp
//! (`make_fixtures.py` there); they are CC0. Blender's own glTF export of the same scenes is the
//! reference: converted geometry, winding, skinning and animation must match it.

use glam::{Mat4, Quat, Vec3};
use marionette_import::fbx::{self, ResolvedImage, TextureRequest, tree};
use marionette_import::library::{AssetKind, ConfiguredFrom, Library};
use marionette_import::{ImportOptions, import};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fbx")
            .join(name),
    )
    .unwrap()
}

/// Resolves textures from the fixtures folder by file name (embedded media first).
fn beside(req: &TextureRequest) -> Result<ResolvedImage, String> {
    let bytes = match &req.embedded {
        Some(b) => b.clone(),
        None => std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/fbx")
                .join(req.file_name()),
        )
        .map_err(|e| e.to_string())?,
    };
    Ok(ResolvedImage {
        bytes,
        mime: "image/png",
        origin: "test".into(),
    })
}

fn convert(name: &str) -> fbx::Converted {
    fbx::convert(&fixture(name), Default::default(), &mut beside).unwrap()
}

/// World-space triangles of a GLB at an animation time (skinning applied), as (a, b, c).
fn triangles(glb: &[u8], time: Option<f32>) -> Vec<[Vec3; 3]> {
    let g = gltf::Gltf::from_slice(glb).expect("valid glTF");
    let blob = g.blob.as_deref().unwrap_or(&[]);
    let buf = |_: gltf::Buffer| Some(blob);
    let doc = &g.document;

    // Local transforms, with animation samples applied.
    let mut trs: Vec<(Vec3, Quat, Vec3)> = doc
        .nodes()
        .map(|n| {
            let (t, r, s) = n.transform().decomposed();
            (Vec3::from(t), Quat::from_array(r), Vec3::from(s))
        })
        .collect();
    if let (Some(t), Some(anim)) = (time, doc.animations().next()) {
        for ch in anim.channels() {
            let r = ch.reader(buf);
            let times: Vec<f32> = r.read_inputs().unwrap().collect();
            let k = times.partition_point(|&x| x <= t).clamp(1, times.len()) - 1;
            let k2 = (k + 1).min(times.len() - 1);
            let f = if k2 == k {
                0.0
            } else {
                ((t - times[k]) / (times[k2] - times[k])).clamp(0.0, 1.0)
            };
            let node = ch.target().node().index();
            use gltf::animation::util::ReadOutputs as O;
            match r.read_outputs().unwrap() {
                O::Translations(v) => {
                    let v: Vec<Vec3> = v.map(Vec3::from).collect();
                    trs[node].0 = v[k].lerp(v[k2], f);
                }
                O::Rotations(v) => {
                    let v: Vec<Quat> = v.into_f32().map(Quat::from_array).collect();
                    trs[node].1 = v[k].slerp(v[k2], f);
                }
                O::Scales(v) => {
                    let v: Vec<Vec3> = v.map(Vec3::from).collect();
                    trs[node].2 = v[k].lerp(v[k2], f);
                }
                _ => {}
            }
        }
    }
    let mut world = vec![Mat4::IDENTITY; trs.len()];
    let scene = doc.default_scene().unwrap();
    let mut stack: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, Mat4::IDENTITY)).collect();
    while let Some((n, parent)) = stack.pop() {
        let (t, r, s) = trs[n.index()];
        world[n.index()] = parent * Mat4::from_scale_rotation_translation(s, r, t);
        stack.extend(n.children().map(|c| (c, world[n.index()])));
    }

    let mut out = Vec::new();
    for n in doc.nodes() {
        let Some(mesh) = n.mesh() else { continue };
        let skin = n.skin().map(|s| {
            let ibm: Vec<Mat4> = s
                .reader(buf)
                .read_inverse_bind_matrices()
                .unwrap()
                .map(|m| Mat4::from_cols_array_2d(&m))
                .collect();
            let joints: Vec<Mat4> = s.joints().zip(ibm).map(|(j, m)| world[j.index()] * m).collect();
            joints
        });
        for p in mesh.primitives() {
            let r = p.reader(buf);
            let pos: Vec<Vec3> = r.read_positions().unwrap().map(Vec3::from).collect();
            let posed: Vec<Vec3> = match (&skin, time) {
                (Some(joints), Some(_)) => {
                    let js: Vec<[u16; 4]> = r.read_joints(0).unwrap().into_u16().collect();
                    let ws: Vec<[f32; 4]> = r.read_weights(0).unwrap().into_f32().collect();
                    pos.iter()
                        .enumerate()
                        .map(|(i, &v)| {
                            (0..4)
                                .map(|k| joints[js[i][k] as usize].transform_point3(v) * ws[i][k])
                                .sum()
                        })
                        .collect()
                }
                _ => pos
                    .iter()
                    .map(|&v| world[n.index()].transform_point3(v))
                    .collect(),
            };
            let idx: Vec<u32> = r.read_indices().unwrap().into_u32().collect();
            for t in idx.chunks(3) {
                out.push([posed[t[0] as usize], posed[t[1] as usize], posed[t[2] as usize]]);
            }
        }
    }
    out
}

/// Largest distance from any corner of one set to the nearest corner of the other.
fn hausdorff(a: &[[Vec3; 3]], b: &[[Vec3; 3]]) -> f32 {
    let one_way = |x: &[[Vec3; 3]], y: &[[Vec3; 3]]| {
        x.iter()
            .flatten()
            .map(|p| {
                y.iter()
                    .flatten()
                    .map(|q| p.distance(*q))
                    .fold(f32::MAX, f32::min)
            })
            .fold(0.0, f32::max)
    };
    one_way(a, b).max(one_way(b, a))
}

/// Signed volume (sign follows the winding) and area of a triangle set.
fn volume_area(t: &[[Vec3; 3]]) -> (f32, f32) {
    t.iter().fold((0.0, 0.0), |(v, a), [p, q, r]| {
        (
            v + p.dot(q.cross(*r)) / 6.0,
            a + (*q - *p).cross(*r - *p).length() / 2.0,
        )
    })
}

fn assert_same_shape(ours: &[[Vec3; 3]], reference: &[[Vec3; 3]], what: &str) {
    assert_eq!(ours.len(), reference.len(), "{what}: triangle count");
    let d = hausdorff(ours, reference);
    assert!(d < 1e-4, "{what}: corners differ by up to {d} m");
    let (v1, a1) = volume_area(ours);
    let (v2, a2) = volume_area(reference);
    assert!(
        (v1 - v2).abs() < 1e-4,
        "{what}: signed volume {v1} vs {v2} (winding)"
    );
    assert!((a1 - a2).abs() < 1e-4, "{what}: area {a1} vs {a2}");
}

fn json_of(glb: &[u8]) -> Value {
    let g = gltf::Gltf::from_slice(glb).unwrap();
    serde_json::to_value(g.document.into_json()).unwrap()
}

#[test]
fn static_meshes_match_blenders_own_gltf() {
    let c = convert("crate.fbx");
    assert_eq!(c.summary.fbx_version, 7400);
    assert!(c.summary.binary);
    assert_eq!(
        (c.summary.nodes, c.summary.meshes, c.summary.triangles),
        (2, 2, 60)
    );
    assert_same_shape(
        &triangles(&c.glb, None),
        &triangles(&fixture("crate.glb"), None),
        "crate",
    );

    // Materials: the textured one carries the image, the plain one its color.
    let j = json_of(&c.glb);
    let mats = j["materials"].as_array().unwrap();
    let wood = mats.iter().find(|m| m["name"] == "CrateWood").unwrap();
    assert_eq!(
        wood["pbrMetallicRoughness"]["baseColorTexture"]["index"],
        json!(0)
    );
    let paint = mats.iter().find(|m| m["name"] == "CratePaint").unwrap();
    let reference = json_of(&fixture("crate.glb"));
    let ref_paint = reference["materials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "CratePaint")
        .unwrap();
    for k in 0..3 {
        let a = paint["pbrMetallicRoughness"]["baseColorFactor"][k]
            .as_f64()
            .unwrap();
        let b = ref_paint["pbrMetallicRoughness"]["baseColorFactor"][k]
            .as_f64()
            .unwrap();
        assert!((a - b).abs() < 1e-3, "base color {k}: {a} vs {b}");
    }
    assert_eq!(j["images"][0]["mimeType"], json!("image/png"));

    // The engine's own loader reads it: two parts per material split, one image.
    let dir = std::env::temp_dir().join(format!("marionette-fbx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("crate.glb");
    std::fs::write(&path, &c.glb).unwrap();
    let model = marionette_assets::load_gltf(&path).unwrap();
    assert_eq!(model.images.len(), 1);
    assert_eq!(model.parts.len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn embedded_textures_are_used() {
    let c = convert("crate_embedded.fbx");
    assert_eq!(c.summary.images, 1);
    assert!(c.notes.iter().any(|n| n.contains("test")), "{:?}", c.notes);
    // Without a resolver answer, the embedded bytes still reach the resolver.
    let mut seen = Vec::new();
    let mut probe = |r: &TextureRequest| {
        seen.push(r.embedded.is_some());
        Err("probe".to_string())
    };
    let c = fbx::convert(&fixture("crate_embedded.fbx"), Default::default(), &mut probe).unwrap();
    assert_eq!(seen, vec![true]);
    assert_eq!(c.summary.images, 0);
    assert!(c.notes.iter().any(|n| n.contains("left out: probe")));
}

#[test]
fn other_axis_systems_and_units_give_the_same_world() {
    let c = convert("crate_zup_units.fbx");
    assert_eq!(c.summary.axes, "+X/+Z/-Y");
    assert_eq!(c.summary.meters_per_unit, 1.0);
    let ours = triangles(&c.glb, None);
    // Blender re-imported its glTF into its default scene, which has no extra meshes; compare
    // the crate's triangles only (the first 60).
    assert_same_shape(&ours, &triangles(&fixture("crate.glb"), None), "Z-up crate");
}

#[test]
fn unreal_convention_fbx_lands_in_meters_y_up() {
    // Unreal Engine's convention: +Z up, 1 unit = 1 cm (UnitScaleFactor 1). The fixture is the
    // same crate written that way (tests/fixtures/unreal/make_fixtures.py, decision 0029).
    let c = convert("../unreal/crate_unreal_cm.fbx");
    assert!(c.summary.axes.contains("+Z"), "{}", c.summary.axes);
    assert!((c.summary.meters_per_unit - 0.01).abs() < 1e-9);
    let ours = triangles(&c.glb, None);
    assert_same_shape(
        &ours,
        &triangles(&fixture("crate.glb"), None),
        "Unreal-convention crate",
    );
}

#[test]
fn fbx_7500_from_a_second_writer_matches_blenders_reading() {
    // Assimp writes FBX 7.5 (64-bit node headers) with its own conventions.
    let c = convert("crate_assimp.fbx");
    assert_eq!(c.summary.fbx_version, 7500);
    assert_same_shape(
        &triangles(&c.glb, None),
        &triangles(&fixture("crate_assimp_blender.glb"), None),
        "assimp crate",
    );
}

#[test]
fn skinned_animation_matches_blenders_own_gltf() {
    let c = convert("bend.fbx");
    let s = &c.summary;
    assert_eq!((s.skins, s.joints, s.animations), (1, 3, 1));
    let j = json_of(&c.glb);
    // Blender names the animation stack after the scene.
    assert_eq!(j["animations"][0]["name"], json!("Scene"));
    let ours = &c.glb;
    let reference = fixture("bend.glb");
    // Bind pose, unskinned.
    assert_same_shape(&triangles(ours, None), &triangles(&reference, None), "bind pose");
    // Blender's glTF keeps the start frame (1 at 24 fps) as time 1/24; ours starts the clip at 0.
    let start = 1.0 / 24.0;
    for t in [0.0, 0.1, 5.0 / 24.0, 0.3, 10.0 / 24.0] {
        assert_same_shape(
            &triangles(ours, Some(t)),
            &triangles(&reference, Some(t + start)),
            &format!("pose at {t} s"),
        );
    }
    // The pose really changes over the clip (the test would pass trivially otherwise).
    let mid = triangles(ours, Some(5.0 / 24.0));
    let rest = triangles(ours, Some(0.0));
    assert!(hausdorff(&mid, &rest) > 0.5);
}

/// Writes a node tree as ASCII FBX (test helper; the format of FBX 7.x ASCII files).
fn to_ascii(doc: &tree::Document) -> String {
    use tree::Prop as P;
    fn num(p: &P) -> String {
        match p {
            P::F32(v) => format!("{v:?}"),
            P::F64(v) => format!("{v:?}"),
            p => p.as_i64().unwrap().to_string(),
        }
    }
    fn b64(b: &[u8]) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in b.chunks(3) {
            let n =
                (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for k in 0..4 {
                if k <= c.len() {
                    s.push(A[(n >> (18 - 6 * k) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }
    fn node(n: &tree::Node, depth: usize, out: &mut String) {
        let pad = "\t".repeat(depth);
        let mut vals = Vec::new();
        for p in &n.props {
            vals.push(match p {
                P::Str(s) => format!("\"{}\"", s.replace('"', "&quot;")),
                P::Raw(b) => format!("\"{}\"", b64(b)),
                P::Bool(b) => (*b as u8).to_string(),
                P::BoolArr(_) | P::I32Arr(_) | P::I64Arr(_) | P::F32Arr(_) | P::F64Arr(_) => {
                    let v: Vec<String> = match p {
                        P::F32Arr(v) => v.iter().map(|x| format!("{x:?}")).collect(),
                        P::F64Arr(v) => v.iter().map(|x| format!("{x:?}")).collect(),
                        p => p.i64s().unwrap().iter().map(i64::to_string).collect(),
                    };
                    format!("*{} {{\n{pad}\ta: {}\n{pad}}}", v.len(), v.join(","))
                }
                p => num(p),
            });
        }
        out.push_str(&format!("{pad}{}: {}", n.name, vals.join(", ")));
        if !n.children.is_empty() {
            out.push_str(" {\n");
            for c in &n.children {
                node(c, depth + 1, out);
            }
            out.push_str(&format!("{pad}}}"));
        }
        out.push('\n');
    }
    let mut out = format!(
        "; FBX {}.{}.0 project file\n",
        doc.version / 1000,
        doc.version % 1000 / 100
    );
    for n in &doc.roots {
        node(n, 0, &mut out);
    }
    out
}

#[test]
fn ascii_fbx_converts_like_binary() {
    for name in ["crate.fbx", "bend.fbx", "crate_embedded.fbx"] {
        let doc = tree::parse(&fixture(name), Default::default()).unwrap();
        let text = to_ascii(&doc);
        assert!(text.contains("FBXHeaderExtension"));
        let back = tree::parse(text.as_bytes(), Default::default()).unwrap();
        assert!(!back.binary);
        assert_eq!(back.version, doc.version);
        let a = fbx::convert(&fixture(name), Default::default(), &mut beside).unwrap();
        let b = fbx::convert(text.as_bytes(), Default::default(), &mut beside).unwrap();
        assert_eq!(a.summary.triangles, b.summary.triangles, "{name}");
        assert_eq!(a.summary.images, b.summary.images, "{name}");
        assert_same_shape(&triangles(&a.glb, None), &triangles(&b.glb, None), name);
        if a.summary.animations > 0 {
            assert_same_shape(&triangles(&a.glb, Some(0.2)), &triangles(&b.glb, Some(0.2)), name);
        }
    }
}

#[test]
fn conversion_is_deterministic() {
    let a = convert("bend.fbx");
    let b = convert("bend.fbx");
    assert_eq!(a.glb, b.glb);
}

#[test]
fn broken_files_are_errors_not_panics() {
    let good = fixture("bend.fbx");
    // Truncated at many points.
    for cut in (0..good.len()).step_by(211) {
        let _ = fbx::convert(&good[..cut], Default::default(), &mut beside);
    }
    // Bytes overwritten at many points (deterministic pattern).
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..300 {
        let mut bad = good.clone();
        for _ in 0..4 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let i = (x as usize) % bad.len();
            bad[i] = (x >> 32) as u8;
        }
        let _ = fbx::convert(&bad, Default::default(), &mut beside);
    }
    // Old FBX versions get a clear message.
    let mut old = good.clone();
    old[23..27].copy_from_slice(&6100u32.to_le_bytes());
    let e = fbx::convert(&old, Default::default(), &mut beside).unwrap_err();
    assert!(e.contains("FBX 2011"), "{e}");
    // A huge declared array is refused by the limit, not allocated.
    let small = tree::TreeLimits {
        max_array_bytes: 1024,
        ..Default::default()
    };
    let e = fbx::convert(&good, small, &mut beside).unwrap_err();
    assert!(e.contains("limit"), "{e}");
}

// ---------------------------------------------------------------------------------------------
// Through the import pipeline (license rules apply to referenced textures).

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-fbx-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Stands in for the person who bought the asset (tests may write declarations; tools may not).
fn declare(path: &Path, allowed: &str, covers: &[&str]) {
    let d = json!({
        "format": "marionette.license_declaration/1",
        "store": "Example Store",
        "product": "Example Pack",
        "license_type": "Standard single-user license",
        "other_engines_allowed": allowed,
        "covers": covers,
    });
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(&d).unwrap()).unwrap();
}

fn put(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn unity_project_fbx_is_converted_with_its_texture() {
    let base = temp("unity");
    let project = base.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let lib = Library::at(&base.join("owned"), &project, ConfiguredFrom::Input).unwrap();
    let src = base.join("Game");
    put(
        &src.join("ProjectSettings/ProjectVersion.txt"),
        b"m_EditorVersion: 6000.0.0f1\n",
    );
    put(&src.join("Assets/Pack/Models/crate.fbx"), &fixture("crate.fbx"));
    put(&src.join("Assets/Pack/Models/bend.fbx"), &fixture("bend.fbx"));
    // Unity keeps textures in a Textures folder beside the model folder.
    put(
        &src.join("Assets/Pack/Textures/checker.png"),
        &fixture("checker.png"),
    );
    let decl = base.join("pack.license.json");
    declare(&decl, "yes", &["Assets/Pack/"]);

    let mut o = ImportOptions::new(&src);
    o.declarations = vec![decl];
    let r = import(&lib, &o).unwrap();
    assert_eq!(r.importer, "unity_project");
    assert!(r.refused.is_empty(), "{:#?}", r.refused);
    let get = |p: &str| r.imported.iter().find(|a| a.path == p).unwrap();
    let c = get("Assets/Pack/Models/crate.fbx");
    assert_eq!(c.kind, AssetKind::Model);
    assert!(c.engine_ready, "{:?}", c.notes);
    assert!(
        c.notes
            .iter()
            .any(|n| n.contains("bundled from `Assets/Pack/Textures/checker.png`")),
        "{:?}",
        c.notes
    );
    let dir = lib.asset_dir(&c.id);
    assert!(dir.join("source.fbx").is_file());
    let rec = lib.get(&c.id).unwrap();
    assert_eq!(rec.main.as_deref(), Some("model.glb"));
    assert_eq!(rec.license.status, "declared");
    let model = marionette_assets::load_gltf(&dir.join("model.glb")).unwrap();
    assert_eq!(model.images.len(), 1);

    let b = get("Assets/Pack/Models/bend.fbx");
    assert!(b.engine_ready, "{:?}", b.notes);
    assert!(
        b.notes
            .iter()
            .any(|n| n.contains("1 skins (3 joints), 1 animations"))
    );
    // No declaration file was written by the import.
    let written: Vec<_> = walk(&base.join("owned"))
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with(".license.json"))
        .collect();
    assert!(written.is_empty(), "{written:?}");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
fn textures_outside_the_models_license_are_not_bundled() {
    let base = temp("license");
    let project = base.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let lib = Library::at(&base.join("owned"), &project, ConfiguredFrom::Input).unwrap();
    let src = base.join("bought");
    put(&src.join("models/crate.fbx"), &fixture("crate.fbx"));
    put(&src.join("models/checker.png"), &fixture("checker.png"));
    // The model is declared; its texture is covered by a declaration that forbids other engines.
    let yes = base.join("models.license.json");
    let no = base.join("texture.license.json");
    declare(&yes, "yes", &["models/"]);
    declare(&no, "no", &["models/checker.png"]);
    let mut o = ImportOptions::new(&src);
    o.declarations = vec![yes, no];
    let r = import(&lib, &o).unwrap();
    assert_eq!(r.refused.len(), 1, "{:#?}", r.refused);
    assert_eq!(r.refused[0].path, "models/checker.png");
    let c = r.imported.iter().find(|a| a.path == "models/crate.fbx").unwrap();
    assert!(c.engine_ready);
    assert!(
        c.notes
            .iter()
            .any(|n| n.contains("texture `checker.png` left out")
                && n.contains("import.other_engines_not_allowed")),
        "{:?}",
        c.notes
    );
    let model = marionette_assets::load_gltf(&lib.asset_dir(&c.id).join("model.glb")).unwrap();
    assert!(model.images.is_empty());

    // Unconvertible bytes keep the old behaviour: kept as source.fbx, not engine-ready.
    let base2 = temp("broken");
    let project2 = base2.join("project");
    std::fs::create_dir_all(&project2).unwrap();
    let lib2 = Library::at(&base2.join("owned"), &project2, ConfiguredFrom::Input).unwrap();
    let f = base2.join("broken.fbx");
    put(&f, b"Kaydara FBX Binary  \0");
    declare(&base2.join("broken.fbx.license.json"), "yes", &[]);
    let r = import(&lib2, &ImportOptions::new(&f)).unwrap();
    let a = &r.imported[0];
    assert!(!a.engine_ready);
    assert!(
        a.notes.iter().any(|n| n.contains("could not be converted")),
        "{:?}",
        a.notes
    );
    assert!(lib2.asset_dir(&a.id).join("source.fbx").is_file());
}
