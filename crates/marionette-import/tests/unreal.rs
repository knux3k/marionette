//! Acceptance tests for the Unreal importer (workstream W6, decision 0029). Every fixture is
//! self-made: the project folder is built at run time, the packages are synthetic byte strings
//! (only the parts the diagnosis looks at), and the FBX is the crate from the FBX tests written
//! in Unreal's convention. Declaration files are written by the tests standing in for the person
//! who bought the asset.

use marionette_import::codes;
use marionette_import::library::{AssetKind, ConfiguredFrom, Library};
use marionette_import::{ImportOptions, ImportReport, import};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-unreal-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn setup(tag: &str) -> (PathBuf, Library) {
    let base = temp(tag);
    let project = base.join("game");
    std::fs::create_dir_all(&project).unwrap();
    let lib = Library::at(&base.join("owned"), &project, ConfiguredFrom::Input).unwrap();
    (base, lib)
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn declare(path: &Path, allowed: &str, extra: Value) {
    let mut d = json!({
        "format": "marionette.license_declaration/1",
        "store": "Fab",
        "product": "Example Crate Pack",
        "license_type": "Standard License",
        "other_engines_allowed": allowed,
    });
    for (k, v) in extra.as_object().unwrap() {
        d[k] = v.clone();
    }
    write(path, &serde_json::to_vec_pretty(&d).unwrap());
}

fn png(rgba: [u8; 4]) -> Vec<u8> {
    let px: Vec<u8> = (0..16).flat_map(|_| rgba).collect();
    marionette_assets::encode_png(4, 4, &px)
}

fn tga(rgba: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba(rgba));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Tga).unwrap();
    out.into_inner()
}

/// Bytes shaped like the start of an editor package: the tag, class names in the name table and
/// the import data's `RelativeFilename`. Not a real package; only what the diagnosis reads.
fn package(classes: &[&str], source: Option<&str>) -> Vec<u8> {
    let mut b = vec![0xC1, 0x83, 0x2A, 0x9E, 0xF7, 0xFF, 0xFF, 0xFF];
    for name in ["/Script/CoreUObject", "/Script/Engine", "Package"]
        .iter()
        .chain(classes)
    {
        b.extend(((name.len() + 1) as i32).to_le_bytes());
        b.extend(name.as_bytes());
        b.push(0);
    }
    if let Some(s) = source {
        b.extend(format!("[{{ \"RelativeFilename\" : \"{s}\", \"Timestamp\" : \"0\" }}]").as_bytes());
    }
    b
}

fn fbx_fixture() -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/unreal/crate_unreal_cm.fbx"))
        .unwrap()
}

const INSTANCE: &str = r#"Begin Object Class=/Script/Engine.MaterialInstanceConstant Name="MI_Crate" ExportPath="/Script/Engine.MaterialInstanceConstant'/Game/Props/MI_Crate.MI_Crate'"
   Parent=/Script/Engine.Material'"/Game/Materials/M_Master.M_Master"'
   ScalarParameterValues(0)=(ParameterInfo=(Name="Roughness"),ParameterValue=0.400000,ExpressionGUID=0)
   ScalarParameterValues(1)=(ParameterInfo=(Name="Normal Strength"),ParameterValue=0.800000,ExpressionGUID=0)
   VectorParameterValues(0)=(ParameterInfo=(Name="Tint"),ParameterValue=(R=1.000000,G=0.500000,B=0.250000,A=1.000000),ExpressionGUID=0)
   TextureParameterValues(0)=(ParameterInfo=(Name="BaseColor"),ParameterValue=/Script/Engine.Texture2D'"/Game/Props/T_Crate_D.T_Crate_D"',ExpressionGUID=0)
   TextureParameterValues(1)=(ParameterInfo=(Name="Normal"),ParameterValue=/Script/Engine.Texture2D'"/Game/Props/T_Crate_N.T_Crate_N"',ExpressionGUID=0)
   TextureParameterValues(2)=(ParameterInfo=(Name="ORM"),ParameterValue=/Script/Engine.Texture2D'"/Game/Props/T_Crate_ORM.T_Crate_ORM"',ExpressionGUID=0)
   TextureParameterValues(3)=(ParameterInfo=(Name="Detail"),ParameterValue=/Script/Engine.Texture2D'"/Game/Props/T_Detail.T_Detail"',ExpressionGUID=0)
   BasePropertyOverrides=(bOverride_BlendMode=True,BlendMode=BLEND_Masked,bOverride_TwoSided=True,TwoSided=True,OpacityMaskClipValue=0.400000)
