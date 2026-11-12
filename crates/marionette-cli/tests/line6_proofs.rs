//! Proofs for plan line 6 (input, physics, audio) on the two example scenes,
//! `examples/playground3d.scene.json` and `examples/playground2d.scene.json`, each driven by
//! its scripted input file. Every check goes through the operation table, the same code the
//! CLI and the MCP server run.

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::PathBuf;

const SCENES: [&str; 2] = ["playground3d", "playground2d"];

/// State hashes after 1,000 ticks, seed 1, with the scripted input. A change here changes
/// simulation results: say so in the commit and in docs/ai/PROOFS.md.
const HASH_3D: &str = "541b991eb45fe2a436c2ec8c0bea9b490c3a84356b51c8740771aa1083547c44";
const HASH_2D: &str = "1071e11381ec030e587cc2e241fedbafca679becc1ef13a886ea389dc184e8ac";

fn agent() -> Agent {
    Agent::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."), false)
}

fn scene(name: &str) -> String {
    format!("examples/{name}.scene.json")
}

fn script(name: &str) -> String {
    format!("examples/{name}.inputs.json")
}

fn temp_file(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("marionette-line6-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(tag)
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

fn run(agent: &mut Agent, name: &str, ticks: u64, input: &str, record: Option<&str>) -> Value {
    call(
        agent,
        "scene_run",
        json!({ "scene": scene(name), "seed": 1, "ticks": ticks, "input": input, "record": record }),
    )
}

fn hash(v: &Value) -> String {
    v["state_hash"].as_str().expect("state_hash").to_string()
}

#[test]
fn physics_scenes_give_the_same_hash_twice_and_match_the_recorded_hash() {
    let mut a = agent();
    for (name, expected) in SCENES.iter().zip([HASH_3D, HASH_2D]) {
        let first = hash(&run(&mut a, name, 1000, &script(name), None));
        let second = hash(&run(&mut a, name, 1000, &script(name), None));
        assert_eq!(first, second, "{name}: two runs differ");
        assert_eq!(first, expected, "{name}: hash changed");
    }
}

#[test]
fn replaying_a_recording_gives_the_original_hash() {
    let mut a = agent();
    for name in SCENES {
        let rec = temp_file(&format!("{name}.rec.inputs.json"));
        let rec = rec.to_str().unwrap();
        let original = run(&mut a, name, 1000, &script(name), Some(rec));
        assert!(
            original["recorded_events"].as_u64().unwrap() > 0,
            "{name}: nothing recorded"
        );
        let text = std::fs::read_to_string(rec).unwrap();
        for round in 0..2 {
            let rec2 = temp_file(&format!("{name}.rec{round}.inputs.json"));
            let replay = run(&mut a, name, 1000, rec, Some(rec2.to_str().unwrap()));
            assert_eq!(hash(&replay), hash(&original), "{name}: replay {round} differs");
            // Replaying a recording records the same recording.
            assert_eq!(
                std::fs::read_to_string(&rec2).unwrap(),
                text,
                "{name}: re-recording differs"
            );
        }
    }
}

#[test]
fn snapshot_at_500_then_restore_continues_exactly() {
    let mut a = agent();
    for name in SCENES {
        let uninterrupted = hash(&run(&mut a, name, 1000, &script(name), None));
        let s = call(
            &mut a,
            "session_start",
            json!({ "scene": scene(name), "seed": 1, "input": script(name), "ticks": 500 }),
        );
        let id = s["session"].as_str().unwrap().to_string();
        call(
            &mut a,
            "session_snapshot",
            json!({ "session": id, "name": "half" }),
        );
        let straight = hash(&call(
            &mut a,
            "session_step",
            json!({ "session": id, "ticks": 500 }),
        ));
        assert_eq!(
            straight, uninterrupted,
            "{name}: session run differs from scene_run"
        );
        for round in 0..2 {
            let back = call(
                &mut a,
                "session_restore",
                json!({ "session": id, "snapshot": "half" }),
            );
            assert_eq!(back["tick"], 500);
            let again = hash(&call(
                &mut a,
                "session_step",
                json!({ "session": id, "ticks": 500 }),
            ));
            assert_eq!(again, uninterrupted, "{name}: restore {round} diverged");
        }
        call(&mut a, "session_stop", json!({ "session": id }));
    }
}

/// (tick, entity, sound) of every sound in a scripted 1,000-tick run.
fn sounds(a: &mut Agent, name: &str) -> Vec<(u64, u64, String)> {
    let s = call(
        a,
        "session_start",
        json!({ "scene": scene(name), "seed": 1, "input": script(name), "ticks": 1000 }),
    );
    let id = s["session"].as_str().unwrap().to_string();
    let e = call(
        a,
        "observe_events",
        json!({ "session": id, "kind": "sound", "limit": 10000 }),
    );
    call(a, "session_stop", json!({ "session": id }));
    assert_eq!(
        e["total"].as_u64().unwrap() as usize,
        e["events"].as_array().unwrap().len()
    );
    e["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["tick"].as_u64().unwrap(),
                v["entity"].as_u64().unwrap(),
                v["detail"]["sound"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn sound_log_lists_the_expected_sounds_at_the_expected_ticks() {
    let mut a = agent();
    // The player (entity 10) jumps where the script presses Space; boxes (20..23) thud
    // when they start touching something.
    let cases = [
        ("playground3d", vec![150, 300, 620, 900]),
        ("playground2d", vec![150, 300, 560, 780]),
    ];
    for (name, jumps) in cases {
        let log = sounds(&mut a, name);
        let jump_ticks: Vec<u64> = log
            .iter()
            .filter(|(_, e, s)| *e == 10 && s == "builtin:jump")
            .map(|(t, _, _)| *t)
            .collect();
        assert_eq!(jump_ticks, jumps, "{name}: jump sounds");
        for (t, e, s) in &log {
            match e {
                10 => assert_eq!(s, "builtin:jump", "{name}: tick {t}"),
                20..=23 => assert_eq!(s, "builtin:thud", "{name}: tick {t}"),
                other => panic!("{name}: unexpected sound from entity {other} at tick {t}"),
            }
        }
        // The resting boxes touch the floor on the first tick; the dropped box lands on
        // box 20 at tick 8.
        let at = |tick: u64| -> Vec<u64> {
            log.iter()
                .filter(|(t, _, _)| *t == tick)
                .map(|(_, e, _)| *e)
                .collect()
        };
        assert_eq!(at(0), vec![20, 22], "{name}: tick 0");
        assert_eq!(at(8), vec![20, 21], "{name}: tick 8");
        // Same log on a second run.
        assert_eq!(
            sounds(&mut a, name),
            log,
            "{name}: sound log differs between runs"
        );
    }
}
