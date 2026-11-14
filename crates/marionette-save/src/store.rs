//! Save slots in one folder, written so a crash or power loss never leaves a slot
//! unloadable.
//!
//! Writing `<slot>`: the bytes go to `.<slot>.<pid>-<n>.tmp` and are flushed to disk; the current
//! `<slot>.msave` (if any) is renamed to `<slot>.msave.bak`; the temporary file is renamed
//! to `<slot>.msave`; on Unix the folder is flushed too. A crash at any point leaves either
//! the new file, or the old file, or (between the two renames) the old file as the backup.
//! Reading falls back to the backup when the slot file is missing or fails its checksum.
//!
//! Temporary files sit next to the save, not in the engine's run folder, because a rename
//! is atomic only inside one volume; [`SaveStore::open`] removes ones left by a crash.

use crate::codes;
use crate::format::{HEADER_MAX, SaveFile, SaveHeader, parse_header};
use marionette_core::{Diagnostic, Errors};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const EXT: &str = "msave";
/// Temporary files older than this are left over from a crash and are removed.
const STALE_TMP: Duration = Duration::from_secs(60);

/// A folder of save slots.
#[derive(Debug, Clone)]
pub struct SaveStore {
    dir: PathBuf,
}

/// One slot as a menu shows it (header only, nothing else read).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SlotInfo {
    pub slot: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<SaveHeader>,
    /// Set when the header cannot be read (loading would try the backup).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    pub has_backup: bool,
}

fn io(what: String) -> impl FnOnce(std::io::Error) -> Errors {
    move |e| Diagnostic::error(codes::IO, format!("{what}: {e}")).into()
}

/// Slot names: 1 to 48 of `a-z 0-9 _ -`, starting with a letter or digit.
pub fn check_slot(slot: &str) -> Result<(), Errors> {
    let ok = (1..=48).contains(&slot.len())
        && slot.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && slot
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(
            Diagnostic::error(codes::BAD_SLOT, format!("bad slot name `{slot}`"))
                .fix("use 1 to 48 of a-z, 0-9, `_`, `-`, starting with a letter or digit (e.g. `slot-1`)")
                .into(),
        )
    }
}

impl SaveStore {
    /// Opens (and creates) `dir`, removing temporary files left by a crash.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, Errors> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(io(format!("cannot create {}", dir.display())))?;
        let store = Self { dir };
        store.sweep(STALE_TMP);
        Ok(store)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self, slot: &str) -> PathBuf {
        self.dir.join(format!("{slot}.{EXT}"))
    }

    pub fn backup_path(&self, slot: &str) -> PathBuf {
        self.dir.join(format!("{slot}.{EXT}.bak"))
    }

    /// Removes temporary files older than `age`. Returns how many.
    pub fn sweep(&self, age: Duration) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let now = SystemTime::now();
        let mut removed = 0;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.starts_with('.') && name.ends_with(".tmp")) {
                continue;
            }
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_none_or(|d| d >= age);
            if old && std::fs::remove_file(e.path()).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Writes `file` to `slot` crash-safely (see the module docs). Returns bytes written.
    pub fn write(&self, slot: &str, file: &SaveFile) -> Result<u64, Errors> {
        check_slot(slot)?;
        let bytes = file.to_bytes()?;
        let target = self.path(slot);
        let tmp = self.dir.join(format!(".{slot}.{}.tmp", std::process::id()));
        let shown = target.display().to_string();
        let result = (|| {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            drop(f);
            if target.exists() {
                std::fs::rename(&target, self.backup_path(slot))?;
            }
            std::fs::rename(&tmp, &target)?;
            sync_dir(&self.dir);
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result.map_err(io(format!("cannot write save {shown}")))?;
        Ok(bytes.len() as u64)
    }

    /// Reads `slot`, falling back to its backup. The second value says why the backup was
    /// used, if it was.
    pub fn read(&self, slot: &str) -> Result<(SaveFile, Option<String>), Errors> {
        check_slot(slot)?;
        let main = read_file(&self.path(slot));
        let main_problem = match main {
            Ok(f) => return Ok((f, None)),
            Err(e) => e,
        };
        match read_file(&self.backup_path(slot)) {
            Ok(f) => Ok((f, Some(main_problem.message))),
            Err(bak) if main_problem.code == codes::NOT_FOUND && bak.code == codes::NOT_FOUND => Err(
                Diagnostic::error(codes::NOT_FOUND, format!("no save in slot `{slot}`"))
                    .fix("save_list shows the slots")
                    .into(),
            ),
            Err(bak) => Err(Diagnostic::error(
                main_problem.code,
                format!(
                    "slot `{slot}` cannot be loaded: {}; backup: {}",
                    main_problem.message, bak.message
                ),
            )
            .into()),
        }
    }

    /// Every slot in the folder, sorted by name, reading only headers.
    pub fn list(&self) -> Result<Vec<SlotInfo>, Errors> {
        let entries =
            std::fs::read_dir(&self.dir).map_err(io(format!("cannot list {}", self.dir.display())))?;
        let mut slots = std::collections::BTreeSet::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let slot = name
                .strip_suffix(&format!(".{EXT}"))
                .or_else(|| name.strip_suffix(&format!(".{EXT}.bak")));
            if let Some(s) = slot
                && check_slot(s).is_ok()
            {
                slots.insert(s.to_string());
            }
        }
        Ok(slots.into_iter().map(|s| self.info(&s)).collect())
    }

    /// One slot's menu entry (header only).
    pub fn info(&self, slot: &str) -> SlotInfo {
        let path = self.path(slot);
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let (header, problem) = match read_header(&path) {
            Ok(h) => (Some(h), None),
            Err(e) => (None, Some(e.message)),
        };
        SlotInfo {
            slot: slot.to_string(),
            bytes,
            header,
            problem,
            has_backup: self.backup_path(slot).exists(),
        }
    }

    /// Removes a slot and its backup. Returns whether anything was there.
    pub fn delete(&self, slot: &str) -> Result<bool, Errors> {
        check_slot(slot)?;
        let mut any = false;
        for p in [self.path(slot), self.backup_path(slot)] {
            match std::fs::remove_file(&p) {
                Ok(()) => any = true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io(format!("cannot delete {}", p.display()))(e)),
            }
        }
        Ok(any)
    }
}

struct Problem {
    code: &'static str,
    message: String,
}

fn read_file(path: &Path) -> Result<SaveFile, Problem> {
    let bytes = std::fs::read(path).map_err(|e| Problem {
        code: if e.kind() == std::io::ErrorKind::NotFound {
            codes::NOT_FOUND
        } else {
            codes::IO
        },
        message: format!("{}: {e}", path.display()),
    })?;
    SaveFile::from_bytes(&bytes).map_err(|e| {
        let first = e.0.into_iter().next();
        Problem {
            code: first.as_ref().map_or(codes::CORRUPT, |d| d.code),
            message: first.map_or_else(String::new, |d| d.message),
        }
    })
}

fn read_header(path: &Path) -> Result<SaveHeader, Problem> {
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take((HEADER_MAX + 64) as u64).read_to_end(&mut buf))
        .map_err(|e| Problem {
            code: codes::IO,
            message: e.to_string(),
        })?;
    parse_header(&buf).map(|(h, _)| h).map_err(|e| Problem {
        code: codes::CORRUPT,
        message: e.0.into_iter().next().map_or_else(String::new, |d| d.message),
    })
}

/// Flushes a folder's entries (the renames) to disk. Unix only; Windows has no folder
/// handle flush and commits renames with the file system journal.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}
