//! Performance proofs that are exact, so they run in every `cargo test` (decision 0023):
//! the factory benchmark scene simulates to a recorded state hash for any worker count, and
//! instancing keeps draw calls flat however many objects a scene has. Timing budgets are in
//! `perf_budgets.rs` (release builds only).

use marionette_agent::{Agent, engine};
use marionette_render::{Gpu, Offscreen, extract};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// State hash of the scale-1 factory benchmark after 60 warm-up and 300 timed ticks, seed 0.
/// A change here changes simulation results: say so in the commit and in docs/ai/STATE.md.
const HASH_FACTORY_1: &str = "13f72b12e6a829c27f4f4834d1255b7fd3bbd50ddc71bb0161d56f9edca22e69";

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("marionette-perf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

/// Writes the factory benchmark scene at `scale` and returns its path.
fn factory_scene(agent: &mut Agent, dir: &Path, scale: u32) -> PathBuf {
    let out = dir.join(format!("factory{scale}.scene.json"));
    let r = call(
        agent,
        "perf_factory_scene",
        json!({ "out": out.to_string_lossy(), "scale": scale }),
    );
    assert_eq!(r["machines"], json!(scale * 238));
    out
}

#[test]
fn factory_benchmark_hash_is_recorded_and_independent_of_threads() {
    let dir = temp_dir();
    let mut agent = Agent::new(&dir, false);
    let scene = factory_scene(&mut agent, &dir, 1);
    let mut hashes = Vec::new();
    for threads in [1, 3] {
        let r = call(
            &mut agent,
            "perf_sim",
            json!({ "scene": scene.to_string_lossy(), "ticks": 300, "threads": threads }),
        );
        assert_eq!(r["threads"], json!(threads));
        hashes.push(r["state_hash"].as_str().unwrap().to_string());
    }
    assert_eq!(hashes[0], hashes[1], "thread count changed the result");
    assert_eq!(hashes[0], HASH_FACTORY_1);
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

#[test]
fn instancing_keeps_draw_calls_flat_as_the_factory_grows() {
    let Some(gpu) = gpu() else { return };
    let dir = temp_dir();
    let mut agent = Agent::new(&dir, false);
    let mut seen = Vec::new();
    for scale in [1, 4] {
        let path = factory_scene(&mut agent, &dir, scale);
        let mut sim = engine::load_file(&path, "factory", 0).unwrap();
        sim.run(240);
        let frame = extract(&sim.world);
        let items: usize = frame.bulk.iter().map(|b| b.transforms.len()).sum();
        assert!(items > 0, "items are on the belts after 240 ticks");
        let mut off = Offscreen::new(&gpu, 64, 64, 1, dir.clone());
        let stats = off.render_eyes(&gpu, &frame, None, 2, 0.064).unwrap();
        // Every machine and every item is the same cube with a per-instance color: one draw
        // call per eye, whatever the size of the factory.
        assert_eq!(stats.views, 2);
        assert_eq!(stats.draw_calls, 2, "scale {scale}: {stats:?}");
        assert_eq!(stats.instances as usize, frame.meshes.len() + items);
        seen.push(stats.instances);
    }
    assert!(
        seen[1] > 3 * seen[0],
        "the larger factory really drew more: {seen:?}"
    );
}
