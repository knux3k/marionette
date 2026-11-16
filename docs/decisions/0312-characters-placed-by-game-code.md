# 0312: Characters placed by game code

Date: 2026-11-16. Status: accepted. Focuses 1 (modularity, custom code), 3 (performance).

## Context

Games with movement tricks (climbing, wall-runs, grapple swings, vaults, rail grinds) move the
player themselves for a while and hand the body back afterwards. Unity games do this by
switching the `CharacterController` off and setting the transform. In the engine a
`CharacterController` walks its kinematic body every step and writes the pose back to the
`Transform`, so game code that sets the `Transform` gets the translation honoured for one
step (and then walked and pulled by gravity from there) and loses any rotation it set.

## Options

1. Remove the `CharacterController` while the move runs. The body is built as a character
   when it is added, so it stays one: the step skips it but writes the old rotation back.
2. Give `CharacterController` an `enabled` field. Changes a saved component every scene and
   game already writes.
3. A marker component `CharacterPlaced`: while present, each step moves the body to the
   `Transform` (translation and rotation) and skips walking, gravity and ground checks.

## Decision

Option 3. Adding or removing a component is how scenes already switch behaviour on and off
(`CharacterInput`, `MoveIntent`), nothing saved changes shape, and it costs one component look
up per character per step. `CharacterState` and `PlanetState` keep their last values while it
is on; game code that ends a move in the air sets `CharacterState.vertical_velocity` before
removing it.

## Consequences

- A game's own move code (its queries through `shape_cast` and `raycast_first`, decision
  0311) places the body; the controller takes over again the step after removal.
- The body is moved as a teleport while placed: it does not push dynamic bodies on the way
  (as with Unity's controller switched off).
- Test: `crates/marionette-physics/tests/character_placed.rs`.
