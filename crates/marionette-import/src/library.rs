//! The owned library: one folder per user, shared by all of that user's projects, never inside
//! the engine repository and never synced to the public pool (ASSETS-AND-LICENSES.md rule 2).
//!
//! Layout:
//! ```text
//! <library>/library.json            marker: format, private, never pooled
//! <library>/assets/<id>/record.json the asset's record (license, source, files, hashes)
//! <library>/assets/<id>/...         the asset's files
//! ```

use crate::codes;
use crate::license::LicenseRecord;
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const LIBRARY_FORMAT: &str = "marionette.owned_library/1";
pub const RECORD_FORMAT: &str = "marionette.owned_asset/1";
/// Environment variable naming the owned library folder.
pub const LIBRARY_ENV: &str = "MARIONETTE_LIBRARY";

/// What kind of asset a record holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Texture,
    Model,
    Material,
}

/// One file of an asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FileRecord {
    /// Path inside the asset folder.
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// Where an asset came from. Holds names inside the source only, never machine paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SourceRecord {
    /// `unity_project`, `unitypackage`, `unreal_project` or `files`.
    pub importer: String,
    /// File or folder name of the source (no directories).
    pub source_name: String,
    /// Path of the asset inside the source, with `/` separators.
    pub path: String,
    /// The Unity GUID from the asset's `.meta`, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unity_guid: Option<String>,
    /// The Unreal object path the file has in its project (`/game/props/t_crate_d`,
    /// lowercase), used to match exported files to material references.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreal_path: Option<String>,
    /// SHA-256 of the original file.
    pub sha256: String,
}

/// The record stored with every owned asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AssetRecord {
    /// Always `marionette.owned_asset/1`.
    pub format: String,
    pub id: String,
    pub kind: AssetKind,
    pub name: String,
    pub license: LicenseRecord,
    pub source: SourceRecord,
    pub files: Vec<FileRecord>,
    /// The file the engine loads (for example `texture.png` or `model.glb`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<String>,
    /// True when `main` is in a format the engine loads today.
    pub engine_ready: bool,
    /// Always true: owned assets are private to the user.
    pub private: bool,
    /// Always `blocked`: owned assets never enter the CC0 pool.
    pub pool: String,
    /// Extra facts (texture usage, material mapping gaps, conversions still needed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Ids of other library assets this one uses (a material's textures).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<String>,
}

/// Where the library path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfiguredFrom {
    Input,
    Environment,
    UserConfig,
}

/// An opened owned library.
#[derive(Debug, Clone)]
pub struct Library {
    root: PathBuf,
    pub configured_from: ConfiguredFrom,
}

/// The per-user configuration file: `%APPDATA%\marionette\config.json` on Windows,
/// `$XDG_CONFIG_HOME/marionette/config.json` or `~/.config/marionette/config.json` elsewhere.
pub fn user_config_file() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("marionette").join("config.json"))
}

#[derive(Deserialize)]
struct UserConfig {
    owned_library: Option<String>,
}

fn normalize(p: &Path) -> PathBuf {
    // Canonicalize the longest existing ancestor so comparisons work before creation.
    let mut existing = p.to_path_buf();
    let mut tail = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(n), Some(parent)) => {
                tail.push(n.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
    let mut out = existing.canonicalize().unwrap_or(existing);
    for n in tail.into_iter().rev() {
        out.push(n);
    }
    out
}

/// True when `dir` is the root of the engine repository (its workspace lists marionette-core).
fn is_engine_root(dir: &Path) -> bool {
    dir.join("crates").join("marionette-core").is_dir()
        && std::fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|t| t.contains("marionette-core"))
}

