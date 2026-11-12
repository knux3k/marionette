# Stability

Every game mode the engine supports has a stability mode: a long, seeded, replayable run that
must not crash, hang, lose determinism, miss its time budget or grow its memory. Decision 0031.

## Rule

A change that adds a game mode, an example scene or a proof scene adds a mode to
`stability/suite.json` in the same change (or an `exempt` entry with a reason). The test
`every_scene_has_a_stability_mode` in `crates/marionette-cli/tests/stability.rs` fails otherwise:
it finds every `*.scene.json` outside `crates/`, `spikes/`, `docs/` and `target/`.

## What one mode run checks

| Check | Failure kind | How |
| --- | --- | --- |
| No panic | `panic` | the run is on its own thread; a panic hook keeps the message and backtrace |
| No hang | `hang` | no finished tick for `hang_s` (default 20 s) |
| Deterministic | `nondeterminism` | a second run with the same scene, seed and input gives the same state hash |
| Snapshots work | `snapshot` | restore the snapshot taken halfway, run to the end, same hash |
| Time budget | `budget` | p99 tick (`tick_budget_ms`) and p99 frame (`frame_budget_ms`) x budget scale; release builds only (debug timings mean nothing) |
| Memory steady | `memory` | growth after warm-up (first fifth of the run) at most `memory_growth_mib` (default 32) |
| Scripts healthy | `script_error` | no `script_error` events unless `allow_script_errors` |
| Loads and operations | `error` | load errors, setup operations and agent operations must succeed |

Input: `random_input` presses every device input of the scene's action map from a seeded
generator (about one change every six ticks; keys toggle, axes in steps of 0.1); `input` adds a
scripted file. The generator is separate from the simulation RNG.

## Kinds of modes

| Kind | What runs |
| --- | --- |
| `sim` | headless ticks (servers, batch runs, the simulation side of every game) |
| `render` | one tick plus one offscreen frame per step: the windowed game loop without the window; skipped without a GPU unless `MARIONETTE_REQUIRE_GPU=1` |
| `agent` | a live session driven through the operation table (step with input, query, stats, events, create and delete entities, snapshot, restore), as the CLI and MCP server run it |

`views: 2` renders both eyes of an emulated headset every frame (`Offscreen::render_eyes`, 64 mm
apart, decision 0025). A mode with `setup` runs operations first, for example
`perf_factory_scene` to generate a factory scene under `target/stability/`.

## Current suite (`stability/suite.json`)

core-headless, physics-3d, physics-2d, scripted, scripted-spawning, render-3d, render-3d-play,
render-2d, render-2d-play, render-ui, animation (skinning), vr-headset (stereo), factory-sim (4x stress factory on the
job system), factory-vr (factory in stereo), a headless and a rendered mode per proof game
(lantern-keeper, cloudstep, ember-run), agent-session. Fast length: 120 to 1,000 ticks
each, a few seconds in a debug build for all twenty-one. Long length: 1 hour of game time per sim
mode (216,000 ticks; drift 1,000,000), 15 minutes per render mode (54,000 frames), 30 minutes
of agent session.

## Commands

```bash
# Fast suite (what CI runs on every push, release build, budgets checked)
cargo run --release -q -p marionette-cli -- stability
# Long soak, 3 seeds from 100
cargo run --release -q -p marionette-cli -- stability --long --seed 100 --seeds 3
# One mode, as a failure report's `reproduce` line says
cargo run --release -q -p marionette-cli -- stability --mode physics-3d --seed 7 --ticks 600
# The same through the operation table (CLI or MCP)
cargo run -q -p marionette-cli -- op stability_run '{"modes":["scripted"],"length":"long"}'
```

The CLI writes `target/stability/report.json` and, per failed run,
`<mode>-seed<N>.crash.json` (the full result: failures with tick, message and backtrace, timing,
memory, the reproduce command) and `<mode>-seed<N>.inputs.json` (the exact input it was fed).
`marionette run <scene> --headless --ticks <N> --seed <S> --input <mode>-seed<S>.inputs.json`
then gives the first run's state hash (test
`nondeterminism_is_caught_and_the_crash_report_replays`). Exit code 1 if any run failed.

## Where it runs

| Where | Length | Timing |
| --- | --- | --- |
| `cargo test` (`crates/marionette-cli/tests/stability.rs`) | fast, every mode | not checked (debug) |
| CI job `stability` in `.github/workflows/ci.yml`, Linux and Windows | fast | checked, `MARIONETTE_BUDGET_SCALE=2` |
| `.github/workflows/stability-soak.yml`, nightly and on demand | long, 3 seeds, first seed = run number | checked, scale 2 |

Reports are kept as workflow artifacts (`stability-fast-*`, `stability-long-*`).

## Fixing a failure (for an agent)

1. Read the crash report: `failures[0].kind`, `tick`, `message`, `backtrace`.
2. Run its `reproduce` command; it gives the same failure (same seed, same input).
3. For `nondeterminism` and `snapshot`, bisect the tick with shorter `--ticks`; check the
   determinism list in ARCHITECTURE.md.
4. For `memory`, compare `engine_events` and `entities_end` with `entities_start`; anything that
   grows per tick is the suspect.
5. Fix, rerun the mode, then the whole fast suite.

## Testing the harness

`stability_run` takes `inject` (panic at a tick, hang at a tick, a different RNG seed in the
replay, a sleep per tick, memory kept per tick). The tests use it to prove each failure kind is
caught. Never use it in the suite file.

## Not covered yet

- A real window and a real audio device (the `render` kind runs the same loop offscreen; opening
  windows on a user's screen needs the owner's approval).
- Memory on Windows: the default probe reads `/proc/self/statm`. A binary can install another
  with `marionette_agent::stability::set_memory_probe` (a counting allocator).
