//! Timing budgets (decision 0023). Wall-clock numbers depend on the machine, so these run
//! only when asked, in a release build, one test at a time:
//!
//! `cargo test --release -p marionette-cli --test perf_budgets -- --ignored --test-threads=1`
//!
//! CI runs them on every push (job `perf`). Each budget is the per-frame share a large VR
//! factory game can spend at 90 Hz (docs/ai/PERFORMANCE.md); the measured value is printed
//! so a regression shows long before it fails.

use marionette_agent::{Agent, engine};
use marionette_render::{Gpu, Offscreen, extract};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Simulation tick of a 4x factory (952 machines, 15k belt slots), p99.
const SIM_4X_P99_MS: f64 = 2.0;
/// Simulation tick of a 32x factory (7,616 machines, 122k belt slots), average.
const SIM_32X_AVG_MS: f64 = 2.0;
/// Loading the 32x factory scene (parse, validate, build module state).
const LOAD_32X_MS: f64 = 5000.0;
/// CPU side of a stereo frame of the 32x factory: extract plus record and submit, median.
const RENDER_CPU_32X_STEREO_MS: f64 = 4.0;

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-budget-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn call(agent: &mut Agent, op: &str, input: Value) -> Value {
    agent
        .call(op, input.clone())
        .unwrap_or_else(|e| panic!("{op} {input} failed: {e}"))
}

fn factory(agent: &mut Agent, dir: &Path, scale: u32) -> String {
    let out = dir
        .join(format!("factory{scale}.scene.json"))
        .to_string_lossy()
        .to_string();
    call(agent, "perf_factory_scene", json!({ "out": out, "scale": scale }));
    out
}

/// Shared CI runners are slower and noisier than the reference desktop, so CI sets
/// `MARIONETTE_BUDGET_SCALE=2` (decision 0023); on the reference desktop it is 1.
fn scale() -> f64 {
    std::env::var("MARIONETTE_BUDGET_SCALE")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|s| *s >= 1.0)
        .unwrap_or(1.0)
}

fn check(name: &str, measured: f64, budget: f64) {
    let budget = budget * scale();
    println!("budget {name}: {measured:.3} ms (budget {budget} ms)");
    assert!(
        measured <= budget,
        "{name}: {measured:.3} ms is over its {budget} ms budget"
    );
}

#[test]
#[ignore = "timing: release builds only, see the file header"]
fn sim_tick_budgets() {
    let d = dir();
    let mut agent = Agent::new(&d, false);
    let s4 = factory(&mut agent, &d, 4);
    let r = call(
        &mut agent,
        "perf_sim",
        json!({ "scene": s4, "ticks": 1200, "warmup": 600 }),
    );
    check("sim 4x p99", r["p99_ms"].as_f64().unwrap(), SIM_4X_P99_MS);
    let s32 = factory(&mut agent, &d, 32);
    let r = call(
        &mut agent,
        "perf_sim",
        json!({ "scene": s32, "ticks": 600, "warmup": 600 }),
    );
    check("sim 32x avg", r["avg_ms"].as_f64().unwrap(), SIM_32X_AVG_MS);
    check("load 32x", r["load_ms"].as_f64().unwrap(), LOAD_32X_MS);
}

#[test]
#[ignore = "timing: release builds only, see the file header"]
fn render_cpu_budget() {
    let gpu = match Gpu::headless() {
        Ok(g) => g,
        Err(e) if std::env::var("MARIONETTE_REQUIRE_GPU").as_deref() != Ok("1") => {
            eprintln!("skipping GPU budget: {e}");
            return;
        }
        Err(e) => panic!("GPU required but unavailable: {e}"),
    };
    let d = dir();
    let mut agent = Agent::new(&d, false);
    let s32 = factory(&mut agent, &d, 32);
    let mut sim = engine::load_file(Path::new(&s32), "factory32", 0).unwrap();
    sim.run(600);
    // A small target keeps GPU time out of the way: this budget is the CPU's part.
    let mut off = Offscreen::new(&gpu, 64, 64, 1, d.clone());
    let mut cpu = Vec::new();
    for i in 0..60 {
        sim.step();
        let t = Instant::now();
        let frame = extract(&sim.world);
        let extract_ms = t.elapsed().as_secs_f64() * 1000.0;
        off.render_eyes(&gpu, &frame, None, 2, 0.064).unwrap();
        if i >= 10 {
            cpu.push(extract_ms + off.last_timing.record_ms);
        }
    }
    cpu.sort_by(f64::total_cmp);
    check(
        "render CPU 32x stereo p50",
        cpu[cpu.len() / 2],
        RENDER_CPU_32X_STEREO_MS,
    );
}
