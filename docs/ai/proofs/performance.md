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
