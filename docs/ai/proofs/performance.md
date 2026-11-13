# Proofs: Performance, porting and saving

## Performance (workstream WP)

Proofs: `cargo test -p marionette-cli --test perf_proofs` (thread-independent factory hash,
draw calls and instances with 2 eyes); `cargo test -p marionette-jobs`;
`cargo test -p marionette-factory`; timing budgets in release:
`cargo test --release -p marionette-cli --test perf_budgets -- --ignored --test-threads=1`.
Numbers and budgets: `docs/ai/PERFORMANCE.md`.

## Porting Burst jobs (decision 0072)

- `cargo test -p marionette-port --test port unity_jobs_partials_and_tuples`: fixture
  `tests/fixtures/unity-jobs` (job struct, data struct, partial class over two files, tuple
  types): role `job`, summary `jobs`, one merged partial class, native-container and `math.*`
  statuses, and the `jobs.rs` skeleton compiles with `rustc` when it is on the path.
- Private slice of a real game (not in the repository): scan without a panic, 3 jobs found,
  `jobs.rs` compiled unchanged; the hand port ran 0.12 ms per tick at 4x and 0.2 ms at 8x
  (release, one thread, container), same state hash with 1 and 4 threads.

## Saving off the tick (decision 0073)

- `cargo test -p marionette-cli --test perf_proofs capture_saved_on_another_thread_equals_snapshot`:
  factory and swarm scenes; the scene built from `Sim::capture` on another thread equals the
  snapshot byte for byte.
- `cargo test --release -p marionette-cli --test perf_budgets -- --ignored save_capture_budget`:
  4x factory capture 0.75 ms (budget 2 ms); the whole snapshot as text 72 ms, 2.3 MB (container).

## Shared performance suite (decision 0100) and the `dist` profile (decision 0101)

- `cargo test -p marionette-cli --test perf_suite`: a quick run repeats state hashes, two runs
  compare as identical, a changed hash is reported, several runs per side compare without
  changes, a held bench lock refuses the run and the run removes its own lock.
- Build settings, same source, suite `core/1`: `release`, thin LTO, fat LTO + 1 codegen unit,
  1 codegen unit, fat LTO + `x86-64-v3`. Every state hash and every image hash (same GPU)
  equal to `release`, in the cloud (lavapipe) and on a Ryzen 7 2700X with an RTX 2080.
  Timing results: decision 0101.

Baseline, `release`, one run (p50 / p99 ms per tick; allocations per tick):

| Case | Cloud 4-core VM | Ryzen 7 2700X | Allocs/tick |
| --- | --- | --- | --- |
| startup/engine (build engine) | 2.97 / 5.35 | 2.78 / 3.29 | |
| startup/process (new process to first tick) | 13.7 | 79.6 (Windows) | |
| sim/factory-32x (load 0.8 s cloud, 1.2 s PC) | 0.157 / 0.340 | 0.131 / 0.287 | 81 |
| sim/game-ember-run | 0.61 / 1.20 | 0.78 | 2,168 |
| sim/game-signal-yard | 0.58 / 1.28 | 0.86 | 1,750 |
| sim/game-cloudstep | 0.45 / 0.80 | 0.68 | 1,803 |
| sim/scripts | 0.092 / 0.199 | | 359 |
| render/looks-1080p frame | software GPU | 1.00 / 1.45 | |
| render/game-crystal-ward-1080p frame | software GPU | 0.96 / 1.48 | |
| render/factory-4x-headset frame | software GPU | 0.89 / 1.36 | |
| render/factory-32x-headset frame | software GPU | 4.03 / 7.99 | |

Not yet measured: GPU time by timestamp queries (only submit-to-idle), PGO, ARM CPUs, AMD or
Intel GPUs. The alternated desktop run of `release` against `dist` is pending (single runs
agree with the cloud). Large allocation counts per tick in script-driven games are the first
target of the optimization pass.
