//! Reading Unreal sources (decision 0029): project, plugin and content folders (as the Unreal
//! Editor, the Epic launcher's vault cache or a Fab download leave them), the engine-content
//! rules for them, a diagnosis of `.uasset` packages (which are not read), and materials exported
//! from the editor as Unreal object text (`.t3d` / `.copy`).
//!
//! What is read: the plain files in the folder (FBX, glTF, OBJ, PNG, TGA, JPEG, BMP, ...), which
//! is what the editor's Export command, the glTF Exporter and Fab's "additional formats"
//! downloads produce, plus T3D material exports. What is not read: `.uasset` and `.umap`
//! packages (an undocumented binary format that changes with every engine version) and cooked
//! containers (`.pak`, `.utoc`, `.ucas`, often encrypted). For a package the importer names its
//! asset class and, when the package records it, the file it was imported from, so the report
//! can say exactly what to export.

use crate::license;
use crate::unity::SourceEntry;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

/// True when `dir` looks like an Unreal project (`*.uproject`), plugin (`*.uplugin`) or a
/// content folder (`Content/` holding packages, as a vault-cache or Fab download has).
pub fn is_unreal_source(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    if descriptor(dir, "uproject").is_some() || descriptor(dir, "uplugin").is_some() {
        return true;
    }
    let content = dir.join("Content");
    content.is_dir() && has_package(&content, 4)
}

/// The first `*.<ext>` file directly in `dir`, by name.
fn descriptor(dir: &Path, ext: &str) -> Option<std::path::PathBuf> {
    let mut found: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && ext_of(&p.to_string_lossy()) == ext)
        .collect();
    found.sort();
    found.into_iter().next()
}

fn has_package(dir: &Path, depth: usize) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 && has_package(&p, depth - 1) {
                return true;
            }
        } else if matches!(ext_of(&p.to_string_lossy()).as_str(), "uasset" | "umap") {
            return true;
        }
    }
    false
}

fn ext_of(rel: &str) -> String {
    rel.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn stem(rel: &str) -> &str {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name)
}

/// Folders the editor and build tools write and that hold nothing a person made: skipped.
const GENERATED_DIRS: [&str; 5] = ["binaries", "intermediate", "saved", "deriveddatacache", "build"];

/// The files of an Unreal source, plus subtrees refused as a whole (plugins made by Epic).
#[derive(Debug, Default)]
pub struct Scan {
    /// Every file, sorted by path; paths are relative to the source folder.
    pub entries: Vec<SourceEntry>,
    /// `(path prefix, reason)`: every file under the prefix is engine content.
    pub engine_prefixes: Vec<(String, String)>,
    /// Content mount points: `(path prefix of a Content folder, Unreal mount)`, longest first.
    /// `Content/` of a project mounts at `/Game`, a plugin's at `/<PluginName>`.
    pub mounts: Vec<(String, String)>,
}

impl Scan {
    /// The Unreal object path (`/Game/Props/T_Crate_D`, lowercase) an editor package at this
    /// place would have, used to match exported files to material references.
    pub fn object_path(&self, rel: &str) -> Option<String> {
        let (prefix, mount) = self
            .mounts
            .iter()
            .find(|(p, _)| rel.to_ascii_lowercase().starts_with(&p.to_ascii_lowercase()))?;
        let inside = &rel[prefix.len()..];
        let dir = inside.rsplit_once('/').map_or("", |(d, _)| d);
        let path = if dir.is_empty() {
            format!("{mount}/{}", stem(rel))
        } else {
            format!("{mount}/{dir}/{}", stem(rel))
        };
        Some(path.to_ascii_lowercase())
    }

    /// Why `rel` is engine content because of the subtree it is in, if it is.
    pub fn prefix_reason(&self, rel: &str) -> Option<&str> {
        let lower = rel.to_ascii_lowercase();
        self.engine_prefixes
            .iter()
            .find(|(p, _)| p.is_empty() || lower.starts_with(&p.to_ascii_lowercase()))
            .map(|(_, r)| r.as_str())
    }
}

