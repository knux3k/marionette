# Proofs: Performance, porting and saving

## Performance (workstream WP)

Proofs: `cargo test -p marionette-example-factory --test perf_proofs` (thread-independent factory hash,
draw calls and instances with 2 eyes); `cargo test -p marionette-jobs`;
`cargo test -p marionette-example-factory` (also its stability modes and the `perf_suite`
factory cases, decision 0290); timing budgets in release:
`cargo test --release -p marionette-example-factory --test perf_budgets -- --ignored --test-threads=1`.
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

## Serializable to serde (decision 0240)

- `cargo test -p marionette-port --test port serde`: a synthetic C# fixture
  (`tests/fixtures/serial`) generates `tests/generated/serial_fixture.rs`, which is committed,
  compiled and checked equal to the generator's output (`MARIONETTE_BLESS=1` rewrites it).
  Checks: missing keys keep initializers, byte order mark ignored, unknown keys ignored,
  unknown enum integers and raw JSON survive a round trip, field order base class first.
- `serde_generation_writes_once_and_reports` (marionette-cli `port_e2e`): the operation returns
  code on a dry run, writes once, refuses to overwrite, and reports skipped fields.
- Not checked against files written by a real Unity build.
- Decision 0241 (`serde_generator_handles_consts_properties_and_external_types`): const and
  static initializers, `float.MinValue`, qualified `new`, expression-bodied properties,
  external types (including a replaced vector type) and unknown element types. On a real
  project's save classes (private) the generator now leaves no unresolved or untranslated
  entries.

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

UNVERIFIED: GPU time by timestamp queries (only submit-to-idle), PGO, ARM CPUs, AMD or
Intel GPUs. The alternated desktop run of `release` against `dist` is pending (single runs
agree with the cloud). Large allocation counts per tick in script-driven games are the first
target of the optimization pass.

## Realistic scene against Unity (decision 0284)

First run, 2026-11-14, main at `223fbc6`, Ryzen 7 2700X, RTX 2080, `--profile dist`; Unity
6000.3.4f1, URP 17.3.0, an empty project built by `tools/unity-bench` (no script changes
needed). 1920x1080, 4x MSAA, 1,000 timed frames after warm-up. Marionette on Vulkan, offscreen;
Unity on Direct3D 12, a window, vertical sync off.

| | Frame p50 / p99 ms | Where the time goes (p50 ms) |
| --- | --- | --- |
| Marionette | 19.31 / 24.73 | GPU wait 10.52, recording 7.95, extract 0.62, sim 0.20 (one after another) |
| Unity | 11.13 / 15.95 | main thread 11.13, render thread 1.40, GPU about 4.3 (overlapped) |
| Unity, static batching | 11.21 / 16.05 | the same |

- Unity is 1.7x faster on this scene. Marionette's bench waits for the GPU every frame, so
  its CPU and GPU time add up; overlapped, it would be bound by its GPU at about 10.5 ms,
  still about 2.4x Unity's GPU time. Recording 1,285 draws (24,897 instances) takes 8 ms.
- Marionette draws 14.9 million triangles after culling (3,776 parts culled).
- Unity's GPU numbers are rough: some frames report 0 ms (average 3.26 below median 4.29).
- Nobody looked at the Unity player's picture; its log shows no errors.
- Headset, 2 eyes at 2160x2160: 21.05 ms p50 (GPU wait 12.49, recording 7.69), 1,542 draws.
  This run drew a pass per eye: `marionette bench --headset` did not turn on single-pass stereo
  (decision 0285) until the fix that followed it.
- Next: the deferred optimization pass (decision 0100) starts from these two gaps, GPU time
  per frame and CPU time per draw.