impl Library {
    /// Finds the library: `explicit` (operation input), then `MARIONETTE_LIBRARY`, then the
    /// user configuration file. Checks the location rules; does not create anything.
    pub fn locate(explicit: Option<&Path>, project_root: &Path) -> Result<Self, Errors> {
        let (path, from) = if let Some(p) = explicit {
            (p.to_path_buf(), ConfiguredFrom::Input)
        } else if let Some(p) = std::env::var_os(LIBRARY_ENV).filter(|v| !v.is_empty()) {
            (PathBuf::from(p), ConfiguredFrom::Environment)
        } else if let Some(p) = user_config_file()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice::<UserConfig>(&b).ok())
            .and_then(|c| c.owned_library)
        {
            (PathBuf::from(p), ConfiguredFrom::UserConfig)
        } else {
            return Err(Diagnostic::error(codes::LIBRARY_NOT_CONFIGURED, "no owned library is configured")
                .fix(format!(
                    "set {LIBRARY_ENV} to an absolute folder outside the engine repository, pass `library`, \
                     or write {{\"owned_library\": \"...\"}} to the user config file (marionette/config.json in the user's config folder)"
                ))
                .into());
        };
        Self::at(&path, project_root, from)
    }

    /// Opens the library at `path`, checking the location rules.
    pub fn at(path: &Path, project_root: &Path, from: ConfiguredFrom) -> Result<Self, Errors> {
        if !path.is_absolute() {
            return Err(Diagnostic::error(
                codes::LIBRARY_RELATIVE,
                "the owned library path must be absolute",
            )
            .fix("use an absolute folder in the user's own space, shared by all their projects")
            .into());
        }
        let root = normalize(path);
        for anc in root.ancestors() {
            if is_engine_root(anc) {
                return Err(Diagnostic::error(
                    codes::LIBRARY_INSIDE_ENGINE,
                    "the owned library must not be inside the engine repository",
                )
                .fix("choose a folder outside the engine repository")
                .into());
            }
        }
        let project = normalize(project_root);
        if root.starts_with(&project) {
            return Err(Diagnostic::error(
                codes::LIBRARY_INSIDE_PROJECT,
                "the owned library must not be inside a project; it is shared by all of the user's projects",
            )
            .fix("choose a folder outside the project")
            .into());
        }
        Ok(Self {
            root,
            configured_from: from,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn exists(&self) -> bool {
        self.root.join("library.json").is_file()
    }

    /// Creates the library folder and marker if needed. Refuses a non-empty folder that is
    /// not already a library, so a mistyped path cannot scatter files into other data.
    pub fn ensure(&self) -> Result<(), Errors> {
        if self.exists() {
            return Ok(());
        }
        if self.root.is_dir() && std::fs::read_dir(&self.root).is_ok_and(|mut d| d.next().is_some()) {
            return Err(Diagnostic::error(
                codes::LIBRARY_NOT_A_LIBRARY,
                "the library folder is not empty and holds no library.json",
            )
            .fix("point the owned library at an empty or new folder")
            .into());
        }
        let marker = serde_json::json!({
            "format": LIBRARY_FORMAT,
            "private": true,
            "note": "Owned assets of one user. Private: never sync this folder to the public CC0 pool."
        });
        self.write(
            &self.root.join("library.json"),
            serde_json::to_string_pretty(&marker).expect("json").as_bytes(),
        )
    }

    pub(crate) fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), Errors> {
        let shown = path
            .strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if path
            .file_name()
            .is_some_and(|n| crate::is_declaration_file_name(&n.to_string_lossy()))
        {
            return Err(crate::declaration_write_refused(&shown).into());
        }
        let err = |e: std::io::Error| {
            Errors::from(
                Diagnostic::error(
                    marionette_core::error::codes::IO_WRITE,
                    format!("cannot write library file {shown}: {e}"),
                )
                .file(&shown),
            )
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        let tmp = path.with_extension("tmp-write");
        std::fs::write(&tmp, bytes).map_err(err)?;
        std::fs::rename(&tmp, path).map_err(err)
    }

    pub fn asset_dir(&self, id: &str) -> PathBuf {
        self.root.join("assets").join(id)
    }

    /// Reads one record.
    pub fn get(&self, id: &str) -> Result<AssetRecord, Errors> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(
                Diagnostic::error(codes::LIBRARY_NO_ASSET, format!("`{id}` is not an asset id")).into(),
            );
        }
        let path = self.asset_dir(id).join("record.json");
        let bytes = std::fs::read(&path).map_err(|_| {
            Errors::from(
                Diagnostic::error(
                    codes::LIBRARY_NO_ASSET,
                    format!("no asset `{id}` in the owned library"),
                )
                .fix("list assets with library_list"),
            )
        })?;
        serde_json::from_slice(&bytes).map_err(|e| {
            Errors::from(Diagnostic::error(
                codes::LIBRARY_BAD_RECORD,
                format!("record of `{id}` is unreadable: {e}"),
            ))
        })
    }

    /// Every record, sorted by id. Unreadable records are skipped.
    pub fn records(&self) -> Vec<AssetRecord> {
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir(self.root.join("assets")) else {
            return out;
        };
        for e in dir.flatten() {
            if let Ok(bytes) = std::fs::read(e.path().join("record.json"))
                && let Ok(r) = serde_json::from_slice::<AssetRecord>(&bytes)
            {
                out.push(r);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }
}

/// A file-name-safe slug of an asset name.
pub(crate) fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-');
    let s: String = s.chars().take(40).collect();
    if s.is_empty() { "asset".into() } else { s }
}