/// Every file of an Unreal project, plugin or content folder, skipping generated folders
/// (Binaries, Intermediate, Saved, DerivedDataCache, Build) and hidden ones.
pub fn scan(root: &Path) -> Scan {
    let mut scan = Scan::default();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        if let Some(desc) = descriptor(&dir, "uplugin") {
            let name = stem(&desc.to_string_lossy()).to_string();
            let prefix = if rel.is_empty() {
                String::new()
            } else {
                format!("{rel}/")
            };
            scan.mounts
                .push((format!("{prefix}Content/"), format!("/{name}")));
            if let Some(by) = plugin_author(&desc).filter(|a| license::is_engine_publisher(a)) {
                scan.engine_prefixes.push((
                    prefix,
                    format!("plugin `{name}` was made by `{by}` (its .uplugin says so)"),
                ));
            }
        } else if rel.is_empty() {
            scan.mounts.push(("Content/".into(), "/Game".into()));
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || crate::is_declaration_file_name(&name) {
                continue;
            }
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let path = e.path();
            if path.is_dir() {
                if !GENERATED_DIRS.contains(&name.to_ascii_lowercase().as_str()) {
                    stack.push((path, child));
                }
            } else {
                scan.entries.push(SourceEntry {
                    rel: child,
                    data: path,
                    meta: None,
                });
            }
        }
    }
    scan.entries.sort_by(|a, b| a.rel.cmp(&b.rel));
    scan.mounts
        .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
    scan.engine_prefixes.sort();
    scan
}

/// `CreatedBy` from a `.uplugin` descriptor (JSON).
fn plugin_author(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("CreatedBy")?.as_str().map(str::to_string)
}

/// Folder names of content Epic ships with the engine's templates and feature packs (rule 7):
/// Starter Content, the template folders and the mannequins.
const TEMPLATE_FOLDERS: [&str; 17] = [
    "startercontent",
    "starter content",
    "thirdperson",
    "thirdpersonbp",
    "firstperson",
    "firstpersonbp",
    "topdown",
    "topdownbp",
    "sidescroller",
    "sidescrollerbp",
    "sidescroller2d",
    "vehicletemplate",
    "levelprototyping",
    "mannequin",
    "mannequins",
    "animstarterpack",
    "featurepacks",
];

/// Why a path inside an Unreal source is engine content (rule 7), if it is: anything under
/// `Engine/` and the template, Starter Content and mannequin folders.
pub fn engine_content_reason(rel: &str) -> Option<String> {
    let parts: Vec<String> = rel.split('/').map(str::to_ascii_lowercase).collect();
    if parts.first().map(String::as_str) == Some("engine") {
        return Some("files under Engine/ are engine content".into());
    }
    let dirs = &parts[..parts.len().saturating_sub(1)];
    for d in dirs {
        if TEMPLATE_FOLDERS.contains(&d.as_str()) {
            return Some(format!(
                "folder `{d}` holds Epic template, starter or sample content"
            ));
        }
        if d.starts_with("com.epicgames.") {
            return Some(format!("folder `{d}` is an engine package"));
        }
    }
    None
}

/// How the importer treats one file of an Unreal source before the shared rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An editor package (`.uasset`, `.umap`): diagnosed, not read.
    Package,
    /// Part of a cooked build (`.pak`, `.utoc`, `.ucas`, `.uexp`, `.ubulk`, ...): not read.
    Cooked,
    /// Unreal object text (`.t3d`, `.copy`): read when it holds a material.
    ObjectText,
    /// Game code: not imported; the porting operations read it.
    Code,
    /// Anything else: the shared classification (textures, models, ...).
    Other,
}

pub fn kind(rel: &str) -> Kind {
    match ext_of(rel).as_str() {
        "uasset" | "umap" => Kind::Package,
        "pak" | "utoc" | "ucas" | "uexp" | "ubulk" | "uptnl" | "sig" | "ushaderbytecode" => Kind::Cooked,
        "t3d" | "copy" => Kind::ObjectText,
        "h" | "hpp" | "cpp" | "cc" | "inl" | "cs" | "uproject" | "uplugin" | "ini" => Kind::Code,
        _ => Kind::Other,
    }
}

/// What can be told about an editor package without reading its format.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PackageInfo {
    /// The file starts with the Unreal package tag.
    pub is_package: bool,
    /// The asset class, from the class names in the package's name table.
    pub class: Option<&'static str>,
    /// File name (no folders) of the file the asset was imported from, if recorded.
    pub source_file: Option<String>,
}

/// Classes in priority order: a level or Blueprint names the classes of what it references, so
/// those come first; a mesh names its material classes, so meshes come before materials.
const CLASSES: [&str; 14] = [
    "World",
    "Blueprint",
    "SkeletalMesh",
    "StaticMesh",
    "AnimSequence",
    "AnimMontage",
    "MaterialInstanceConstant",
    "MaterialFunction",
    "Material",
    "TextureCube",
    "Texture2D",
    "SoundWave",
    "Skeleton",
    "PhysicsAsset",
];

/// How much of a package is read for the diagnosis (the name table is near the start).
pub const PACKAGE_PEEK: u64 = 4 << 20;

