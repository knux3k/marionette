//! Reading Unity sources: project folders, `.unitypackage` archives (gzip + tar, one folder per
//! GUID holding `asset`, `asset.meta` and `pathname`), `.meta` files and text `.mat` materials.

use crate::codes;
use crate::tar::{self, Limits, TarError};
use marionette_core::{Diagnostic, Errors};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// One file found in a source.
#[derive(Debug, Clone)]
pub struct SourceEntry {
    /// Path inside the source with `/` separators (`Assets/...` for Unity sources).
    pub rel: String,
    /// Where its bytes are on disk (the original file, or a staged copy).
    pub data: PathBuf,
    /// Text of its `.meta` file, if any.
    pub meta: Option<String>,
}

impl SourceEntry {
    pub fn guid(&self) -> Option<String> {
        self.meta
            .as_deref()
            .and_then(|m| meta_value(m, "guid"))
            .map(str::to_string)
    }
}

/// An entry refused while reading the source (before any license rule).
#[derive(Debug, Clone)]
pub struct BadEntry {
    pub rel: String,
    pub code: &'static str,
    pub reason: String,
}

/// The first `key: value` in a `.meta` (or other Unity YAML) text.
pub fn meta_value<'a>(meta: &'a str, key: &str) -> Option<&'a str> {
    meta.lines().find_map(|l| {
        let t = l.trim_start();
        let rest = t.strip_prefix(key)?.strip_prefix(':')?;
        let v = rest.trim();
        (!v.is_empty()).then_some(v)
    })
}

/// True when `dir` looks like a Unity project (has `Assets/` and `ProjectSettings/`).
pub fn is_unity_project(dir: &Path) -> bool {
    dir.join("Assets").is_dir() && dir.join("ProjectSettings").is_dir()
}

/// Unity ignores folders and files whose names start with `.` or end with `~`.
fn unity_hidden(name: &str) -> bool {
    name.starts_with('.') || name.ends_with('~')
}

