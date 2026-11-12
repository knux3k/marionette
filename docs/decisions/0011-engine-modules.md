# 0011: Engine modules and fixed stages

Date: 2026-11-11. Status: accepted.

## Decision

- `marionette-core` defines a `Module` trait (`crates/marionette-core/src/module.rs`): `register`
  (types), `add_systems` (into a `Stage`), `init` (build runtime state after a scene loads, also
  after reload and restore), `save` / `load` (opaque bytes for snapshots).
- Every tick runs the stages in order, single-threaded: `Input`, `Logic` (built-in systems),
  `Physics`, `Effects` (sounds), `End` (tick counter).
- `Sim::with_modules` builds the registry from the modules (`registry_with`); `Sim::from_text`
  keeps the module-free core for core tests.
- The set used by the CLI and MCP is `marionette_agent::engine::modules()`: input, physics, audio.
- `SceneDir` (scene file directory) and `EventLog` (collisions, jumps, sounds) are core resources
  that are not part of the scene text or the state hash.
- `Sim::save_state` / `Sim::restore` give exact snapshots: canonical scene text, tick, RNG state
  and each module's bytes.
- CPU asset loaders moved to `crates/marionette-assets` so physics can build mesh colliders
  without linking the GPU renderer; `marionette_render::assets` re-exports it.

## Why

Optional parts must not make the core depend on Rapier, kira or wgpu, and each part must be
able to save the state its components do not capture. Fixed stages give one documented order
instead of ordering edges between systems in different crates.

## Consequences

A module that keeps hidden state must implement `save` and `load`, or snapshots will not restore
exactly.
