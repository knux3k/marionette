//! Co-op networking proofs through the operation table (decisions 0210 to 0213), on
//! `examples/coop.scene.json` (each player's entity reads `p<slot>.*` actions).

use marionette_agent::Agent;
use marionette_net::{Frame, LockstepConfig, PeerId, RosterEvent, apply_frame};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const SCENE: &str = "examples/coop.scene.json";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn agent() -> Agent {
    Agent::new(repo(), true)
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

#[test]
fn four_players_late_join_and_bad_network_end_equal() {
    let mut a = agent();
    let r = call(
        &mut a,
        "net_simulate",
        json!({"scene": SCENE, "players": 4, "ticks": 600, "late_join_at": 200,
               "actions": ["jump", "move_x", "move_y"],
               "link": {"latency_ms": 80, "jitter_ms": 30, "loss": 0.1, "duplicate": 0.02}}),
    );
    assert_eq!(r["all_equal"], true, "{r}");
    assert_eq!(r["peers"].as_array().unwrap().len(), 4, "three clients and the late joiner");
    assert!(r["hash_checks"].as_u64().unwrap() >= 30);
    // Same request, same result (virtual time, seeded network).
    let again = call(
        &mut a,
        "net_simulate",
        json!({"scene": SCENE, "players": 4, "ticks": 600, "late_join_at": 200,
               "actions": ["jump", "move_x", "move_y"],
               "link": {"latency_ms": 80, "jitter_ms": 30, "loss": 0.1, "duplicate": 0.02}}),
    );
    assert_eq!(r, again);
}

#[test]
fn frames_drive_the_named_players_only() {
    let a = agent();
    let mut sim = a.load_sim(SCENE, 1).unwrap();
    let cfg = LockstepConfig {
        actions: vec!["jump".into(), "move_x".into(), "move_y".into()],
        ..LockstepConfig::default()
    };
    for t in 0..60 {
        let mut f = Frame {
            tick: t,
            ..Frame::default()
        };
        if t == 0 {
            f.roster = vec![
                RosterEvent::Join { slot: 0, peer: PeerId(1) },
                RosterEvent::Join { slot: 1, peer: PeerId(2) },
            ];
            f.inputs = vec![(1, 1, 1.0)]; // player 1 holds move_x right
        }
        apply_frame(&mut sim, &cfg, &f).unwrap();
        sim.step();
    }
    let scene: Value = serde_json::from_str(&sim.snapshot().to_text()).unwrap();
    let x = |name: &str| {
        scene["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == name)
            .unwrap()["components"]["Position"]["x"]
            .as_f64()
            .unwrap()
    };
    assert_eq!(x("player-0"), -3.0, "player 0 did not move");
    assert!((x("player-1") - (-1.0 + 3.0)).abs() < 0.1, "player 1 moved about 3 m in a second");
    assert_eq!(x("player-2"), 1.0, "slot 2 is not in the session");
}

#[test]
fn achievements_file_is_checked() {
    let mut a = agent();
    let r = call(&mut a, "achievements_check", json!({"file": "examples/coop.achievements.json"}));
    assert_eq!(r["achievements"], json!(["FIRST_JUMP", "JUMPS_100", "FULL_HOUSE"]));
    assert_eq!(r["stat_driven"], 1);
    let e = a
        .call("achievements_check", json!({"file": SCENE}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("platform.definitions"), "{e}");
}
