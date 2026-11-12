//! Proofs for plan line 9: the proof scenes under `games/` (decision 0050). Each game is a
//! project (`marionette.json`) with a build recipe (`build.ops.json`), its scenes, TypeScript
//! scripts, and scenario tests (`tests/*.test.json`) that play it with scripted input.
//!
//! For every game this checks, through the operation table (the code the CLI and the MCP
//! server run):
//! 1. Replaying the recipe with `batch_run` in an empty copy gives byte-identical scene files,
//!    so the scenes really are built through operations.
//! 2. Its scenario tests pass, twice, with identical results (state hashes included).
//! 3. It has at least one test that plays it with scripted input and pins the state hash.

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The proof games. A new game is added here and in `docs/ai/PROOF-SCENES.md`.
const GAMES: [&str; 6] = [
    "lantern-keeper",
    "ember-run",
    "cloudstep",
    "crystal-ward",
    "dust-circuit",
    "signal-yard",
];

fn games_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../games")
}

fn copy_dir(from: &Path, to: &Path, skip: &dyn Fn(&Path) -> bool) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let dest = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &dest, skip);
        } else if !skip(&path) {
            std::fs::copy(&path, &dest).unwrap();
        }
    }
}

fn scene_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(scene_files(&path));
        } else if path.to_string_lossy().ends_with(".scene.json") {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn every_game_directory_is_listed() {
    let mut found: Vec<String> = std::fs::read_dir(games_dir())
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().join("marionette.json").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    let mut listed: Vec<String> = GAMES.iter().map(|s| s.to_string()).collect();
    listed.sort();
    assert_eq!(
        found, listed,
        "games/ and GAMES in this test must name the same games"
    );
}

#[test]
fn recipes_rebuild_the_committed_scenes() {
    for game in GAMES {
        let src = games_dir().join(game);
        let tmp = std::env::temp_dir().join(format!("marionette-proof-{}-{game}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        copy_dir(&src, &tmp, &|p| p.to_string_lossy().ends_with(".scene.json"));
        let mut agent = Agent::new(&tmp, false);
        agent
            .call("batch_run", json!({ "file": "build.ops.json" }))
            .unwrap_or_else(|e| panic!("{game}: recipe failed: {e}"));
        let committed = scene_files(&src);
        assert!(!committed.is_empty(), "{game} has no scene");
        for scene in committed {
            let rel = scene.strip_prefix(&src).unwrap();
            let rebuilt = std::fs::read(tmp.join(rel))
                .unwrap_or_else(|_| panic!("{game}: the recipe does not create {}", rel.display()));
            assert!(
                rebuilt == std::fs::read(&scene).unwrap(),
                "{game}: {} differs from what build.ops.json makes; change the scene through \
                 operations and record them in the recipe",
                rel.display()
            );
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

fn run_tests(game: &str) -> Value {
    let mut agent = Agent::new(games_dir().join(game), false);
    agent
        .call("test_run", json!({}))
        .unwrap_or_else(|e| panic!("{game}: test_run failed: {e}"))
}

#[test]
fn scenario_tests_pass_twice_with_identical_results() {
    for game in GAMES {
        let first = run_tests(game);
        for r in first["results"].as_array().unwrap() {
            assert_eq!(
                r["passed"],
                true,
                "{game}: {}",
                serde_json::to_string_pretty(r).unwrap()
            );
        }
        assert_eq!(first["failed"], 0, "{game}");
        let second = run_tests(game);
        assert_eq!(first, second, "{game}: the second run differs from the first");
    }
}

#[test]
fn each_game_is_played_with_scripted_input_and_pins_its_hash() {
    for game in GAMES {
        let tests = games_dir().join(game).join("tests");
        let played = std::fs::read_dir(&tests)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with(".test.json"))
            .map(|p| serde_json::from_str::<Value>(&std::fs::read_to_string(p).unwrap()).unwrap())
            .filter(|t| t["input"].is_string())
            .filter(|t| {
                t["assertions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a["state_hash"].is_string())
            })
            .count();
        assert!(
            played > 0,
            "{game}: no test plays it with scripted input and checks the state hash"
        );
    }
}
