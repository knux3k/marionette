# Performance

Target: a large VR factory game, stereo at 90 Hz (11.1 ms per frame), on a mid-range desktop
GPU. Decisions: 0023 (budgets), 0024 (jobs), 0025 (renderer), 0026 (factory module).

## Budgets

| What | Budget | Checked by |
| --- | --- | --- |
| Whole frame (sim + render CPU + GPU) | 11.1 ms | not in CI yet (needs a real GPU) |
| Factory sim tick, 4x stress (952 machines, 15k slots), p99 | 2 ms | `perf_budgets::sim_tick_budgets` |
| Factory sim tick, 32x (7,616 machines, 122k slots), average | 2 ms | `perf_budgets::sim_tick_budgets` |
| Load the 32x factory scene | 5 s | `perf_budgets::sim_tick_budgets` |
| Main-thread part of a save, 4x factory (`Sim::capture`) | 2 ms | `perf_budgets::save_capture_budget` |
| Render CPU, 32x factory, 2 eyes, p50 | 4 ms | `perf_budgets::render_cpu_budget` |
| Draw calls for a factory frame | fixed per asset and part, independent of scale | `perf_proofs` (every `cargo test`) |
| Sim result independent of job threads | same state hash with 1 and 3 threads | `perf_proofs` (every `cargo test`) |

CI multiplies timing budgets by `MARIONETTE_BUDGET_SCALE=2` (job `perf` in
`.github/workflows/ci.yml`).

## How to measure

```bash
# Timing budgets (release, one test at a time)
cargo test --release -p marionette-cli --test perf_budgets -- --ignored --test-threads=1 --nocapture

# A factory of a given size, then its tick cost
cargo run --release -q -p marionette-cli -- op perf_factory_scene '{"out":"target/f4.scene.json","scale":4}'
cargo run --release -q -p marionette-cli -- op perf_sim '{"scene":"target/f4.scene.json","ticks":1200}'

# Rendering with an emulated headset (2 eyes, 2160x2160 each, 64 mm IPD, head yaw)
cargo run --release -q -p marionette-cli -- bench target/f4.scene.json --headset --frames 120
```

`perf_sim` reports average, p50, p99 and max per tick, load time, thread count and the state
hash. `bench` reports phase medians (sim, extract, record, GPU wait) and `RenderStats` (views,
draw calls, instances, triangles). Set `MARIONETTE_JOB_THREADS` to compare thread counts.

## Results (Linux container, Mesa lavapipe software GPU, release)

| Measure | Value |
| --- | --- |
| Sim tick 4x, p99 | 0.03 ms |
| Sim tick 32x, average | 0.20 ms |
| Sim tick 128x, average | 0.8 ms |
| Load 32x | 0.53 s |
| Save 4x: `Sim::capture` / whole snapshot as text | 0.75 ms / 72 ms (2.3 MB) |
| Render CPU 32x stereo, p50 | 3.1 ms (extract about 1.2 ms) |
| 10,000 cubes, mono, CPU frame, before / after instancing | 153 ms / 12 ms, 1 draw call |

GPU time on a real desktop GPU and headset is UNVERIFIED (`docs/ai/STATE.md`).

## The shared suite (decision 0100)

Every session that changes speed runs the same suite before and after, on the same machine,
and compares the two reports. Outputs are checked before timings.

```bash
cargo build --release -p marionette-cli
target/release/marionette op perf_suite '{"label":"before"}'     # writes .marionette/perf/before.json
# ... change code, rebuild ...
target/release/marionette op perf_suite '{"label":"after"}'
target/release/marionette op perf_compare '{"base":".marionette/perf/before.json","new":".marionette/perf/after.json"}'
```

- Cases (suite `core/1`, `crates/marionette-agent/src/perf_suite.rs`): `startup/*` (engine
  build, a new process to its first tick), `sim/*` (examples, factory 4x and 32x, the six proof
  games), `render/*` (1920x1080 mono and 2x2160x2160 headset, MSAA 4). Each case: load time,
  tick or frame percentiles (frame split into sim, extract, record, GPU wait), allocations per
  tick, counted heap, run-to-run noise, state hash, and SHA-256 of a fixed final image.
- `perf_compare` says `outputs_identical: false` when any state hash differs, or any image hash
  differs on the same GPU. A speed change that changes output must be opt-in and labelled.
- `length: "quick"` is a smoke test (a tenth of the ticks, one repeat). `only: ["sim/"]`
  picks cases; `extra_scenes` adds a game's own scenes. `no_gpu` skips render cases.
- Software rasterizers (lavapipe, WARP; `gpu_kind: "Cpu"`) say nothing about GPU time.
- Results and the machine they came from go in [proofs/performance.md](proofs/performance.md), section "Shared performance suite (decision 0100)".

## Saving

`Sim::snapshot` builds the canonical scene text: tens of milliseconds for a large factory
(decision 0073). In a game loop call `Sim::capture()` on the tick thread (a typed copy) and
`Capture::into_scene(&registry).to_text()` on another thread. `state_hash` also builds the
whole text; do not call it every tick.

## Known next steps

- Frustum culling per view (every instance is drawn in both eyes today).
- Persistent instance buffers for objects that do not move.
- Fewer allocations in extract (the largest share of render CPU).
- A simulation rate separate from the frame rate, with interpolation everywhere (factory items
  already interpolate).
- Game-authored job bodies: done as job TypeScript (decision 0030, `docs/ai/JOBS-TS.md`):
  about 50x the script interpreter, about 4x the time of a Rust job body on one thread
  (`cargo run --release -p marionette-jobs-ts --example jobs_ts_bench`).
