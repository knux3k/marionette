---
name: engine-measure-performance
description: Measure engine speed the shared way, before and after a change, with the fixed benchmark suite and its comparison, on a shared machine without disturbing other runs. Use when optimizing the engine, changing build settings, touching a hot path (tick, extract, render, load), claiming a speedup, or running timed benchmarks on the owner's shared PC.
---

# Measure engine performance

One suite, one report format, outputs checked before timings (decision 0100). Numbers from
different machines, builds or suite versions are never compared.

## Before and after

1. Build release from the commit you start from: `cargo build --release -p marionette-cli`.
2. `target/release/marionette op perf_suite '{"label": "before"}'` writes
   `.marionette/perf/before.json` and prints one line per case.
3. Change the code, rebuild, run again with `"label": "after"`.
4. `target/release/marionette op perf_compare '{"base": ".marionette/perf/before.json", "new": ".marionette/perf/after.json"}'`.
5. `outputs_identical` must be `true`. If a state hash or (same GPU) image hash moved, the
   change altered results: make it opt-in and labelled, or fix it. Only then read `changes`.
6. Read `warnings`: other load during a run, different machine or suite make timings void.
   For changes under about 10 percent (build settings, small fixes), run the two binaries
   alternately three times each (`before-1`, `after-1`, `before-2`, ...) and pass lists:
   `"base": ["...before-1.json", "...before-2.json", "...before-3.json"]`. A change then
   counts only when the two sides' ranges do not overlap.
7. Put the numbers, machine class and GPU in `docs/ai/proofs/performance.md`.

Inputs worth knowing: `only` (case name prefixes such as `sim/` or `render/factory`),
`length: "quick"` (smoke test, not for numbers), `no_gpu`, `extra_scenes` (a game's own
scenes), `repeats`. Field meanings: `docs/ai/PERFORMANCE.md`, section
"The shared suite (decision 0100)".

## On the owner's shared PC

Several agents use one PC. Each thread works in its own worktree; never check out a branch in
another thread's worktree.

1. Build first. Never build while the bench lock exists; otherwise cap builds with
   `CARGO_BUILD_JOBS=4`.
2. Run with the lock and idle checks; the suite takes the lock, waits, and removes it itself
   (also on failure):
   `marionette op perf_suite '{"label": "<area>-before", "lock": "<ai folder>/pc-bench.lock", "lock_owner": "<area>", "min_idle_s": 300, "wait_s": 3600}'`.
   It refuses while someone else's lock exists, while the desktop was used in the last
   `min_idle_s` seconds, or while other processes keep the CPU busy (`max_busy_pct`, default
   15).
3. The report's `conditions` record the lock and other CPU load; a run with load between cases
   above 10 percent is rerun, not reported.
4. Windows and GPU runs also need the Unity lock free. Close everything you started.

## Traps

- A software GPU (`gpu_kind` `Cpu`: lavapipe, WARP) measures the CPU, not GPU time.
- `gpu_wait` is time from submit to idle, not exact GPU time.
- Debug builds: numbers mean nothing.
- Fewer than 3 repeats or `noise_pct` above 10: do not claim a change under that size.