End Object
"#;

const GRAPH: &str = r#"Begin Object Class=/Script/Engine.Material Name="M_Graph" ExportPath="/Script/Engine.Material'/Game/Props/M_Graph.M_Graph'"
   Begin Object Class=/Script/Engine.MaterialExpressionTextureSample Name="MaterialExpressionTextureSample_0"
   End Object
   Begin Object Class=/Script/Engine.MaterialExpressionTextureSample Name="MaterialExpressionTextureSample_1"
   End Object
   Begin Object Class=/Script/Engine.MaterialExpressionMultiply Name="MaterialExpressionMultiply_0"
   End Object
   Begin Object Class=/Script/Engine.MaterialEditorOnlyData Name="M_Graph_EditorOnlyData"
   End Object
   Begin Object Name="MaterialExpressionTextureSample_0"
      Texture=/Script/Engine.Texture2D'"/Game/Props/T_Crate_D.T_Crate_D"'
   End Object
   Begin Object Name="MaterialExpressionTextureSample_1"
      Texture=/Script/Engine.Texture2D'"/Game/Props/T_Crate_ORM.T_Crate_ORM"'
   End Object
   Begin Object Name="M_Graph_EditorOnlyData"
      BaseColor=(Expression=/Script/Engine.MaterialExpressionTextureSample'"MaterialExpressionTextureSample_0"')
      AmbientOcclusion=(Expression=/Script/Engine.MaterialExpressionTextureSample'"MaterialExpressionTextureSample_1"',OutputIndex=1)
      Roughness=(Expression=/Script/Engine.MaterialExpressionTextureSample'"MaterialExpressionTextureSample_1"',OutputIndex=2)
      Metallic=(Expression=/Script/Engine.MaterialExpressionTextureSample'"MaterialExpressionTextureSample_1"',OutputIndex=3)
      EmissiveColor=(Expression=/Script/Engine.MaterialExpressionMultiply'"MaterialExpressionMultiply_0"')
   End Object
   BlendMode=BLEND_Opaque
End Object
"#;

