//! Admission: the only way an asset enters the pool. Every rule runs on every addition; there is
//! no override (ASSETS-AND-LICENSES.md rules 4, 5, 13 and decision 0041).
//!
//! 1. Owned gate: `marionette_import::pool::check` on the original location and on the staged
//!    files, compared with the owned library when one is configured.
//! 2. Generated gate: a file `*.provenance.json` of format `marionette.generated_asset/1` (what
//!    the generation workstream writes beside provider output) refuses the asset unless a
//!    person-written dedication `*.cc0.json` (format `marionette.cc0_dedication/1`) is present.
//!    Tools refuse to write dedications, as they refuse license declarations.
//! 3. License files: any license text shipped with the asset must say CC0 (or public domain)
//!    and must not name another license.
//! 4. Proof: `made` assets need a method; `online` assets need a known CC0 library, a page on
//!    that library's domain, and downloads from its domains.
//! 5. Files: safe paths; the main file decodes (PNG) or loads (glTF) in the engine.

use crate::sources;
use crate::{
    ASSET_FORMAT, Origin, Pool, PoolAsset, PoolFile, PoolKind, PoolLicense, Proof, Provenance, codes, err,
    sha256_hex, slug, valid_id, write_file,
};
use marionette_core::Errors;
use marionette_import::Library;
use schemars::JsonSchema;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const DEDICATION_FORMAT: &str = "marionette.cc0_dedication/1";
pub const GENERATED_FORMAT: &str = "marionette.generated_asset/1";

/// True for a CC0 dedication file name (`*.cc0.json`): only a person may write one.
pub fn is_dedication_file_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".cc0.json")
}

fn is_provenance_file_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".provenance.json")
}

/// True for files that hold license text (`LICENSE`, `License.txt`, `licence.md`, `COPYING`).
pub fn is_license_file_name(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let stem = name.split('.').next().unwrap_or("");
    matches!(
        stem,
        "license" | "licence" | "copying" | "license-cc0" | "licence-cc0"
    ) || name.starts_with("license_")
        || name.starts_with("license-")
}

/// Phrases that say "CC0 / public domain".
const CC0_MARKERS: &[&str] = &[
    "cc0",
    "creative commons zero",
    "publicdomain/zero",
    "public domain",
];

/// Phrases that name a license other than CC0.
const OTHER_MARKERS: &[&str] = &[
    "cc by",
    "cc-by",
    "attribution 2.0",
    "attribution 3.0",
    "attribution 4.0",
    "attribution-sharealike",
    "attribution-noncommercial",
    "noncommercial",
    "non-commercial",
    "sharealike",
    "share-alike",
    "no derivatives",
    "noderivatives",
    "gnu general public",
    "gpl-",
    "all rights reserved",
    "royalty-free license",
    "standard unity asset store eula",
    "unreal engine eula",
];

/// What a license text says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseText {
    Cc0,
    /// Names another license (the phrases found).
    Other(Vec<String>),
    /// Says neither.
    Unclear,
}

pub fn classify_license_text(text: &str) -> LicenseText {
    let t = text.to_ascii_lowercase();
    let other: Vec<String> = OTHER_MARKERS
        .iter()
        .filter(|m| t.contains(*m))
        .map(|m| m.to_string())
        .collect();
    if !other.is_empty() {
        return LicenseText::Other(other);
    }
    if CC0_MARKERS.iter().any(|m| t.contains(m)) {
        LicenseText::Cc0
    } else {
        LicenseText::Unclear
    }
}

/// An asset offered to the pool.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub kind: PoolKind,
    pub name: String,
    /// Optional id; default `<slug of name>-<12 hex of the content hash>`.
    pub id: Option<String>,
    pub tags: Vec<String>,
    pub source: Provenance,
    /// Extra proof (license files found, API fields checked).
    pub proof: Vec<Proof>,
    /// Files as (path inside the asset, bytes).
    pub files: Vec<(String, Vec<u8>)>,
    pub main: Option<String>,
    pub notes: Vec<String>,
    /// Where the files came from on disk, for the owned gate (it also checks the folder's
    /// ancestors for an owned library).
    pub origin_path: Option<PathBuf>,
    /// Folders holding generated assets and their provenance records (the project's
    /// `assets/generated/` by default); a file whose bytes match a record there is a copy of
    /// a generated asset (decision 0045).
    pub generated_dirs: Vec<PathBuf>,
}