/// Every file under `Assets/` of a Unity project, with its `.meta`. Sorted by path.
pub fn scan_project(root: &Path) -> Vec<SourceEntry> {
    let mut out = Vec::new();
    let mut stack = vec![(root.join("Assets"), "Assets".to_string())];
    while let Some((dir, rel)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if unity_hidden(&name) {
                continue;
            }
            let path = e.path();
            let child = format!("{rel}/{name}");
            if path.is_dir() {
                stack.push((path, child));
            } else if !name.to_ascii_lowercase().ends_with(".meta") {
                let meta = std::fs::read_to_string(dir.join(format!("{name}.meta"))).ok();
                out.push(SourceEntry {
                    rel: child,
                    data: path,
                    meta,
                });
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

/// Every file of a plain (non-Unity) owned source: a single file, or a folder walked
/// recursively. Paths are relative to the folder (or the file name).
pub fn scan_files(source: &Path) -> Vec<SourceEntry> {
    if source.is_file() {
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        return vec![SourceEntry {
            rel: name,
            data: source.to_path_buf(),
            meta: None,
        }];
    }
    let mut out = Vec::new();
    let mut stack = vec![(source.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
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
                stack.push((path, child));
            } else {
                out.push(SourceEntry {
                    rel: child,
                    data: path,
                    meta: None,
                });
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

fn is_guid(s: &str) -> bool {
    s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Checks a path read from a package's `pathname` file. Returns the clean path, or why not.
pub fn clean_package_path(raw: &str) -> Result<String, String> {
    let p = raw.lines().next().unwrap_or("").trim().replace('\\', "/");
    if p.is_empty() {
        return Err("empty path".into());
    }
    if p.starts_with('/') || p.contains(':') {
        return Err(format!("absolute path `{p}`"));
    }
    if p.split('/').any(|c| c == ".." || c == "." || c.is_empty()) {
        return Err(format!("path `{p}` has empty, `.` or `..` parts"));
    }
    if !(p.starts_with("Assets/") || p.starts_with("Packages/")) {
        return Err(format!("path `{p}` is not under Assets/ or Packages/"));
    }
    Ok(p)
}

/// Removes a staging folder when dropped (also on errors).
pub struct Staging(pub PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Reads a `.unitypackage` into `staging` (files are named only by validated GUIDs, so archive
/// names never choose where bytes go). Returns the entries with their package paths.
pub fn stage_package(
    package: &Path,
    staging: &Path,
    limits: Limits,
) -> Result<(Vec<SourceEntry>, Vec<BadEntry>), Errors> {
    let shown = package
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file = std::fs::File::open(package).map_err(|e| {
        Errors::from(
            Diagnostic::error(codes::IMPORT_SOURCE, format!("cannot open {shown}: {e}")).file(&shown),
        )
    })?;
    let gz = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let mut pathnames: BTreeMap<String, String> = BTreeMap::new();
    let mut ignored = 0usize;
    tar::read_entries(gz, limits, |name, _size, r| {
        let name = name.trim_start_matches("./");
        let Some((guid, file)) = name.split_once('/') else {
            ignored += 1;
            return Ok(());
        };
        if !is_guid(guid) {
            ignored += 1;
            return Ok(());
        }
        let guid = guid.to_ascii_lowercase();
        match file {
            "pathname" => {
                let mut s = String::new();
                r.take(64 * 1024)
                    .read_to_string(&mut s)
                    .map_err(|e| TarError::Corrupt(e.to_string()))?;
                pathnames.insert(guid, s);
            }
            "asset" | "asset.meta" => {
                let dir = staging.join(&guid);
                std::fs::create_dir_all(&dir)?;
                let mut out = std::fs::File::create(dir.join(file))?;
                std::io::copy(r, &mut out)?;
            }
            _ => ignored += 1,
        }
        Ok(())
    })
    .map_err(|e| {
        let code = match e {
            TarError::TooLarge(_) => codes::IMPORT_TOO_LARGE,
            _ => codes::IMPORT_BAD_ARCHIVE,
        };
        Errors::from(
            Diagnostic::error(code, format!("{shown}: {e}"))
                .file(&shown)
                .fix("check the file is a .unitypackage exported by Unity (gzip-compressed tar)"),
        )
    })?;
    let _ = ignored;
    let mut entries = Vec::new();
    let mut bad = Vec::new();
    for (guid, raw) in pathnames {
        let asset = staging.join(&guid).join("asset");
        if !asset.is_file() {
            continue; // a folder entry
        }
        match clean_package_path(&raw) {
            Ok(rel) => {
                let meta = std::fs::read_to_string(staging.join(&guid).join("asset.meta"))
                    .ok()
                    .or_else(|| Some(format!("guid: {guid}\n")));
                entries.push(SourceEntry {
                    rel,
                    data: asset,
                    meta,
                });
            }
            Err(reason) => bad.push(BadEntry {
                rel: raw.lines().next().unwrap_or("").trim().to_string(),
                code: codes::REFUSED_BAD_PATH,
                reason,
            }),
        }
    }
    entries.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok((entries, bad))
}

/// A texture slot of a Unity material.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TexEnv {
    pub guid: Option<String>,
    pub scale: [f32; 2],
    pub offset: [f32; 2],
}

/// The parts of a text-serialized Unity material this importer uses.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UnityMaterial {
    pub name: String,
    pub shader_file_id: i64,
    pub shader_guid: String,
    pub textures: BTreeMap<String, TexEnv>,
    pub floats: BTreeMap<String, f32>,
    pub colors: BTreeMap<String, [f32; 4]>,
}

/// `{a: 1, b: x}` to a map.
fn inline_map(s: &str) -> BTreeMap<String, String> {
    let s = s.trim().trim_start_matches('{').trim_end_matches('}');
    s.split(',')
        .filter_map(|kv| {
            let (k, v) = kv.split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn f(m: &BTreeMap<String, String>, k: &str, default: f32) -> f32 {
    m.get(k).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Parses a text-serialized Unity `.mat`. Errors when the file is binary-serialized or has no
/// material document.
pub fn parse_material(text: &str) -> Result<UnityMaterial, String> {
    if !text.starts_with("%YAML") {
        return Err("not a text-serialized Unity file".into());
    }
    #[derive(PartialEq)]
    enum Sec {
        None,
        Tex,
        Floats,
        Colors,
    }
    let mut m = UnityMaterial::default();
    let mut started = false;
    let mut sec = Sec::None;
    let mut tex: Option<String> = None;
    for line in text.lines() {
        if !started {
            started = line.trim_end() == "Material:";
            continue;
        }
        if line.starts_with("--- ") {
            break;
        }
        let indent = line.len() - line.trim_start().len();
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if indent == 2 {
            if let Some(v) = t.strip_prefix("m_Name:") {
                m.name = v.trim().to_string();
            } else if let Some(v) = t.strip_prefix("m_Shader:") {
                let map = inline_map(v);
                m.shader_file_id = map.get("fileID").and_then(|v| v.parse().ok()).unwrap_or(0);
                m.shader_guid = map.get("guid").cloned().unwrap_or_default();
            }
        }
        if !t.starts_with("- ") && indent <= 4 {
            let key = t.split(':').next().unwrap_or("");
            sec = match key {
                "m_TexEnvs" => Sec::Tex,
                "m_Floats" | "m_Ints" => Sec::Floats,
                "m_Colors" => Sec::Colors,
                _ => Sec::None,
            };
            tex = None;
            continue;
        }
        if let Some(item) = t.strip_prefix("- ") {
            let Some((k, rest)) = item.split_once(':') else {
                continue;
            };
            let k = k.trim().to_string();
            match sec {
                Sec::Tex => {
                    m.textures.insert(
                        k.clone(),
                        TexEnv {
                            guid: None,
                            scale: [1.0, 1.0],
                            offset: [0.0, 0.0],
                        },
                    );
                    tex = Some(k);
                }
                Sec::Floats => {
                    if let Ok(v) = rest.trim().parse::<f32>() {
                        m.floats.insert(k, v);
                    }
                }
                Sec::Colors => {
                    let c = inline_map(rest);
                    m.colors.insert(
                        k,
                        [f(&c, "r", 0.0), f(&c, "g", 0.0), f(&c, "b", 0.0), f(&c, "a", 1.0)],
                    );
                }
                Sec::None => {}
            }
            continue;
        }
        if sec == Sec::Tex
            && let Some(name) = &tex
            && let Some(slot) = m.textures.get_mut(name)
        {
            if let Some(v) = t.strip_prefix("m_Texture:") {
                let map = inline_map(v);
                let file_id = map.get("fileID").map(String::as_str).unwrap_or("0");
                slot.guid = map
                    .get("guid")
                    .filter(|g| file_id != "0" && is_guid(g))
                    .map(|g| g.to_ascii_lowercase());
            } else if let Some(v) = t.strip_prefix("m_Scale:") {
                let map = inline_map(v);
                slot.scale = [f(&map, "x", 1.0), f(&map, "y", 1.0)];
            } else if let Some(v) = t.strip_prefix("m_Offset:") {
                let map = inline_map(v);
                slot.offset = [f(&map, "x", 0.0), f(&map, "y", 0.0)];
            }
        }
    }
    if !started {
        return Err("no Material document".into());
    }
    Ok(m)
}

/// Name of a shader Unity ships, from the material's shader reference.
pub fn known_shader(file_id: i64, guid: &str) -> Option<&'static str> {
    const BUILTIN: &str = "0000000000000000f000000000000000";
    match (file_id, guid) {
        (46, BUILTIN) => Some("Standard"),
        (45, BUILTIN) => Some("Standard (Specular setup)"),
        (_, "933532a4fcc9baf4fa0491de14d08ed7") => Some("Universal Render Pipeline/Lit"),
        (_, "650dd9526735d5b46b79224bc6e94025") => Some("Universal Render Pipeline/Unlit"),
        (_, "6e4ae4064600d784cac1e41a9e6f2e59") => Some("HDRP/Lit"),
        _ => None,
    }
}

/// sRGB-encoded channel to linear.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// How a Unity metallic texture packs its channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetalLayout {
    /// Built-in Standard and URP Lit: R metallic, A smoothness.
    MetallicSmoothness,
    /// HDRP mask map: R metallic, G occlusion, A smoothness.
    HdrpMask,
}

/// A Unity material mapped to glTF-style metallic-roughness PBR. Texture fields hold Unity
/// GUIDs; the importer replaces them with library asset ids.
#[derive(Debug, Clone, PartialEq)]
pub struct PbrMapping {
    pub shader: Option<&'static str>,
    pub base_color_factor: [f32; 4],
    pub base_color_texture: Option<String>,
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    pub metal_texture: Option<(String, MetalLayout)>,
    /// Multiplier on texture smoothness (`_GlossMapScale`), used when converting.
    pub smoothness_scale: f32,
    pub normal_texture: Option<String>,
    pub normal_scale: f32,
    pub occlusion_texture: Option<String>,
    pub occlusion_strength: f32,
    pub emissive_factor: [f32; 3],
    pub emissive_texture: Option<String>,
    /// `OPAQUE`, `MASK` or `BLEND`, as in glTF.
    pub alpha_mode: &'static str,
    pub alpha_cutoff: f32,
    pub uv_scale: [f32; 2],
    pub uv_offset: [f32; 2],
    /// Texture slots that hold a texture but have no glTF equivalent here.
    pub unmapped_textures: Vec<String>,
}

const MAPPED_TEXTURES: [&str; 10] = [
    "_MainTex",
    "_BaseMap",
    "_BaseColorMap",
    "_MetallicGlossMap",
    "_MaskMap",
    "_BumpMap",
    "_NormalMap",
    "_OcclusionMap",
    "_EmissionMap",
    "_EmissiveColorMap",
];

/// Maps a parsed material to PBR. Unity serializes material colors sRGB-encoded; glTF factors
/// are linear, so colors are converted.
pub fn map_pbr(m: &UnityMaterial) -> PbrMapping {
    let tex = |names: &[&str]| -> Option<(String, &TexEnv)> {
        names.iter().find_map(|n| {
            let t = m.textures.get(*n)?;
            Some((t.guid.clone()?, t))
        })
    };
    let float = |names: &[&str], d: f32| names.iter().find_map(|n| m.floats.get(*n).copied()).unwrap_or(d);
    let color = |names: &[&str]| names.iter().find_map(|n| m.colors.get(*n).copied());

    let base = color(&["_BaseColor", "_Color"]).unwrap_or([1.0; 4]);
    let base_color_factor = [
        srgb_to_linear(base[0]),
        srgb_to_linear(base[1]),
        srgb_to_linear(base[2]),
        base[3],
    ];
    let base_tex = tex(&["_BaseMap", "_MainTex", "_BaseColorMap"]);
    let (uv_scale, uv_offset) = base_tex
        .as_ref()
        .map_or(([1.0, 1.0], [0.0, 0.0]), |(_, t)| (t.scale, t.offset));

    let metal = tex(&["_MetallicGlossMap"])
        .map(|(g, _)| (g, MetalLayout::MetallicSmoothness))
        .or_else(|| tex(&["_MaskMap"]).map(|(g, _)| (g, MetalLayout::HdrpMask)));
    let smoothness = float(&["_Smoothness", "_Glossiness"], 0.5);
    let (metallic_factor, roughness_factor) = if metal.is_some() {
        (1.0, 1.0)
    } else {
        (float(&["_Metallic"], 0.0), 1.0 - smoothness)
    };

    let em = color(&["_EmissionColor", "_EmissiveColor"]).unwrap_or([0.0; 4]);
    let emissive_factor = [
        srgb_to_linear(em[0]),
        srgb_to_linear(em[1]),
        srgb_to_linear(em[2]),
    ];

    let surface_transparent = float(&["_Surface"], 0.0) >= 0.5;
    let clip = float(&["_AlphaClip", "_AlphaCutoffEnable"], 0.0) >= 0.5;
    let mode = float(&["_Mode"], 0.0);
    let alpha_mode = if surface_transparent || mode >= 1.5 {
        "BLEND"
    } else if clip || (0.5..1.5).contains(&mode) {
        "MASK"
    } else {
        "OPAQUE"
    };

    let unmapped_textures = m
        .textures
        .iter()
        .filter(|(k, t)| t.guid.is_some() && !MAPPED_TEXTURES.contains(&k.as_str()))
        .map(|(k, _)| k.clone())
        .collect();

    PbrMapping {
        shader: known_shader(m.shader_file_id, &m.shader_guid),
        base_color_factor,
        base_color_texture: base_tex.map(|(g, _)| g),
        metallic_factor,
        roughness_factor,
        metal_texture: metal,
        smoothness_scale: float(&["_GlossMapScale"], 1.0),
        normal_texture: tex(&["_BumpMap", "_NormalMap"]).map(|(g, _)| g),
        normal_scale: float(&["_BumpScale", "_NormalScale"], 1.0),
        occlusion_texture: tex(&["_OcclusionMap"]).map(|(g, _)| g),
        occlusion_strength: float(&["_OcclusionStrength"], 1.0),
        emissive_factor,
        emissive_texture: tex(&["_EmissionMap", "_EmissiveColorMap"]).map(|(g, _)| g),
        alpha_mode,
        alpha_cutoff: float(&["_Cutoff", "_AlphaCutoff"], 0.5),
        uv_scale,
        uv_offset,
        unmapped_textures,
    }
}

/// Converts a Unity metallic texture (RGBA8) to glTF metallic-roughness channel layout:
/// R occlusion (HDRP mask) or 255, G roughness = 1 - smoothness * scale, B metallic.
pub fn convert_metal_texture(rgba: &mut [u8], layout: MetalLayout, smoothness_scale: f32) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let metal = px[0];
        let smooth = px[3] as f32 / 255.0 * smoothness_scale;
        let rough = ((1.0 - smooth.clamp(0.0, 1.0)) * 255.0).round() as u8;
        let occ = if layout == MetalLayout::HdrpMask {
            px[1]
        } else {
            255
        };
        px[0] = occ;
        px[1] = rough;
        px[2] = metal;
        px[3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const MAT: &str = "%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!21 &2100000
Material:
  serializedVersion: 8
  m_Name: Crate
  m_Shader: {fileID: 46, guid: 0000000000000000f000000000000000, type: 0}
  m_SavedProperties:
    serializedVersion: 3
    m_TexEnvs:
    - _BumpMap:
        m_Texture: {fileID: 2800000, guid: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb, type: 3}
        m_Scale: {x: 1, y: 1}
        m_Offset: {x: 0, y: 0}
    - _DetailAlbedoMap:
        m_Texture: {fileID: 0}
        m_Scale: {x: 1, y: 1}
        m_Offset: {x: 0, y: 0}
    - _MainTex:
        m_Texture: {fileID: 2800000, guid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa, type: 3}
        m_Scale: {x: 2, y: 2}
        m_Offset: {x: 0.5, y: 0}
    m_Ints: []
    m_Floats:
    - _BumpScale: 0.8
    - _Glossiness: 0.25
    - _Metallic: 0.5
    - _Mode: 0
    m_Colors:
    - _Color: {r: 1, g: 0.5, b: 0, a: 1}
    - _EmissionColor: {r: 0, g: 0, b: 0, a: 1}
  m_BuildTextureStacks: []
";

    #[test]
    fn parses_and_maps_a_standard_material() {
        let m = parse_material(MAT).unwrap();
        assert_eq!(m.name, "Crate");
        assert_eq!(m.textures["_MainTex"].scale, [2.0, 2.0]);
        assert_eq!(m.textures["_DetailAlbedoMap"].guid, None);
        let p = map_pbr(&m);
        assert_eq!(p.shader, Some("Standard"));
        assert_eq!(
            p.base_color_texture.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(
            p.normal_texture.as_deref(),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        assert!((p.base_color_factor[1] - 0.2140).abs() < 1e-3);
        assert_eq!(p.metallic_factor, 0.5);
        assert!((p.roughness_factor - 0.75).abs() < 1e-6);
        assert!((p.normal_scale - 0.8).abs() < 1e-6);
        assert_eq!(p.uv_scale, [2.0, 2.0]);
        assert_eq!(p.alpha_mode, "OPAQUE");
        assert!(p.unmapped_textures.is_empty());
    }

    #[test]
    fn binary_material_and_bad_paths_are_rejected() {
        assert!(parse_material("\0\0binary").is_err());
        assert!(clean_package_path("Assets/a/b.png\n00").is_ok());
        for bad in [
            "../x.png",
            "/etc/x",
            "C:/x.png",
            "Assets/../../x",
            "Other/x.png",
            "Assets//x",
        ] {
            assert!(clean_package_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn metal_texture_conversion() {
        let mut px = [200, 50, 0, 255];
        convert_metal_texture(&mut px, MetalLayout::MetallicSmoothness, 1.0);
        assert_eq!(px, [255, 0, 200, 255]);
        let mut px = [10, 77, 0, 0];
        convert_metal_texture(&mut px, MetalLayout::HdrpMask, 1.0);
        assert_eq!(px, [77, 255, 10, 255]);
    }
}
