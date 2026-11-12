# Glossary

- **Tick**: one fixed simulation step (`Sim::step`), length `FixedStep.dt` (default 1/60 s).
  All stages run once per tick, in order. `SimClock.tick` counts them.
- **State hash**: SHA-256 of the canonical scene text of the world plus tick and RNG state
  (`Sim::state_hash`). Same scene, seed, ticks and input must give the same hash.
- **Scene**: a canonical JSON file (`*.scene.json`) listing entities with stable ids and their
  components, plus resources.
- **Canonical form**: the exact text `scene fmt` writes; loading and saving it is byte-identical.
- **Session**: a scene loaded into memory that stays alive between operation calls (MCP server),
  with its own tick and RNG. Edited, stepped, observed, snapshotted, saved.
- **Snapshot**: a complete copy of a session's state (`session_snapshot`), restorable exactly.
- **Operation**: one entry of the shared table in `crates/marionette-agent` (for example
  `entity_create`), callable as `marionette op <name>` and as an MCP tool.
- **Module**: an optional engine part (input, physics, audio) plugged into the simulation through
  `marionette_core::Module`.
- **Action**: a named input value (`move_x`, `jump`) computed per tick from device bindings or set
  directly; stored in `ActionState`.
- **Recording**: the list of action changes of a run (`*.inputs.json`); replaying it reproduces
  the run.
- **Scenario**: a test file (`*.test.json`): scene, seed, ticks, optional input, assertions,
  optional reference image.
- **Reference image**: a committed PNG a render is compared with, within the tolerance of
  decision 0006.
- **Recipe**: the description of how a generated asset was made (inputs, providers, steps); the
  output is cached by content hash so the asset can be rebuilt.
- **Pool**: the public CC0 asset collection in the assets repository.
- **Owned library**: a per-user folder of assets the user owns (for example imported from Unity
  or Unreal), shared across their projects, private, never pooled.
- **Protocol file**: `PROTOCOL.md` in a game project: the project owner's rules every agent reads
  first (template: `docs/PROTOCOL.md`).
