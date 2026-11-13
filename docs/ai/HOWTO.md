# How-to recipes

Run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` and `cargo test` after each.

## Add a component (or resource)

1. Define it with `#[derive(Component, Reflect, Debug, Clone, PartialEq)]` (resources:
   `Resource`; `Clone` is required, saves copy values with it, decision 0073) and a doc comment; the comment becomes the schema description. Core data goes in
   `crates/marionette-core/src/` (for example `spatial.rs`); module data goes in that module's
   crate (for example `crates/marionette-physics/src/components.rs`).
2. Register it: core types in `register_builtins` (`crates/marionette-core/src/builtin.rs`),
   module types in the module's `Module::register`.
3. All fields are required in scene files; keep field names short and units in the doc comment.
4. A reference to another entity uses `SceneRef` so the validator reports dangling ids.
5. Run `cargo run -q -p marionette-cli -- docs` and commit `docs/ai/reference/components.md`.

## Add a simulation system

1. Write it in the owning crate. Follow the determinism list in ARCHITECTURE.md.
2. Add it to a stage: built-ins in `add_builtin_systems` (`crates/marionette-core/src/builtin.rs`),
   module systems in `Module::add_systems` with `.in_set(Stage::...)`.
3. If it keeps state outside components, implement `Module::save` and `Module::load`.
4. Add a test that runs a scene twice with the same seed and compares `Sim::state_hash`.

## Add an operation

1. Pick the module in `crates/marionette-agent/src/` (`project.rs`, `edit.rs`, `session.rs`,
   `observe.rs`, `scenario.rs`).
2. Write input and output structs deriving `Deserialize`/`Serialize` and `JsonSchema`, with doc
   comments; add `#[serde(deny_unknown_fields)]` on inputs without flattened fields.
3. Write `fn name(agent: &mut Agent, input: In) -> Result<Out, Errors>`; return diagnostics with
   stable codes, never panic on input.
4. Register it in that module's `ops()` with `op("group_verb", "description", mutating, name)`.
   Mutating means it writes files or changes a session.
5. Extend `crates/marionette-cli/tests/agent_e2e.rs` (CLI and, if relevant, MCP).
6. Run `cargo run -q -p marionette-cli -- docs` and commit `docs/ai/reference/`.

## Add a game's own Rust module or operation (no engine edit)

Template: `examples/rust-module` (decision 0062).

1. Make a crate for the game depending on `marionette-core`, `marionette-agent` and
   `marionette-cli` (`default-features = false`, `features = [...]` for the parts it uses:
   `script`, `import`, `pool`, `generate`, `port`, `jobs-ts`, `save`).
2. Write a `Module` (unique `name`, `register`, `add_systems` in a `Stage`) and operations
   with `marionette_agent::op("group_verb", "...", mutating, handler)`. Handlers get
   `&mut Agent`: `session_sim`, `load_sim`, `write_file`, `resolve`, `call`.
3. `Engine::builder().module(M).ops(ops()).build()?`, then in `main.rs`
   `marionette_cli::main_with(engine)` plus the `#[global_allocator]` line from
   `crates/marionette-cli/src/main.rs`. Tests use `Agent::with_engine` or `Engine::load_file`.
4. Test the crate alone (`cargo test -p <crate>`) so its features are not unified with the
   workspace's.

## Add save games to a game

Decision 0076; code `crates/marionette-save`.

1. Add resource `SaveFormat` to the game's scene: `game` (id), `version` (start at 1),
   `autosave_every` (ticks, 0 for requests only), `autosave_slots`, `migrations` (file
   relative to the scene, or `""`). Everything the player keeps must be in registered
   components or resources; anything else is lost on load.
2. Ask for saves at safe moments from scripts: `engine.emit("save_request")` (autosave slot)
   or `engine.emit("save_request", { slot: "checkpoint", label: "Gate" })`.
3. Changing saved data: raise `version` and add a step `{ "from": <old>, "changes": [...] }`
   to the migrations file (`rename_type`, `remove_type`, `rename_field`, `set_field`,
   `remove_field`), or a Rust step with `SaveModule::with(Migrations::new().rust(old, f))`
   and `Engine::builder().replace(...)`. Test it: keep an old save under `tests/` and run
   `save_check`, then `save_load`.
