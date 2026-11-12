# 0024: Factory simulation module

Date: 2026-11-12. Status: accepted.

## Decision

- `crates/marionette-factory` simulates machines and belts as flat arrays, not one entity per
  item. Authoring components: `FactoryMachine` (recipe, buffer), `FactoryBelt` (from, to,
  slots, step in milliticks); resource `FactoryRecipes` (recipes, item looks).
- Runtime state is the registered resource `FactoryState` (machine rows, belt rows, slots,
  counters), so it is hashed, saved and snapshotted like any other state. A layout fingerprint
  (FNV over the authoring data) decides whether a loaded state is kept or rebuilt.
- Time is integer milliticks (1000 per tick); no floats in the simulation.
- One tick, three phases: machines craft (parallel chunks), belts advance downstream-first
  (parallel per belt line, `Jobs::for_each_segment_mut`), hand-offs between belts and machines
  (sequential, in belt order). Full belts and buffers stop upstream work (back-pressure).
- Items are drawn through a bulk draw hook, interpolated between ticks.
- Errors: `factory.unknown_recipe`, `factory.bad_recipe`, `factory.bad_belt`.

## Why

A VR factory game needs thousands of moving items per tick inside a 2 ms budget. Flat arrays
and integer time keep the tick cache-friendly and the hash identical on every machine; the
design mirrors how such games are usually built, which also makes porting one straightforward.

## Consequences

- Power, fluids, logic networks and trains are not modelled yet; they would be further phases
  over the same arrays.
- Tests: `crates/marionette-factory/tests/factory.rs` (flow, back-pressure, thread-independent
  hash, snapshot/restore, scene round trip, layout change, error codes).
