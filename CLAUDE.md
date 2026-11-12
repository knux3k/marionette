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
cargo run -p marionette-cli -- screenshot examples/showcase.scene.json --ticks 60 --seed 1 --size 1280x720 --out shot.png --json
cargo run -p marionette-cli -- bench examples/showcase.scene.json --frames 1000 --size 1920x1080 --json
cargo run -p marionette-cli -- run examples/sprites.scene.json   # opens a window; Escape closes
```

GPU tests skip when no GPU adapter exists; set `MARIONETTE_REQUIRE_GPU=1` to make that a
failure (CI does). Use `--test-threads=1` on software rasterizers (WARP crashed with
concurrent devices). `MARIONETTE_UPDATE_REFERENCES=1` rewrites the reference images: only for
an intended visual change, after looking at the new images.

Keep the build cache small: build with `CARGO_INCREMENTAL=0`, use `--release` only for
benchmarks, and run `cargo clean` when a session ends.

## Layout

- `crates/marionette-core`: ECS world (`bevy_ecs`), schema registry, scene format and validator,
  fixed-step loop, seeded RNG, errors as data. No graphics, no network.
- `crates/marionette-render`: wgpu renderer (PBR models, batched sprites), glTF/PNG loading,
  offscreen capture, image comparison, the window. Reads the world through `extract(&World)`
  and never writes simulation state.
- `crates/marionette-agent`: the operation set (project, edit, session, observe, test) and
  the MCP server. The CLI and MCP both call `Agent::call`; there is no second implementation.
- `crates/marionette-cli`: the `marionette` binary. Thin: parse args, call operations, print.
- `examples/`: scene files (canonical: `marionette scene fmt --check`), `assets/` (only CC0
  or self-made, each listed with its source in `assets/LICENSE.md`) and `reference/` images.
- `docs/decisions/`: one short record per decision (`NNNN-title.md`: context, decision, why,
  consequences). Add one whenever you pick between real alternatives.

## The working loop (for agents using the engine)

Every engine action is an operation in `marionette-agent`, reachable two ways that run the same
code: `marionette op <name> '<json>' --json` (list them with `marionette ops`) and the MCP server
`marionette mcp` (one tool per operation). In a game project:

1. Read the project's `PROTOCOL.md` first and follow it (what to ask before building, what
   needs approval, who to hand work to). `project_init` puts one in every new project; the
   template is `docs/PROTOCOL.md`.
2. Validate: `project_validate`.
3. Edit through operations only (`scene_create`, `entity_create`, `entity_update`,
   `entity_delete`, `component_set`, `component_remove`). Edits are validated before anything is
   written; a refused edit changes nothing and returns diagnostics with a JSON pointer.
4. Run: `scene_run` (one-off hash) or `session_start` / `session_step` (world kept between
   calls). Session edits change only the live world; `session_save` writes it to a file
   (docs/decisions/0009).
5. Observe: `observe_tree`, `observe_query`, `observe_entity`, `observe_screenshot` (returns a
   PNG path), `observe_logs`, `observe_stats`. Lists are paged.
6. Test: scenarios in `tests/*.test.json` (scene, seed, ticks, assertions, optional reference
   image), run with `test_run`. Add one for every behavior.

When adding an operation: write typed input/output structs (`JsonSchema`), register it in its
module's `ops()`, mark it mutating if it changes files or sessions, return `Errors` (never
panic), and extend `crates/marionette-cli/tests/agent_e2e.rs`.

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
