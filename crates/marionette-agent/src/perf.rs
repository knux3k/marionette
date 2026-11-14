//! Performance operations: generate benchmark scenes and time the simulation tick
//! (decision 0023). Timings are measurements of this machine and never feed back into a
//! simulation; the state hash they report is the same for any thread count.

use crate::{Agent, Op, op};
use marionette_core::Errors;
use marionette_jobs::Jobs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub(crate) fn ops() -> Vec<Op> {
    vec![op(
        "perf_sim",
        "Time the simulation tick of a scene: run `warmup` ticks, then `ticks` timed ticks; \
             returns per-tick milliseconds (avg, p50, p99, max), the worker thread count and the state hash.",
        false,
        sim,
    )]
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimInput {
    pub scene: String,
    #[serde(default)]
    pub seed: u64,
    /// Timed ticks (default 600).
    #[serde(default = "default_ticks")]
    pub ticks: u64,
    /// Untimed ticks first (default 60), so belts fill and caches settle.
    #[serde(default = "default_warmup")]
    pub warmup: u64,
    /// Worker threads for jobs (1 = inline). Default: one per core, or MARIONETTE_JOB_THREADS.
    pub threads: Option<usize>,
}

fn default_ticks() -> u64 {
    600
}
fn default_warmup() -> u64 {
    60
}

#[derive(Serialize, JsonSchema)]
pub struct SimOutput {
    pub scene: String,
    pub ticks: u64,
    pub threads: usize,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Load time of the scene (parse, validate, build module state), milliseconds.
    pub load_ms: f64,
    pub state_hash: String,
}

fn sim(agent: &mut Agent, input: SimInput) -> Result<SimOutput, Errors> {
    let started = Instant::now();
    let mut sim = crate::project::load_sim(agent, &input.scene, input.seed)?;
    let load_ms = started.elapsed().as_secs_f64() * 1000.0;
    let jobs = match input.threads {
        Some(n) => Jobs::with_threads(n),
        None => Jobs::shared(),
    };
    let threads = jobs.threads();
    sim.world.insert_resource(jobs);
    sim.run(input.warmup);
    let mut times = Vec::with_capacity(input.ticks as usize);
    for _ in 0..input.ticks {
        let t = Instant::now();
        sim.step();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let round = |x: f64| (x * 1000.0).round() / 1000.0;
    let avg = if times.is_empty() {
        0.0
    } else {
        times.iter().sum::<f64>() / times.len() as f64
    };
    times.sort_by(f64::total_cmp);
    let pct = |p: f64| {
        times
            .get(((times.len().max(1) - 1) as f64 * p).round() as usize)
            .copied()
            .unwrap_or(0.0)
    };
    Ok(SimOutput {
        scene: input.scene,
        ticks: input.ticks,
        threads,
        avg_ms: round(avg),
        p50_ms: round(pct(0.5)),
        p99_ms: round(pct(0.99)),
        max_ms: round(times.last().copied().unwrap_or(0.0)),
        load_ms: round(load_ms),
        state_hash: sim.state_hash(),
    })
}