/// Reads up to [`PACKAGE_PEEK`] bytes of a package and diagnoses it.
pub fn inspect_package_file(path: &Path) -> std::io::Result<PackageInfo> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(PACKAGE_PEEK)
        .read_to_end(&mut bytes)?;
    Ok(inspect_package(&bytes))
}

/// Diagnoses an editor package from its first bytes: the tag, the asset class (a class name is
/// stored in the name table as a length-prefixed, zero-terminated string) and the
/// `RelativeFilename` of its import data.
pub fn inspect_package(bytes: &[u8]) -> PackageInfo {
    let is_package = bytes.starts_with(&[0xC1, 0x83, 0x2A, 0x9E]);
    let class = CLASSES.iter().copied().find(|c| {
        let mut needle = ((c.len() + 1) as i32).to_le_bytes().to_vec();
        needle.extend_from_slice(c.as_bytes());
        needle.push(0);
        find(bytes, &needle).is_some()
    });
    let source_file = find(bytes, b"RelativeFilename").and_then(|at| {
        let rest = &bytes[at + b"RelativeFilename".len()..];
        let start = rest
            .iter()
            .take(16)
            .position(|b| !matches!(b, b'"' | b' ' | b':' | b'\t' | b'\\'))?;
        let rest = &rest[start..];
        let end = rest.iter().take(1024).position(|b| *b == b'"')?;
        let path = std::str::from_utf8(&rest[..end]).ok()?;
        // Keep only the file name: the recorded path can name folders on the author's machine.
        let name = path.rsplit(['/', '\\']).next()?.trim();
        let ok = !name.is_empty()
            && name.len() <= 200
            && name.contains('.')
            && name.chars().all(|c| !c.is_control());
        ok.then(|| name.to_string())
    });
    PackageInfo {
        is_package,
        class,
        source_file,
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// What a person can do to bring in the asset a package holds.
pub fn package_advice(info: &PackageInfo, cooked: bool) -> String {
    let mut s = if cooked {
        "cooked Unreal package (a .uexp sits beside it): cooked data is built for one engine version and platform and is not read".to_string()
    } else if info.is_package {
        "Unreal editor package: its binary format is not read by this importer".to_string()
    } else {
        "not an Unreal package (the file does not start with the package tag)".to_string()
    };
    let what = match info.class {
        Some("StaticMesh" | "SkeletalMesh") => {
            "in the Unreal Editor use Asset Actions > Export to save it as FBX (or as glTF with the glTF Exporter plugin) into this folder, then import again"
        }
        Some("AnimSequence" | "AnimMontage") => {
            "export it as FBX from the Unreal Editor (Asset Actions > Export) and import the FBX"
        }
        Some("Texture2D" | "TextureCube") => {
            "export it as PNG or TGA from the Unreal Editor (Asset Actions > Export) and import the image"
        }
        Some("Material" | "MaterialInstanceConstant" | "MaterialFunction") => {
            "export it as Unreal object text (Asset Actions > Export, .T3D); the importer maps T3D materials to PBR"
        }
        Some("World") => "levels are not imported; export the meshes it uses one by one",
        Some("Blueprint") => {
            "Blueprints are code: copy the graph as text into a .t3d file and use port_scan (docs/ai/PORTING.md)"
        }
        Some("SoundWave") => "audio import is not built yet",
        _ => "export the asset from the Unreal Editor (Asset Actions > Export) and import the exported file",
    };
    if let Some(c) = info.class {
        s.push_str(&format!("; asset class {c}"));
    }
    if let Some(f) = &info.source_file {
        s.push_str(&format!(
            "; it was imported from `{f}`: if you still have that file, import it directly"
        ));
    }
    s.push_str("; ");
    s.push_str(what);
    s
}

// ---------------------------------------------------------------------------------------------
// Unreal object text (T3D)

/// One object in Unreal object text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct T3dObject {
    /// Class path as written (`/Script/Engine.Material` or `Material`).
    pub class: String,
    pub name: String,
    /// Nesting depth (0 = top level).
    pub depth: usize,
    /// `Key=Value` lines in order.
    pub props: Vec<(String, String)>,
}

impl T3dObject {
    /// The class name without its script path (`Material`).
    pub fn short_class(&self) -> &str {
        self.class.rsplit(['.', '/']).next().unwrap_or(&self.class)
    }

    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// Value of `Key=` in a `Begin Object` line.
fn attr(line: &str, key: &str) -> Option<String> {
    let at = line.find(&format!(" {key}="))? + key.len() + 2;
    let rest = &line[at..];
    let v = if let Some(r) = rest.strip_prefix('"') {
        r.split('"').next().unwrap_or("")
    } else {
        rest.split_whitespace().next().unwrap_or("")
    };
    Some(v.to_string())
}

/// Parses Unreal object text into a flat list of objects. A sub-object declared once with its
/// class and again with its properties (as the editor writes them) becomes one object.
pub fn parse_t3d(text: &str) -> Vec<T3dObject> {
    let mut objects: Vec<T3dObject> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Begin Object") {
            let rest = format!(" {rest}");
            let name = attr(&rest, "Name").unwrap_or_default();
            let class = attr(&rest, "Class").unwrap_or_default();
            let depth = stack.len();
            let existing = objects
                .iter()
                .position(|o| !name.is_empty() && o.name == name && o.depth == depth);
            let i = match existing {
                Some(i) => {
                    if objects[i].class.is_empty() {
                        objects[i].class = class;
                    }
                    i
                }
                None => {
                    objects.push(T3dObject {
                        class,
                        name,
                        depth,
                        props: Vec::new(),
                    });
                    objects.len() - 1
                }
            };
            stack.push(i);
        } else if t.starts_with("End Object") {
            stack.pop();
        } else if let Some(&i) = stack.last()
            && let Some((k, v)) = t.split_once('=')
            && !k.contains(' ')
        {
            objects[i].props.push((k.to_string(), v.to_string()));
        }
    }
    objects
}

