//! License declarations, license records and the refusal rules
//! (`docs/ai/ASSETS-AND-LICENSES.md`, rules 6 to 13).

use crate::codes;
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The declaration file format tag.
pub const DECLARATION_FORMAT: &str = "marionette.license_declaration/1";

/// Yes or no, written out the way a person answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum YesNo {
    Yes,
    No,
}

/// A license declaration, written by a person (never by a tool or an AI agent) in a file named
/// `*.license.json`. It covers one pack: a `.unitypackage`, a Unity project (or folders of it
/// via `covers`), or a plain owned file or folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    /// Must be `marionette.license_declaration/1`.
    pub format: String,
    /// Where the asset was obtained (store name, or "made by me", "commissioned", ...).
    pub store: String,
    /// The product name or a link to its page.
    pub product: String,
    /// The license type as the store names it.
    pub license_type: String,
    /// Whether the license allows use in engines other than the one it was sold for.
    pub other_engines_allowed: YesNo,
    /// The publisher, if known. Unity and Epic are refused (engine-locked content).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// Paths inside the source this declaration covers (prefixes such as `Assets/Pack/`).
    /// Empty means the whole source.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    /// License text the person pasted in. An asset is refused when it limits use to one engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_text: Option<String>,
}

/// A loaded declaration plus where it came from (file name and content hash only).
#[derive(Debug, Clone)]
pub struct LoadedDeclaration {
    pub declaration: Declaration,
    pub file_name: String,
    pub sha256: String,
}

impl LoadedDeclaration {
    /// Reads and checks a declaration file.
    pub fn load(path: &Path) -> Result<Self, Errors> {
        let shown = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = std::fs::read(path).map_err(|e| {
            Errors::from(
                Diagnostic::error(
                    codes::IMPORT_BAD_DECLARATION,
                    format!("cannot read declaration: {e}"),
                )
                .file(&shown),
            )
        })?;
        let bad = |m: String| {
            Errors::from(
                Diagnostic::error(codes::IMPORT_BAD_DECLARATION, m)
                    .file(&shown)
                    .fix(
                        "see the declaration format in docs/ai/ASSETS-AND-LICENSES.md; a person must fix it",
                    ),
            )
        };
        let declaration: Declaration =
            serde_json::from_slice(&bytes).map_err(|e| bad(format!("invalid declaration: {e}")))?;
        if declaration.format != DECLARATION_FORMAT {
            return Err(bad(format!("`format` must be `{DECLARATION_FORMAT}`")));
        }
        for (name, v) in [
            ("store", &declaration.store),
            ("product", &declaration.product),
            ("license_type", &declaration.license_type),
        ] {
            if v.trim().is_empty() {
                return Err(bad(format!("`{name}` is empty")));
            }
        }
        Ok(Self {
            declaration,
            file_name: shown,
            sha256: crate::sha256_hex(&bytes),
        })
    }

    /// Length of the longest `covers` prefix matching `rel`, or 0 when it covers everything.
    /// `None` when it does not cover `rel`.
    pub fn coverage(&self, rel: &str) -> Option<usize> {
        if self.declaration.covers.is_empty() {
            return Some(0);
        }
        let rel = rel.to_ascii_lowercase();
        self.declaration
            .covers
            .iter()
            .map(|c| c.replace('\\', "/").to_ascii_lowercase())
            .filter(|c| {
                let c = c.trim_end_matches('/');
                rel == c || rel.starts_with(&format!("{c}/"))
            })
            .map(|c| c.len())
            .max()
    }
}

/// The license record stored on every imported asset (rule 6 and 11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LicenseRecord {
    /// Always `declared`: undeclared assets are never imported.
    pub status: String,
    pub store: String,
    pub product: String,
    pub license_type: String,
    /// Always true on an imported asset.
    pub other_engines_allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// File name of the declaration that covers this asset.
    pub declaration_file: String,
    /// SHA-256 of that declaration file, so a later change to it is visible.
    pub declaration_sha256: String,
}

impl LicenseRecord {
    pub fn from_declaration(d: &LoadedDeclaration) -> Self {
        Self {
            status: "declared".into(),
            store: d.declaration.store.clone(),
            product: d.declaration.product.clone(),
            license_type: d.declaration.license_type.clone(),
            other_engines_allowed: d.declaration.other_engines_allowed == YesNo::Yes,
            publisher: d.declaration.publisher.clone(),
            declaration_file: d.file_name.clone(),
            declaration_sha256: d.sha256.clone(),
        }
    }
}

/// One phrase in pasted license text that limits use to one engine.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct EngineLimit {
    /// The engine the text limits use to.
    pub engine: String,
    /// The matching phrase, normalized (lowercase, single spaces).
    pub phrase: String,
}

const ENGINES: [&str; 5] = ["unity", "unity engine", "unreal", "unreal engine", "ue"];

