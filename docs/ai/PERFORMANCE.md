# Performance

Target: a large VR factory game, stereo at 90 Hz (11.1 ms per frame), on a mid-range desktop
GPU. Decisions: 0021 (budgets), 0022 (jobs), 0023 (renderer), 0024 (factory module).

## Budgets

| What | Budget | Checked by |
| --- | --- | --- |
| Whole frame (sim + render CPU + GPU) | 11.1 ms | not in CI yet (needs a real GPU) |
| Factory sim tick, 4x stress (952 machines, 15k slots), p99 | 2 ms | `perf_budgets::sim_tick_budgets` |
| Factory sim tick, 32x (7,616 machines, 122k slots), average | 2 ms | `perf_budgets::sim_tick_budgets` |
| Load the 32x factory scene | 5 s | `perf_budgets::sim_tick_budgets` |
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
| Render CPU 32x stereo, p50 | 3.1 ms (extract about 1.2 ms) |
| 10,000 cubes, mono, CPU frame, before / after instancing | 153 ms / 12 ms, 1 draw call |

GPU time on a real desktop GPU and headset is UNVERIFIED (`docs/ai/STATE.md`).

## Known next steps

- Frustum culling per view (every instance is drawn in both eyes today).
- Persistent instance buffers for objects that do not move.
- Fewer allocations in extract (the largest share of render CPU).
- A simulation rate separate from the frame rate, with interpolation everywhere (factory items
  already interpolate).
- Game-authored job bodies in a typed TypeScript subset compiled to WASM (separate workstream).
