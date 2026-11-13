---
name: marionette-add-rust-code
description: Add a game's own Rust module (components, resources, systems) and its own operations to Marionette without forking or editing the engine, building the game as its own binary. Use when gameplay or simulation needs native speed, when porting Burst jobs, when a game needs a custom operation or MCP tool, or when trimming unused engine parts from a build.
---

# Add a game's own Rust code

A game extends the engine from its own crate. Nothing in the engine is edited. The engine
repository's `rust-module` example (under its examples folder) is the template: copy it.

Prefer a script for per-entity gameplay and a job file for number loops; reach for Rust when
the data needs structs, 64-bit integers, or the last factor of speed.

## Steps

1. Make a crate that depends on `marionette-core`, `marionette-agent` and `marionette-cli`
   with `default-features = false` and only the features the game uses: `script`, `import`,
   `pool`, `generate`, `port`, `jobs-ts`.
2. Components and resources: derive `Component` or `Resource`, plus `Reflect`, `Debug`,
   `Clone`, `PartialEq`, with a doc comment (it becomes the schema description). All fields
   are required in scene files. Entity references use the scene reference type so the
   validator catches dangling ids.
3. Write a module: a unique `name`, `register` for the types, `add_systems` placing each
   system in a stage. State kept outside components needs the module's `save` and `load`.
4. Operations: `marionette_agent::op("group_verb", "description", mutating, handler)`. Input
   and output structs derive the serde traits and `JsonSchema`; inputs deny unknown fields.
   Handlers get `&mut Agent` and return diagnostics with stable codes, never panic on input.
5. Build the engine and the binary:
   `Engine::builder().module(M).ops(ops()).build()?`, then in `main.rs`
   `marionette_cli::main_with(engine)` and the global allocator line from the template.
   The game's binary now has every `marionette` command plus its own operations, over the
   command line and MCP alike.
6. Run `skills_install` from the game's binary so agents working on the game get these
   skills, and add a skill of the game's own for each custom operation group.

## Determinism rules for systems

- No wall-clock time, no thread timing, no unordered-map iteration deciding state
  (use ordered maps).
- Random numbers come from the simulation's seeded generator.
- Parallel work goes through the engine's job system, each chunk writing only its own slice.
- `sin`, `cos`, `pow` from the standard library differ between machines: precompute, or use
  the `libm` crate.

## Memory and crash rules

- Child processes start through `marionette_host::spawn`, temporary files through
  `marionette_host::temp_dir`, so nothing is left behind after a crash.
- Every cache or log that lives as long as the process has a bound.

## Checks

- Test the crate alone (`cargo test -p <crate>`) so its features are not unified with others.
- A test runs a scene twice with the same seed and compares state hashes; `perf_sim` with 1
  and several threads gives the same hash.
- Scenarios cover each operation (skill `marionette-test-game`).
