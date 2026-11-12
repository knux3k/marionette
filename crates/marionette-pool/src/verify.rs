//! Checks a whole pool without changing it: every record readable and CC0 with proof, every
//! listed file present with its hash, no unlisted files, no declarations or provenance records
//! left in an asset, online assets from a known library's domain, and `index.json` current.
//! The assets repository's CI runs this through the `pool_verify` operation.

use crate::admit::{is_dedication_file_name, is_license_file_name};
use crate::{ASSET_FORMAT, Origin, POOL_FORMAT, Pool, PoolAsset, net, sha256_hex, sources, valid_id};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

/// One thing wrong with the pool.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Problem {
    /// The asset id (folder name), or empty for the pool as a whole.
    pub asset: String,
    /// A short code: `pool_json`, `record`, `license`, `file_missing`, `file_changed`,
    /// `file_unlisted`, `declaration`, `main`, `origin`, `index`.
    pub code: String,
    pub message: String,
}

/// What `verify` found.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct VerifyReport {
    pub assets: usize,
    pub files: usize,
    pub problems: Vec<Problem>,
}

fn problem(out: &mut Vec<Problem>, asset: &str, code: &str, message: String) {
    out.push(Problem {
        asset: asset.into(),
        code: code.into(),
        message,
    });
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        match std::fs::symlink_metadata(e.path()) {
            Ok(m) if m.is_dir() => walk(&e.path(), &rel, out),
            Ok(_) => out.push(rel),
            Err(_) => {}
        }
    }
}

fn check_asset(dir: &Path, folder: &str, out: &mut Vec<Problem>) -> usize {
    let rec: PoolAsset = match std::fs::read(dir.join("asset.json"))
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
    {
        Ok(r) => r,
        Err(e) => {
            problem(
                out,
                folder,
                "record",
                format!("asset.json is missing or unreadable: {e}"),
            );
            return 0;
        }
    };
    if rec.format != ASSET_FORMAT {
        problem(
            out,
            folder,
            "record",
            format!("format is `{}`, expected `{ASSET_FORMAT}`", rec.format),
        );
    }
    if rec.id != folder || !valid_id(&rec.id) {
        problem(
            out,
            folder,
            "record",
            format!("id `{}` does not match its folder or is not a valid id", rec.id),
        );
    }
    if rec.license.spdx != "CC0-1.0" {
        problem(
            out,
            folder,
            "license",
            format!("license is `{}`, only CC0-1.0 is allowed", rec.license.spdx),
        );
    }
    if rec.license.proof.is_empty() {
        problem(out, folder, "license", "no proof that the asset is CC0".into());
    }
    match rec.source.origin {
        Origin::Made => {
            if rec.source.method.as_deref().is_none_or(|m| m.trim().is_empty()) {
                problem(out, folder, "origin", "a made asset needs `source.method`".into());
            }
        }
        Origin::Online => match rec.source.library.as_deref().and_then(sources::library) {
            None => problem(
                out,
                folder,
                "origin",
                "an online asset needs a known CC0 library".into(),
            ),
            Some(lib) => {
                for url in rec.source.downloads.iter().chain(rec.source.page.iter()) {
                    if !lib.domains.iter().any(|d| net::on_domain(url, d)) {
                        problem(
                            out,
                            folder,
                            "origin",
                            format!("`{url}` is not on {}'s domains", lib.name),
                        );
                    }
                }
            }
        },
    }

    let mut listed = BTreeSet::new();
    for f in &rec.files {
        listed.insert(f.path.clone());
        match std::fs::read(dir.join(&f.path)) {
            Err(_) => problem(
                out,
                folder,
                "file_missing",
                format!("`{}` is listed but missing", f.path),
            ),
            Ok(bytes) => {
                if sha256_hex(&bytes) != f.sha256 || bytes.len() as u64 != f.bytes {
                    problem(
                        out,
                        folder,
                        "file_changed",
                        format!("`{}` does not match its recorded hash", f.path),
                    );
                }
            }
        }
    }
    if let Some(main) = &rec.main
        && !listed.contains(main)
    {
        problem(
            out,
            folder,
            "main",
            format!("main file `{main}` is not one of the asset's files"),
        );
    }
    let mut on_disk = Vec::new();
    walk(dir, "", &mut on_disk);
    on_disk.sort();
    for rel in &on_disk {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        let lower = name.to_ascii_lowercase();
        if marionette_import::is_declaration_file_name(name) {
            problem(
                out,
                folder,
                "declaration",
                format!("`{rel}` is an owned-asset license declaration"),
            );
        } else if lower.ends_with(".provenance.json") {
            problem(
                out,
                folder,
                "declaration",
                format!("`{rel}` is a generated asset's provenance record"),
            );
        } else if is_dedication_file_name(name) && !listed.contains(rel) {
            problem(
                out,
                folder,
                "declaration",
                format!("`{rel}` is a dedication left beside the files"),
            );
        } else if rel != "asset.json" && !listed.contains(rel) {
            let hint = if is_license_file_name(rel) {
                " (license files must be listed too)"
            } else {
                ""
            };
            problem(
                out,
                folder,
                "file_unlisted",
                format!("`{rel}` is not listed in asset.json{hint}"),
            );
        }
    }
    rec.files.len()
}

/// Checks the pool at `pool`. Never writes.
pub fn verify(pool: &Pool) -> VerifyReport {
    let mut problems = Vec::new();
    let root = pool.root();
    match std::fs::read(root.join("pool.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    {
        Some(v) if v["format"] == POOL_FORMAT => {}
        _ => problem(
            &mut problems,
            "",
            "pool_json",
            format!("pool.json is missing or not `{POOL_FORMAT}`"),
        ),
    }
    let mut folders: Vec<String> = std::fs::read_dir(root.join("assets"))
        .map(|d| {
            d.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    folders.sort();
    let mut files = 0;
    for f in &folders {
        files += check_asset(&root.join("assets").join(f), f, &mut problems);
    }
    let expected = pool.index_text();
    match std::fs::read_to_string(root.join("index.json")) {
        Ok(t) if t == expected => {}
        Ok(_) => problem(
            &mut problems,
            "",
            "index",
            "index.json is out of date: run pool_index".into(),
        ),
        Err(_) => problem(
            &mut problems,
            "",
            "index",
            "index.json is missing: run pool_index".into(),
        ),
    }
    VerifyReport {
        assets: folders.len(),
        files,
        problems,
    }
}
