# 0022: Deterministic data-parallel jobs

Date: 2026-11-12. Status: accepted.

## Decision

- `crates/marionette-jobs` provides `Jobs` (a resource) over a rayon pool. Thread count:
  `MARIONETTE_JOB_THREADS` (1 to 64), default the machine's cores; `Jobs::shared()` reuses one
  pool per process; `Jobs::inline()` runs everything on the calling thread.
- Operations: `for_each_chunk_mut`, `for_each_chunk_mut2` (two arrays chunked together),
  `for_each_segment_mut` (variable-length segments, e.g. one belt line each) and `map_chunks`
  (results returned in chunk order).
- Rules that make results independent of thread count:
  - chunk boundaries come from the data (fixed chunk size or segment ends), never from the
    thread count;
  - each chunk writes only its own slice; no shared mutable state, no atomics in results;
  - reductions combine per-chunk results in chunk order;
  - below `PARALLEL_MIN_LEN` items the work runs inline (thread start-up costs more than it
    saves).
- Systems still run on one thread (`SingleThreadedExecutor`); jobs parallelize inside a system.
- Test: `crates/marionette-jobs/tests/determinism.rs` (1, 2, 3 and 8 threads give equal results).

## Why

This is the engine's answer to Unity's job system plus Burst: Rust already compiles job bodies
to native SIMD-friendly code, so what was missing was a scheduler that keeps the state hash
identical on every machine. A typed TypeScript subset compiled to WASM for game-authored job
bodies is a later, separate workstream.

## Consequences

- The determinism rule "no multithreaded execution" in `docs/ai/ARCHITECTURE.md` now reads
  "only through `marionette-jobs`, following its rules".
- Memory-bound work (belt slot shuffles) gains little from threads; measure before
  parallelizing.