/// Splits `(A=1,B=(C=2),D="x,y")` into its top-level `key=value` fields.
pub fn struct_fields(s: &str) -> Vec<(String, String)> {
    let s = s.trim();
    let inner = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')).unwrap_or(s);
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, false, 0usize);
    let mut push = |part: &str| {
        if let Some((k, v)) = part.split_once('=') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    };
    for (i, c) in inner.char_indices() {
        match c {
            '"' => quote = !quote,
            '(' if !quote => depth += 1,
            ')' if !quote => depth -= 1,
            ',' if !quote && depth == 0 => {
                push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    push(&inner[start..]);
    out
}

fn field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

/// The quoted part of an object reference (`Texture2D'"/Game/T.T"'` gives `/Game/T.T`).
fn quoted(v: &str) -> Option<&str> {
    let a = v.find('\'')?;
    let b = v.rfind('\'')?;
    (b > a).then(|| v[a + 1..b].trim_matches('"'))
}

/// The package path of an asset reference, lowercase without the object name
/// (`Texture2D'"/Game/Props/T_Crate_D.T_Crate_D"'` gives `/game/props/t_crate_d`).
pub fn asset_path(v: &str) -> Option<String> {
    let v = v.trim();
    if v.eq_ignore_ascii_case("none") || v.is_empty() {
        return None;
    }
    let q = quoted(v).unwrap_or(v).trim_matches('"');
    if !q.starts_with('/') {
        return None;
    }
    let (dir, last) = q.rsplit_once('/')?;
    let last = last.split(['.', ':']).next().unwrap_or(last);
    Some(format!("{dir}/{last}").to_ascii_lowercase())
}

/// The object name an expression reference points at (the last `.`/`:` part of the quote).
fn ref_name(v: &str) -> Option<String> {
    let q = quoted(v)?;
    Some(q.rsplit(['.', ':']).next().unwrap_or(q).to_string())
}

fn float(v: &str) -> Option<f32> {
    v.trim().trim_end_matches('f').parse().ok()
}

/// `(R=1,G=0.5,B=0,A=1)` to RGBA.
fn color(v: &str) -> Option<[f32; 4]> {
    let f = struct_fields(v);
    let c = |k: &str, d: f32| field(&f, k).and_then(float).unwrap_or(d);
    field(&f, "R")?;
    Some([c("R", 0.0), c("G", 0.0), c("B", 0.0), c("A", 1.0)])
}

/// Where one material input gets its value.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// A texture (object path, lowercase) and the channel used (`None` = RGB, 0..=3 = R..A).
    Texture {
        path: String,
        channel: Option<u8>,
    },
    /// A constant color, linear (Unreal colors are linear).
    Color([f32; 4]),
    Scalar(f32),
    /// Computed by other nodes; the class of the node feeding the input.
    Graph(String),
}

/// A material read from object text, before it is mapped to PBR.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UnrealMaterial {
    pub name: String,
    /// `Material` or `MaterialInstanceConstant`.
    pub class: String,
    /// The parent material of an instance (object path).
    pub parent: Option<String>,
    /// Inputs by glTF meaning: `base_color`, `base_color_tint`, `normal`, `roughness`,
    /// `metallic`, `occlusion`, `orm`, `emissive`, `emissive_color`, `emissive_intensity`,
    /// `normal_strength`, `tiling`.
    pub inputs: BTreeMap<String, Input>,
    /// Parameters or inputs that have no glTF meaning here.
    pub unmapped: Vec<String>,
    /// `OPAQUE`, `MASK` or `BLEND`.
    pub alpha_mode: &'static str,
    pub alpha_cutoff: Option<f32>,
    pub two_sided: bool,
}

