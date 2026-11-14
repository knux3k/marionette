//! Times the factory step alone (no other systems) for 1..N worker threads.
//! `cargo run --release -p marionette-example-factory --example factory_step_cost -- <scene> [ticks]`

use marionette_core::Sim;
use marionette_example_factory::{FactoryModule, FactoryState, FactoryTables, step};
use marionette_jobs::Jobs;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: factory_step_cost <scene> [ticks]");
    let ticks: usize = args.get(2).and_then(|t| t.parse().ok()).unwrap_or(600);
    let text = std::fs::read_to_string(path).expect("read scene");
    let sim = Sim::with_modules(
        &text,
        Some(path),
        1,
        Arc::new(vec![Box::new(FactoryModule)]),
        Default::default(),
    )
    .expect("scene loads");
    let tables = sim.world.resource::<FactoryTables>().clone();
    let start = sim.world.resource::<FactoryState>().clone();
    let threads_max = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut threads = 1;
    while threads <= threads_max {
        let jobs = Jobs::with_threads(threads);
        let mut state = start.clone();
        for _ in 0..600 {
            step(&jobs, &tables, &mut state);
        }
        let t = Instant::now();
        for _ in 0..ticks {
            step(&jobs, &tables, &mut state);
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / ticks as f64;
        println!(
            "threads {threads}: {ms:.4} ms/tick ({} machines, {} slots, made {})",
            state.machines.len(),
            state.slots.len(),
            state.made
        );
        threads *= 2;
    }
}