/// What an addition did.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AddReport {
    pub id: String,
    /// `added`, `already_in_pool` or `checked` (dry run).
    pub status: String,
    pub engine_ready: bool,
    pub files: Vec<String>,
    pub proof: Vec<Proof>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

fn owned_refusal(path: &Path, library: Option<&Library>) -> Result<(), Errors> {
    let v = marionette_import::pool::check(path, library);
    if let Some(r) = v.refusals.first() {
        let all: Vec<String> = v
            .refusals
            .iter()
            .map(|r| format!("{} ({})", r.path, r.reason))
            .collect();
        let code = match r.code.as_str() {
            c if c == marionette_import::codes::POOL_OWNED_ASSET => {
                marionette_import::codes::POOL_OWNED_ASSET
            }
            c if c == marionette_import::codes::POOL_OWNED_CONTENT => {
                marionette_import::codes::POOL_OWNED_CONTENT
            }
            _ => codes::NOT_CC0,
        };
        return Err(err(
            code,
            format!("refused by the owned-asset gate: {}", all.join("; ")),
            "owned assets and copies of them never enter the public pool",
        ));
    }
    Ok(())
}

/// Checks a candidate against every rule and, unless `dry_run`, writes it into the pool.
pub fn add(
    pool: &Pool,
    mut c: Candidate,
    library: Option<&Library>,
    dry_run: bool,
) -> Result<AddReport, Errors> {
    let bad = |m: String| err(codes::BAD_INPUT, m, "see the pool_add input schema");
    if c.name.trim().is_empty() || c.name.len() > 120 {
        return Err(bad("an asset needs a name of 1 to 120 characters".into()));
    }
    c.tags = c
        .tags
        .iter()
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    c.tags.sort();
    c.tags.dedup();
    if c.files.is_empty() {
        return Err(bad("an asset needs at least one file".into()));
    }

    // Owned gate on where the files came from.
    if let Some(p) = &c.origin_path {
        owned_refusal(p, library)?;
    }

    // Split off provenance and dedication files; they become proof, not pooled files.
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut provenance: Vec<(String, serde_json::Value)> = Vec::new();
    let mut dedications: Vec<(String, serde_json::Value, String)> = Vec::new();
    for (path, bytes) in std::mem::take(&mut c.files) {
        let clean = crate::zip::clean_name(&path).ok_or_else(|| bad(format!("unsafe file path `{path}`")))?;
        let name = clean.rsplit('/').next().unwrap_or(&clean).to_string();
        if marionette_import::is_declaration_file_name(&name) {
            return Err(err(
                codes::NOT_CC0,
                format!("`{clean}` is an owned-asset license declaration"),
                "owned assets never enter the public pool",
            ));
        }
        if is_provenance_file_name(&name) {
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
            provenance.push((clean, v));
            continue;
        }
        if is_dedication_file_name(&name) {
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
            dedications.push((clean, v, sha256_hex(&bytes)));
            continue;
        }
        if name == "asset.json" {
            return Err(bad(
                "`asset.json` is the pool's record; it cannot be an asset file".into(),
            ));
        }
        if files.iter().any(|(p, _)| p.eq_ignore_ascii_case(&clean)) {
            return Err(bad(format!("file `{clean}` is listed twice")));
        }
        files.push((clean, bytes));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    if files.is_empty() {
        return Err(bad(
            "an asset needs at least one file besides provenance and dedication files".into(),
        ));
    }

    // Generated gate: a provenance record offered with the files, or (decision 0045) a file on
    // disk that `marionette_generate` recognizes as generated output or a copy of it.
    let mut generated: Option<(String, serde_json::Value)> = provenance
        .iter()
        .find(|(_, v)| v["format"] == GENERATED_FORMAT || v.get("provider").is_some())
        .cloned();
    if generated.is_none()
        && let Some(origin) = &c.origin_path
    {
        let dirs: Vec<&Path> = c.generated_dirs.iter().map(PathBuf::as_path).collect();
        let mut on_disk = Vec::new();
        list_files(origin, &mut on_disk);
        on_disk.sort();
        if let Some(f) = on_disk
            .iter()
            .find(|f| marionette_generate::provenance::pool_status(f, &dirs).generated)
        {
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            generated = Some((
                format!("{name} (matches a generated asset's provenance record)"),
                serde_json::json!({ "format": GENERATED_FORMAT }),
            ));
        }
    }
    let mut proof: Vec<Proof> = Vec::new();
    if let Some((p, v)) = &generated {
        let dedication = dedications.iter().find(|(_, d, _)| {
            d["format"] == DEDICATION_FORMAT
                && d["dedicated_by"].as_str().is_some_and(|s| !s.trim().is_empty())
        });
        match dedication {
            Some((dp, d, sha)) => proof.push(Proof {
                kind: "person_dedication".into(),
                detail: format!(
                    "generated by {} ({}); dedicated to the public domain under CC0 by a person in `{dp}` (sha256 {sha}): {}",
                    v["provider"].as_str().unwrap_or("a provider"),
                    p,
                    d["statement"].as_str().unwrap_or("no statement")
                ),
                at: None,
            }),
            None => {
                return Err(err(
                    codes::GENERATED,
                    format!("`{p}` marks this as provider output; provider terms differ and it is not CC0 unless a person dedicates it"),
                    "a person (never an agent or tool) may write a `<name>.cc0.json` dedication (format marionette.cc0_dedication/1, with `dedicated_by` and `statement`)",
                ));
            }
        }
    }

    // License files.
    for (path, bytes) in &files {
        if !is_license_file_name(path) {
            continue;
        }
        match classify_license_text(&String::from_utf8_lossy(bytes)) {
            LicenseText::Cc0 => proof.push(Proof {
                kind: "license_file".into(),
                detail: format!(
                    "`{path}` states CC0 / public domain (sha256 {})",
                    sha256_hex(bytes)
                ),
                at: Some(path.clone()),
            }),
            LicenseText::Other(m) => {
                return Err(err(
                    codes::NOT_CC0,
                    format!("`{path}` names a license other than CC0: {}", m.join(", ")),
                    "the pool accepts CC0 assets only",
                ));
            }
            LicenseText::Unclear => {
                return Err(err(
                    codes::NOT_CC0,
                    format!("`{path}` is a license file that does not say CC0 or public domain"),
                    "the pool accepts CC0 assets only; leave the file out only if it is not a license",
                ));
            }
        }
    }

    // Proof by origin.
    match c.source.origin {
        Origin::Made => {
            if c.source.method.as_deref().is_none_or(|m| m.trim().is_empty()) {
                return Err(err(
                    codes::NO_PROOF,
                    "a made asset needs `method`: how it was made (and `generator` if a program in the engine repository makes it)",
                    "describe how the asset was made; assets from anywhere else are added as online assets",
                ));
            }
            if c.source.library.is_some() || c.source.page.is_some() {
                return Err(bad("a made asset has no library or page".into()));
            }
            if generated.is_none() {
                proof.insert(
                    0,
                    Proof {
                        kind: "made_for_project".into(),
                        detail: "made for the Marionette project and dedicated to the public domain under CC0 1.0 by the Marionette Contributors".into(),
                        at: Some("https://creativecommons.org/publicdomain/zero/1.0/".into()),
                    },
                );
            }
        }
        Origin::Online => {
            let lib = c.source.library.as_deref().unwrap_or("");
            let Some(src) = sources::library(lib) else {
                return Err(err(
                    codes::UNKNOWN_SOURCE,
                    format!("`{lib}` is not a CC0 library the pool knows"),
                    &format!(
                        "known libraries: {}",
                        sources::LIBRARIES
                            .iter()
                            .map(|l| l.id)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            };
            let page = c.source.page.as_deref().unwrap_or("");
            if !src.owns(page) {
                return Err(err(
                    codes::NO_PROOF,
                    format!("page `{page}` is not on {}", src.name),
                    &format!(
                        "give the asset's https page on {}",
                        src.domains
                            .iter()
                            .chain(src.prefixes)
                            .copied()
                            .collect::<Vec<_>>()
                            .join(" or ")
                    ),
                ));
            }
            if let Some(d) = c.source.downloads.iter().find(|d| !src.owns(d)) {
                return Err(err(
                    codes::NO_PROOF,
                    format!("download `{d}` is not from {}", src.name),
                    "files must come from the library itself",
                ));
            }
            if src.needs_license_file && !proof.iter().any(|p| p.kind == "license_file") {
                return Err(err(
                    codes::NO_PROOF,
                    format!(
                        "{} assets must carry the library's CC0 license file (License.txt in its archive)",
                        src.name
                    ),
                    "add the archive as downloaded, with its license file",
                ));
            }
            if c.source.retrieved.is_none() {
                c.source.retrieved = Some(crate::today());
            }
            proof.insert(0, src.policy_proof());
        }
    }
    proof.extend(std::mem::take(&mut c.proof));

    // Stage, then the owned gate on the exact bytes.
    // In the process's run folder, so a crash leaves nothing behind (decision 0032).
    let stage =
        marionette_host::temp_dir("pool-stage").map_err(|e| bad(format!("cannot stage files: {e}")))?;
    for (p, b) in &files {
        let dest = stage.path().join(p);
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d).map_err(|e| bad(format!("cannot stage files: {e}")))?;
        }
        std::fs::write(&dest, b).map_err(|e| bad(format!("cannot stage files: {e}")))?;
    }
    owned_refusal(stage.path(), library)?;

    // The main file must load.
    let mut notes = c.notes.clone();
    let main = c.main.clone().or_else(|| default_main(c.kind, &files));
    if let Some(m) = &main
        && !files.iter().any(|(p, _)| p == m)
    {
        return Err(bad(format!("main file `{m}` is not among the files")));
    }
    let engine_ready = match &main {
        Some(m) => match check_main(c.kind, &stage.path().join(m), m) {
            Ok(()) => true,
            Err(why) if c.kind == PoolKind::Texture || c.kind == PoolKind::Model => {
                return Err(err(
                    codes::BAD_FILE,
                    format!("`{m}` does not load: {why}"),
                    "pool only files the engine can use",
                ));
            }
            Err(why) => {
                notes.push(why);
                false
            }
        },
        None => false,
    };

    // Id.
    let mut all = String::new();
    for (p, b) in &files {
        all.push_str(p);
        all.push_str(&sha256_hex(b));
    }
    let content = sha256_hex(all.as_bytes());
    let id = match &c.id {
        Some(id) if valid_id(id) => id.clone(),
        Some(id) => {
            return Err(bad(format!(
                "`{id}` is not a valid id (lowercase letters, digits, dashes)"
            )));
        }
        None => format!("{}-{}", slug(&c.name), &content[..12]),
    };
    let record = PoolAsset {
        format: ASSET_FORMAT.into(),
        id: id.clone(),
        kind: c.kind,
        name: c.name.trim().to_string(),
        tags: c.tags.clone(),
        license: PoolLicense {
            spdx: "CC0-1.0".into(),
            proof: proof.clone(),
        },
        source: c.source.clone(),
        files: files
            .iter()
            .map(|(p, b)| PoolFile {
                path: p.clone(),
                sha256: sha256_hex(b),
                bytes: b.len() as u64,
            })
            .collect(),
        main: main.clone(),
        engine_ready,
        notes: notes.clone(),
    };
    let report = |status: &str| AddReport {
        id: id.clone(),
        status: status.into(),
        engine_ready,
        files: files.iter().map(|(p, _)| p.clone()).collect(),
        proof: proof.clone(),
        notes: notes.clone(),
    };
    if let Ok(existing) = pool.get(&id) {
        if existing.files == record.files {
            return Ok(report("already_in_pool"));
        }
        return Err(err(
            codes::EXISTS,
            format!("the pool already has `{id}` with other files"),
            "choose another id, or omit `id` to get one from the content",
        ));
    }
    if dry_run {
        return Ok(report("checked"));
    }
    pool.ensure()?;
    let dir = pool.asset_dir(&id);
    for (p, b) in &files {
        write_file(&dir.join(p), b)?;
    }
    let text = serde_json::to_string_pretty(&record).expect("record serializes") + "\n";
    write_file(&dir.join("asset.json"), text.as_bytes())?;
    Ok(report("added"))
}

fn default_main(kind: PoolKind, files: &[(String, Vec<u8>)]) -> Option<String> {
    let exts: &[&str] = match kind {
        PoolKind::Texture => &["png"],
        PoolKind::Model => &["glb", "gltf"],
        PoolKind::Sound => &["wav", "ogg"],
        PoolKind::Material => &["json"],
        PoolKind::Other => &[],
    };
    for e in exts {
        let mut found: Vec<&String> = files
            .iter()
            .map(|(p, _)| p)
            .filter(|p| p.to_ascii_lowercase().ends_with(&format!(".{e}")) && !is_license_file_name(p))
            .collect();
        found.sort_by_key(|p| (p.matches('/').count(), p.len()));
        if let Some(f) = found.first() {
            return Some((*f).clone());
        }
    }
    None
}

fn check_main(kind: PoolKind, path: &Path, shown: &str) -> Result<(), String> {
    match kind {
        PoolKind::Texture => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            marionette_assets::decode_png(&bytes).map(|_| ())
        }
        PoolKind::Model => marionette_assets::load_gltf(path).map(|_| ()),
        PoolKind::Sound => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") || bytes.starts_with(b"OggS") {
                Ok(())
            } else {
                Err(format!("`{shown}` is not WAV or OGG"))
            }
        }
        PoolKind::Material | PoolKind::Other => Err(format!(
            "`{shown}`: {} assets are kept as files; the engine does not load them directly yet",
            kind.name()
        )),
    }
}

