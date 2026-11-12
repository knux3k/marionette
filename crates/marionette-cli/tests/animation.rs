//! Proofs for skeletal animation (decision 0025) on `examples/animation.scene.json`: four
//! copies of the self-made rigged character (`examples/assets/puppet.glb`) walking, waving,
//! blending both, and idling (with a script, `examples/scripts/greeter.ts`, that makes it wave
//! every three seconds). Simulation checks go through the operation table; GPU checks
//! skip without an adapter unless MARIONETTE_REQUIRE_GPU=1. Set
//! MARIONETTE_UPDATE_REFERENCES=1 to rewrite `examples/reference/animation.png`.

use marionette_agent::{Agent, engine};
use marionette_render::assets::{decode_png, encode_png};
use marionette_render::compare::{REFERENCE_THRESHOLD, diff, within_reference_tolerance};
use marionette_render::{Gpu, Offscreen, extract};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const SCENE: &str = "examples/animation.scene.json";

/// State hash after 600 ticks, seed 1. A change here changes simulation results: say so in
/// the commit and in docs/ai/STATE.md.
const HASH_600: &str = "f409e570ea3b9d3c1e92a01adc3070dbbd49b39d147b0dd97caa39297dba6d13";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn agent() -> Agent {
    Agent::new(repo(), false)
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("marionette-anim-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::copy(
        repo().join("examples/assets/puppet.glb"),
        dir.join("assets/puppet.glb"),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::copy(
        repo().join("examples/scripts/greeter.ts"),
        dir.join("scripts/greeter.ts"),
    )
    .unwrap();
    dir
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

fn hash(v: &Value) -> String {
    v["state_hash"].as_str().expect("state_hash").to_string()
}

#[test]
fn animation_scene_gives_the_same_hash_twice_and_matches_the_recorded_hash() {
    let mut a = agent();
    let run = |a: &mut Agent| {
        hash(&call(
            a,
            "scene_run",
            json!({ "scene": SCENE, "seed": 1, "ticks": 600 }),
        ))
    };
    let first = run(&mut a);
    assert_eq!(first, run(&mut a), "two runs differ");
    assert_eq!(first, HASH_600, "hash changed");
}

#[test]
fn snapshot_mid_clip_then_restore_continues_exactly() {
    let mut a = agent();
    let uninterrupted = hash(&call(
        &mut a,
        "scene_run",
        json!({ "scene": SCENE, "seed": 1, "ticks": 600 }),
    ));
    let s = call(
        &mut a,
        "session_start",
        json!({ "scene": SCENE, "seed": 1, "ticks": 317 }),
    );
    let id = s["session"].as_str().unwrap().to_string();
    call(
        &mut a,
        "session_snapshot",
        json!({ "session": id, "name": "mid" }),
    );
    let bone = |a: &mut Agent| {
        call(
            a,
            "anim_pose",
            json!({ "session": id, "id": 5, "nodes": ["lower_arm.R"] }),
        )
    };
    let pose_at_317 = bone(&mut a);
    let straight = hash(&call(
        &mut a,
        "session_step",
        json!({ "session": id, "ticks": 283 }),
    ));
    assert_eq!(straight, uninterrupted, "session run differs from scene_run");
    for round in 0..2 {
        call(
            &mut a,
            "session_restore",
            json!({ "session": id, "snapshot": "mid" }),
        );
        // The pose is rebuilt on restore, before any tick runs.
        assert_eq!(bone(&mut a), pose_at_317, "restore {round}: pose differs");
        let again = hash(&call(
            &mut a,
            "session_step",
            json!({ "session": id, "ticks": 283 }),
        ));
        assert_eq!(again, uninterrupted, "restore {round} diverged");
    }
}

#[test]
fn bones_move_and_poses_repeat_exactly() {
    let mut a = agent();
    let hand = |a: &mut Agent, ticks: u64| {
        call(
            a,
            "anim_pose",
            json!({ "scene": SCENE, "seed": 1, "ticks": ticks, "id": 5, "nodes": ["lower_arm.R", "hat"] }),
        )["nodes"]
            .clone()
    };
    let t0 = hand(&mut a, 0);
    let t15 = hand(&mut a, 15);
    assert_ne!(t0[0]["position"], t15[0]["position"], "the waving hand moved");
    assert_eq!(t15, hand(&mut a, 15), "same tick, same pose");
    // The wave clip is one second long and loops: tick 75 is a full cycle after tick 15.
    let t75 = hand(&mut a, 75);
    for k in 0..3 {
        let (p, q) = (
            t15[0]["position"][k].as_f64().unwrap(),
            t75[0]["position"][k].as_f64().unwrap(),
        );
        assert!((p - q).abs() < 1e-4, "loop: {p} vs {q}");
    }
    // The hat is a rigid prop under the head bone: it sits above the hand's shoulder line.
    assert!(t15[1]["position"][1].as_f64().unwrap() > 1.6);
}

#[test]
fn play_cross_fades_and_finished_layers_disappear() {
    let mut a = agent();
    let s = call(&mut a, "session_start", json!({ "scene": SCENE, "seed": 1 }));
    let id = s["session"].as_str().unwrap().to_string();
    call(
        &mut a,
        "anim_play",
        json!({ "session": id, "id": 4, "clip": "idle", "fade": 0.5 }),
    );
    let layers = |a: &mut Agent| {
        call(a, "observe_entity", json!({ "session": id, "id": 4 }))["components"]["Animator"]["layers"]
            .clone()
    };
    let l = layers(&mut a);
    assert_eq!(
        l.as_array().unwrap().len(),
        2,
        "walk fading out, idle fading in: {l}"
    );
    call(&mut a, "session_step", json!({ "session": id, "ticks": 15 }));
    let l = layers(&mut a);
    let w: Vec<f64> = l
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["weight"].as_f64().unwrap())
        .collect();
    assert!(
        (w[0] - 0.5).abs() < 1e-3 && (w[1] - 0.5).abs() < 1e-3,
        "halfway: {w:?}"
    );
    call(&mut a, "session_step", json!({ "session": id, "ticks": 20 }));
    let l = layers(&mut a);
    assert_eq!(l.as_array().unwrap().len(), 1, "walk removed after its fade: {l}");
    assert_eq!(l[0]["clip"], "idle");

    // A one-shot clip logs when it ends.
    call(
        &mut a,
        "anim_play",
        json!({ "session": id, "id": 4, "clip": "wave", "fade": 0.0, "looping": false }),
    );
    call(&mut a, "session_step", json!({ "session": id, "ticks": 70 }));
    let events = call(
        &mut a,
        "observe_events",
        json!({ "session": id, "kind": "anim_finished" }),
    );
    let list = events["events"].as_array().unwrap();
    assert_eq!(list.len(), 1, "{events}");
    assert_eq!(list[0]["entity"], 4);
    assert_eq!(list[0]["detail"]["clip"], "wave");

    // Unknown clips are refused with the list of real ones; nothing changes.
    let before = layers(&mut a);
    let err = a
        .call("anim_play", json!({ "session": id, "id": 4, "clip": "run" }))
        .unwrap_err();
    assert_eq!(err.0[0].code, "anim.unknown_clip");
    assert!(err.0[0].fix.as_deref().unwrap().contains("idle, walk, wave"));
    assert_eq!(layers(&mut a), before);

    call(
        &mut a,
        "anim_stop",
        json!({ "session": id, "id": 4, "fade": 0.0 }),
    );
    assert_eq!(layers(&mut a), json!([]));
}

#[test]
fn scripts_play_clips_with_the_same_result_as_the_operation() {
    let mut a = agent();
    let layers = |a: &mut Agent, ticks: u64| {
        call(
            a,
            "observe_entity",
            json!({ "scene": SCENE, "seed": 1, "ticks": ticks, "id": 7 }),
        )["components"]["Animator"]["layers"]
            .clone()
    };
    // greeter.ts: playClip("wave", { fade: 0.25 }) at tick 30, playClip("idle", ...) at 90.
    let at_31 = layers(&mut a, 31);
    let s = call(
        &mut a,
        "session_start",
        json!({ "scene": SCENE, "seed": 1, "ticks": 30 }),
    );
    let id = s["session"].as_str().unwrap().to_string();
    call(
        &mut a,
        "entity_update",
        json!({ "session": id, "id": 7, "remove": ["Script"] }),
    );
    call(
        &mut a,
        "anim_play",
        json!({ "session": id, "id": 7, "clip": "wave", "fade": 0.25 }),
    );
    call(&mut a, "session_step", json!({ "session": id, "ticks": 1 }));
    let by_op =
        call(&mut a, "observe_entity", json!({ "session": id, "id": 7 }))["components"]["Animator"]["layers"]
            .clone();
    assert_eq!(at_31, by_op, "script and operation disagree");
    assert_eq!(
        layers(&mut a, 45),
        json!([{ "clip": "wave", "fade": 0.25, "looping": true, "speed": 1.0, "target_weight": 1.0, "time": 0.25, "weight": 1.0 }])
    );
    let at_100: Vec<String> = layers(&mut a, 100)
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["clip"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(at_100, ["wave", "idle"]);
}

#[test]
fn clip_listing_names_clips_bones_and_morph_targets() {
    let mut a = agent();
    let r = call(&mut a, "anim_clips", json!({ "scene": SCENE, "id": 4 }));
    let names: Vec<&str> = r["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["idle", "walk", "wave"]);
    assert_eq!(r["clips"][2]["morph"], true);
    assert_eq!(r["skins"][0]["joints"].as_array().unwrap().len(), 15);
    assert_eq!(r["morph_targets"], json!([["body", 1]]));
    let same = call(
        &mut a,
        "anim_clips",
        json!({ "asset": "examples/assets/puppet.glb" }),
    );
    assert_eq!(same["clips"], r["clips"]);
}

fn gpu() -> Option<Gpu> {
    match Gpu::headless() {
        Ok(g) => Some(g),
        Err(e) if std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipping GPU test: {e}");
            None
        }
        Err(e) => panic!("GPU required but unavailable: {e}"),
    }
}

fn render(
    gpu: &Gpu,
    scene: &Path,
    ticks: u64,
    size: (u32, u32),
) -> (Vec<u8>, marionette_render::RenderStats) {
    let mut sim = engine::load_file(scene, "animation", 1).unwrap();
    sim.run(ticks);
    let dir = scene.parent().unwrap().to_path_buf();
    let mut off = Offscreen::new(gpu, size.0, size.1, 4, dir);
    let stats = off.render(gpu, &extract(&sim.world), None).unwrap();
    (off.read_pixels(gpu).unwrap(), stats)
}

#[test]
fn skinned_characters_render_posed_and_match_the_reference() {
    let Some(gpu) = gpu() else { return };
    let scene = repo().join(SCENE);
    let size = (320, 180);
    let (posed, stats) = render(&gpu, &scene, 15, size);
    let (again, _) = render(&gpu, &scene, 15, size);
    assert!(posed == again, "two renders differ");
    let (rest, _) = render(&gpu, &scene, 0, size);
    let moved = diff(&posed, &rest, REFERENCE_THRESHOLD);
    assert!(
        moved.over_threshold > 0.005,
        "animation changes the image: {moved:?}"
    );
    // Ground, hat (static), body and skin (skinned): four draws for four characters.
    assert_eq!(stats.draw_calls, 4, "{stats:?}");

    let reference = repo().join("examples/reference/animation.png");
    if std::env::var("MARIONETTE_UPDATE_REFERENCES").as_deref() == Ok("1") {
        std::fs::write(&reference, encode_png(size.0, size.1, &posed)).unwrap();
    }
    let expected = decode_png(&std::fs::read(&reference).unwrap()).unwrap();
    let d = diff(&posed, &expected.rgba, REFERENCE_THRESHOLD);
    assert!(within_reference_tolerance(&d), "animation.png: {d:?}");
}

#[test]
fn skinned_draw_calls_stay_flat_as_the_crowd_grows() {
    let Some(gpu) = gpu() else { return };
    let dir = temp_dir("crowd");
    let text = std::fs::read_to_string(repo().join(SCENE)).unwrap();
    let mut scene: Value = serde_json::from_str(&text).unwrap();
    let template = scene["entities"][3].clone(); // the walker
    let entities = scene["entities"].as_array_mut().unwrap();
    for i in 0..60u64 {
        let mut e = template.clone();
        e["id"] = json!(100 + i);
        e["name"] = json!(format!("walker{i}"));
        e["components"]["Transform"]["translation"]["x"] = json!(-6.0 + (i % 12) as f64);
        e["components"]["Transform"]["translation"]["z"] = json!(-2.0 - (i / 12) as f64);
        e["components"]["Animator"]["layers"][0]["time"] = json!((i % 7) as f64 * 0.13);
        entities.push(e);
    }
    let path = dir.join("crowd.scene.json");
    std::fs::write(&path, serde_json::to_string_pretty(&scene).unwrap()).unwrap();
    let (_, stats) = render(&gpu, &path, 10, (160, 90));
    assert_eq!(stats.draw_calls, 4, "{stats:?}");
    assert_eq!(stats.instances, 1 + 64 * 3, "{stats:?}");
}

#[test]
fn morph_targets_change_the_mesh() {
    let Some(gpu) = gpu() else { return };
    let scene = repo().join(SCENE);
    let mut sim = engine::load_file(&scene, "animation", 1).unwrap();
    sim.run(15); // the wave clip drives the `squash` blend shape to 0.6 at 0.25 s
    let frame = extract(&sim.world);
    let waver = frame.meshes.iter().position(|m| m.entity == 5).unwrap();
    let pose = frame.meshes[waver].pose.clone().expect("animated");
    let body = pose.morph_weights.iter().position(|w| !w.is_empty()).unwrap();
    assert!(
        (pose.morph_weights[body][0] - 0.6).abs() < 1e-5,
        "{:?}",
        pose.morph_weights[body]
    );
    let draw = |frame: &marionette_render::FrameData| {
        let mut off = Offscreen::new(&gpu, 160, 90, 1, scene.parent().unwrap().to_path_buf());
        off.render(&gpu, frame, None).unwrap();
        off.read_pixels(&gpu).unwrap()
    };
    let squashed = draw(&frame);
    let mut flat = frame.clone();
    let mut p = (*pose).clone();
    p.morph_weights[body][0] = 0.0;
    flat.meshes[waver].pose = Some(std::sync::Arc::new(p));
    assert!(
        squashed != draw(&flat),
        "the blend shape weight changes the image"
    );
}
