//! Approval of a generated image before credit is spent on a 3D model made from it
//! (ASSETS-AND-LICENSES.md rule 15). The approval is a file beside the image,
//! `<name>.approval.json`, binding the image's SHA-256: regenerating or changing the image
//! voids it.
//!
//! Only a person approves. No operation writes the file and every write helper refuses its
//! name; the CLI command `marionette generate approve <name>` writes it after a person types
//! the image name at an interactive terminal. Like the spend cap, this keeps an agent from
//! approving its own spending by accident; it is not a security boundary against a program
//! that writes files directly.
//!
//! An owner who does not want the step can waive it for named providers in their personal
//! config, never in a project (decision 0047): `"waive_reference_approval": ["qwen", "tripo"]`
//! in `%APPDATA%\marionette\config.json` (Windows) or `~/.config/marionette/config.json`. A
//! model made by a listed provider skips the approval of its reference image (images never
//! need one, so listing an image provider such as `qwen` only records the owner's intent).
//! Provenance then records `approval_waived: true` beside `approved: false`, so the
//! asset still says no person approved the image. No environment variable sets it.

use crate::codes;
use marionette_core::{Diagnostic, Errors};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FORMAT: &str = "marionette.approval/1";
pub const SUFFIX: &str = ".approval.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub format: String,
    /// The approved image file name, for example `crate_ref.png`.
    pub file: String,
    pub sha256: String,
}

pub fn is_approval_file_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(SUFFIX)
}

pub fn write_refused(shown: &str) -> Diagnostic {
    Diagnostic::error(
        codes::APPROVAL_WRITE_REFUSED,
        format!(
            "`{shown}` is an image approval; only a person may approve, with `marionette generate approve`"
        ),
    )
    .file(shown)
    .fix("ask the project owner to look at the image and run `marionette generate approve <name>`")
}

/// The providers the owner's personal config lists under `waive_reference_approval`,
/// lowercased (see the module docs).
pub fn waived_providers(var: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    marionette_providers::spend::user_config_file(var)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(t.trim_start_matches('\u{feff}')).ok())
        .and_then(|v| {
            v.get("waive_reference_approval")
                .and_then(|l| l.as_array().cloned())
        })
        .unwrap_or_default()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(|n| n.trim().to_ascii_lowercase())
        .filter(|n| !n.is_empty())
        .collect()
}

/// Whether `dir/<name>.approval.json` approves exactly these image bytes.
pub fn is_approved(dir: &Path, name: &str, image_sha256: &str) -> bool {
    std::fs::read_to_string(dir.join(format!("{name}{SUFFIX}")))
        .ok()
        .and_then(|t| serde_json::from_str::<Approval>(&t).ok())
        .is_some_and(|a| a.format == FORMAT && a.sha256 == image_sha256)
}

/// Writes the approval. Call only after a person confirmed at an interactive terminal (the
/// CLI does this); `confirmed_by_person` documents that at the call site.
pub fn approve_by_person(
    dir: &Path,
    name: &str,
    image_file: &str,
    image_sha256: &str,
    confirmed_by_person: bool,
) -> Result<(), Errors> {
    if !confirmed_by_person {
        return Err(Diagnostic::error(
            codes::APPROVAL_NEEDS_PERSON,
            "an image approval needs a person at an interactive terminal",
        )
        .fix("run `marionette generate approve <name>` yourself in a terminal")
        .into());
    }
    let a = Approval {
        format: FORMAT.into(),
        file: image_file.into(),
        sha256: image_sha256.into(),
    };
    let mut text = serde_json::to_string_pretty(&a).expect("serializes");
    text.push('\n');
    let path = dir.join(format!("{name}{SUFFIX}"));
    std::fs::write(&path, text).map_err(|e| {
        Diagnostic::error(
            marionette_core::error::codes::IO_WRITE,
            format!("cannot write {name}{SUFFIX}: {e}"),
        )
        .into()
    })
}
