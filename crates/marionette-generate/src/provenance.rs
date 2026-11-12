//! Provenance: `<name>.provenance.json` beside every generated asset. It records where the
//! asset came from (recipe, provider, model, prompt, reference image, cache key, cost), what
//! post-processing did, the license status, and that the asset is blocked from the CC0 pool.
//! It holds no time stamps, so rebuilding a recipe from the cache gives identical files.

use crate::recipe::OutputKind;
use marionette_providers::Kind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FORMAT: &str = "marionette.generated_asset/1";
pub const SUFFIX: &str = ".provenance.json";
/// Pool status of every generated asset.
pub const POOL_BLOCKED: &str = "blocked";
pub const POOL_REASON: &str = "generated assets are not offered to the CC0 pool: the provider's terms decide who owns the output, so it is not CC0 by default (decision 0045)";

/// The reference image a model was made from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FromImage {
    pub name: String,
    pub file: String,
    pub sha256: String,
    /// A person approved this exact image (required when the model provider is paid).
    pub approved: bool,
    /// The owner's personal config waived the approval for this model's provider
    /// (`waive_reference_approval`, decision 0047).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub approval_waived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub provider: String,
    pub model: String,
    /// The provider's request format has been checked against the live service.
    pub provider_verified: bool,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[u32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<FromImage>,
    /// Cache key of the provider call (`.marionette/generate-cache/<key>.*`).
    pub cache_key: String,
    /// SHA-256 of the provider's bytes before post-processing.
    pub raw_sha256: String,
    /// What the provider call cost when it was made, US dollars.
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct License {
    /// Always `generated`.
    pub status: String,
    pub terms: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub format: String,
    pub name: String,
    pub output: OutputKind,
    /// The asset file, beside this record.
    pub file: String,
    pub sha256: String,
    pub recipe: String,
    pub recipe_sha256: String,
    pub source: Source,
    /// Post-processing report (image or model).
    pub postprocess: serde_json::Value,
    pub license: License,
    /// Always `blocked`.
    pub pool: String,
    pub pool_reason: String,
}

impl Provenance {
    pub fn to_text(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("serializes");
        s.push('\n');
        s
    }
}

/// Terms note for a provider's output.
pub fn terms(provider: &str) -> String {
    match provider {
        "mock" => "made by the offline mock provider; no third-party terms apply".into(),
        p => format!(
            "made with the `{p}` provider; its terms of service decide who owns this output and how it may be used. Check them before publishing"
        ),
    }
}

/// Reads `dir/<name>.provenance.json`, if present and valid.
pub fn read(dir: &Path, name: &str) -> Option<Provenance> {
    std::fs::read_to_string(dir.join(format!("{name}{SUFFIX}")))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

/// Pool answer for one file.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PoolStatus {
    pub generated: bool,
    pub admissible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// For the CC0 pool's admission check: a file is a generated asset when a provenance record
/// sits beside it (`<stem>.provenance.json` naming this file), or when its bytes match the
/// `sha256` of any provenance record in `generated_dirs` (copies). Generated assets are never
/// admissible.
pub fn pool_status(file: &Path, generated_dirs: &[&Path]) -> PoolStatus {
    let blocked = PoolStatus {
        generated: true,
        admissible: false,
        reason: Some(POOL_REASON.into()),
    };
    let fname = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if let (Some(dir), Some(stem)) = (file.parent(), fname.rsplit_once('.').map(|(s, _)| s))
        && read(dir, stem).is_some_and(|p| p.file == fname)
    {
        return blocked;
    }
    if let Ok(bytes) = std::fs::read(file) {
        let sha = crate::sha256_hex(&bytes);
        for dir in generated_dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for e in entries.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if let Some(stem) = n.strip_suffix(SUFFIX)
                    && read(dir, stem).is_some_and(|p| p.sha256 == sha || p.source.raw_sha256 == sha)
                {
                    return blocked;
                }
            }
        }
    }
    PoolStatus {
        generated: false,
        admissible: true,
        reason: None,
    }
}
