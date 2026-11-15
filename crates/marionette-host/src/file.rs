//! Replacing a file so a crash or power loss never leaves it unreadable (decision 0077).
//!
//! [`write_replacing`] writes the new bytes to a temporary file next to the target
//! (`.<name>.<pid>.tmp`, flushed to disk), renames the old target to the backup path if one is
//! given, then renames the temporary file onto the target and flushes the folder on Unix. A
//! crash at any point leaves the new file, the old file, or (between the two renames) the old
//! file as the backup, so a reader that falls back to the backup always finds a whole file.
//!
//! The temporary file sits beside the target, not in the run folder ([`crate::run_dir`]),
//! because a rename is atomic only inside one volume. It is removed when the write fails.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Writes `bytes` to `target` crash-safely, keeping the previous file at `backup` when given
/// (an existing backup is replaced). Returns the number of bytes written.
pub fn write_replacing(target: &Path, bytes: &[u8], backup: Option<&Path>) -> std::io::Result<u64> {
    let tmp = temp_beside(target)?;
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if let Some(backup) = backup
            && target.exists()
        {
            std::fs::rename(target, backup)?;
        }
        std::fs::rename(&tmp, target)?;
        if let Some(dir) = target.parent() {
            sync_dir(dir);
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map(|()| bytes.len() as u64)
}

/// `.<name>.<pid>.tmp` in the target's folder.
fn temp_beside(target: &Path) -> std::io::Result<PathBuf> {
    let Some(name) = target.file_name() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} names no file", target.display()),
        ));
    };
    let mut tmp = std::ffi::OsString::from(".");
    tmp.push(name);
    tmp.push(format!(".{}.tmp", std::process::id()));
    Ok(target.with_file_name(tmp))
}

/// Flushes a folder's entries (the renames) to disk. Windows has no folder flush; NTFS logs
/// renames itself.
pub fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_and_keeps_a_backup() {
        let dir = crate::temp_dir("write-replacing").unwrap();
        let target = dir.path().join("a.json");
        let backup = dir.path().join("a.json.bak");
        assert_eq!(write_replacing(&target, b"one", Some(&backup)).unwrap(), 3);
        assert!(!backup.exists(), "no old file, no backup");
        write_replacing(&target, b"two", Some(&backup)).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"two");
        assert_eq!(std::fs::read(&backup).unwrap(), b"one");
        write_replacing(&target, b"three", None).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"three");
        assert_eq!(
            std::fs::read(&backup).unwrap(),
            b"one",
            "no backup asked: left alone"
        );
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty(), "no temporary file left");
    }

    #[test]
    fn a_failed_write_leaves_the_old_file() {
        let dir = crate::temp_dir("write-replacing-fail").unwrap();
        let target = dir.path().join("b.json");
        write_replacing(&target, b"old", None).unwrap();
        // A backup path in a folder that does not exist: the rename fails after the temp write.
        let backup = dir.path().join("missing").join("b.json.bak");
        assert!(write_replacing(&target, b"new", Some(&backup)).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "temp removed");
    }
}
