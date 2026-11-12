# 0062: Games add Rust modules and operations; optional engine parts are cargo features

Date: 2026-11-12. Status: accepted.

## Context

Focus 1 (decision 0060) is modularity, including custom code. Before this decision the module
list (`engine::modules()`, a static) and the operation table (`Agent::new`) were fixed inside
`marionette-agent`: a game could script in TypeScript but could not add a Rust module or an
operation without editing the engine. `marionette-agent` also linked every crate, so every
game carried asset import, the CC0 pool, generation, porting and job TypeScript (wasmtime)
whether it used them or not.

## Decision

- **`Engine` and `EngineBuilder`** (`crates/marionette-agent/src/engine.rs`). An `Engine` holds
  the modules in a fixed order, their registry and the operation table. `Engine::builder()`
  starts from the built-in parts; `.module(m)`, `.op(op)`, `.ops(v)` add a game's own; `build`
  refuses duplicate module or operation names (`agent.duplicate`) and operation names that are
  not `lower_snake_case` (`agent.bad_name`). Game modules run after the built-in ones, so
  existing scenes keep their state hashes.
- **The agent runs an engine**: `Agent::with_engine(root, read_only, engine)`; `Agent::new` is
  the built-in engine. Every scene load inside operations, stability runs and setup agents
  uses the agent's engine. `op` and `Op` are public, and `Agent` gains `load_sim`,
  `session_sim` and `write_file` for game operations.
- **The CLI is a library**: `marionette_cli::main_with(engine)` gives a game's binary every
  command (run, op, ops, mcp, docs, window, bench) with its parts. `marionette_cli::main()` is
  the built-in engine.
- **The script module takes a closure** for the registry (was a `fn` pointer), so scripts
  reach a game's components by name. The engine passes a weak handle to its module list to
  avoid a reference cycle.
- **Cargo features on `marionette-agent` and `marionette-cli`**: `script`, `import`, `pool`,
  `generate`, `port`, `jobs-ts` (all on by default; `net` stays off). Each adds its operations;
  `script` also adds the script module. `marionette_agent::FEATURES` lists what a build has.
- **Person-only file rules move to core** (`marionette_core::person_only`), so every build
  refuses to write license declarations and image approvals whichever features it has.
- **Example**: `examples/rust-module` (a `Spring` component, a `Stage::Logic` system, the
  `spring_set` operation, the `spring-game` binary), built with `script` only. CI tests it on
  its own so features are not unified with the rest of the workspace.

## Alternatives

- A global "install the game's engine" call: fewer signature changes, but two engines in one
  process (tests, a tool hosting two games) would clash. Rejected.
- Dynamic loading (Rust `dylib` or a C ABI plugin): no stable Rust ABI, unsafe loading, and
  hot reload of native code would break determinism guarantees. Rejected for now; TypeScript
  scripts and job TypeScript cover hot reload.
- Making physics, audio, input or animation optional too: they are part of the scene format
  most games use and cost little to build. Left for later if a game needs it.

## Consequences

- A game is a crate depending on `marionette-agent` and `marionette-cli` with the features it
  needs, plus a five-line `main.rs`. Measured on Linux (debug): the example binary links 489
  crates against 559 for the full `marionette` (374 with no optional parts), 103 MB against
  147 MB.
- A workspace-wide build unifies features, so lean builds are checked by
  `cargo clippy -p marionette-agent -p marionette-cli --no-default-features` and
  `cargo test -p marionette-example-rust-module` (CI step "Lean builds").
- CLI integration tests for an optional part carry `#![cfg(feature = "...")]`.
- `marionette docs` writes only the reference files of parts the build has.
