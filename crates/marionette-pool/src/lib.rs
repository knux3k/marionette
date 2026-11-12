//! The public CC0 asset pool (workstream W3, decision 0041).
//!
//! - [`Pool`]: a folder (the assets repository, or a local copy of it) holding one folder per
//!   asset with its record (`asset.json`, format `marionette.pool_asset/1`): kind, name, tags,
//!   files with SHA-256, where it came from, and the proof that it is CC0.
//! - [`admit`]: the admission rules. Every addition passes `marionette_import::pool::check`
//!   (no owned assets or copies of them), the generated-asset rule (no provider output unless a
//!   person dedicated it), the license-file rule (any license text shipped with the asset must
//!   be CC0) and the proof rule (made for the project, or from a known CC0 library with its page).
//! - [`search`]: the asset lookup order of `docs/ai/ASSETS-AND-LICENSES.md` rule 1: project
//!   files, owned library, pool, online CC0 libraries, then generation.
//! - [`sources`]: Poly Haven and ambientCG (search and ingest through their APIs), Kenney and
//!   Quaternius (ingest from a downloaded archive that carries their CC0 license text).
//! - [`net`]: moving bytes; tests use a scripted transport, the CLI uses the system `curl`.

pub mod admit;
pub mod net;
pub mod search;
pub mod seed;
pub mod sources;
pub mod zip;

use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const POOL_FORMAT: &str = "marionette.cc0_pool/1";
pub const ASSET_FORMAT: &str = "marionette.pool_asset/1";
/// Environment variable naming the pool folder.
pub const POOL_ENV: &str = "MARIONETTE_POOL";

/// Stable error and refusal codes. Codes never change meaning.
pub mod codes {
    /// No pool is configured (input, `MARIONETTE_POOL`, or the user config file `cc0_pool`).
    pub const NOT_CONFIGURED: &str = "pool.not_configured";
    /// The folder exists, is not empty, and holds no `pool.json`.
    pub const NOT_A_POOL: &str = "pool.not_a_pool";
    /// The pool folder is inside an owned library.
    pub const INSIDE_LIBRARY: &str = "pool.inside_library";
    /// No asset with that id in the pool.
    pub const NO_ASSET: &str = "pool.no_asset";
    /// The request is malformed (bad id, name, kind, path).
    pub const BAD_INPUT: &str = "pool.bad_input";
    /// No acceptable proof that the asset is CC0.
    pub const NO_PROOF: &str = "pool.no_proof";
    /// A provider-generated asset without a person's CC0 dedication.
    pub const GENERATED: &str = "pool.generated";
    /// A license text shipped with the asset is not CC0.
    pub const NOT_CC0: &str = marionette_import::codes::POOL_NOT_CC0;
    /// The source library is not one the pool knows to be CC0.
    pub const UNKNOWN_SOURCE: &str = "pool.unknown_source";
    /// A file is not usable (cannot be decoded or loaded).
    pub const BAD_FILE: &str = "pool.bad_file";
    /// The network could not be reached or answered with an error.
    pub const NETWORK: &str = "pool.network";
    /// An online source answered with data the pool does not understand.
    pub const SOURCE_FORMAT: &str = "pool.source_format";
    /// An asset with this id already exists with different content.
    pub const EXISTS: &str = "pool.exists";
}

pub(crate) fn err(code: &'static str, msg: impl Into<String>, fix: &str) -> Errors {
    Diagnostic::error(code, msg.into()).fix(fix).into()
}

/// SHA-256 as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What an asset is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PoolKind {
    Texture,
    Model,
    Material,
    Sound,
    Other,
}

impl PoolKind {
    pub fn name(self) -> &'static str {
        match self {
            PoolKind::Texture => "texture",
            PoolKind::Model => "model",
            PoolKind::Material => "material",
            PoolKind::Sound => "sound",
            PoolKind::Other => "other",
        }
    }
}

/// One file of an asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PoolFile {
    /// Path inside the asset folder, `/` separators.
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// How an asset came to the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// Made for the project (by hand, by code in the engine repository, or by an agent without
    /// any provider) and dedicated to the public domain under CC0 by the Marionette Contributors.
    Made,
    /// Taken from a known online CC0 library.
    Online,
}

