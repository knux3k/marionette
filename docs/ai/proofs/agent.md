# Proofs: Agent interface, game Rust modules, skills and handoffs (plan line 5)

Line 5 (agent interface): `crates/marionette-cli/tests/agent_e2e.rs`
(`cli_only_sequence_succeeds_and_is_deterministic`, `mcp_sequence_matches_cli_state_hash`,
`mcp_read_only_refuses_mutations_with_structured_error`,
`invalid_edits_are_refused_and_leave_the_file_alone`, `cli_read_only_refuses_mutations`).

Stability suite (decision 0031, `docs/ai/STABILITY.md`):
- `cargo run --release -q -p marionette-cli -- stability` passes every mode (twenty-seven when first written)
  (core-headless, physics-3d, physics-2d, scripted, scripted-spawning, render-3d,
  render-3d-play, render-2d, render-2d-play, render-ui, animation, vr-headset, factory-sim,
  factory-vr, proof-lantern-keeper, proof-cloudstep, proof-ember-run, proof-crystal-ward,
  proof-dust-circuit, proof-signal-yard (each proof game also `-render`),
  agent-session); `--long --seeds 1` passed for every mode up to proof-ember-run (the last
  three proof games have the fast suite only so far) (Linux container, lavapipe, release;
  factory-vr frame p99 12.4 ms at 256x256 per eye on the software GPU, under the CI scale).
- Tests: `crates/marionette-cli/tests/stability.rs` (`fast_suite_passes_for_every_mode`,
  `drift_mode_reaches_the_pinned_drift_hash`, `injected_panic_is_reported_with_tick_and_backtrace`,
  `injected_hang_is_detected`, `nondeterminism_is_caught_and_the_crash_report_replays`,
  `budget_miss_is_caught`, `the_cli_writes_crash_reports_and_exits_1`, `memory_growth_is_caught`,
  `every_scene_has_a_stability_mode`, `bad_suites_and_modes_are_refused_as_data`).
- CI: job `stability` in `.github/workflows/ci.yml` (fast, release, budgets x2);
  `.github/workflows/stability-soak.yml` (nightly long soak, 3 seeds).
- Hardware GPU (2026-11-13, main at `e8814b5`, NVIDIA GeForce RTX 2080 on Vulkan, Windows,
  release, budget scale 1, timing enforced): all 31 fast modes passed, frame budgets checked
  (`frame_budget_checked: true`) in every render mode; no panic, hang or driver warning. Tick
  p99 (budget 2 ms): highest `proof-signal-yard` 1.19 ms, `proof-crystal-ward` 0.90,
  `proof-ember-run` 0.84, `agent-session` 0.69 (budget 4), `factory-sim` 0.036. Frame p99
  (budget 11.1 ms, size per mode in the suite): `proof-crystal-ward-render` 1.61 ms,
  `proof-ember-run-render` 1.45, `proof-signal-yard-render` 1.38, `proof-cloudstep-render`
  1.32, `render-3d-looks` (two eyes) 1.18, `job-swarm-render` 1.00, `animation` 0.91,
  `factory-vr` (two eyes) 0.50, `vr-headset` (two eyes) 0.42, the rest 0.40 to 0.69. Worst
  single frames after warm-up were over budget in two modes, `proof-crystal-ward-render` 42 ms
  and `proof-signal-yard-render` 32.5 ms: a script spawns a model no earlier frame drew and it
  was decoded inside the frame (about 28 ms). Fixed by decision 0068. Windows had no memory
  check in that run (resident size only on Linux); decision 0069 adds one.
- Late model streaming (decision 0068), lavapipe, release, seed 1, worst frame after warm-up
  before and after: `proof-crystal-ward-render` 61.1 to 29.5 ms (p50 about 19 ms, the
  software rasterizer), `proof-signal-yard-render` 48.8 to 20.5 ms.
- The same on the hardware GPU (RTX 2080, branch head `9a4b679`): the full fast suite passed
  again, every mode with the heap probe (growth 0 to 0.24 MiB, `factory-sim` 4.0,
  `factory-vr` 1.06, job modes about 1.0). Worst frame after warm-up, in the suite and three
  more runs each: `proof-crystal-ward-render` 5.1 to 6.2 ms (was 42), `proof-signal-yard-render`
  5.4 to 7.3 ms (was 32.5); p99 1.9 to 5.7 ms. In one of four `proof-signal-yard-render` runs the
  heap grew 12.6 MiB (limit 64; others 0.15): a streamed model that finished decoding after the
  warm-up sample is counted as growth. One-time, not a leak.
- Heap probe (decision 0069), `factory-sim` seed 1, debug: counted heap +4.0 MiB after warm-up
  at 300 and at 1,500 ticks (resident +33.8 and +23.6 MiB): no leak.