/// An Unreal project with own content, engine content, packages, a cooked file, code, an Epic
/// plugin and a seller's plugin.
fn project(base: &Path) -> PathBuf {
    let p = base.join("CrateGame");
    write(&p.join("CrateGame.uproject"), br#"{ "FileVersion": 3 }"#);
    write(
        &p.join("Content/Props/SM_Crate.uasset"),
        &package(
            &["StaticMesh", "MaterialInstanceConstant"],
            Some("../../Art/SM_Crate.fbx"),
        ),
    );
    write(
        &p.join("Content/Props/T_Crate_D.uasset"),
        &package(&["Texture2D"], Some("T_Crate_D.png")),
    );
    write(
        &p.join("Content/Props/MI_Crate.uasset"),
        &package(&["MaterialInstanceConstant", "Texture2D"], None),
    );
    write(&p.join("Content/Props/T_Crate_N.tga"), &tga([128, 64, 255, 255]));
    write(&p.join("Content/Props/T_Crate_ORM.png"), &png([255, 128, 0, 255]));
    write(
        &p.join("Content/StarterContent/Textures/T_Brick.png"),
        &png([200, 0, 0, 255]),
    );
    write(
        &p.join("Content/Characters/Mannequins/Meshes/SKM_Quinn.fbx"),
        &fbx_fixture(),
    );
    write(&p.join("Content/Paks/CrateGame-Windows.pak"), b"cooked");
    write(&p.join("Exports/T_Crate_D.png"), &png([10, 20, 30, 255]));
    write(&p.join("Exports/SM_Crate.fbx"), &fbx_fixture());
    write(&p.join("Exports/MI_Crate.T3D"), INSTANCE.as_bytes());
    write(&p.join("Exports/M_Graph.T3D"), GRAPH.as_bytes());
    write(
        &p.join("Exports/BP_Door.T3D"),
        b"Begin Object Class=/Script/Engine.Blueprint Name=\"BP_Door\"\nEnd Object\n",
    );
    write(&p.join("Source/CrateGame/CrateGame.cpp"), b"// code");
    write(&p.join("Saved/Autosaves/T_Old.png"), &png([1, 1, 1, 255]));
    write(&p.join("Intermediate/Junk.png"), &png([1, 1, 1, 255]));
    write(
        &p.join("Plugins/EpicTool/EpicTool.uplugin"),
        br#"{ "FriendlyName": "Epic Tool", "CreatedBy": "Epic Games, Inc." }"#,
    );
    write(
        &p.join("Plugins/EpicTool/Content/T_Epic.png"),
        &png([0, 0, 0, 255]),
    );
    write(
        &p.join("Plugins/SellerKit/SellerKit.uplugin"),
        br#"{ "FriendlyName": "Seller Kit", "CreatedBy": "Some Studio" }"#,
    );
    write(
        &p.join("Plugins/SellerKit/Content/Decals/T_Decal.png"),
        &png([0, 255, 0, 255]),
    );
    p
}

fn run(lib: &Library, source: &Path) -> ImportReport {
    import(lib, &ImportOptions::new(source)).unwrap()
}

fn find<'a>(
    list: &'a [marionette_import::import::NotImported],
    path: &str,
) -> &'a marionette_import::import::NotImported {
    list.iter()
        .find(|x| x.path == path)
        .unwrap_or_else(|| panic!("{path} not listed"))
}

fn material(lib: &Library, r: &ImportReport, path: &str) -> (String, Value) {
    let a = r.imported.iter().find(|a| a.path == path).unwrap();
    assert_eq!(a.kind, AssetKind::Material);
    let text = std::fs::read(lib.asset_dir(&a.id).join("material.json")).unwrap();
    (a.id.clone(), serde_json::from_slice(&text).unwrap())
}

fn id_of(r: &ImportReport, path: &str) -> String {
    r.imported
        .iter()
        .find(|a| a.path == path)
        .unwrap_or_else(|| panic!("{path} not imported"))
        .id
        .clone()
}

