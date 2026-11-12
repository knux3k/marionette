//! The import pipeline: read a source, apply every license rule, bring in what is allowed and
//! report everything else with a reason (ASSETS-AND-LICENSES.md rules 6 to 13).

use crate::codes;
use crate::library::{AssetKind, AssetRecord, FileRecord, Library, RECORD_FORMAT, SourceRecord, slug};
use crate::license::{self, LicenseRecord, LoadedDeclaration, YesNo};
use crate::sha256_hex;
use crate::tar::Limits;
use crate::unity::{self, BadEntry, MetalLayout, SourceEntry};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What to import and how.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// A `.unitypackage`, a Unity project folder, or a plain owned file or folder.
    pub source: PathBuf,
    /// Declaration files to use, in addition to `<source>.license.json` next to the source.
    pub declarations: Vec<PathBuf>,
    /// Report what would happen without writing to the library.
    pub dry_run: bool,
    pub limits: Limits,
}

impl ImportOptions {
    pub fn new(source: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            declarations: Vec::new(),
            dry_run: false,
            limits: Limits::default(),
        }
    }
}

/// A declaration used by an import.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DeclarationSummary {
    pub file: String,
    pub store: String,
    pub product: String,
    pub license_type: String,
    pub other_engines_allowed: bool,
    pub covers: Vec<String>,
}

/// An asset brought into (or already in) the library.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ImportedAsset {
    pub id: String,
    pub kind: AssetKind,
    /// Path inside the source.
    pub path: String,
    pub engine_ready: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// A refused or skipped file, with its reason.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NotImported {
    /// Path inside the source.
    pub path: String,
    /// Stable reason code (`import.*`).
    pub code: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Counts {
    pub imported: usize,
    pub already_in_library: usize,
    pub refused: usize,
    pub skipped: usize,
}

/// The import report (rule 12: every refused asset is listed with its reason).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ImportReport {
    /// File or folder name of the source.
    pub source_name: String,
    /// `unitypackage`, `unity_project` or `files`.
    pub importer: String,
    pub dry_run: bool,
    pub declarations: Vec<DeclarationSummary>,
    pub counts: Counts,
    pub imported: Vec<ImportedAsset>,
    pub already_in_library: Vec<ImportedAsset>,
    /// Refused for license or engine-content reasons.
    pub refused: Vec<NotImported>,
    /// Not imported for other reasons (unsupported type, binary serialization, ...).
    pub skipped: Vec<NotImported>,
}

enum Class {
    Texture,
    Model,
    Material,
    Companion,
    Unsupported(&'static str),
}

fn ext_of(rel: &str) -> String {
    rel.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn classify(rel: &str) -> Class {
    match ext_of(rel).as_str() {
        "png" | "jpg" | "jpeg" | "tga" | "psd" | "tif" | "tiff" | "exr" | "hdr" | "bmp" | "gif" => {
            Class::Texture
        }
        "gltf" | "glb" | "fbx" | "obj" | "blend" | "dae" | "3ds" | "max" => Class::Model,
        "mat" => Class::Material,
        "bin" => Class::Companion,
        "cs" | "js" | "dll" | "shader" | "shadergraph" | "hlsl" | "cginc" | "compute" => {
            Class::Unsupported("code and shaders are not imported")
        }
        "unity" | "prefab" | "asset" | "controller" | "anim" | "overridecontroller" | "mask"
        | "physicmaterial" => {
            Class::Unsupported("Unity scenes, prefabs and editor data are not imported yet")
        }
        "wav" | "ogg" | "mp3" | "aif" | "aiff" | "flac" => {
            Class::Unsupported("audio import is not built yet")
        }
        _ => Class::Unsupported("file type not imported"),
    }
}

fn stem(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).to_string()
}

fn read(path: &Path, rel: &str) -> Result<Vec<u8>, Errors> {
    std::fs::read(path).map_err(|e| {
        Errors::from(Diagnostic::error(codes::IMPORT_SOURCE, format!("cannot read `{rel}`: {e}")).file(rel))
    })
}

/// Files of one asset about to be written.
struct Pending {
    kind: AssetKind,
    name: String,
    rel: String,
    guid: Option<String>,
    original_sha: String,
    files: Vec<(String, Vec<u8>)>,
    main: Option<String>,
    engine_ready: bool,
    notes: Vec<String>,
    uses: Vec<String>,
    /// glTF models are validated after writing.
    validate_gltf: bool,
}

struct Ctx<'a> {
    library: &'a Library,
    importer: &'static str,
    source_name: String,
    dry_run: bool,
    report: ImportReport,
}

