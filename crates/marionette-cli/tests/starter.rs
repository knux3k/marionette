//! Proofs for the game starter (decision 0150): a prompt or a brief becomes a playable,
//! tested project through the operation table (the code the CLI and the MCP server run).
//!
//! 1. Every template and view: `game_make` from a prompt writes a project whose autopilot test
//!    wins and whose idle test does not, and `test_run` passes in the new project.
//! 2. The same brief makes byte-identical files (levels, scripts, tests, hashes).
//! 3. `refresh` keeps edits outside the script's CONFIG block and re-pins the tests.
//! 4. Bad briefs are refused with every problem listed.

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn temp_root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-starter-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("marionette.json"),
        r#"{"format":"marionette.project","name":"t","version":1}"#,
    )
    .unwrap();
    d
}

fn files(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files(&p, base, out);
        } else {
            out.push((
                p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"),
                std::fs::read(&p).unwrap(),
            ));
        }
    }
}

const PROMPTS: [(&str, &str, &str); 3] = [
    (
        "maze2d",
        "a cozy game where you play as a moth who collects paper lanterns and avoids bats",
        "2d",
    ),
    (
        "maze3d",
        "a 3d neon city maze: a courier robot collects data chips and dodges drones",
        "3d",
    ),
    (
        "platformer",
        "a space platformer, jump over alien slime and collect star crystals",
        "2d",
    ),
];

#[test]
fn every_template_makes_a_game_the_autopilot_wins() {
    let root = temp_root("make");
    let mut agent = Agent::new(&root, false);
    for (name, prompt, view) in PROMPTS {
        let out = agent
            .call("game_make", json!({"path": name, "prompt": prompt}))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(out["view"], view, "{name}");
        assert_eq!(out["autoplay"]["won"], true, "{name}");
        assert_eq!(out["tests_passed"], 2, "{name}: {out}");
        assert_eq!(out["tests_failed"], 0, "{name}: {out}");
        // Nothing is generated here, so every slot waits for art with a recipe.
        let pending = out["pending"].as_array().unwrap();
        assert!(!pending.is_empty());
        for slot in pending {
            assert!(
                root.join(name)
                    .join(format!("assets/generated/{}.recipe.json", slot.as_str().unwrap()))
                    .is_file(),
                "{name}: no recipe for {slot}"
            );
        }
        // The project stands on its own: its tests pass from its own root.
        let mut game = Agent::new(root.join(name), false);
        let t = game.call("test_run", json!({})).unwrap();
        assert_eq!(t["failed"], 0, "{name}: {t}");
        assert_eq!(t["passed"], 2, "{name}: {t}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_same_brief_makes_the_same_files() {
    let root = temp_root("same");
    let mut agent = Agent::new(&root, false);
    let brief = agent.call("game_brief", json!({"prompt": PROMPTS[2].1})).unwrap()["brief"].clone();
    for dir in ["a", "b"] {
        agent
            .call("game_make", json!({"path": dir, "brief": brief}))
            .unwrap();
    }
    let (mut a, mut b) = (Vec::new(), Vec::new());
    files(&root.join("a"), &root.join("a"), &mut a);
    files(&root.join("b"), &root.join("b"), &mut b);
    assert_eq!(a.len(), b.len());
    for ((pa, ba), (pb, bb)) in a.iter().zip(&b) {
        assert_eq!(pa, pb);
        // The project name in marionette.json is the folder name.
        if pa != "marionette.json" {
            assert!(ba == bb, "{pa} differs between two makes of the same brief");
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn refresh_keeps_edits_outside_the_config_block() {
    let root = temp_root("refresh");
    let mut agent = Agent::new(&root, false);
    let out = agent
        .call("game_make", json!({"path": "g", "prompt": PROMPTS[0].1}))
        .unwrap();
    let title = out["title"].as_str().unwrap().to_string();
    let script = root.join("g/scenes/scripts/game.ts");
    let edited = std::fs::read_to_string(&script).unwrap().replace(
        "// Everything that changes",
        "// MY NOTE\n// Everything that changes",
    );
    std::fs::write(&script, &edited).unwrap();
    // A new word for the pickups goes through brief.json.
    let brief_path = root.join("g/brief.json");
    let mut brief: Value = serde_json::from_str(&std::fs::read_to_string(&brief_path).unwrap()).unwrap();
    brief["words"]["pickups"] = json!("fireflies");
    std::fs::write(&brief_path, serde_json::to_string_pretty(&brief).unwrap()).unwrap();

    let again = agent
        .call("game_make", json!({"path": "g", "refresh": true}))
        .unwrap();
    assert_eq!(again["title"], title.as_str());
    assert_eq!(again["tests_failed"], 0, "{again}");
    let text = std::fs::read_to_string(&script).unwrap();
    assert!(
        text.contains("// MY NOTE"),
        "an edit outside the CONFIG block was lost"
    );
    assert!(
        text.contains("\"FIREFLIES\""),
        "the CONFIG block was not rewritten"
    );

    // Without the markers, refresh refuses instead of overwriting the script.
    std::fs::write(&script, text.replace("// BEGIN GAME CONFIG", "")).unwrap();
    let e = agent
        .call("game_make", json!({"path": "g", "refresh": true}))
        .unwrap_err();
    assert_eq!(e.0[0].code, "starter.no_config_block");
    // A new make never writes over an existing folder.
    let e = agent
        .call("game_make", json!({"path": "g", "prompt": "anything"}))
        .unwrap_err();
    assert_eq!(e.0[0].code, "agent.exists");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn bad_briefs_list_every_problem() {
    let mut agent = Agent::new(temp_root("bad"), false);
    let e = agent
        .call(
            "game_brief",
            json!({"brief": {"title": "", "template": "platformer", "view": "3d",
                "colors": {"background": "blue"}, "assets": {"dragon": {"describe": "x"}}}}),
        )
        .unwrap_err();
    let msgs: Vec<&str> = e.0.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(e.0.len(), 4, "{msgs:?}");
    assert!(e.0.iter().all(|d| d.code == "starter.bad_brief"));
}