/// Normalized parameter name: lowercase letters and digits only.
fn norm(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The glTF meaning of a material instance parameter, by its name.
fn texture_meaning(n: &str) -> Option<&'static str> {
    Some(match n {
        "basecolor" | "basecolortexture" | "basecolormap" | "albedo" | "albedotexture" | "albedomap"
        | "diffuse" | "diffusetexture" | "diffusemap" | "color" | "colortexture" | "colormap" | "texture"
        | "maintexture" | "bc" | "d" => "base_color",
        "normal" | "normalmap" | "normaltexture" | "normals" | "n" => "normal",
        "orm"
        | "arm"
        | "ormtexture"
        | "armtexture"
        | "ormmap"
        | "occlusionroughnessmetallic"
        | "aoroughnessmetallic" => "orm",
        "ao" | "ambientocclusion" | "occlusion" | "aotexture" | "aomap" => "occlusion",
        "emissive"
        | "emissivetexture"
        | "emissivemap"
        | "emission"
        | "emissivecolor"
        | "emissivecolortexture"
        | "e" => "emissive",
        _ => return None,
    })
}

fn color_meaning(n: &str) -> Option<&'static str> {
    Some(match n {
        "basecolor" | "basecolortint" | "tint" | "color" | "colortint" | "albedotint" | "albedocolor"
        | "diffusecolor" => "base_color_tint",
        "emissive" | "emissivecolor" | "emissivetint" | "emissioncolor" => "emissive_color",
        "tiling" | "uvtiling" | "texturetiling" | "uvscale" => "tiling",
        _ => return None,
    })
}

fn scalar_meaning(n: &str) -> Option<&'static str> {
    Some(match n {
        "roughness" | "roughnessscale" | "roughnessmultiplier" => "roughness",
        "metallic" | "metalness" | "metallicscale" => "metallic",
        "normalstrength" | "normalintensity" | "normalscale" | "normalamount" => "normal_strength",
        "emissiveintensity" | "emissivestrength" | "emissivebrightness" | "emissivepower" => {
            "emissive_intensity"
        }
        "tiling" | "uvtiling" | "texturetiling" | "uvscale" => "tiling",
        _ => return None,
    })
}

