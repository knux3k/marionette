//! [`OfflinePlatform`]: achievements, stats and cloud files in a local folder.
//!
//! Layout: `<dir>/achievements.json` (unlocked ids and stats, written by `store`) and
//! `<dir>/cloud/<name>` (one file per cloud file). Writes go to a temporary name first and are
//! renamed into place, so a crash leaves the old or the new file, never half of one.

use crate::{Achievements, CloudFiles, PlatformError, Result, StatValue, check_name, codes};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Default, Serialize, Deserialize)]
struct State {
    unlocked: BTreeSet<String>,
    stats: BTreeMap<String, StatValue>,
}

pub struct OfflinePlatform {
    dir: PathBuf,
    state: State,
    cloud_enabled: bool,
}

fn io(path: &Path, e: std::io::Error) -> PlatformError {
    PlatformError::new(codes::IO, format!("{}: {e}", path.display()))
}

pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
    }
    let tmp = path.with_extension("tmp-write");
    std::fs::write(&tmp, data).map_err(|e| io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| io(path, e))
}

impl OfflinePlatform {
    /// Opens (or starts) the platform folder `dir`.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        let path = dir.join("achievements.json");
        let state = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b)
                .map_err(|e| PlatformError::new(codes::IO, format!("{}: {e}", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => return Err(io(&path, e)),
        };
        Ok(Self {
            dir,
            state,
            cloud_enabled: true,
        })
    }

    /// Turns the simulated cloud off (as a player who disabled it).
    pub fn set_cloud_enabled(&mut self, on: bool) {
        self.cloud_enabled = on;
    }

    fn cloud_path(&self, name: &str) -> Result<PathBuf> {
        check_name(name)?;
        Ok(self.dir.join("cloud").join(name))
    }

    fn need_cloud(&self) -> Result<()> {
        if self.cloud_enabled {
            Ok(())
        } else {
            Err(PlatformError::new(codes::UNAVAILABLE, "cloud storage is off"))
        }
    }
}

impl Achievements for OfflinePlatform {
    fn unlock(&mut self, id: &str) -> Result<bool> {
        Ok(self.state.unlocked.insert(id.to_string()))
    }
    fn is_unlocked(&self, id: &str) -> Result<bool> {
        Ok(self.state.unlocked.contains(id))
    }
    fn clear(&mut self, id: &str) -> Result<()> {
        self.state.unlocked.remove(id);
        Ok(())
    }
    fn set_stat(&mut self, id: &str, value: StatValue) -> Result<()> {
        self.state.stats.insert(id.to_string(), value);
        Ok(())
    }
    fn stat(&self, id: &str) -> Result<Option<StatValue>> {
        Ok(self.state.stats.get(id).copied())
    }
    fn store(&mut self) -> Result<()> {
        let text = serde_json::to_vec_pretty(&self.state).expect("state serializes");
        write_atomic(&self.dir.join("achievements.json"), &text)
    }
}

impl CloudFiles for OfflinePlatform {
    fn enabled(&self) -> bool {
        self.cloud_enabled
    }
    fn write(&mut self, name: &str, data: &[u8]) -> Result<()> {
        self.need_cloud()?;
        write_atomic(&self.cloud_path(name)?, data)
    }
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.need_cloud()?;
        let p = self.cloud_path(name)?;
        match std::fs::read(&p) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(&p, e)),
        }
    }
    fn delete(&mut self, name: &str) -> Result<()> {
        self.need_cloud()?;
        let p = self.cloud_path(name)?;
        match std::fs::remove_file(&p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io(&p, e)),
        }
    }
    fn list(&self) -> Result<Vec<(String, u64)>> {
        self.need_cloud()?;
        let dir = self.dir.join("cloud");
        let mut out = Vec::new();
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(io(&dir, e)),
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".tmp-write") {
                continue;
            }
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            out.push((name, len));
        }
        out.sort();
        Ok(out)
    }
}