#[test]
fn unreal_project_imports_exports_and_refuses_engine_content() {
    let (base, lib) = setup("project");
    let src = project(&base);
    declare(
        &base.join("CrateGame.license.json"),
        "yes",
        json!({ "publisher": "Some Studio" }),
    );
    let r = run(&lib, &src);
    assert_eq!(r.importer, "unreal_project");

    // Generated folders are not even listed.
    let all: Vec<&str> = r
        .imported
        .iter()
        .map(|a| a.path.as_str())
        .chain(r.refused.iter().map(|x| x.path.as_str()))
        .chain(r.skipped.iter().map(|x| x.path.as_str()))
        .collect();
    assert!(
        !all.iter()
            .any(|p| p.starts_with("Saved/") || p.starts_with("Intermediate/"))
    );

    // Engine content, refused with no override even though a declaration covers everything.
    for p in [
        "Content/StarterContent/Textures/T_Brick.png",
        "Content/Characters/Mannequins/Meshes/SKM_Quinn.fbx",
        "Plugins/EpicTool/Content/T_Epic.png",
    ] {
        assert_eq!(find(&r.refused, p).code, codes::REFUSED_ENGINE_CONTENT, "{p}");
    }
    assert!(
        find(&r.refused, "Plugins/EpicTool/Content/T_Epic.png")
            .reason
            .contains(".uplugin")
    );

    // Packages are diagnosed, not read, and the reason says what to export.
    let mesh = find(&r.skipped, "Content/Props/SM_Crate.uasset");
    assert_eq!(mesh.code, codes::SKIPPED_UNREAL_PACKAGE);
    assert!(
        mesh.reason.contains("StaticMesh") && mesh.reason.contains("FBX"),
        "{}",
        mesh.reason
    );
    assert!(
        mesh.reason.contains("`SM_Crate.fbx`") && !mesh.reason.contains("Art/"),
        "{}",
        mesh.reason
    );
    assert!(
        find(&r.skipped, "Content/Props/T_Crate_D.uasset")
            .reason
            .contains("Texture2D")
    );
    assert!(
        find(&r.skipped, "Content/Props/MI_Crate.uasset")
            .reason
            .contains(".T3D")
    );
    assert_eq!(
        find(&r.skipped, "Content/Paks/CrateGame-Windows.pak").code,
        codes::SKIPPED_UNREAL_COOKED
    );
    assert!(
        find(&r.skipped, "Source/CrateGame/CrateGame.cpp")
            .reason
            .contains("port_scan")
    );
    assert!(
        find(&r.skipped, "Exports/BP_Door.T3D")
            .reason
            .contains("Blueprint")
    );

    // The FBX export (Z up, centimeters) is converted to glTF (Y up, meters).
    let fbx = r
        .imported
        .iter()
        .find(|a| a.path == "Exports/SM_Crate.fbx")
        .unwrap();
    assert!(fbx.engine_ready, "{:?}", fbx.notes);
    assert!(
        fbx.notes[0].contains("+Z") && fbx.notes[0].contains("0.01 m per unit"),
        "{}",
        fbx.notes[0]
    );

    // Textures: TGA converted, naming-convention notes, object paths recorded (plugins too).
    let n = r
        .imported
        .iter()
        .find(|a| a.path == "Content/Props/T_Crate_N.tga")
        .unwrap();
    assert!(
        n.engine_ready && n.notes.iter().any(|x| x.contains("DirectX")),
        "{:?}",
        n.notes
    );
    let decal = lib
        .get(&id_of(&r, "Plugins/SellerKit/Content/Decals/T_Decal.png"))
        .unwrap();
    assert_eq!(
        decal.source.unreal_path.as_deref(),
        Some("/sellerkit/decals/t_decal")
    );
    assert_eq!(decal.source.importer, "unreal_project");
    assert!(decal.private && decal.pool == "blocked");
    assert_eq!(decal.license.publisher.as_deref(), Some("Some Studio"));

    // Material instance: mapped by parameter names, textures resolved by object path or by a
    // unique exported file of the same name, normal map flipped to glTF convention.
    let d = id_of(&r, "Exports/T_Crate_D.png");
    let orm = id_of(&r, "Content/Props/T_Crate_ORM.png");
    let (mi, m) = material(&lib, &r, "Exports/MI_Crate.T3D");
    assert_eq!(m["base_color_texture"], json!({ "asset": d }));
    assert_eq!(
        m["base_color_factor"],
        json!([1.0, 0.5, 0.25, 1.0]),
        "linear, not converted"
    );
    assert_eq!(m["metallic_roughness_texture"], json!({ "asset": orm }));
    assert_eq!(m["occlusion_texture"], json!({ "asset": orm }));
    assert_eq!(m["roughness_factor"], json!(1.0));
    assert_eq!(m["normal_texture"], json!({ "file": "normal.png" }));
    assert!((m["normal_scale"].as_f64().unwrap() - 0.8).abs() < 1e-6);
    assert_eq!(m["alpha_mode"], "MASK");
    assert!((m["alpha_cutoff"].as_f64().unwrap() - 0.4).abs() < 1e-6);
    assert_eq!(m["double_sided"], json!(true));
    assert_eq!(m["unreal"]["parent"], "/game/materials/m_master");
    assert_eq!(m["unreal"]["unmapped"], json!(["parameter `Detail`"]));
    let flipped =
        marionette_assets::decode_png(&std::fs::read(lib.asset_dir(&mi).join("normal.png")).unwrap())
            .unwrap();
    assert_eq!(&flipped.rgba[..4], &[128, 191, 255, 255]);
    let rec = lib.get(&mi).unwrap();
    assert!(rec.uses.contains(&d) && rec.uses.contains(&orm));

    // Material graph: direct connections; ORM channels read as the glTF packing; the multiply
    // node feeding emissive is named in the notes.
    let (_, g) = material(&lib, &r, "Exports/M_Graph.T3D");
    assert_eq!(g["base_color_texture"], json!({ "asset": d }));
    assert_eq!(g["metallic_roughness_texture"], json!({ "asset": orm }));
    assert_eq!(g["occlusion_texture"], json!({ "asset": orm }));
    assert_eq!(g["alpha_mode"], "OPAQUE");
    let notes = &r
        .imported
        .iter()
        .find(|a| a.path == "Exports/M_Graph.T3D")
        .unwrap()
        .notes;
    assert!(
        notes.iter().any(|n| n.contains("MaterialExpressionMultiply")),
        "{notes:?}"
    );

    // Importing again is a no-op.
    let again = run(&lib, &src);
    assert!(again.imported.is_empty());
    assert_eq!(again.already_in_library.len(), r.imported.len());
}

