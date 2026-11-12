//! Kill tests: a process is killed mid-run and nothing it started or wrote may survive it.
//!
//! The test binary plays every role: it re-runs itself with `ROLE` set to "middle" (an
//! engine process), "leaf" (a child the engine started) or "owner" (an engine process with a
//! run folder). Each helper role gives up after a minute so a failing test cannot leave
//! processes behind either.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const ROLE: &str = "MARIONETTE_HOST_TEST_ROLE";
const DIR: &str = "MARIONETTE_HOST_TEST_DIR";
const HELPER_LIFETIME: Duration = Duration::from_secs(60);

fn helper(test: &str, role: &str, dir: &Path) -> Command {
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(ROLE, role)
        .env(DIR, dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}

fn role() -> Option<(String, PathBuf)> {
    Some((std::env::var(ROLE).ok()?, PathBuf::from(std::env::var_os(DIR)?)))
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-host-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// True while some process holds the lock on `path`.
fn locked(path: &Path) -> bool {
    let f = OpenOptions::new().read(true).write(true).open(path).unwrap();
    matches!(f.try_lock(), Err(TryLockError::WouldBlock))
}

fn idle() -> ! {
    std::thread::sleep(HELPER_LIFETIME);
    std::process::exit(0);
}

fn kill(mut c: Child) {
    c.kill().unwrap();
    c.wait().unwrap();
}

#[test]
fn children_die_when_the_engine_is_killed() {
    const NAME: &str = "children_die_when_the_engine_is_killed";
    if let Some((role, dir)) = role() {
        match role.as_str() {
            "middle" => {
                marionette_host::init();
                let mut leaf = marionette_host::spawn(helper(NAME, "leaf", &dir)).unwrap();
                std::thread::sleep(HELPER_LIFETIME);
                let _ = leaf.kill();
                let _ = leaf.wait();
                std::process::exit(0);
            }
            "leaf" => {
                let f = File::create(dir.join("leaf.lock")).unwrap();
                f.lock().unwrap();
                std::fs::write(dir.join("ready"), b"").unwrap();
                idle();
            }
            _ => return,
        }
    }
    let dir = scratch("children");
    let middle = helper(NAME, "middle", &dir).spawn().unwrap();
    wait_for("the leaf to start", Duration::from_secs(30), || {
        dir.join("ready").exists()
    });
    assert!(
        locked(&dir.join("leaf.lock")),
        "the leaf holds its lock while alive"
    );
    // SIGKILL / TerminateProcess: no destructors, no exit handlers run in the middle process.
    kill(middle);
    wait_for("the leaf to die with its parent", Duration::from_secs(10), || {
        !locked(&dir.join("leaf.lock"))
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_killed_process_leaves_a_run_folder_that_the_next_start_removes() {
    const NAME: &str = "a_killed_process_leaves_a_run_folder_that_the_next_start_removes";
    if let Some((role, dir)) = role() {
        if role == "owner" {
            let t = marionette_host::temp_dir("work").unwrap();
            std::fs::write(t.path().join("partial.bin"), vec![7u8; 4096]).unwrap();
            std::fs::write(dir.join("ready"), b"").unwrap();
            idle();
        }
        return;
    }
    let dir = scratch("runs");
    let root = dir.join("runs");
    let owner = helper(NAME, "owner", &dir)
        .env(marionette_host::RUN_ROOT_ENV, &root)
        .spawn()
        .unwrap();
    wait_for("the owner to write", Duration::from_secs(30), || {
        dir.join("ready").exists()
    });
    let folders = || -> Vec<PathBuf> {
        std::fs::read_dir(&root)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    assert_eq!(folders().len(), 1);
    // A live owner is never swept, however old its folder is.
    std::thread::sleep(Duration::from_millis(2100));
    assert!(marionette_host::sweep_stale_in(&root).is_empty());
    kill(owner);
    assert_eq!(folders().len(), 1, "a kill leaves the folder behind");
    let swept = marionette_host::sweep_stale_in(&root);
    assert_eq!(swept.len(), 1, "the next start removes it");
    assert!(folders().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shutdown_removes_the_run_folder_and_temp_dirs_clean_up_on_drop() {
    let t = marionette_host::temp_dir("drop").unwrap();
    let inner = t.path().to_path_buf();
    std::fs::write(inner.join("x"), b"x").unwrap();
    let run = marionette_host::run_dir().unwrap();
    assert!(inner.starts_with(&run));
    drop(t);
    assert!(!inner.exists());
    marionette_host::shutdown();
    assert!(!run.exists());
}