impl Ctx<'_> {
    fn id_for(p: &Pending) -> String {
        format!("{}-{}", slug(&p.name), &p.original_sha[..12])
    }

    /// Writes the asset (files first, record last) unless it is already present.
    fn put(&mut self, mut p: Pending, decl: &LoadedDeclaration) -> Result<String, Errors> {
        let id = Self::id_for(&p);
        let dir = self.library.asset_dir(&id);
        if !self.dry_run && dir.join("record.json").is_file() {
            let existing = self.library.get(&id)?;
            self.report.already_in_library.push(ImportedAsset {
                id: id.clone(),
                kind: existing.kind,
                path: p.rel,
                engine_ready: existing.engine_ready,
                notes: Vec::new(),
            });
            return Ok(id);
        }
        let mut files = Vec::new();
        for (name, bytes) in &p.files {
            files.push(FileRecord {
                path: name.clone(),
                sha256: sha256_hex(bytes),
                bytes: bytes.len() as u64,
            });
            if !self.dry_run {
                self.library.write(&dir.join(name), bytes)?;
            }
        }
        if p.validate_gltf {
            if self.dry_run {
                p.notes.push("glTF not validated in a dry run".into());
            } else if let Some(main) = &p.main
                && let Err(e) = marionette_assets::load_gltf(&dir.join(main))
            {
                p.engine_ready = false;
                p.notes.push(format!("the engine could not load the glTF: {e}"));
            }
        }
        let record = AssetRecord {
            format: RECORD_FORMAT.into(),
            id: id.clone(),
            kind: p.kind,
            name: p.name.clone(),
            license: LicenseRecord::from_declaration(decl),
            source: SourceRecord {
                importer: self.importer.into(),
                source_name: self.source_name.clone(),
                path: p.rel.clone(),
                unity_guid: p.guid.clone(),
                sha256: p.original_sha.clone(),
            },
            files,
            main: p.main.clone(),
            engine_ready: p.engine_ready,
            private: true,
            pool: "blocked".into(),
            notes: p.notes.clone(),
            uses: p.uses.clone(),
        };
        if !self.dry_run {
            let text = serde_json::to_string_pretty(&record).expect("record serializes");
            self.library.write(&dir.join("record.json"), text.as_bytes())?;
        }
        self.report.imported.push(ImportedAsset {
            id: id.clone(),
            kind: p.kind,
            path: p.rel,
            engine_ready: p.engine_ready,
            notes: p.notes,
        });
        Ok(id)
    }

    fn refuse(&mut self, rel: &str, code: &str, reason: impl Into<String>) {
        self.report.refused.push(NotImported {
            path: rel.into(),
            code: code.into(),
            reason: reason.into(),
        });
    }

    fn skip(&mut self, rel: &str, code: &str, reason: impl Into<String>) {
        self.report.skipped.push(NotImported {
            path: rel.into(),
            code: code.into(),
            reason: reason.into(),
        });
    }
}

