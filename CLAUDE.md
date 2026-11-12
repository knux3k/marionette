# Working rules for Claude (Marionette)

This file and `AGENTS.md` hold the same rules. `docs/DESIGN.md` is the source of truth for
what the engine is and in what order it is built; read it before large changes.

## Build and test

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

All four must pass before every commit. CI runs them on Windows and Linux.

Try the CLI (every command takes `--json` and then prints exactly one JSON object):

```bash
cargo run -p marionette-cli -- schema
cargo run -p marionette-cli -- validate examples/drift.scene.json --json
cargo run -p marionette-cli -- run examples/drift.scene.json --headless --ticks 1000 --seed 1 --json
cargo run -p marionette-cli -- scene fmt examples/drift.scene.json
```

## Layout

- `crates/marionette-core`: ECS world (`bevy_ecs`), schema registry, scene format and validator,
  fixed-step loop, seeded RNG, errors as data. No graphics, no network.
- `crates/marionette-cli`: the `marionette` binary. Thin: parse args, call core, print.
- `examples/`: scene files. They must stay canonical (`marionette scene fmt --check`).
- `docs/decisions/`: one short record per decision (`NNNN-title.md`: context, decision, why,
  consequences). Add one whenever you pick between real alternatives.

## Design principles (short form)

1. Text is the source of truth: scenes are canonical JSON with stable ids.
2. Headless first: every feature works with no window.
3. The engine describes itself: register every component/resource in the `Registry`;
   `marionette schema` must cover it. Derive `Reflect` and add a doc comment, which becomes
   the schema `description`.
4. Deterministic by default (rules below).
5. Errors are data: return `Diagnostic`s with a stable code from `error::codes`, location
   where known, the entity, and a concrete `fix`. Never change what an existing code means.
6. Small, legible crates: keep each crate small enough to read in one sitting.
7. Human in control: never push, publish, upload, delete user data or spend money on an API
   without the project owner's explicit yes.

## Determinism rules

Simulation state must not depend on:

- wall-clock time (no `Instant`, `SystemTime`, frame time in sim code; use `FixedStep`/`SimClock`),
- hash-map or hash-set iteration order (use `BTreeMap`/`BTreeSet`, or sort before iterating),
- thread timing (the sim schedule is single-threaded and `.chain()`ed; systems that conflict
  must have an explicit order),
- ECS query order when it matters (for example when drawing random numbers: sort by `SceneId`),
- platform maths (avoid `sin`, `cos`, `powf` etc. in sim code until a portable math decision exists;
  plain `+ - * /` on `f32` is fine),
- unseeded randomness (only `SimRng`, seeded from `--seed`).

The acceptance tests in `crates/marionette-core/tests/acceptance.rs` check that the example
scene gives the same state hash twice for a seed, a different hash for another seed, and that
load-then-save is byte-identical. Keep them passing; add a similar test for every new system.

## Adding a component

1. Define it with `#[derive(Component, Reflect, ...)]` and a doc comment.
2. Register it in `builtin::register_builtins` (or your crate's equivalent).
3. If it refers to another entity, use `SceneRef` so the validator can check the reference.
4. Add or extend a test; run `marionette schema` to check its JSON Schema.