4. Agents: `save_write`, `save_load`, `save_list`, `save_check`, `save_delete` on a live
   session. Windowed: `marionette run <scene> --saves <dir>` (autosave, F5, F9). A game's
   own binary: `SaveWriter::new(SaveStore::open(dir)?, registry)` plus
   `Autosaver::after_step` after each step, and `marionette_save::load` to load.
## Network a game (co-op lockstep, Steam, achievements)

Decisions 0210 to 0213; skill `marionette-multiplayer`.

1. Scripts read per-player actions `p<slot>.<action>` and the `NetPlayers` resource
   (`examples/scripts/coop-player.ts`). Prove the scene with
   `marionette op net_simulate '{"scene":"...","players":4,"late_join_at":200}'`:
   `all_equal` must be true.
2. In the game's binary add `marionette_net::NetModule` to the engine, pick a transport
   (`Endpoint::new(UdpDatagram::bind(..))`, or `marionette_steam::Steam::load(None)` then
   `.transport(2)`), gather players with a `Lobbies` backend, then each fixed step:
   host `poll`, `set_local`, `next_frame`, `apply_frame` + `Sim::step`, hash when
   `should_hash`, `provide_snapshot` when `needs_snapshot`, `flush`; client `poll`,
   `set_local`, run `frames_due` frames, `flush`.
3. Achievements: a `*.achievements.json` (check with `achievements_check`), a `Tracker` over
   `OfflinePlatform` or `Steam::platform()`. Cloud saves: `sync_dir(save_folder, &[".save"], ..)`
   at start and after saving, or Steam Auto-Cloud.
4. Steam needs the game's own `steam_api` library next to the executable (never in the engine
   repository) and `steam_appid.txt` during development.

## Add a game mode, example or proof scene

1. Add the scene (and its action map and input file) under `examples/` or the proof folder.
2. Add a mode to `stability/suite.json`: name, description, kind (`sim`, `render` or `agent`),
   scene, `random_input` if it has an action map, `fast_ticks`, `long_ticks`, budgets.
3. Run `cargo run --release -q -p marionette-cli -- stability --mode <name>` and
   `--long --seeds 1`; fix every failure (STABILITY.md, "Fixing a failure").
4. `cargo test -p marionette-cli --test stability` checks every scene has a mode.

## Add a crate

1. Create `crates/marionette-<name>/` with `Cargo.toml` using `version.workspace`,
   `edition.workspace`, `license.workspace`, `rust-version.workspace`.
2. Add it to `members` in the root `Cargo.toml`.
3. If it adds simulation behavior, implement `marionette_core::Module` and add it to
   `EngineBuilder::standard` (`crates/marionette-agent/src/engine.rs`). If games may leave it
   out, make it a cargo feature of `marionette-agent` and `marionette-cli` (decision 0062).
4. Add it to the crate map in ARCHITECTURE.md with its forbidden dependencies.
5. Record why in a decision.

## Start a child process or write a temporary file

Rules: `docs/ai/MEMORY-AND-CRASHES.md`.

1. Add `marionette-host.workspace = true` to the crate's dependencies.
2. Child process: build a `std::process::Command`, then `marionette_host::spawn(cmd)` (never
   `cmd.spawn()`, `output()` or `status()`; wait on the returned `Child` yourself, or use
   `marionette_host::output(cmd)` to run it to completion). It dies
   with the engine on Windows and Linux, even on a kill.
3. Temporary files: `let dir = marionette_host::temp_dir("what-for")?;` and write under
   `dir.path()`. The folder is removed when `dir` drops, and by the next engine start if the
   process dies first. Never write straight into `std::env::temp_dir()` outside tests.
4. A new cache or log that lives as long as a session or the process needs a bound; add a
   round of the work that fills it to `crates/marionette-agent/tests/leaks.rs`.

## Add an example scene with a reference image

1. Write the scene as JSON under `examples/`, then canonicalize:
   `cargo run -q -p marionette-cli -- scene fmt examples/<name>.scene.json`.
2. Use only CC0 or self-made assets; list each file in `examples/assets/LICENSE.md`.
3. Add the name to the lists in `crates/marionette-render/tests/render.rs`
   (`same_inputs_give_identical_images`, `matches_reference_images`).