fn texture(entry: &SourceEntry, bytes: Vec<u8>) -> Pending {
    let ext = ext_of(&entry.rel);
    let mut notes = Vec::new();
    if let Some(meta) = &entry.meta {
        if unity::meta_value(meta, "textureType") == Some("1") {
            notes.push("usage: normal map".into());
        }
        if unity::meta_value(meta, "sRGBTexture") == Some("0") {
            notes.push("color space: linear (not sRGB)".into());
        }
    }
    let original_sha = sha256_hex(&bytes);
    let (files, main, ready) = match ext.as_str() {
        "png" => match marionette_assets::decode_png(&bytes) {
            Ok(_) => (
                vec![("texture.png".to_string(), bytes)],
                Some("texture.png"),
                true,
            ),
            Err(e) => {
                notes.push(format!("PNG could not be decoded: {e}"));
                (
                    vec![("texture.png".to_string(), bytes)],
                    Some("texture.png"),
                    false,
                )
            }
        },
        "jpg" | "jpeg" | "tga" => {
            let format = if ext == "tga" {
                image::ImageFormat::Tga
            } else {
                image::ImageFormat::Jpeg
            };
            match image::load_from_memory_with_format(&bytes, format) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let png = marionette_assets::encode_png(rgba.width(), rgba.height(), rgba.as_raw());
                    notes.push(format!(
                        "converted from {ext} to PNG; original kept as source.{ext}"
                    ));
                    (
                        vec![(format!("source.{ext}"), bytes), ("texture.png".to_string(), png)],
                        Some("texture.png"),
                        true,
                    )
                }
                Err(e) => {
                    notes.push(format!("{ext} could not be decoded: {e}"));
                    (vec![(format!("source.{ext}"), bytes)], None, false)
                }
            }
        }
        _ => {
            notes.push(format!(
                "{ext} is kept as source.{ext}; convert it to PNG before the engine can use it"
            ));
            (vec![(format!("source.{ext}"), bytes)], None, false)
        }
    };
    Pending {
        kind: AssetKind::Texture,
        name: stem(&entry.rel),
        rel: entry.rel.clone(),
        guid: entry.guid(),
        original_sha,
        files,
        main: main.map(str::to_string),
        engine_ready: ready,
        notes,
        uses: Vec::new(),
        validate_gltf: false,
    }
}

/// Resolves a relative glTF URI against the model's folder inside the source.
fn resolve_uri(model_rel: &str, uri: &str) -> Option<String> {
    if uri.contains(':') || uri.starts_with('/') {
        return None;
    }
    let uri = uri.replace("%20", " ");
    let mut parts: Vec<&str> = model_rel.split('/').collect();
    parts.pop();
    for c in uri.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            c => parts.push(c),
        }
    }
    Some(parts.join("/"))
}

fn model(
    entry: &SourceEntry,
    bytes: Vec<u8>,
    by_rel: &BTreeMap<String, &SourceEntry>,
) -> Result<Pending, Errors> {
    let ext = ext_of(&entry.rel);
    let original_sha = sha256_hex(&bytes);
    let mut notes = Vec::new();
    let mut p = Pending {
        kind: AssetKind::Model,
        name: stem(&entry.rel),
        rel: entry.rel.clone(),
        guid: entry.guid(),
        original_sha,
        files: Vec::new(),
        main: None,
        engine_ready: false,
        notes: Vec::new(),
        uses: Vec::new(),
        validate_gltf: false,
    };
    match ext.as_str() {
        "glb" => {
            p.files.push(("model.glb".into(), bytes));
            p.main = Some("model.glb".into());
            p.engine_ready = true;
            p.validate_gltf = true;
        }
        "gltf" => {
            let mut doc: Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    notes.push(format!("glTF JSON is invalid: {e}"));
                    p.files.push(("source.gltf".into(), bytes));
                    p.notes = notes;
                    return Ok(p);
                }
            };
            let mut ready = true;
            let mut deps: Vec<(String, Vec<u8>)> = Vec::new();
            for list in ["buffers", "images"] {
                let Some(items) = doc.get_mut(list).and_then(Value::as_array_mut) else {
                    continue;
                };
                for item in items {
                    let Some(uri) = item.get("uri").and_then(Value::as_str).map(str::to_string) else {
                        continue;
                    };
                    if uri.starts_with("data:") {
                        continue;
                    }
                    let found = resolve_uri(&entry.rel, &uri)
                        .and_then(|r| by_rel.get(&r.to_ascii_lowercase()).copied());
                    let Some(dep) = found else {
                        ready = false;
                        notes.push(format!("referenced file `{uri}` is not in the source"));
                        continue;
                    };
                    let file = dep.rel.rsplit('/').next().unwrap_or("file").to_string();
                    let mut name = format!("deps/{file}");
                    if deps.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                        name = format!("deps/{}-{file}", deps.len());
                    }
                    deps.push((name.clone(), read(&dep.data, &dep.rel)?));
                    item["uri"] = Value::String(name);
                }
            }
            let text = serde_json::to_vec_pretty(&doc).expect("json");
            p.files.push(("model.gltf".into(), text));
            p.files.extend(deps);
            p.main = Some("model.gltf".into());
            p.engine_ready = ready;
            p.validate_gltf = ready;
        }
        _ => {
            notes.push(format!(
                "{ext} is kept as source.{ext}; it needs conversion to glTF before the engine can use it (converter not built yet, decision 0018)"
            ));
            p.files.push((format!("source.{ext}"), bytes));
            p.main = Some(format!("source.{ext}"));
        }
    }
    p.notes = notes;
    Ok(p)
}

