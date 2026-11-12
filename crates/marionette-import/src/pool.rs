//! The pool admission gate for owned assets (ASSETS-AND-LICENSES.md rules 5 and 13).
//!
//! The CC0 pool MUST call [`check`] before accepting anything. This gate refuses owned
//! assets: anything inside an owned library, anything carrying an owned-asset record or a
//! license declaration, and any file whose content equals a file in the owned library (so a
//! copied or renamed owned file is caught too). The pool adds its own CC0-and-source rule on
//! top; passing this gate alone does not make an asset CC0.

use crate::codes;
use crate::library::{LIBRARY_FORMAT, Library, RECORD_FORMAT};
use crate::sha256_hex;
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One reason the pool must refuse a candidate.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PoolRefusal {
    /// File inside the candidate (relative), or `.` for the candidate itself.
    pub path: String,
    /// `pool.owned_asset`, `pool.owned_content` or `pool.not_cc0`.
    pub code: String,
    pub reason: String,
}

/// The gate's answer.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PoolVerdict {
    /// False when any refusal applies. True only means "not an owned asset".
    pub admitted_by_owned_gate: bool,
    pub files_checked: usize,
    /// Whether file contents were compared against an owned library.
    pub compared_with_library: bool,
    pub refusals: Vec<PoolRefusal>,
}

fn json_format(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("format")?.as_str().map(str::to_string)
}

fn files_under(p: &Path) -> Vec<PathBuf> {
    if p.is_file() {
        return vec![p.to_path_buf()];
    }
    let mut out = Vec::new();
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Checks a pool candidate (a file or folder) against the owned-asset rules.
pub fn check(candidate: &Path, library: Option<&Library>) -> PoolVerdict {
    let mut refusals = Vec::new();
    let candidate = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    let base = if candidate.is_file() {
        candidate.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        candidate.clone()
    };
    let refuse = |refusals: &mut Vec<PoolRefusal>, path: String, code: &str, reason: String| {
        refusals.push(PoolRefusal {
            path,
            code: code.into(),
            reason,
        })
    };

    // Inside an owned library or an owned asset folder?
    for anc in base.ancestors() {
        if json_format(&anc.join("library.json")).as_deref() == Some(LIBRARY_FORMAT) {
            refuse(
                &mut refusals,
                ".".into(),
                codes::POOL_OWNED_ASSET,
                "the candidate is inside an owned library".into(),
            );
            break;
        }
        if json_format(&anc.join("record.json")).as_deref() == Some(RECORD_FORMAT) {
            refuse(
                &mut refusals,
                ".".into(),
                codes::POOL_OWNED_ASSET,
                "the candidate is an owned-library asset".into(),
            );
            break;
        }
    }

    let owned: BTreeMap<String, String> = library
        .map(|lib| {
            let mut m = BTreeMap::new();
            for r in lib.records() {
                m.insert(r.source.sha256.clone(), r.id.clone());
                for f in &r.files {
                    m.insert(f.sha256.clone(), r.id.clone());
                }
            }
            m
        })
        .unwrap_or_default();

    let files = files_under(&candidate);
    for f in &files {
        let rel = f
            .strip_prefix(&base)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        let name = f
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if crate::is_declaration_file_name(&name) {
            refuse(
                &mut refusals,
                rel.clone(),
                codes::POOL_NOT_CC0,
                "carries an owned-asset license declaration".into(),
            );
        }
        if name == "record.json" && json_format(f).as_deref() == Some(RECORD_FORMAT) {
            refuse(
                &mut refusals,
                rel.clone(),
                codes::POOL_OWNED_ASSET,
                "an owned-library asset record".into(),
            );
        }
        if !owned.is_empty()
            && let Ok(bytes) = std::fs::read(f)
            && let Some(id) = owned.get(&sha256_hex(&bytes))
        {
            refuse(
                &mut refusals,
                rel,
                codes::POOL_OWNED_CONTENT,
                format!("same content as owned asset `{id}`"),
            );
        }
    }
    PoolVerdict {
        admitted_by_owned_gate: refusals.is_empty(),
        files_checked: files.len(),
        compared_with_library: library.is_some(),
        refusals,
    }
}
