# 0100: One shared performance suite with output checks (`perf_suite`, `perf_compare`)

Date: 2026-11-13. Status: accepted.

## Context

Many sessions change engine code at once and several share one desktop PC for timed runs.
Before this, each measured its own way (`perf_sim`, `bench`, stability timings, ad hoc
scripts), on different scenes, with numbers that could not be compared and no check that a
speedup left results unchanged. The owner's goal is more speed with no loss of quality, so
every speed claim needs the same before and after, with outputs proven identical.

## Decision

- Operation `perf_suite` (`crates/marionette-agent/src/perf_suite.rs`) runs a fixed, versioned
  case list (suite `core/1`): `startup/engine`, `startup/process`, 15 `sim/*` cases (examples,
  factory 4x and 32x, the six proof games) and 9 `render/*` cases (1920x1080 mono and an
  emulated headset, MSAA 4). Per case: load time, tick or frame percentiles (frame split into
  sim, extract, record, GPU wait), allocations per tick, counted heap, spread between repeats,
  the state hash and the SHA-256 of a fixed final image. A case repeats until 2 s of timed
  ticks are pooled (at most 100 runs). One JSON report per run (`marionette-perf/1`) with
  build profile, features, CPU features, commit, CPU model, cores, GPU and its kind.
- Operation `perf_compare` checks outputs first (every state hash, and image hashes when the
  GPU is the same), then timings. It takes one report or several runs per side; with several,
  a change counts only beyond the threshold and outside the other side's range.
- Shared machines: `lock` (a lock file taken with create-new and removed on exit, also on a
  panic, only if still ours), `min_idle_s` (desktop input idle, Windows), `max_busy_pct`
  (other processes' CPU load before starting), `wait_s`. The report records the lock and the
  CPU load in the pauses between cases; `perf_compare` warns above 10 percent.
- `marionette_host::machine`: CPU model, whole-machine CPU load, desktop idle time. Hardware
  model only; no host or user names.
- Skill `engine-measure-performance` is the procedure.

## Alternatives

- **criterion or another benchmark crate**: good statistics for microbenchmarks, but it does
  not load scenes, render, check hashes or images, or run as an operation an agent calls over
  MCP. Rejected for the suite; usable inside a crate for a single function.
- **Extend `bench` and `perf_sim`**: each covers one part; a fixed list with one report
  format and an output check is a separate concern. They stay for single-scene work.
- **Time-based tick counts** (run each case for N seconds): fewer runs for slow machines, but
  the tick count would decide the state hash, so hashes would differ between builds. The suite
  keeps tick counts fixed and repeats whole runs instead.
- **GPU timestamp queries**: exact GPU time, but the renderer is shared with other sessions;
  `gpu_wait` (submit to idle) is used until the optimization pass adds queries.

## Consequences

- Focus 3: every speed change has one command before and after; first use found that every
  build-setting variant gives identical state hashes and images (decision 0101).
- Focus 2: the operation returns one line per case; the full report stays in a file.
- Focus 1: `extra_scenes` runs a game's own scenes through the same suite.
- Cloud VMs are noisy (per-repeat spread 30 to 100 percent on microsecond ticks); only
  alternated runs, three per side, give usable differences there.