/// Runs an import into `library`.
pub fn import(library: &Library, opts: &ImportOptions) -> Result<ImportReport, Errors> {
    let source = &opts.source;
    let source_name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !source.exists() {
        return Err(Diagnostic::error(
            codes::IMPORT_SOURCE,
            format!("source `{source_name}` does not exist"),
        )
        .fix("pass a .unitypackage, a Unity project folder, or an owned file or folder")
        .into());
    }
    let importer: &'static str = if source.is_file() && ext_of(&source_name) == "unitypackage" {
        "unitypackage"
    } else if source.is_dir() && unity::is_unity_project(source) {
        "unity_project"
    } else {
        "files"
    };

    // Declarations: explicit ones plus `<source>.license.json` beside the source.
    let mut decl_paths = opts.declarations.clone();
    let beside = source.with_file_name(format!("{source_name}.license.json"));
    if beside.is_file() {
        decl_paths.push(beside);
    }
    let mut decls: Vec<LoadedDeclaration> = Vec::new();
    for p in &decl_paths {
        let d = LoadedDeclaration::load(p)?;
        if !decls.iter().any(|x| x.sha256 == d.sha256) {
            decls.push(d);
        }
    }

    if !opts.dry_run {
        library.ensure()?;
    }

    // Unpacked archives go in the process's run folder: removed when `staging` drops, and by
    // the next engine start if this process dies first (decision 0029).
    let mut staging = None;
    let (entries, bad): (Vec<SourceEntry>, Vec<BadEntry>) = match importer {
        "unitypackage" => {
            let dir = marionette_host::temp_dir("import-stage").map_err(|e| {
                Errors::from(Diagnostic::error(
                    codes::IMPORT_SOURCE,
                    format!("cannot create a staging folder: {e}"),
                ))
            })?;
            let r = unity::stage_package(source, dir.path(), opts.limits)?;
            staging = Some(dir);
            r
        }
        "unity_project" => (unity::scan_project(source), Vec::new()),
        _ => (unity::scan_files(source), Vec::new()),
    };

    let mut ctx = Ctx {
        library,
        importer,
        source_name: source_name.clone(),
        dry_run: opts.dry_run,
        report: ImportReport {
            source_name: source_name.clone(),
            importer: importer.into(),
            dry_run: opts.dry_run,
            declarations: decls
                .iter()
                .map(|d| DeclarationSummary {
                    file: d.file_name.clone(),
                    store: d.declaration.store.clone(),
                    product: d.declaration.product.clone(),
                    license_type: d.declaration.license_type.clone(),
                    other_engines_allowed: d.declaration.other_engines_allowed == YesNo::Yes,
                    covers: d.declaration.covers.clone(),
                })
                .collect(),
            counts: Counts::default(),
            imported: Vec::new(),
            already_in_library: Vec::new(),
            refused: Vec::new(),
            skipped: Vec::new(),
        },
    };
    for b in bad {
        ctx.refuse(&b.rel, b.code, b.reason);
    }

    let package_reason = (importer == "unitypackage")
        .then(|| license::package_location_reason(source))
        .flatten();
    let by_rel: BTreeMap<String, &SourceEntry> =
        entries.iter().map(|e| (e.rel.to_ascii_lowercase(), e)).collect();

    // License gate for every entry; returns the declaration that allows it.
    let gate = |ctx: &mut Ctx, e: &SourceEntry| -> Option<usize> {
        if let Some(r) = license::engine_content_reason(&e.rel).filter(|_| importer != "files") {
            ctx.refuse(
                &e.rel,
                codes::REFUSED_ENGINE_CONTENT,
                format!("engine content, refused with no override: {r}"),
            );
            return None;
        }
        if let Some(r) = &package_reason {
            ctx.refuse(
                &e.rel,
                codes::REFUSED_ENGINE_CONTENT,
                format!("engine content, refused with no override: {r}"),
            );
            return None;
        }
        let best = decls
            .iter()
            .enumerate()
            .filter_map(|(i, d)| d.coverage(&e.rel).map(|c| (c, i)))
            .max_by_key(|(c, i)| (*c, std::cmp::Reverse(*i)))
            .map(|(_, i)| i);
        let Some(i) = best else {
            ctx.refuse(
                &e.rel,
                codes::REFUSED_LICENSE_UNKNOWN,
                "license unknown: no declaration covers this asset; a person must write one (an AI agent may not)",
            );
            return None;
        };
        let d = &decls[i].declaration;
        if let Some(p) = d.publisher.as_deref().filter(|p| license::is_engine_publisher(p)) {
            ctx.refuse(
                &e.rel,
                codes::REFUSED_ENGINE_CONTENT,
                format!("engine content, refused with no override: published by `{p}`"),
            );
            return None;
        }
        if d.other_engines_allowed == YesNo::No {
            ctx.refuse(
                &e.rel,
                codes::REFUSED_OTHER_ENGINES_NO,
                format!(
                    "declaration `{}` says use in other engines is not allowed",
                    decls[i].file_name
                ),
            );
            return None;
        }
        if let Some(text) = &d.license_text {
            let limits = license::engine_limits(text);
            if !limits.is_empty() {
                let phrases: Vec<String> = limits.iter().map(|l| format!("\"{}\"", l.phrase)).collect();
                ctx.refuse(
                    &e.rel,
                    codes::REFUSED_ENGINE_LOCKED_LICENSE,
                    format!(
                        "the pasted license text limits use to one engine: {}",
                        phrases.join(", ")
                    ),
                );
                return None;
            }
        }
        Some(i)
    };

    let mut materials: Vec<(&SourceEntry, usize)> = Vec::new();
    let mut by_guid: BTreeMap<String, String> = BTreeMap::new();
    for e in &entries {
        match classify(&e.rel) {
            Class::Unsupported(why) => ctx.skip(&e.rel, codes::SKIPPED_UNSUPPORTED, why),
            Class::Companion => ctx.skip(
                &e.rel,
                codes::SKIPPED_COMPANION,
                "glTF buffer; imported together with the model that uses it",
            ),
            Class::Material => {
                if let Some(i) = gate(&mut ctx, e) {
                    materials.push((e, i));
                }
            }
            class @ (Class::Texture | Class::Model) => {
                let Some(i) = gate(&mut ctx, e) else { continue };
                let bytes = read(&e.data, &e.rel)?;
                let p = if matches!(class, Class::Texture) {
                    texture(e, bytes)
                } else {
                    model(e, bytes, &by_rel)?
                };
                let id = ctx.put(p, &decls[i])?;
                if let Some(g) = e.guid() {
                    by_guid.insert(g, id);
                }
            }
        }
    }

    // Materials last, so their textures resolve to library ids (this import first, then any
    // earlier import that recorded the same Unity GUID).
    if !materials.is_empty() {
        for r in library.records() {
            if let Some(g) = r.source.unity_guid {
                by_guid.entry(g).or_insert(r.id);
            }
        }
    }
    for (e, i) in materials {
        let bytes = read(&e.data, &e.rel)?;
        let text = String::from_utf8_lossy(&bytes);
        let m = match unity::parse_material(&text) {
            Ok(m) => m,
            Err(why) => {
                ctx.skip(
                    &e.rel,
                    codes::SKIPPED_BINARY_SERIALIZED,
                    format!("material not read: {why}"),
                );
                continue;
            }
        };
        let pending = material(&ctx, e, &bytes, &m, &by_guid, &entries);
        ctx.put(pending, &decls[i])?;
    }

    let r = &mut ctx.report;
    r.counts = Counts {
        imported: r.imported.len(),
        already_in_library: r.already_in_library.len(),
        refused: r.refused.len(),
        skipped: r.skipped.len(),
    };
    drop(staging);
    Ok(ctx.report)
}

