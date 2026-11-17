//! Proofs for the game starter (decision 0150): a prompt or a brief becomes a playable,
//! tested project through the operation table (the code the CLI and the MCP server run).
//!
//! 1. Every template and view: `game_make` from a prompt writes a project whose autopilot test
//!    wins and whose idle test does not, and `test_run` passes in the new project.
//! 2. The same brief makes byte-identical files (levels, scripts, tests, hashes).
//! 3. `refresh` keeps edits outside the script's CONFIG block and re-pins the tests.
//! 4. Bad briefs are refused with every problem listed.
//! 5. The interview (decision 0151): a bare prompt is refused until the person was asked;
//!    answers change the brief and are kept in interview.json.

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
            .call(
                "game_make",
                json!({"path": name, "prompt": prompt, "skip_interview": true}),
            )
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
        .call(
            "game_make",
            json!({"path": "g", "prompt": PROMPTS[0].1,
                   "answers": {"style": "soft watercolor", "extra": "a lantern counter that glows"}}),
        )
        .unwrap();
    assert_eq!(out["wishes"], "a lantern counter that glows");
    let iv: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("g/interview.json")).unwrap()).unwrap();
    assert_eq!(iv["answers"]["style"], "soft watercolor");
    let brief0: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("g/brief.json")).unwrap()).unwrap();
    assert_eq!(brief0["style"], "soft watercolor");
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

#[test]
fn a_bare_prompt_needs_the_interview() {
    let root = temp_root("interview");
    let mut agent = Agent::new(&root, false);
    let e = agent
        .call("game_make", json!({"path": "g", "prompt": PROMPTS[0].1}))
        .unwrap_err();
    assert_eq!(e.0[0].code, "starter.interview_needed");
    assert!(
        !root.join("g").exists(),
        "nothing is written before the interview"
    );

    let iv = agent
        .call("game_interview", json!({"prompt": PROMPTS[0].1}))
        .unwrap();
    let ids: Vec<&str> = iv["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids[0], "genre");
    assert!(ids.contains(&"extra"));
    // Answered questions drop out; a platformer has no view question.
    let iv = agent
        .call(
            "game_interview",
            json!({"prompt": PROMPTS[0].1, "answers": {"genre": "platformer", "title": "Moth Run"}}),
        )
        .unwrap();
    assert_eq!(iv["answered"], 2);
    assert!(
        iv["questions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|q| q["id"] != "genre" && q["id"] != "view")
    );
    let b = agent
        .call(
            "game_brief",
            json!({"prompt": PROMPTS[0].1, "answers": {"genre": "platformer", "title": "Moth Run"}}),
        )
        .unwrap();
    assert_eq!(b["brief"]["template"], "platformer");
    assert_eq!(b["brief"]["title"], "Moth Run");
    let e = agent
        .call(
            "game_interview",
            json!({"prompt": "x", "answers": {"genre": "racing"}}),
        )
        .unwrap_err();
    assert_eq!(e.0[0].code, "starter.bad_answer");
    let _ = std::fs::remove_dir_all(&root);
}

/// 6. Counts (decision 0152): a number in the prompt or an answer sets the maze's pickups and
///    enemies exactly; the README and STATE.md state the real counts; a refresh refuses to
///    erase a hand-edited CONFIG block unless told to, and keeps a backup when it does.
#[test]
fn counts_are_exact_and_refresh_protects_hand_edits() {
    let root = temp_root("counts");
    let mut agent = Agent::new(&root, false);
    let out = agent
        .call(
            "game_make",
            json!({"path": "g", "prompt": "a garden maze: collect 3 gems then reach the exit",
                   "template": "maze",
                   "answers": {"pickups": "gems", "enemy_count": "1", "length": "small"}}),
        )
        .unwrap();
    assert_eq!(out["tests_failed"], 0, "{out}");
    let script = root.join("g/scenes/scripts/game.ts");
    let text = std::fs::read_to_string(&script).unwrap();
    // The map rows are quoted in the CONFIG block: count the marks there.
    let marks = |m: char| {
        text.lines()
            .filter(|l| l.trim_start().starts_with('"') && l.contains('#'))
            .flat_map(|l| l.chars())
            .filter(|c| *c == m)
            .count()
    };
    assert_eq!(marks('*'), 3, "pickup count");
    assert_eq!(marks('G'), 1, "enemy count");
    let readme = std::fs::read_to_string(root.join("g/README.md")).unwrap();
    assert!(readme.contains("This level has 3 gems, 1 enemies"), "{readme}");
    let state = std::fs::read_to_string(root.join("g/docs/ai/STATE.md")).unwrap();
    assert!(state.contains("3 gems, 1 enemies"), "{state}");
    assert!(state.contains("autoplay.test.json") && !state.contains("main.test.json"));

    // Out-of-range counts are refused.
    let brief_path = root.join("g/brief.json");
    let mut brief: Value = serde_json::from_str(&std::fs::read_to_string(&brief_path).unwrap()).unwrap();
    brief["pickup_count"] = json!(40);
    std::fs::write(&brief_path, serde_json::to_string_pretty(&brief).unwrap()).unwrap();
    let e = agent
        .call("game_make", json!({"path": "g", "refresh": true}))
        .unwrap_err();
    assert_eq!(e.0[0].code, "starter.bad_brief");
    brief["pickup_count"] = json!(4);
    std::fs::write(&brief_path, serde_json::to_string_pretty(&brief).unwrap()).unwrap();

    // A hand edit inside the CONFIG block stops the refresh.
    std::fs::write(&script, text.replace("\"lives\"", "\"lives\": 9, \"oldlives\"")).unwrap();
    let e = agent
        .call("game_make", json!({"path": "g", "refresh": true}))
        .unwrap_err();
    assert_eq!(e.0[0].code, "starter.config_edited");
    assert!(std::fs::read_to_string(&script).unwrap().contains("\"lives\": 9"));
    // Told to overwrite, it does, and keeps the old block.
    let again = agent
        .call(
            "game_make",
            json!({"path": "g", "refresh": true, "overwrite_config": true}),
        )
        .unwrap();
    assert_eq!(again["tests_failed"], 0, "{again}");
    assert!(
        std::fs::read_to_string(root.join("g/.config.backup"))
            .unwrap()
            .contains("\"lives\": 9")
    );
    // A plain refresh works again.
    agent
        .call("game_make", json!({"path": "g", "refresh": true}))
        .unwrap();
    let _ = std::fs::remove_dir_all(&root);
}