/// Where an asset came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Provenance {
    pub origin: Origin,
    /// Online: the library id (`polyhaven`, `ambientcg`, `kenney`, `quaternius`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<String>,
    /// Online: the asset's page at the library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// Online: the asset's id at the library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// Creator as the library credits them (CC0 needs no credit; recorded as a courtesy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Online: the date (YYYY-MM-DD, UTC) the files were retrieved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieved: Option<String>,
    /// Online: the URLs the files were downloaded from, in file order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub downloads: Vec<String>,
    /// Made: how it was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Made: the program in the engine repository that makes it, if any (repo-relative path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
}

/// One piece of evidence that an asset is CC0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Proof {
    /// `made_for_project`, `library_policy`, `license_file` or `api_license_field`.
    pub kind: String,
    /// What was checked, in words.
    pub detail: String,
    /// Where the evidence is (a license page, or a file inside the asset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// The license of a pooled asset: always CC0, with its proof.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PoolLicense {
    /// Always `CC0-1.0`.
    pub spdx: String,
    pub proof: Vec<Proof>,
}

/// The record stored with every pooled asset (`assets/<id>/asset.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PoolAsset {
    /// Always `marionette.pool_asset/1`.
    pub format: String,
    pub id: String,
    pub kind: PoolKind,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub license: PoolLicense,
    pub source: Provenance,
    pub files: Vec<PoolFile>,
    /// The file the engine loads, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<String>,
    /// True when `main` loads in the engine today.
    pub engine_ready: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Where the pool path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PoolFrom {
    Input,
    Environment,
    UserConfig,
}

/// An opened pool.
#[derive(Debug, Clone)]
pub struct Pool {
    root: PathBuf,
    pub configured_from: PoolFrom,
}

#[derive(Deserialize)]
struct UserConfig {
    cc0_pool: Option<String>,
}

/// A file-name-safe slug.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s: String = s.trim_end_matches('-').chars().take(48).collect();
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "asset".into() } else { s }
}

/// True when `id` is a valid asset id (lowercase letters, digits and dashes).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
}