- Known finding: `scripted` at 216,000 ticks grows 48 MiB (unbounded `EventLog`, about 670
  bytes per event); the nightly soak fails on it until the event log is bounded.
Memory and crash hygiene (decision 0032, rules in `docs/ai/MEMORY-AND-CRASHES.md`):
- `crates/marionette-agent/tests/leaks.rs` (`memory_stays_flat_under_repeated_work`: session
  load/unload, a long run past the event-log bound, script hot reload, entity churn,
  screenshots; heap growth under 64 KB per scenario, counted by `marionette_host::alloc`).
- `gpu_objects_come_back_after_repeated_setups_and_trims` in
  `crates/marionette-render/tests/render.rs`.
- `crates/marionette-host/tests/crash.rs` (`children_die_when_the_engine_is_killed`,
  `a_killed_process_leaves_a_run_folder_that_the_next_start_removes`,
  `shutdown_removes_the_run_folder_and_temp_dirs_clean_up_on_drop`);
  `crates/marionette-cli/tests/crash.rs` (`a_run_killed_mid_simulation_is_cleaned_up_by_the_next_start`,
  `a_clean_exit_leaves_no_run_folder`).
- `crates/marionette-script/tests/misbehaving.rs` (endless loop, out of memory, deep recursion,
  engine call at load); `a_panicking_operation_is_an_error_and_the_agent_keeps_serving` in
  `crates/marionette-agent/src/lib.rs`; `the_event_log_is_bounded_and_positions_stay_absolute`
  in `crates/marionette-core/src/module.rs`.
- CI job `leaks (valgrind)`: core, host and script tests under valgrind, no definite leaks
  (checked locally on Linux).

Repository rules: `crates/marionette-cli/tests/repo_hygiene.rs` (no identifying details, every
path in `docs/ai/` exists, generated reference is current).

## Game Rust modules and cargo features (decision 0062)

- `cargo test -p marionette-example-rust-module` (built with `script` only): the game module
  is deterministic and settles at its anchor, the built-in engine refuses its component, the
  `spring_set` operation changes a live session and is refused read-only, duplicate or badly
  named parts are refused, and the `spring-game` binary runs scenes and lists only the
  operations of the parts it was built with (`examples/rust-module/tests/spring.rs`).
- `cargo clippy -p marionette-agent -p marionette-cli --all-targets --no-default-features -- -D warnings`
  passes, and with each single feature (`script`, `import`, `pool`, `generate`, `port`,
  `jobs-ts`).
- Linux debug build: example binary 489 linked crates and 103 MB, full `marionette` 559 and
  147 MB, `marionette-cli --no-default-features` 374 crates.

## Skills (decision 0160)

- `cargo test -p marionette-cli --test skills`: `skills_name_only_things_that_exist` (every
  operation, CLI command and flag, type, repository path, doc heading and skill a skill names),
  `the_checker_catches_stale_names` (ten stale bodies, each reported), `registry_index_and_folders_agree`
  (folders, `skills.rs`, `AGENTS.md`, description budget), `games_get_the_skills_and_handoff_files`
  (`project_init`, `skills_install`, `skills_list`, `skills_get`).
- By hand in a fresh project (Linux container, 2026-11-13): every command line the game skills
  show ran as written (`project_schema`, `asset_search`, `generate_recipe`, `generate_plan`,
  `script_types`, `jobs_check`, `jobs_run`, `port_convert`, `observe_screenshot`, `perf_sim`,
  `test_run`, `marionette run`, `marionette bench`), and the script and job examples compiled
  and ran.
- Sizes: 16 descriptions 5.3 KB together; bodies 1.5 to 3.4 KB.
- Handoffs (decision 0161): `handoffs_keep_parallel_agents_apart` in the same test file: init
  without overwriting, two claims with separate decision blocks, overlapping area and topic
  refused, merged colliding claims reported, notes and release, size warning, read-only.
- Indexed edits (decision 0270): `cargo test -p marionette-core --test scene_index` (index
  against `format_text`) and `cargo test -p marionette-agent --test indexed_edits`: the same
  steps, more than ten of them refusals, through the indexed path and the whole-scene path give the same
  answers and bytes, for files and for live sessions (hash, tree and log), one call at a time
  and in batches, including a refusal only a module can give in the middle of a batch.
  Timing: `scene_edit_budgets` in `perf_budgets` (release, Linux container, 2026-11-14): 10,000
  creates in one batch 525 to 912 ms (budget 2,000), one file edit at 10,000 entities 4.5 ms
  (budget 20), 10,000 creates into a live session in one batch 614 to 1,003 ms (budget 2,500),
  one live edit at 10,000 entities 130 ms (budget 500).