#[test]
fn unreal_assets_follow_the_license_rules() {
    // No declaration: everything readable is refused as license unknown; nothing is written.
    let (base, lib) = setup("rules");
    let src = project(&base);
    let r = run(&lib, &src);
    assert!(r.imported.is_empty());
    assert_eq!(
        find(&r.refused, "Exports/SM_Crate.fbx").code,
        codes::REFUSED_LICENSE_UNKNOWN
    );
    assert!(lib.records().is_empty());

    // Declared as Epic content (for example a free Epic sample pack): refused, no override.
    declare(
        &base.join("CrateGame.license.json"),
        "yes",
        json!({ "publisher": "Epic Games" }),
    );
    let r = run(&lib, &src);
    assert!(r.imported.is_empty());
    assert_eq!(
        find(&r.refused, "Exports/T_Crate_D.png").code,
        codes::REFUSED_ENGINE_CONTENT
    );

    // A license limited to Unreal Engine (pasted text), even with a "yes" answer.
    declare(
        &base.join("CrateGame.license.json"),
        "yes",
        json!({ "license_text": "This content is licensed for use exclusively in Unreal Engine projects." }),
    );
    let r = run(&lib, &src);
    assert!(r.imported.is_empty());
    assert_eq!(
        find(&r.refused, "Exports/T_Crate_D.png").code,
        codes::REFUSED_ENGINE_LOCKED_LICENSE
    );

    // "No" to other engines.
    declare(&base.join("CrateGame.license.json"), "no", json!({}));
    let r = run(&lib, &src);
    assert!(r.imported.is_empty());
    assert_eq!(
        find(&r.refused, "Exports/T_Crate_D.png").code,
        codes::REFUSED_OTHER_ENGINES_NO
    );
    assert!(lib.records().is_empty());
}

#[test]
fn a_content_folder_from_a_vault_cache_is_an_unreal_source() {
    // The launcher's vault cache and Fab downloads hold just `Content/` (no .uproject).
    let (base, lib) = setup("vault");
    let src = base.join("CratePack");
    write(
        &src.join("Content/CratePack/Meshes/SM_Crate.uasset"),
        &package(&["StaticMesh"], None),
    );
    write(
        &src.join("Content/CratePack/Textures/T_Crate_D.png"),
        &png([9, 9, 9, 255]),
    );
    declare(&base.join("CratePack.license.json"), "yes", json!({}));
    let r = run(&lib, &src);
    assert_eq!(r.importer, "unreal_project");
    assert_eq!(r.imported.len(), 1);
    let rec = lib.get(&r.imported[0].id).unwrap();
    assert_eq!(
        rec.source.unreal_path.as_deref(),
        Some("/game/cratepack/textures/t_crate_d")
    );
    assert_eq!(
        find(&r.skipped, "Content/CratePack/Meshes/SM_Crate.uasset").code,
        codes::SKIPPED_UNREAL_PACKAGE
    );
}
