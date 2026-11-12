//! The per-process run folder and the sweep of folders left by dead processes.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Environment variable that moves the runs root (tests use it to stay isolated).
pub const RUN_ROOT_ENV: &str = "MARIONETTE_RUN_ROOT";

/// Name of the lock file in every run folder. Its OS lock is held for the owner's lifetime.
const LOCK: &str = "owner.lock";

/// A folder is only judged once its lock file is this old, so a process that has created
/// the file but not locked it yet is never mistaken for a dead one.
const GRACE: Duration = Duration::from_secs(2);

struct Run {
    path: PathBuf,
    _lock: File,
}

static RUN: Mutex<Option<Run>> = Mutex::new(None);

/// Where run folders live: `$MARIONETTE_RUN_ROOT` if set, else `<temp>/marionette/runs`.
pub fn runs_root() -> PathBuf {
    match std::env::var_os(RUN_ROOT_ENV) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => std::env::temp_dir().join("marionette").join("runs"),
    }
}

fn lock_run() -> std::sync::MutexGuard<'static, Option<Run>> {
    RUN.lock().unwrap_or_else(|p| p.into_inner())
}

/// This process's run folder, created on first use. Everything the engine writes outside a
/// project goes here, so a crash leaves at most this folder, which the next start removes.
pub fn run_dir() -> io::Result<PathBuf> {
    let mut run = lock_run();
    if let Some(r) = run.as_ref() {
        return Ok(r.path.clone());
    }
    let root = runs_root();
    std::fs::create_dir_all(&root)?;
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let path = root.join(format!(
        "{}-{nanos:08x}{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path.join(LOCK))?;
    lock.lock()?;
    *run = Some(Run {
        path: path.clone(),
        _lock: lock,
    });
    Ok(path)
}

/// A folder inside the run folder, removed when dropped (and by the next start's sweep if the
/// process dies first).
#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A new empty folder in the run folder; `tag` names what it is for.
pub fn temp_dir(tag: &str) -> io::Result<TempDir> {
    static N: AtomicU64 = AtomicU64::new(0);
    let path = run_dir()?.join(format!("{tag}-{}", N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&path)?;
    Ok(TempDir { path })
}

/// Removes this process's run folder (clean exit). Later calls to [`run_dir`] make a new one.
pub fn shutdown() {
    let taken = lock_run().take();
    if let Some(Run { path, _lock }) = taken {
        drop(_lock);
        let _ = std::fs::remove_dir_all(path);
    }
}

/// Removes run folders under [`runs_root`] whose owner process is gone.
pub fn sweep_stale() -> Vec<PathBuf> {
    sweep_stale_in(&runs_root())
}

/// Removes run folders under `root` whose owner process is gone. A folder is stale when
/// its lock file can be locked (the OS drops a dead process's locks) or is missing, and the
/// lock file or folder is older than a short grace period.
pub fn sweep_stale_in(root: &Path) -> Vec<PathBuf> {
    let own = lock_run().as_ref().map(|r| r.path.clone());
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut swept = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if Some(&path) == own.as_ref() || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let lock_path = path.join(LOCK);
        let judged = std::fs::metadata(&lock_path).or_else(|_| std::fs::metadata(&path));
        let old_enough = judged
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age >= GRACE);
        if !old_enough {
            continue;
        }
        let dead = match OpenOptions::new().read(true).write(true).open(&lock_path) {
            Ok(f) => match f.try_lock() {
                Ok(()) => true,
                Err(TryLockError::WouldBlock) => false,
                Err(TryLockError::Error(_)) => false,
            },
            Err(e) => e.kind() == io::ErrorKind::NotFound,
        };
        if dead && std::fs::remove_dir_all(&path).is_ok() {
            swept.push(path);
        }
    }
    swept
}
