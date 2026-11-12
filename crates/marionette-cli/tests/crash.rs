//! Killing the engine mid-run leaves nothing that outlives the next start (decision 0032).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_marionette");

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-cli-crash-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn folders(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

fn marionette(root: &Path) -> Command {
    let mut c = Command::new(BIN);
    c.current_dir(repo())
        .env("MARIONETTE_RUN_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}

#[test]
fn a_clean_exit_leaves_no_run_folder() {
    let root = scratch("clean").join("runs");
    let ok = marionette(&root)
        .args(["validate", "examples/drift.scene.json"])
        .status()
        .unwrap();
    assert!(ok.success());
    assert!(folders(&root).is_empty(), "{:?}", folders(&root));
}

#[test]
fn a_run_killed_mid_simulation_is_cleaned_up_by_the_next_start() {
    let dir = scratch("kill");
    let root = dir.join("runs");
    let mut child = marionette(&root)
        .args([
            "run",
            "examples/scripted.scene.json",
            "--headless",
            "--ticks",
            "1000000000",
        ])
        .spawn()
        .unwrap();
    let start = Instant::now();
    while folders(&root).is_empty() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the run folder never appeared"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Let it simulate for a moment, then kill it with no chance to clean up.
    std::thread::sleep(Duration::from_millis(300));
    assert!(child.try_wait().unwrap().is_none(), "still running when killed");
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(folders(&root).len(), 1, "the kill leaves the run folder behind");
    // The sweep only judges folders older than its grace period.
    std::thread::sleep(Duration::from_millis(2_100));
    let ok = marionette(&root)
        .args(["validate", "examples/drift.scene.json"])
        .status()
        .unwrap();
    assert!(ok.success());
    assert!(
        folders(&root).is_empty(),
        "the next start removed it: {:?}",
        folders(&root)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
