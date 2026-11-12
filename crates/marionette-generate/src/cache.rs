//! The generation cache: what a provider returned, keyed by the request, so the same request
//! is never paid for twice. Entries live in `<project>/.marionette/generate-cache/` as
//! `<key>.<png|glb>` (the provider's bytes, before post-processing) and `<key>.json`.
//!
//! The key is the SHA-256 of a canonical JSON object naming the provider, the model, the kind
//! and every request field that reaches the provider (a reference image by its SHA-256).
//! Post-processing options are not in the key: changing a texture size or a model height
//! reuses the paid output.

use crate::codes;
use marionette_core::{Diagnostic, Errors};
use marionette_providers::Kind;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

pub const DIR: &str = ".marionette/generate-cache";
pub const KEY_FORMAT: &str = "marionette.generate_key/1";

/// Everything that reaches the provider.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyInput<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub kind: Kind,
    pub prompt: Option<&'a str>,
    pub size: Option<[u32; 2]>,
    /// SHA-256 of the reference image, for a model made from an image.
    pub image_sha256: Option<&'a str>,
}

pub fn key(k: &KeyInput) -> String {
    let v = json!({
        "format": KEY_FORMAT,
        "provider": k.provider,
        "model": k.model,
        "kind": k.kind.as_str(),
        "prompt": k.prompt,
        "size": k.size,
        "image_sha256": k.image_sha256,
    });
    crate::sha256_hex(serde_json::to_string(&v).expect("serializes").as_bytes())
}

/// Metadata stored with each entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub key: String,
    pub provider: String,
    pub model: String,
    pub kind: Kind,
    /// What the call cost when it was made (zero for free providers).
    pub cost_usd: f64,
    pub provider_verified: bool,
    pub sha256: String,
}

fn ext(kind: Kind) -> &'static str {
    match kind {
        Kind::Image => "png",
        Kind::Model3d => "glb",
        Kind::Language => "txt",
    }
}

fn paths(root: &Path, key: &str, kind: Kind) -> (PathBuf, PathBuf) {
    let dir = root.join(DIR);
    (
        dir.join(format!("{key}.{}", ext(kind))),
        dir.join(format!("{key}.json")),
    )
}

fn corrupt(shown: &str, why: &str) -> Errors {
    Diagnostic::error(codes::CACHE, format!("cache entry {shown} is damaged: {why}"))
        .file(shown)
        .fix("delete the entry's two files (the next run calls the provider again, which may cost money)")
        .into()
}

/// The cached bytes and metadata, if present and intact.
pub fn get(root: &Path, key: &str, kind: Kind) -> Result<Option<(Vec<u8>, Meta)>, Errors> {
    let (data, meta) = paths(root, key, kind);
    let shown = format!("{DIR}/{key}.json");
    let meta_text = match std::fs::read_to_string(&meta) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(corrupt(&shown, &e.to_string())),
    };
    let m: Meta = serde_json::from_str(&meta_text).map_err(|e| corrupt(&shown, &e.to_string()))?;
    let bytes = std::fs::read(&data).map_err(|e| corrupt(&shown, &format!("data file: {e}")))?;
    if crate::sha256_hex(&bytes) != m.sha256 || m.key != key || m.kind != kind {
        return Err(corrupt(&shown, "the data does not match its record"));
    }
    Ok(Some((bytes, m)))
}

/// Stores an entry (data first, then the record, so a crash leaves no record without data).
pub fn put(root: &Path, meta: &Meta, bytes: &[u8]) -> Result<(), Errors> {
    let (data, rec) = paths(root, &meta.key, meta.kind);
    crate::write_file(&data, &format!("{DIR}/{}.{}", meta.key, ext(meta.kind)), bytes)?;
    let mut text = serde_json::to_string_pretty(meta).expect("serializes");
    text.push('\n');
    crate::write_file(&rec, &format!("{DIR}/{}.json", meta.key), text.as_bytes())
}
