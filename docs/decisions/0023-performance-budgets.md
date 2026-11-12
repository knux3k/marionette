# 0023: Performance budgets, checked by tests

Date: 2026-11-12. Status: accepted.

## Decision

- Performance is a proven property, sized for a large VR factory game: stereo at 90 Hz
  (11.1 ms per frame), a factory simulation of hundreds to thousands of machines and belts.
- Two kinds of proof:
  - Exact checks in every `cargo test` (`crates/marionette-cli/tests/perf_proofs.rs`): the
    generated factory gives the same state hash with 1 and 3 job threads, and a scale 1 and
    scale 4 factory render in a fixed number of draw calls with 2 eyes (instances = meshes +
    items). These catch regressions in batching and determinism on any machine.
  - Timing budgets (`crates/marionette-cli/tests/perf_budgets.rs`, `#[ignore]`, release only):
    sim 4x p99 <= 2 ms, sim 32x average <= 2 ms, load 32x <= 5 s, render CPU of a 32x stereo
    frame (extract + record + submit) p50 <= 4 ms. CI job `perf` runs them on every push with
    `MARIONETTE_BUDGET_SCALE=2` (shared runners are slower and noisier); on the reference
    desktop the scale is 1.
- Benchmark inputs are generated, not checked in: `perf_factory_scene` writes a factory of
  `scale` x 34 cells (one cell = 7 machines, 6 belts, 128 belt slots). `perf_sim` and
  `marionette bench --headset` measure it.
- Budgets and how to measure: `docs/ai/PERFORMANCE.md`.

## Why

Timing on shared CI is noisy, so a hard timing gate alone would be flaky; exact counts (draw
calls, instances, hashes) are not noisy and catch most regressions first. A scale factor keeps
the timing gate in CI without lowering the real budget.

## Consequences

- A change that adds draw calls or breaks thread-count independence fails `cargo test`.
- The GPU side (time on a real headset GPU) is not measured by CI: lavapipe is a CPU rasterizer.
  See UNVERIFIED in `docs/ai/STATE.md`.
