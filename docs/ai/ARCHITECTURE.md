# Architecture

## Crate map

| Crate | Responsibility | Depends on | Must NOT depend on |
| --- | --- | --- | --- |
| `crates/marionette-core` | ECS world (`bevy_ecs`), type registry, JSON Schema, scene format and validator, fixed-step `Sim`, seeded RNG, errors as data, module trait and stages, data components (spatial, UI, actions) | bevy_ecs, bevy_reflect, serde, serde_json, sha2 | any other marionette crate, wgpu, winit, rapier, kira, network |
| `crates/marionette-assets` | CPU assets: glTF/GLB models, PNG, primitive shapes, built-in images and font | glam, gltf, png | wgpu, winit, any marionette crate |
| `crates/marionette-render` | wgpu renderer (PBR, sprites, UI), offscreen capture, image diff, the window | core, assets, wgpu, winit, glam, bytemuck, pollster | physics, input, audio, agent |
| `crates/marionette-input` | Action maps, input event queue, recording, `Stage::Input` system | core | render, winit, physics, audio |
| `crates/marionette-physics` | Rapier 3D/2D bodies from components, character controller, events, raycast, snapshots | core, assets, rapier3d, rapier2d, postcard | render, input, audio |
| `crates/marionette-audio` | Sound triggers, headless sound log, kira player | core, kira | render, input, physics |
| `crates/marionette-import` | Owned library, license declarations and records, Unity project and `.unitypackage` import, pool admission gate | core, assets, flate2, image | render, input, physics, audio |
| `crates/marionette-agent` | The operation table (project, edit, session, observe, test), engine module set, generated docs, MCP server | all crates above, rmcp, tokio, schemars | clap |
| `crates/marionette-cli` | The `marionette` binary: argument parsing and printing only | agent, render, core, clap | anything new: add logic to an operation instead |

Planned crates (not created yet): marionette-script (line 4), marionette-providers (line 7),
marionette-ai-runtime (line 10). `marionette-assets` will grow the asset pipeline (line 8).

## One tick

`Sim::step` (`crates/marionette-core/src/sim.rs`) runs one `Schedule` on one thread
(`SingleThreadedExecutor`), stages in this order (`crates/marionette-core/src/module.rs`):

1. `Stage::Input`: `apply_input` (`crates/marionette-input/src/lib.rs`) takes this tick's queued
   events, updates `DeviceState`, recomputes `ActionState` from the action map, records changes.
2. `Stage::Logic`: `random_impulse`, `follow`, `integrate` (`crates/marionette-core/src/builtin.rs`).
3. `Stage::Physics`: `physics_tick` (`crates/marionette-physics/src/lib.rs`): kinematic targets,
   character controllers, Rapier step, collision events, write-back to components.
4. `Stage::Effects`: `play_sounds` (`crates/marionette-audio/src/lib.rs`): sound events.
5. `Stage::End`: `advance_clock` increments `SimClock.tick`.

`dt` comes from the `FixedStep` resource (default 1/60). Nothing reads the wall clock.

## Schema, loader and saver from one source

- Types are registered in `Registry` (`crates/marionette-core/src/registry.rs`) by short type
  name; each entry keeps `bevy_reflect` `TypeInfo` and type-erased `from_reflect`, `insert`,
  `get` functions.
- JSON Schema: `crates/marionette-core/src/schema.rs` walks `TypeInfo`; doc comments become
  `description`.
- Loading: `crates/marionette-core/src/scene.rs` parses JSON, validates each value through
  `TypedReflectDeserializer`, reports problems with exact positions (`json_span.rs`) and JSON
  pointers, then inserts.
- Saving: `extract` reads every registered type back through `TypedReflectSerializer`;
  `Scene::to_text` writes canonical JSON (sorted keys, entities by id, shortest floats, LF).
- Engine-wide registry: `marionette_agent::engine::registry()` = built-ins + input, physics,
  audio types. Generated reference: `docs/ai/reference/components.md`.

## Operations: one table for CLI and MCP

- `crates/marionette-agent/src/lib.rs`: `op(name, description, mutating, fn)` turns a typed
  function into a table entry with schemas from `schemars`. Modules: `project.rs`, `edit.rs`,
  `session.rs`, `observe.rs`, `scenario.rs`.
- CLI: `marionette op <name> '<json>'` and shortcuts in `crates/marionette-cli/src/main.rs`.
- MCP: `crates/marionette-agent/src/mcp.rs` (rmcp, stdio), one tool per operation.
- Read-only mode refuses mutating operations (`agent.read_only`). Generated reference:
  `docs/ai/reference/operations.md`.

## Live sessions and scene files

A session (`session_start`) is an in-memory copy of a scene with its own tick and RNG, alive as
long as the process holding it (the MCP server). Edits name exactly one target: `scene` (file:
validated, written atomically, canonical) or `session` (live world: validated, reloaded keeping
tick and RNG). File edits never touch sessions; session edits never touch files;
`session_save` writes a session to a file. Snapshots (`session_snapshot`, `session_restore`) use
`Sim::save_state` / `Sim::restore`. See decision 0009.

## Renderer

`crates/marionette-render/src/renderer.rs`, one render pass per view:

1. PBR models. Bind groups: 0 view (view-projection, eye), 1 lights, 2 material textures,
   3 per-draw uniform (model and normal matrix, material factors) at dynamic offsets.
2. Sprites, same view, alpha blended, depth-tested without depth writes.
3. UI in screen pixels (separate view buffer), no depth test.

A `View` is a value passed to `Renderer::render`, not renderer state, so stereo (OpenXR) can
render two views without a rewrite. `extract(&World)` (`extract.rs`) copies what to draw, so
rendering cannot change simulation state. Offscreen capture: `gpu.rs`; window: `window.rs`.

## Determinism

Forbidden in simulation code (systems and anything they call):
- wall-clock time (`Instant`, `SystemTime`);
- iterating `HashMap` / `HashSet` (use `BTreeMap` / `BTreeSet`, or sort);
- multithreaded execution or thread timing;
- relying on ECS query order where order matters (sort by `SceneId`);
- `sin`, `cos`, `powf`, `exp`, `atan2` and other platform math in per-tick code (plain
  `+ - * /` and `sqrt` are fine; physics 2D converts angles with `atan2` only at load);
- unseeded randomness (only `SimRng`, seeded from `--seed` / `seed`);
- reading files during a tick (load in `Module::init`).

Rapier runs with `enhanced-determinism`. Test: same scene, seed, ticks and input give the same
hash (`crates/marionette-core/tests/acceptance.rs`, `crates/marionette-cli/tests/agent_e2e.rs`).

## What feeds the state hash

`Sim::state_hash` = SHA-256 of the canonical scene text of the current world (every registered
component on entities with a `SceneId`, every registered resource present, including
`ActionState`, physics-written `Transform` / `Velocity` / `AngularVelocity` / `CharacterState`)
followed by `tick=<n>` and `rng=<state>`.

Not in the hash: `EventLog` (collisions, jumps, sounds), `SceneDir`, input `DeviceState`,
`InputQueue`, `InputRecording`, Rapier internals (`Physics` resource), session logs and stats.
Rapier internals and input state are carried by snapshots instead.
