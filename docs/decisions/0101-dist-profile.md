# 0101: Shipping builds use fat LTO with one codegen unit (`--profile dist`); `release` stays

Date: 2026-11-13. Status: accepted.

## Context

`release` uses Cargo defaults: no LTO, 16 codegen units, `opt-level = 3`. The suite (decision
0100) was run with five variants built from the same source, in a 4-core x86-64 cloud VM and
on the owner's desktop (Ryzen 7 2700X, RTX 2080).

## Measurements

- Outputs: every variant gave the same state hash in every case as `release`, and the same
  image hash in every render case on the same GPU (cloud lavapipe, desktop RTX 2080).
- Cloud, `release` vs fat LTO + 1 codegen unit, alternated three runs each: 22 metrics faster
  with non-overlapping ranges, none slower. Scripts tick -13 to -17 percent, spawner -11 to
  -19, proof game ticks -6 to -9 (median p50), engine build -19, scene loads -5 to -27.
- Desktop, single runs: fat LTO about 5 to 10 percent faster on loads, engine start and
  scripts; thin LTO, one codegen unit alone and `target-cpu=x86-64-v3` no consistent gain.
- Clean build time: cloud 436 s (`release`) vs 719 s (fat); desktop 477 s vs 1012 s.

## Decision

- New profile `dist` (root `Cargo.toml`): inherits `release`, `lto = "fat"`,
  `codegen-units = 1`. Use it for builds given to players and for benchmarks of record.
- `release` is unchanged: CI, stability and daily builds on a shared PC would pay twice the
  build time for each run.
- `panic` stays `unwind`: the stability suite and the operation table catch panics.
- No `target-cpu` in any profile: `x86-64-v3` showed no gain over fat LTO and would refuse to
  start on older CPUs.

## Alternatives

- **Fat LTO in `release`**: every session pays about 2x build time; rejected while many
  sessions build on one PC.
- **Thin LTO**: half the extra build time, no measurable gain here.
- **PGO**: not tried yet; needs `llvm-tools` and a training run; revisit in the optimization
  pass with the suite as the training workload.

## Consequences

- Packaging uses `cargo build --profile dist` (binary in `target/dist/`).
- Numbers quoted against other engines come from `dist` builds.