fn limit_phrases() -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for e in ENGINES {
        let engine = if e.contains("real") || e == "ue" {
            "Unreal Engine"
        } else {
            "Unity"
        };
        for adverb in ["only", "exclusively", "solely"] {
            for be in ["", "be used ", "for use ", "used "] {
                for prep in ["with", "in", "for", "within", "inside"] {
                    for the in ["", "the "] {
                        out.push((format!("{adverb} {be}{prep} {the}{e} "), engine));
                    }
                }
            }
        }
        for prep in ["for", "in", "with", "within"] {
            out.push((format!("{prep} {e} only"), engine));
            out.push((format!("{prep} the {e} only"), engine));
        }
        out.push((format!("{e}-only"), engine));
        out.push((format!("{e} engine-only"), engine));
        for verb in ["restricted", "limited"] {
            for the in ["", "the "] {
                out.push((format!("use is {verb} to {the}{e} "), engine));
                out.push((format!("{verb} to use with {the}{e} "), engine));
                out.push((format!("{verb} to use in {the}{e} "), engine));
            }
        }
        for the in ["", "the "] {
            out.push((format!("not be used outside {the}{e} "), engine));
            out.push((format!("not be used outside of {the}{e} "), engine));
        }
    }
    for any in ["", "any "] {
        for game in ["", "game "] {
            for prep in ["with", "in"] {
                out.push((
                    format!("not be used {prep} {any}other {game}engine"),
                    "one engine",
                ));
            }
        }
    }
    out
}

/// Checks license text a person pasted in (rule 10). Returns every phrase that limits use to
/// one engine; an empty list means no such phrase was found (it never grants permission: a
/// declaration saying `other_engines_allowed: yes` is still required).
pub fn engine_limits(text: &str) -> Vec<EngineLimit> {
    let norm: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let hay = format!(" {norm} ");
    let mut found: Vec<EngineLimit> = Vec::new();
    for (phrase, engine) in limit_phrases() {
        if hay.contains(&format!(" {phrase}")) {
            let phrase = phrase.trim().to_string();
            if !found
                .iter()
                .any(|f| f.phrase.contains(&phrase) || phrase.contains(&f.phrase))
            {
                found.push(EngineLimit {
                    engine: engine.into(),
                    phrase,
                });
            }
        }
    }
    found
}

/// Publishers whose content is engine-locked (rule 7).
pub fn is_engine_publisher(name: &str) -> bool {
    let n: String = name
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    matches!(
        n.as_str(),
        "unity" | "unitytechnologies" | "unitytechnologiesinc" | "epic" | "epicgames" | "epicgamesinc"
    )
}

/// Folder names that hold engine sample or starter content, or content Unity publishes.
const ENGINE_FOLDERS: [&str; 9] = [
    "standard assets",
    "standardassets",
    "starterassets",
    "starter assets",
    "tutorialinfo",
    "unity technologies",
    "unitytechnologies",
    "textmesh pro",
    "startercontent",
];

/// Why a path inside a Unity source is engine content (rule 7), if it is.
pub fn engine_content_reason(rel: &str) -> Option<String> {
    let parts: Vec<String> = rel.split('/').map(str::to_ascii_lowercase).collect();
    if parts.first().map(String::as_str) == Some("packages") {
        return Some("files under Packages/ are engine or package-manager content".into());
    }
    if parts.first().map(String::as_str) == Some("assets")
        && parts.get(1).map(String::as_str) == Some("samples")
    {
        return Some("Assets/Samples/ holds engine package samples".into());
    }
    let dirs = &parts[..parts.len().saturating_sub(1)];
    for d in dirs {
        if ENGINE_FOLDERS.contains(&d.as_str()) {
            return Some(format!(
                "folder `{d}` holds engine sample, starter or publisher content"
            ));
        }
        if d.starts_with("com.unity.") || d.starts_with("com.epicgames.") {
            return Some(format!("folder `{d}` is an engine package"));
        }
    }
    None
}

/// Why a whole package is engine content, judged from where the store saved it (the Asset
/// Store cache keeps one folder per publisher), if it is.
pub fn package_location_reason(path: &Path) -> Option<String> {
    let comps: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    let store = comps.iter().position(|c| c.starts_with("asset store"))?;
    let publisher = comps.get(store + 1)?;
    is_engine_publisher(publisher)
        .then(|| format!("the package was published by `{publisher}` (Asset Store cache)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_limited_text_is_found() {
        for t in [
            "This content may only be used with Unity.",
            "Licensed for use exclusively in the Unreal Engine and nowhere else.",
            "Unreal Engine-only content.",
            "The assets may not be used in any other game engine.",
            "Use is restricted to Unity products.",
            "For Unity only.",
        ] {
            assert!(!engine_limits(t).is_empty(), "missed: {t}");
        }
        for t in [
            "You may use this asset in any engine or software.",
            "Royalty free, commercial use allowed. Works great with Unity and Godot.",
            "Made in Unity; usable only with attribution.",
        ] {
            assert!(
                engine_limits(t).is_empty(),
                "false positive: {t} -> {:?}",
                engine_limits(t)
            );
        }
    }

    #[test]
    fn engine_content_paths() {
        assert!(engine_content_reason("Assets/Standard Assets/Characters/x.fbx").is_some());
        assert!(engine_content_reason("Assets/Samples/Universal RP/14.0/x.png").is_some());
        assert!(engine_content_reason("Assets/StarterAssets/x.mat").is_some());
        assert!(engine_content_reason("Packages/com.unity.render-pipelines/x.png").is_some());
        assert!(engine_content_reason("Assets/MyPack/Textures/x.png").is_none());
        assert!(engine_content_reason("Assets/MyPack/Samples/x.png").is_none());
        assert!(is_engine_publisher("Unity Technologies"));
        assert!(is_engine_publisher("Epic Games"));
        assert!(!is_engine_publisher("Some Studio"));
        let p = Path::new("cache/Asset Store-5.x/Unity Technologies/3D/Pack.unitypackage");
        assert!(package_location_reason(p).is_some());
        let p = Path::new("cache/Asset Store-5.x/Some Studio/3D/Pack.unitypackage");
        assert!(package_location_reason(p).is_none());
    }
}