4. Create the reference: `MARIONETTE_UPDATE_REFERENCES=1 cargo test -p marionette-render --test render matches_reference`,
   look at `examples/reference/<name>.png`, and check that `git status` shows no other changed
   reference.
5. Add the name to `every_example_is_valid_and_canonical` in `crates/marionette-cli/tests/cli.rs`.
6. Record the reference hash in PROOFS.md.

## Add a game script

1. Write `<name>.ts` next to the scene (or in a subfolder): an ES module exporting
   `update(self: Entity): void` and optionally `start`, `onCollisionEnter`, `onCollisionExit`,
   `onTriggerEnter`, `onTriggerExit`. Example: `examples/scripts/bouncer.ts`.
2. Attach it: `component_set` with `Script` `{ "path": "<name>.ts" }` (relative to the scene).
3. Keep state in `self.state` or components, never in module-level variables (they reset on
   reload and restore). Use `engine.rand()`, never `Math.random` (it does not exist).
4. Optional type check: `marionette op script_types '{}'` writes `marionette.d.ts`, then
   `tsc --noEmit --strict --target es2023 --lib es2023 --module esnext marionette.d.ts <name>.ts`.
5. In a live session, `script_reload` swaps in the edited file; load errors come back as
   `script.*` diagnostics with line and column, runtime throws as `script_error` events.
6. Add a test that runs the scene twice with the same seed and compares the state hash.
## Add a job (hot loop in TypeScript)

1. Write `name.job.ts` with exported functions over `Float32Array`/`Float64Array`/`Int32Array`
   parameters (rules: `docs/ai/JOBS-TS.md`).
2. `marionette op jobs_check '{"path":"name.job.ts"}'` until `ok` is true; apply each `fix`.
3. Try it on data: `marionette op jobs_run '{"path":"name.job.ts","function":"f","args":[...]}'`.
4. From Rust: `JobProgram::load(path)?.run(&jobs, "f", len, chunk, &mut [Arg::F32(&mut a), ...])`,
   or keep a `JobLibrary` for hot reload.
5. Test it: same output with `Jobs::inline()` and `Jobs::with_threads(n)`.

## Add an AI provider

1. Add `crates/marionette-providers/src/<name>.rs` with a unit struct implementing `Provider`:
   a `static ProviderInfo` (name, kinds, `key_env`, `network`, a default model for every kind
   offered, `verified: false` until a live call has checked the format), `estimate_usd` (an
   upper bound; unknown models at the most expensive known price) and `run`.
2. Network plug-ins build `HttpRequest` values (keys only through `secret_header`), send them
   with `net::send` and parse with `net::json` / `net::str_at`; job APIs use `net::poll` with
   `ctx.max_polls` and `ctx.wait`. Never read the environment or the clock inside `run`.
3. Add it to `providers::all()` in `crates/marionette-providers/src/lib.rs` (the order is the
   order `provider_list` shows).
4. Test it in `crates/marionette-providers/tests/providers.rs` with the `Scripted` transport:
   request shape, success, job failure, bad answers, and that the key never appears in errors
   or debug output. Update the name list in `crates/marionette-cli/tests/providers_e2e.rs`.
5. Run `cargo run -q -p marionette-cli -- docs`; record the formats and prices in a decision.

## Add a decision record

1. Next number in `docs/decisions/` (four digits), file `NNNN-short-title.md`.
2. Sections: title line `# NNNN: Title`, `Date`, `Status`, `Decision`, `Why`, and optional
   `Consequences`, `Rejected`, `Not done yet`.
3. Add one line to DECISIONS.md.

## Add a test scenario (game project)

1. In a project created by `project_init`, add `tests/<name>.test.json`: `scene`, `seed`,
   `ticks`, optional `input` (an `*.inputs.json` file), `assertions` (`field`, `count`,
   `state_hash`, `event`), optional `image` reference.
2. Run `marionette op test_run '{}' --json`; results are data (`passed`, `failed`, per assertion).
3. Format: the module docs at the top of `crates/marionette-agent/src/scenario.rs` and the
   `test_run` entry in `docs/ai/reference/operations.md`.
