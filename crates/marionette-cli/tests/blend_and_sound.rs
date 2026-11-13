//! Proofs for blend spaces, overlays (decision 0074), 3D sound and music layers (decision
//! 0075), mostly on `examples/blend-and-sound.scene.json` (built by
//! `examples/blend-and-sound.ops.json`). Everything goes through the operation table.

use marionette_agent::Agent;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const SCENE: &str = "examples/blend-and-sound.scene.json";

/// State hash after 600 ticks, seed 1. A change here changes simulation results: say so in
/// the commit and in docs/ai/PROOFS.md.
const HASH_600: &str = "d344027414b14c0f555f9de4b443777a25a1b6f07886cda765a49bfd0938e236";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

/// A project with the puppet and a scene of two puppets (ids 1 and 2), both walking.
fn two_walkers(tag: &str) -> (Agent, PathBuf) {
    let dir = std::env::temp_dir().join(format!("marionette-blend-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::copy(
        repo().join("examples/assets/puppet.glb"),
        dir.join("assets/puppet.glb"),
    )
    .unwrap();
    let mut a = Agent::new(&dir, false);
    call(&mut a, "scene_create", json!({ "scene": "s.scene.json" }));
    for x in [-1.0, 1.0] {
        call(
            &mut a,
            "entity_create",
            json!({ "scene": "s.scene.json", "components": {
                "Model": { "asset": "assets/puppet.glb" },
                "Transform": { "translation": { "x": x, "y": 0.0, "z": 0.0 },
                    "rotation": { "x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0 },
                    "scale": { "x": 1.0, "y": 1.0, "z": 1.0 } } } }),
        );
    }
    for id in [1, 2] {
        call(
            &mut a,
            "anim_play",
            json!({ "scene": "s.scene.json", "id": id, "clip": "walk", "fade": 0.0 }),
        );
    }
    (a, dir)
}

/// Local-to-entity rotation of `node` on entity `id` after `ticks`.
fn rotation(a: &mut Agent, id: u64, node: &str, ticks: u64) -> [f64; 4] {
    let r = call(
        a,
        "anim_pose",
        json!({ "scene": "s.scene.json", "id": id, "ticks": ticks, "nodes": [node] }),
    );
    let q = &r["nodes"][0]["rotation"];
    [0, 1, 2, 3].map(|i| q[i].as_f64().unwrap())
}

fn same(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
}

#[test]
fn example_gives_the_same_hash_twice_and_matches_the_recorded_hash() {
    let mut a = Agent::new(repo(), false);
    let run = |a: &mut Agent| {
        call(a, "scene_run", json!({ "scene": SCENE, "ticks": 600, "seed": 1 }))["state_hash"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let first = run(&mut a);
    assert_eq!(first, run(&mut a));
    assert_eq!(first, HASH_600, "simulation results changed");
}

#[test]
fn synced_blend_space_runs_at_the_blended_cycle_length() {
    let (mut a, dir) = two_walkers("sync");
    // idle is 2 s, walk 1 s. Halfway, a synced cycle lasts 1.5 s.
    call(
        &mut a,
        "anim_space",
        json!({ "scene": "s.scene.json", "id": 1, "name": "move", "x": 0.75,
            "points": [{ "clip": "idle", "x": 0.0 }, { "clip": "walk", "x": 1.5 }] }),
    );
    call(
        &mut a,
        "anim_play",
        json!({ "scene": "s.scene.json", "id": 1, "clip": "move", "fade": 0.0 }),
    );
    let s = call(&mut a, "session_start", json!({ "scene": "s.scene.json" }))["session"]
        .as_str()
        .unwrap()
        .to_string();
    call(&mut a, "session_step", json!({ "session": s, "ticks": 60 }));
    let e = call(&mut a, "observe_entity", json!({ "session": s, "id": 1 }));
    let layer = &e["components"]["Animator"]["layers"][0];
    assert_eq!(layer["clip"], "move");
    let phase = layer["time"].as_f64().unwrap();
    assert!((phase - 1.0 / 1.5).abs() < 1e-3, "phase {phase}");
    // Steering with a rate glides the parameter.
    call(
        &mut a,
        "anim_space",
        json!({ "session": s, "id": 1, "name": "move", "x": 1.5, "rate": 1.5 }),
    );
    call(&mut a, "session_step", json!({ "session": s, "ticks": 15 }));
    let e = call(&mut a, "observe_entity", json!({ "session": s, "id": 1 }));
    let x = e["components"]["BlendSpaces"]["spaces"][0]["x"].as_f64().unwrap();
    assert!((x - 1.125).abs() < 1e-3, "x {x}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn blend_space_at_an_edge_poses_like_that_clip() {
    let (mut a, dir) = two_walkers("edge");
    call(
        &mut a,
        "anim_space",
        json!({ "scene": "s.scene.json", "id": 1, "name": "move", "x": 1.5, "sync": false,
            "points": [{ "clip": "idle", "x": 0.0 }, { "clip": "walk", "x": 1.5 }] }),
    );
    call(
        &mut a,
        "anim_play",
        json!({ "scene": "s.scene.json", "id": 1, "clip": "move", "fade": 0.0 }),
    );
    for node in ["upper_leg.L", "chest", "upper_arm.R"] {
        assert!(
            same(rotation(&mut a, 1, node, 37), rotation(&mut a, 2, node, 37)),
            "{node}"
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn masked_overlay_changes_only_the_masked_bones() {
    let (mut a, dir) = two_walkers("mask");
    call(
        &mut a,
        "anim_overlay",
        json!({ "scene": "s.scene.json", "id": 1, "name": "arm", "clip": "wave",
            "mask": ["upper_arm.R"], "fade": 0.0 }),
    );
    let t = 20;
    // Outside the mask: exactly the walk, even where the wave clip moves bones.
    for node in ["upper_leg.L", "upper_arm.L", "head"] {
        assert!(
            same(rotation(&mut a, 1, node, t), rotation(&mut a, 2, node, t)),
            "{node}"
        );
    }
    // Inside the mask (and its child): the wave.
    for node in ["upper_arm.R", "lower_arm.R"] {
        assert!(
            !same(rotation(&mut a, 1, node, t), rotation(&mut a, 2, node, t)),
            "{node}"
        );
    }
    // Additive at weight 0 is the walk again.
    call(
        &mut a,
        "anim_overlay",
        json!({ "scene": "s.scene.json", "id": 1, "name": "arm", "additive": true, "weight": 0.0, "fade": 0.0 }),
    );
    assert!(same(
        rotation(&mut a, 1, "upper_arm.R", t),
        rotation(&mut a, 2, "upper_arm.R", t)
    ));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unknown_clips_in_spaces_and_overlays_are_refused() {
    let (mut a, dir) = two_walkers("unknown");
    let err = a
        .call(
            "anim_space",
            json!({ "scene": "s.scene.json", "id": 1, "name": "m", "points": [{ "clip": "fly", "x": 0.0 }] }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("anim.unknown_clip"), "{err}");
    let err = a
        .call(
            "anim_overlay",
            json!({ "scene": "s.scene.json", "id": 1, "name": "o", "clip": "fly" }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("anim.unknown_clip"), "{err}");
    let err = a
        .call(
            "anim_space",
            json!({ "scene": "s.scene.json", "id": 1, "name": "nope", "x": 1.0 }),
        )
        .unwrap_err();
    assert!(err.to_string().contains("no blend space"), "{err}");
    let _ = std::fs::remove_dir_all(dir);
}

fn mix(a: &mut Agent, ticks: u64) -> Value {
    call(a, "audio_mix", json!({ "scene": SCENE, "ticks": ticks }))
}

#[test]
fn the_listener_hears_the_orbiting_hum_around_its_head() {
    let mut a = Agent::new(repo(), false);
    // Quarter turn: to the right and in front, open.
    let m = mix(&mut a, 90);
    let hum = &m["sources"][0];
    assert!(m["listener"].as_bool().unwrap());
    assert!(hum["pan"].as_f64().unwrap() > 0.5, "{m}");
    assert_eq!(hum["lowpass_hz"], 20000.0);
    assert_eq!(hum["behind"], false);
    // Half turn: behind the head, muffled, and louder (closer).
    let m2 = mix(&mut a, 180);
    let hum2 = &m2["sources"][0];
    assert_eq!(hum2["behind"], true);
    assert!(hum2["lowpass_hz"].as_f64().unwrap() < 5000.0, "{m2}");
    assert!(hum2["gain"].as_f64().unwrap() > hum["gain"].as_f64().unwrap());
    // Three quarters: on the left.
    assert!(mix(&mut a, 270)["sources"][0]["pan"].as_f64().unwrap() < -0.5);
}

#[test]
fn music_layers_fade_in_with_intensity() {
    let mut a = Agent::new(repo(), false);
    let levels = |m: &Value| -> Vec<f64> {
        m["music"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["level"].as_f64().unwrap())
            .collect()
    };
    // Intensity 1 from tick 0, fade 1.5 s: pad and bass rising, drums and lead silent.
    let m = mix(&mut a, 45);
    let l = levels(&m);
    assert!((l[0] - 0.5).abs() < 0.02 && (l[1] - 0.5).abs() < 0.02, "{l:?}");
    assert_eq!((l[2], l[3]), (0.0, 0.0));
    // Intensity 2 at tick 240: drums arrive.
    let l = levels(&mix(&mut a, 400));
    assert_eq!((l[0], l[1], l[2], l[3]), (1.0, 1.0, 1.0, 0.0));
    // The music operation steers it too.
    let s = call(&mut a, "session_start", json!({ "scene": SCENE, "ticks": 400 }))["session"]
        .as_str()
        .unwrap()
        .to_string();
    call(
        &mut a,
        "audio_music",
        json!({ "session": s, "id": 7, "playing": false, "fade": 0.5 }),
    );
    call(&mut a, "session_step", json!({ "session": s, "ticks": 30 }));
    let m = call(&mut a, "audio_mix", json!({ "session": s }));
    assert!(levels(&m).iter().all(|&v| v == 0.0), "{m}");
    let events = call(
        &mut a,
        "observe_events",
        json!({ "session": s, "kind": "music_layer" }),
    );
    let outs = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["detail"]["state"] == "out")
        .count();
    assert_eq!(outs, 3);
}