fn material(
    ctx: &Ctx,
    e: &SourceEntry,
    bytes: &[u8],
    m: &unity::UnityMaterial,
    by_guid: &BTreeMap<String, String>,
    entries: &[SourceEntry],
) -> Pending {
    let map = unity::map_pbr(m);
    let mut uses = Vec::new();
    let mut unresolved = Vec::new();
    let mut notes = Vec::new();
    let mut tex_ref = |slot: &str, guid: &Option<String>| -> Value {
        let Some(g) = guid else { return Value::Null };
        match by_guid.get(g) {
            Some(id) => {
                if !uses.contains(id) {
                    uses.push(id.clone());
                }
                json!({ "asset": id })
            }
            None => {
                unresolved.push(slot.to_string());
                Value::Null
            }
        }
    };
    let base = tex_ref("base_color_texture", &map.base_color_texture);
    let normal = tex_ref("normal_texture", &map.normal_texture);
    let mut occlusion = tex_ref("occlusion_texture", &map.occlusion_texture);
    let emissive = tex_ref("emissive_texture", &map.emissive_texture);

    // Unity packs metallic and smoothness differently from glTF; convert into this folder.
    let mut files = Vec::new();
    let mut metal_ref = Value::Null;
    if let Some((guid, layout)) = &map.metal_texture {
        match by_guid.get(guid) {
            Some(id) if !ctx.dry_run => {
                let png = ctx.library.asset_dir(id).join("texture.png");
                match std::fs::read(&png)
                    .map_err(|e| e.to_string())
                    .and_then(|b| marionette_assets::decode_png(&b))
                {
                    Ok(mut img) => {
                        unity::convert_metal_texture(&mut img.rgba, *layout, map.smoothness_scale);
                        files.push((
                            "metallic_roughness.png".to_string(),
                            marionette_assets::encode_png(img.width, img.height, &img.rgba),
                        ));
                        metal_ref = json!({ "file": "metallic_roughness.png" });
                        if !uses.contains(id) {
                            uses.push(id.clone());
                        }
                        if *layout == MetalLayout::HdrpMask && occlusion.is_null() {
                            occlusion = metal_ref.clone();
                        }
                        notes.push(
                            "metallic texture converted to glTF channel layout (B metallic, G roughness)"
                                .into(),
                        );
                    }
                    Err(err) => notes.push(format!("metallic texture not converted: {err}")),
                }
            }
            Some(_) => notes.push("metallic texture conversion skipped in a dry run".into()),
            None => unresolved.push("metallic_roughness_texture".into()),
        }
    }
    let shader = map.shader.map(str::to_string).or_else(|| {
        entries
            .iter()
            .find(|x| x.guid().as_deref() == Some(m.shader_guid.as_str()))
            .map(|x| format!("{} (from the source)", stem(&x.rel)))
    });
    match &shader {
        Some(s) => notes.push(format!("mapped from Unity shader `{s}`")),
        None => notes.push("shader not recognized; mapped by common property names".into()),
    }
    if !unresolved.is_empty() {
        notes.push(format!("textures not in the library: {}", unresolved.join(", ")));
    }
    if !map.unmapped_textures.is_empty() {
        notes.push(format!(
            "texture slots with no glTF equivalent: {}",
            map.unmapped_textures.join(", ")
        ));
    }
    notes.push("material.json is mapped data; the renderer does not load it yet".into());
    let doc = json!({
        "format": "marionette.material/1",
        "name": if m.name.is_empty() { stem(&e.rel) } else { m.name.clone() },
        "base_color_factor": map.base_color_factor,
        "base_color_texture": base,
        "metallic_factor": map.metallic_factor,
        "roughness_factor": map.roughness_factor,
        "metallic_roughness_texture": metal_ref,
        "normal_texture": normal,
        "normal_scale": map.normal_scale,
        "occlusion_texture": occlusion,
        "occlusion_strength": map.occlusion_strength,
        "emissive_factor": map.emissive_factor,
        "emissive_texture": emissive,
        "alpha_mode": map.alpha_mode,
        "alpha_cutoff": map.alpha_cutoff,
        "uv_scale": map.uv_scale,
        "uv_offset": map.uv_offset,
        "unity": {
            "shader": shader,
            "shader_guid": m.shader_guid,
            "unmapped_textures": map.unmapped_textures,
            "unresolved_textures": unresolved,
        }
    });
    files.insert(
        0,
        (
            "material.json".to_string(),
            serde_json::to_vec_pretty(&doc).expect("json"),
        ),
    );
    Pending {
        kind: AssetKind::Material,
        name: stem(&e.rel),
        rel: e.rel.clone(),
        guid: e.guid(),
        original_sha: sha256_hex(bytes),
        files,
        main: Some("material.json".into()),
        engine_ready: false,
        notes,
        uses,
        validate_gltf: false,
    }
}