impl Pool {
    /// Finds the pool: `explicit` (operation input; relative paths are the caller's job), then
    /// `MARIONETTE_POOL`, then `cc0_pool` in the user config file.
    pub fn locate(explicit: Option<&Path>) -> Result<Self, Errors> {
        let (path, from) = if let Some(p) = explicit {
            (p.to_path_buf(), PoolFrom::Input)
        } else if let Some(p) = std::env::var_os(POOL_ENV).filter(|v| !v.is_empty()) {
            (PathBuf::from(p), PoolFrom::Environment)
        } else if let Some(p) = marionette_import::library::user_config_file()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice::<UserConfig>(&b).ok())
            .and_then(|c| c.cc0_pool)
        {
            (PathBuf::from(p), PoolFrom::UserConfig)
        } else {
            return Err(err(
                codes::NOT_CONFIGURED,
                "no CC0 pool is configured",
                &format!(
                    "pass `pool`, set {POOL_ENV} to a checkout of the assets repository, or write {{\"cc0_pool\": \"...\"}} to the user config file"
                ),
            ));
        };
        Self::at(&path, from)
    }

    /// Opens the pool at `path`. A pool may never sit inside an owned library.
    pub fn at(path: &Path, from: PoolFrom) -> Result<Self, Errors> {
        for anc in path.ancestors() {
            if anc.join("library.json").is_file()
                && std::fs::read_to_string(anc.join("library.json"))
                    .is_ok_and(|t| t.contains(marionette_import::library::LIBRARY_FORMAT))
            {
                return Err(err(
                    codes::INSIDE_LIBRARY,
                    "the CC0 pool must not be inside an owned library",
                    "keep the pool (a checkout of the assets repository) outside the owned library",
                ));
            }
        }
        Ok(Self {
            root: path.to_path_buf(),
            configured_from: from,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn exists(&self) -> bool {
        self.root.join("pool.json").is_file()
    }

    /// Creates the pool marker if needed; refuses a non-empty folder that is not a pool.
    pub fn ensure(&self) -> Result<(), Errors> {
        if self.exists() {
            return Ok(());
        }
        if self.root.is_dir() && std::fs::read_dir(&self.root).is_ok_and(|mut d| d.next().is_some()) {
            return Err(err(
                codes::NOT_A_POOL,
                "the pool folder is not empty and holds no pool.json",
                "point the pool at an empty folder or at a checkout of the assets repository",
            ));
        }
        let marker = serde_json::json!({
            "format": POOL_FORMAT,
            "license": "CC0-1.0",
            "note": "Public CC0 asset pool of the Marionette engine. Every asset folder holds asset.json with its source and proof that it is CC0. Owned or non-CC0 assets are never accepted."
        });
        write_file(
            &self.root.join("pool.json"),
            (serde_json::to_string_pretty(&marker).expect("json") + "\n").as_bytes(),
        )
    }

    pub fn asset_dir(&self, id: &str) -> PathBuf {
        self.root.join("assets").join(id)
    }

    pub fn get(&self, id: &str) -> Result<PoolAsset, Errors> {
        if !valid_id(id) {
            return Err(err(
                codes::BAD_INPUT,
                format!("`{id}` is not a pool asset id"),
                "ids are lowercase letters, digits and dashes",
            ));
        }
        let path = self.asset_dir(id).join("asset.json");
        let bytes = std::fs::read(&path).map_err(|_| {
            err(
                codes::NO_ASSET,
                format!("the pool has no asset `{id}`"),
                "list the pool with pool_list or search with asset_search",
            )
        })?;
        serde_json::from_slice(&bytes).map_err(|e| {
            err(
                codes::SOURCE_FORMAT,
                format!("asset.json of `{id}` is unreadable: {e}"),
                "fix or re-add the asset",
            )
        })
    }

    /// Every record, sorted by id. Unreadable records are skipped.
    pub fn records(&self) -> Vec<PoolAsset> {
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir(self.root.join("assets")) else {
            return out;
        };
        for e in dir.flatten() {
            if let Ok(bytes) = std::fs::read(e.path().join("asset.json"))
                && let Ok(r) = serde_json::from_slice::<PoolAsset>(&bytes)
            {
                out.push(r);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// The index the assets repository publishes (`index.json`): one line per asset, sorted.
    pub fn index(&self) -> serde_json::Value {
        let items: Vec<serde_json::Value> = self
            .records()
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "kind": r.kind,
                    "name": r.name,
                    "tags": r.tags,
                    "main": r.main,
                    "origin": r.source.origin,
                    "library": r.source.library,
                    "engine_ready": r.engine_ready,
                })
            })
            .collect();
        serde_json::json!({ "format": "marionette.cc0_pool_index/1", "license": "CC0-1.0", "assets": items })
    }
}

/// Writes a file atomically. Refuses license declarations and CC0 dedications: only a person
/// may write those (ASSETS-AND-LICENSES.md rule 9).
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<(), Errors> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if marionette_import::is_declaration_file_name(&name) || admit::is_dedication_file_name(&name) {
        return Err(marionette_import::declaration_write_refused(&name).into());
    }
    let io = |e: std::io::Error| {
        Errors::from(Diagnostic::error(
            marionette_core::error::codes::IO_WRITE,
            format!("cannot write {name}: {e}"),
        ))
    };
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(io)?;
    }
    let tmp = path.with_extension("tmp-write");
    std::fs::write(&tmp, bytes).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

/// Today's date (UTC) as YYYY-MM-DD, from the system clock (records only; never simulation).
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    civil(secs.div_euclid(86_400))
}

/// Days since 1970-01-01 to YYYY-MM-DD (proleptic Gregorian).
pub fn civil(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_slugs() {
        assert_eq!(civil(0), "1970-01-01");
        assert_eq!(civil(20_404), "2025-11-12");
        assert_eq!(slug("Brick Wall (Red) 01"), "brick-wall-red-01");
        assert!(valid_id("brick-wall-01"));
        assert!(!valid_id("../x"));
        assert!(!valid_id("Upper"));
    }
}
