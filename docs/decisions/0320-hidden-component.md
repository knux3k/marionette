# 0320: Hide an entity without removing it

Date: 2026-11-16. Status: accepted. Focuses 1 (modularity), 3 (performance), 4 (graphics).

## Context

Games switch whole parts of a scene off for a while and back on: a space view replaces the
planet below, a menu scene hides the level behind it, a cutscene hides the player's own body.
Unity does this with `SetActive(false)`. The engine had no way to stop drawing an entity
except despawning it or taking its `Model` off, which loses the component's values or makes
every game keep its own copies to put back.

## Options

1. Despawn and respawn. Rebuilding a big generated mesh or a scene part costs a hitch, and
   scene ids and children have to be rebuilt.
2. A `visible` field on `Model`, `Sprite`, lights and emitters. Changes components every saved
   scene already writes, and one switch needs four writes.
3. A marker component `Hidden`: while present, the renderer skips the entity's model, sprite,
   particles and point and spot lights. Nothing else changes.

## Decision

Option 3. `Hidden` is a built-in component with no fields. The extract step checks it per
entity only once some entity in the world has one, so scenes without it pay nothing. It does
not reach children (each child needs its own) and does not touch physics or scripts. Bulk draws
and quad hooks are the game's own and decide for themselves.

## Consequences

- Scenes and operations can set `"Hidden": {}` like any component (`add_component`,
  `remove_component`).
- Test: `crates/marionette-render/tests/hidden.rs`.