/// Reads the blend mode, mask clip value and two-sidedness from the top object's text.
fn blend(m: &mut UnrealMaterial, top: &T3dObject) {
    let all: String = top.props.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    m.alpha_mode = if all.contains("BLEND_Masked") {
        "MASK"
    } else if [
        "BLEND_Translucent",
        "BLEND_Additive",
        "BLEND_Modulate",
        "BLEND_AlphaComposite",
    ]
    .iter()
    .any(|b| all.contains(b))
    {
        "BLEND"
    } else {
        "OPAQUE"
    };
    let key = "OpacityMaskClipValue=";
    m.alpha_cutoff = all.find(key).and_then(|at| {
        let rest = &all[at + key.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .unwrap_or(rest.len());
        float(&rest[..end])
    });
    m.two_sided = all.contains("TwoSided=True") || all.contains("bTwoSided=True");
}

/// Reads a material or material instance from Unreal object text. Errors say what the text
/// holds instead.
pub fn parse_material(text: &str) -> Result<UnrealMaterial, String> {
    let objects = parse_t3d(text);
    let Some(top) = objects.iter().find(|o| o.depth == 0) else {
        return Err("no object in the text".into());
    };
    let class = top.short_class().to_string();
    let mut m = UnrealMaterial {
        name: top.name.clone(),
        class: class.clone(),
        alpha_mode: "OPAQUE",
        ..Default::default()
    };
    blend(&mut m, top);
    match class.as_str() {
        "MaterialInstanceConstant" => instance(&mut m, top),
        "Material" => graph(&mut m, &objects),
        other => {
            return Err(format!(
                "the text holds a `{other}`; only Material and MaterialInstanceConstant exports are read"
            ));
        }
    }
    Ok(m)
}

fn param_name(fields: &[(String, String)]) -> Option<String> {
    if let Some(info) = field(fields, "ParameterInfo") {
        let f = struct_fields(info);
        if let Some(n) = field(&f, "Name") {
            return Some(n.trim_matches('"').to_string());
        }
    }
    field(fields, "ParameterName").map(|n| n.trim_matches('"').to_string())
}

fn instance(m: &mut UnrealMaterial, top: &T3dObject) {
    m.parent = top.prop("Parent").and_then(asset_path);
    for (k, v) in &top.props {
        let list = k.split('(').next().unwrap_or(k);
        let fields = struct_fields(v);
        let Some(name) = param_name(&fields) else { continue };
        let value = field(&fields, "ParameterValue").unwrap_or("");
        let n = norm(&name);
        let mapped = match list {
            "TextureParameterValues" => texture_meaning(&n).and_then(|meaning| {
                let path = asset_path(value)?;
                Some((meaning, Input::Texture { path, channel: None }))
            }),
            "VectorParameterValues" => {
                color_meaning(&n).and_then(|meaning| Some((meaning, Input::Color(color(value)?))))
            }
            "ScalarParameterValues" => {
                scalar_meaning(&n).and_then(|meaning| Some((meaning, Input::Scalar(float(value)?))))
            }
            _ => continue,
        };
        match mapped {
            Some((meaning, input)) => {
                m.inputs.entry(meaning.to_string()).or_insert(input);
            }
            None => m.unmapped.push(format!("parameter `{name}`")),
        }
    }
}

/// The material inputs Unreal writes on a Material (or its editor-only data in newer versions),
/// with their glTF meaning.
const GRAPH_INPUTS: [(&str, &str); 7] = [
    ("BaseColor", "base_color"),
    ("Metallic", "metallic"),
    ("Roughness", "roughness"),
    ("Normal", "normal"),
    ("EmissiveColor", "emissive"),
    ("AmbientOcclusion", "occlusion"),
    ("Specular", ""),
];

fn graph(m: &mut UnrealMaterial, objects: &[T3dObject]) {
    let by_name: BTreeMap<&str, &T3dObject> = objects.iter().map(|o| (o.name.as_str(), o)).collect();
    // Inputs are on the Material (older versions) or on its MaterialEditorOnlyData sub-object.
    let holders = objects.iter().filter(|o| {
        o.depth == 0 || o.short_class() == "MaterialEditorOnlyData" || o.name.ends_with("EditorOnlyData")
    });
    for holder in holders {
        for (key, meaning) in GRAPH_INPUTS {
            let Some(v) = holder.prop(key) else { continue };
            let fields = struct_fields(v);
            let Some(expr) = field(&fields, "Expression").and_then(ref_name) else {
                continue;
            };
            if meaning.is_empty() {
                m.unmapped.push(format!("input {key}"));
                continue;
            }
            let Some(node) = by_name.get(expr.as_str()) else {
                m.unmapped
                    .push(format!("input {key} (node `{expr}` not in the text)"));
                continue;
            };
            let output = field(&fields, "OutputIndex").and_then(|v| v.parse::<u8>().ok());
            let mask = |c: &str| field(&fields, c) == Some("1");
            let channel = match output {
                Some(i @ 1..=4) => Some(i - 1),
                _ if mask("Mask") => {
                    let on: Vec<u8> = ["MaskR", "MaskG", "MaskB", "MaskA"]
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| mask(c))
                        .map(|(i, _)| i as u8)
                        .collect();
                    (on.len() == 1).then(|| on[0])
                }
                _ => None,
            };
            let class = node.short_class();
            let input = match class {
                "MaterialExpressionTextureSample"
                | "MaterialExpressionTextureSampleParameter2D"
                | "MaterialExpressionTextureObjectParameter" => node
                    .prop("Texture")
                    .and_then(asset_path)
                    .map(|path| Input::Texture { path, channel }),
                "MaterialExpressionConstant3Vector" | "MaterialExpressionConstant4Vector" => {
                    node.prop("Constant").and_then(color).map(Input::Color)
                }
                "MaterialExpressionVectorParameter" => {
                    node.prop("DefaultValue").and_then(color).map(Input::Color)
                }
                "MaterialExpressionConstant" => {
                    Some(Input::Scalar(node.prop("R").and_then(float).unwrap_or(0.0)))
                }
                "MaterialExpressionScalarParameter" => Some(Input::Scalar(
                    node.prop("DefaultValue").and_then(float).unwrap_or(0.0),
                )),
                other => Some(Input::Graph(other.to_string())),
            };
            match input {
                Some(i) => {
                    m.inputs.entry(meaning.to_string()).or_insert(i);
                }
                None => m.unmapped.push(format!("input {key} (node has no value)")),
            }
        }
    }
}

/// Texture fields of a material mapped to glTF, before texture paths are resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapped {
    pub pbr: crate::unity::PbrMapping,
    /// Things not mapped, for notes.
    pub notes: Vec<String>,
}