/// Reads a folder or a single file as candidate files (relative paths, sorted). Hidden files
/// are skipped.
pub fn read_folder(path: &Path) -> Result<Vec<(String, Vec<u8>)>, Errors> {
    let io = |e: std::io::Error| {
        err(
            codes::BAD_INPUT,
            format!("cannot read the asset files: {e}"),
            "check the path",
        )
    };
    if path.is_file() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Ok(vec![(name, std::fs::read(path).map_err(io)?)]);
    }
    if !path.is_dir() {
        return Err(err(
            codes::BAD_INPUT,
            "the asset path does not exist",
            "pass a file or folder",
        ));
    }
    let mut out = Vec::new();
    let mut stack = vec![(path.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        for e in std::fs::read_dir(&dir).map_err(io)?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let p = e.path();
            if p.is_dir() {
                stack.push((p, child));
            } else {
                out.push((child, std::fs::read(&p).map_err(io)?));
            }
        }
        if out.len() > 10_000 {
            return Err(err(
                codes::BAD_INPUT,
                "more than 10,000 files",
                "add one asset at a time",
            ));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Keeps a source's archive entries: the files of the asset plus its license files.
pub fn strip_common_root(entries: Vec<(String, Vec<u8>)>) -> Vec<(String, Vec<u8>)> {
    let first = entries
        .first()
        .and_then(|(p, _)| p.split_once('/').map(|(a, _)| a.to_string()));
    match first {
        Some(root) if entries.iter().all(|(p, _)| p.starts_with(&format!("{root}/"))) => entries
            .into_iter()
            .map(|(p, b)| (p[root.len() + 1..].to_string(), b))
            .collect(),
        _ => entries,
    }
}

/// Every file under `p` (or `p` itself when it is a file), skipping symbolic links.
fn list_files(p: &Path, out: &mut Vec<PathBuf>) {
    let Ok(meta) = std::fs::symlink_metadata(p) else {
        return;
    };
    if meta.is_file() {
        out.push(p.to_path_buf());
    } else if meta.is_dir()
        && let Ok(entries) = std::fs::read_dir(p)
    {
        for e in entries.flatten() {
            list_files(&e.path(), out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_texts() {
        let kenney = "License: (Creative Commons Zero, CC0)\nhttp://creativecommons.org/publicdomain/zero/1.0/\nCredit is not mandatory.";
        assert_eq!(classify_license_text(kenney), LicenseText::Cc0);
        assert!(matches!(
            classify_license_text("Licensed under CC BY 4.0, Attribution 4.0 International"),
            LicenseText::Other(_)
        ));
        assert_eq!(
            classify_license_text("Thanks for downloading!"),
            LicenseText::Unclear
        );
        assert!(is_license_file_name("Pack/License.txt"));
        assert!(is_license_file_name("LICENSE"));
        assert!(!is_license_file_name("licensed_art.png"));
    }
}
