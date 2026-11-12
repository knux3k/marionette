# 0031: Stability suite per game mode

Date: 2026-11-12. Status: accepted.

## Decision

- Stability is a tested property of every game mode: no panic, no hang, deterministic replay,
  working snapshots, time budgets held, memory steady, no script errors. One operation,
  `stability_run` (`crates/marionette-agent/src/stability.rs`), runs a suite file
  (`stability/suite.json`) of modes; `marionette stability` is its CLI shortcut and writes the
  reports. Details: `docs/ai/STABILITY.md`.
- A mode is data (name, kind `sim` / `render` / `agent`, scene, input, fast and long tick
  counts, budgets), so adding one needs no code. Every scene in the repository must have a mode
  or a reasoned exemption; a test enforces it.
- Input is seeded random presses over the scene's action map, generated outside the
  simulation, so a failing run is reproduced exactly from its seed (and its input is written
  next to the crash report as a normal events file).
- Each run happens on its own thread under a supervisor: panics are caught with their backtrace,
  hangs are detected from tick progress. The CLI exits after reporting, so an abandoned hung
  thread never outlives the process.
- Two lengths: fast on every push (debug inside `cargo test` without timing; release CI job with
  timing, budget scale 2), long nightly and on demand (several seeds, one hour of game time per
  simulation mode).
- Time budgets reuse the performance conventions of decision 0023: the 11.1 ms frame (90 Hz),
  2 ms per simulation tick, `MARIONETTE_BUDGET_SCALE` for shared runners.
- Memory is read through a replaceable probe (default: resident set size on Linux), so a
  counting allocator can take over on every platform.

## Alternatives considered

- Fuzzing with `cargo fuzz`: good for parsers, but it needs nightly Rust and does not measure
  time, memory or determinism over long runs. Can be added for the scene and input parsers later.
- Separate test files per mode in Rust: each new mode would need code and could drift from the
  CLI; a data file read by one operation keeps CLI, MCP and CI identical.
- Child processes per mode (kill on hang): stronger isolation, but slower and harder to read
  results from; a supervising thread plus process exit after a hang gives the same result for
  the CLI.

## Consequences

- New game modes and proof scenes (W8) add a line to `stability/suite.json`.
- A long soak found the first real issue: the event log grows without bound (about 670 bytes per
  event; a scripted scene grew 48 MiB in one hour of game time). Bounding it belongs to the
  memory work; until then the nightly soak reports it.
- Windows has no memory check until a counting allocator probe is installed.
- VR is covered by rendering both eyes of an emulated headset (`views: 2`); a real OpenXR
  runtime is not.