/// Maps an Unreal material to glTF metallic-roughness. Texture fields hold object paths.
/// Colors are already linear. A texture feeding roughness from G and metallic from B (an ORM
/// texture, also read when named `ORM`/`ARM` in an instance) is the glTF packing as it is.
pub fn map_pbr(m: &UnrealMaterial) -> Mapped {
    use crate::unity::{MetalLayout, PbrMapping};
    let mut notes = Vec::new();
    let mut unmapped_textures = Vec::new();
    let tex = |k: &str| match m.inputs.get(k) {
        Some(Input::Texture { path, channel }) => Some((path.clone(), *channel)),
        _ => None,
    };
    let scalar = |k: &str| match m.inputs.get(k) {
        Some(Input::Scalar(v)) => Some(*v),
        _ => None,
    };
    let col = |k: &str| match m.inputs.get(k) {
        Some(Input::Color(c)) => Some(*c),
        Some(Input::Scalar(v)) => Some([*v, *v, *v, 1.0]),
        _ => None,
    };
    for (k, i) in &m.inputs {
        if let Input::Graph(class) = i {
            notes.push(format!(
                "input `{k}` is computed by a `{class}` node graph; not mapped (bake it to a texture to keep it)"
            ));
        }
    }

    let base = tex("base_color");
    let base_color_factor = col("base_color")
        .or_else(|| col("base_color_tint"))
        .unwrap_or([1.0; 4]);

    // Metallic and roughness: an ORM texture, or separate graph inputs from one texture's G
    // and B channels.
    let mut metal_texture = tex("orm").map(|(p, _)| (p, MetalLayout::Gltf));
    if metal_texture.is_none()
        && let (Some((rp, Some(1))), Some((mp, Some(2)))) = (tex("roughness"), tex("metallic"))
        && rp == mp
    {
        metal_texture = Some((rp, MetalLayout::Gltf));
    }
    let mut occlusion_texture = None;
    if let Some((p, _)) = &metal_texture {
        match tex("occlusion") {
            Some((op, Some(0))) if &op == p => occlusion_texture = Some(op),
            _ if m.inputs.contains_key("orm") => occlusion_texture = Some(p.clone()),
            _ => {}
        }
    }
    if occlusion_texture.is_none() {
        match tex("occlusion") {
            Some((p, None | Some(0))) => occlusion_texture = Some(p),
            Some((p, Some(c))) => notes.push(format!(
                "occlusion comes from channel {} of `{p}`; glTF reads R; not mapped",
                "RGBA".as_bytes()[c as usize] as char
            )),
            None => {}
        }
    }
    let (metallic_factor, roughness_factor) = if metal_texture.is_some() {
        if scalar("roughness").is_some() || scalar("metallic").is_some() {
            notes.push(
                "roughness/metallic scalars are ignored next to the ORM texture (their use in the parent graph is unknown)"
                    .into(),
            );
        }
        (1.0, 1.0)
    } else {
        for k in ["roughness", "metallic"] {
            if let Some((p, c)) = tex(k) {
                let ch = c.map_or("RGB".to_string(), |c| {
                    ("RGBA".as_bytes()[c as usize] as char).to_string()
                });
                notes.push(format!(
                    "{k} comes from channel {ch} of `{p}`; glTF needs roughness in G and metallic in B of one texture; not mapped"
                ));
                unmapped_textures.push(k.to_string());
            }
        }
        (
            scalar("metallic").unwrap_or(0.0),
            scalar("roughness").unwrap_or(0.5),
        )
    };

    let emissive_texture = tex("emissive").map(|(p, _)| p);
    let mut emissive =
        col("emissive")
            .or_else(|| col("emissive_color"))
            .unwrap_or(if emissive_texture.is_some() {
                [1.0; 4]
            } else {
                [0.0; 4]
            });
    if let Some(k) = scalar("emissive_intensity") {
        for c in &mut emissive[..3] {
            *c *= k;
        }
    }
    if emissive[..3].iter().any(|c| *c > 1.0) {
        notes.push(
            "emissive factor is above 1 (Unreal emissive is unbounded); glTF needs KHR_emissive_strength for that"
                .into(),
        );
    }
    let tiling = match m.inputs.get("tiling") {
        Some(Input::Scalar(v)) => [*v, *v],
        Some(Input::Color(c)) => [c[0], c[1]],
        _ => [1.0, 1.0],
    };
    if let Some(p) = &m.parent {
        notes.push(format!(
            "instance of `{p}`: parameters it does not override come from that parent, which is not in this export; export the parent as .T3D too to see its defaults"
        ));
    }
    for u in &m.unmapped {
        notes.push(format!("{u} has no glTF equivalent here"));
    }
    Mapped {
        pbr: PbrMapping {
            shader: None,
            base_color_factor,
            base_color_texture: base.map(|(p, _)| p),
            metallic_factor,
            roughness_factor,
            metal_texture,
            smoothness_scale: 1.0,
            normal_texture: tex("normal").map(|(p, _)| p),
            normal_scale: scalar("normal_strength").unwrap_or(1.0),
            occlusion_texture,
            occlusion_strength: 1.0,
            emissive_factor: [emissive[0], emissive[1], emissive[2]],
            emissive_texture,
            alpha_mode: m.alpha_mode,
            alpha_cutoff: m.alpha_cutoff.unwrap_or(0.3333),
            uv_scale: tiling,
            uv_offset: [0.0, 0.0],
            unmapped_textures,
        },
        notes,
    }
}

