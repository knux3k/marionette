//! Keeping a local save folder and the platform's cloud files in step.
//!
//! Three-way, per file: the local copy, the cloud copy and the copy both had after the last
//! sync (the "base", kept in `<dir>/.cloud-sync.json`). Whichever side changed since the base
//! wins; if both changed, nothing is overwritten: the cloud copy is saved next to the local
//! one as `<name>.cloud-conflict` and the file is reported, so the game can ask the player.
//! The cloud side keeps a manifest (`cloud-sync.json`) with each file's hash, so checking
//! costs one small read, not a download of every save.
//!
//! Stores with automatic folder sync (Steam Auto-Cloud) need none of this; it is for games
//! that use the cloud file API directly, and it works the same against [`OfflinePlatform`].
//!
//! [`OfflinePlatform`]: crate::OfflinePlatform

use crate::offline::write_atomic;
use crate::{CloudFiles, PlatformError, Result, check_name, codes};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const BASE_FILE: &str = ".cloud-sync.json";
const MANIFEST: &str = "cloud-sync.json";
const CONFLICT_SUFFIX: &str = ".cloud-conflict";

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SyncReport {
    pub uploaded: Vec<String>,
    pub downloaded: Vec<String>,
    pub deleted_local: Vec<String>,
    pub deleted_cloud: Vec<String>,
    /// Changed on both sides; the cloud copy is in `<name>.cloud-conflict`.
    pub conflicts: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Hashes {
    files: BTreeMap<String, String>,
}

fn hash(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn read_json(path: &Path) -> Result<Hashes> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b)
            .map_err(|e| PlatformError::new(codes::IO, format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Hashes::default()),
        Err(e) => Err(PlatformError::new(codes::IO, format!("{}: {e}", path.display()))),
    }
}

/// Syncs the files in `dir` whose names end with one of `suffixes` (for example
/// `[".save"]`) with `cloud`. Returns what moved.
pub fn sync_dir(dir: &Path, suffixes: &[&str], cloud: &mut dyn CloudFiles) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    if !cloud.enabled() {
        return Ok(report);
    }
    let wanted = |n: &str| suffixes.iter().any(|s| n.ends_with(s)) && !n.ends_with(CONFLICT_SUFFIX);
    let base_path = dir.join(BASE_FILE);
    let base = read_json(&base_path)?;
    let mut manifest: Hashes = match cloud.read(MANIFEST)? {
        Some(b) => serde_json::from_slice(&b).unwrap_or_default(),
        None => Hashes::default(),
    };
    let mut local = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if wanted(&name) && e.path().is_file() {
                let data = std::fs::read(e.path())
                    .map_err(|err| PlatformError::new(codes::IO, format!("{name}: {err}")))?;
                local.insert(name, hash(&data));
            }
        }
    }
    let names: BTreeSet<String> = local
        .keys()
        .chain(manifest.files.keys().filter(|n| wanted(n)))
        .cloned()
        .collect();
    let mut new_base = BTreeMap::new();
    let mut manifest_changed = false;
    for name in names {
        check_name(&name)?;
        let l = local.get(&name);
        let c = manifest.files.get(&name).cloned();
        let b = base.files.get(&name);
        let path = dir.join(&name);
        if l == c.as_ref() {
            if let Some(h) = l {
                new_base.insert(name, h.clone());
            }
            continue;
        }
        let local_changed = l != b;
        let cloud_changed = c.as_ref() != b;
        match (local_changed, cloud_changed) {
            (true, false) => match l {
                Some(h) => {
                    let data = std::fs::read(&path)
                        .map_err(|e| PlatformError::new(codes::IO, format!("{name}: {e}")))?;
                    cloud.write(&name, &data)?;
                    manifest.files.insert(name.clone(), h.clone());
                    new_base.insert(name.clone(), h.clone());
                    report.uploaded.push(name);
                    manifest_changed = true;
                }
                None => {
                    cloud.delete(&name)?;
                    manifest.files.remove(&name);
                    report.deleted_cloud.push(name);
                    manifest_changed = true;
                }
            },
            (false, true) => match c {
                Some(h) => match cloud.read(&name)? {
                    Some(data) if hash(&data) == h => {
                        write_atomic(&path, &data)?;
                        new_base.insert(name.clone(), h);
                        report.downloaded.push(name);
                    }
                    // Manifest and file disagree (an interrupted upload): leave both.
                    _ => {
                        if let Some(h) = b {
                            new_base.insert(name.clone(), h.clone());
                        }
                    }
                },
                None => {
                    let _ = std::fs::remove_file(&path);
                    report.deleted_local.push(name);
                }
            },
            _ => {
                if let Some(data) = cloud.read(&name)? {
                    write_atomic(&dir.join(format!("{name}{CONFLICT_SUFFIX}")), &data)?;
                }
                if let Some(h) = b {
                    new_base.insert(name.clone(), h.clone());
                }
                report.conflicts.push(name);
            }
        }
    }
    if manifest_changed {
        cloud.write(
            MANIFEST,
            &serde_json::to_vec_pretty(&manifest).expect("serializes"),
        )?;
    }
    let text = serde_json::to_vec_pretty(&Hashes { files: new_base }).expect("serializes");
    write_atomic(&base_path, &text)?;
    Ok(report)
}