/// Notes for a texture by Unreal naming convention (`T_Name_N`, `T_Name_ORM`).
pub fn texture_name_notes(rel: &str) -> Vec<String> {
    let s = stem(rel).to_ascii_lowercase();
    let suffix = s.rsplit('_').next().unwrap_or("");
    match suffix {
        "n" | "normal" | "nrm" | "nm" => vec![
            "name suggests a normal map; Unreal normal maps are DirectX style (green points down); glTF wants green up, so a material import flips it, and a direct use must flip G"
                .into(),
        ],
        "orm" | "arm" => vec![
            "name suggests an ORM texture (R occlusion, G roughness, B metallic): already the glTF packing"
                .into(),
        ],
        _ => Vec::new(),
    }
}

/// Flips the green channel of an RGBA8 normal map (DirectX to OpenGL/glTF convention).
pub fn flip_green(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        px[1] = 255 - px[1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_content_paths() {
        assert!(engine_content_reason("Content/StarterContent/Textures/T_Brick.png").is_some());
        assert!(engine_content_reason("Content/Characters/Mannequins/Meshes/SKM_Quinn.fbx").is_some());
        assert!(engine_content_reason("Engine/Content/x.png").is_some());
        assert!(engine_content_reason("Content/MyPack/Textures/T_Crate_D.png").is_none());
        assert!(engine_content_reason("Content/MyPack/StarterContent.png").is_none());
    }

    #[test]
    fn package_diagnosis() {
        let mut b = vec![0xC1, 0x83, 0x2A, 0x9E, 0, 0, 0, 0];
        for name in ["/Script/Engine", "StaticMesh", "MaterialInstanceConstant"] {
            b.extend(((name.len() + 1) as i32).to_le_bytes());
            b.extend(name.as_bytes());
            b.push(0);
        }
        b.extend(b"[{ \"RelativeFilename\" : \"../../Art/Props/SM_Crate.fbx\", \"Timestamp\" : \"0\" }]");
        let i = inspect_package(&b);
        assert!(i.is_package);
        assert_eq!(i.class, Some("StaticMesh"));
        assert_eq!(i.source_file.as_deref(), Some("SM_Crate.fbx"));
        let advice = package_advice(&i, false);
        assert!(
            advice.contains("FBX") && advice.contains("SM_Crate.fbx"),
            "{advice}"
        );
        // A substring of a longer name does not count as a class.
        let mut b = vec![0xC1, 0x83, 0x2A, 0x9E];
        b.extend(b"MyStaticMeshThing");
        assert_eq!(inspect_package(&b).class, None);
        assert!(!inspect_package(b"plain").is_package);
    }

    #[test]
    fn struct_fields_and_references() {
        let f = struct_fields(
            "(ParameterInfo=(Name=\"Base Color\",Index=-1),ParameterValue=Texture2D'\"/Game/Props/T_Crate_D.T_Crate_D\"',X=\"a,b\")",
        );
        assert_eq!(f.len(), 3);
        assert_eq!(param_name(&f).as_deref(), Some("Base Color"));
        assert_eq!(
            asset_path(field(&f, "ParameterValue").unwrap()).as_deref(),
            Some("/game/props/t_crate_d")
        );
        assert_eq!(
            asset_path("/Script/Engine.Texture2D'/Game/T_N.T_N'").as_deref(),
            Some("/game/t_n")
        );
        assert_eq!(asset_path("None"), None);
        assert_eq!(
            ref_name("/Script/Engine.MaterialExpressionTextureSample'\"MaterialExpressionTextureSample_0\"'")
                .as_deref(),
            Some("MaterialExpressionTextureSample_0")
        );
        assert_eq!(
            ref_name(
                "MaterialExpressionTextureSample'MaterialGraphNode_2.MaterialExpressionTextureSample_1'"
            )
            .as_deref(),
            Some("MaterialExpressionTextureSample_1")
        );
    }

    #[test]
    fn green_flip() {
        let mut px = [128, 10, 255, 255, 0, 255, 0, 7];
        flip_green(&mut px);
        assert_eq!(px, [128, 245, 255, 255, 0, 0, 0, 7]);
    }
}
