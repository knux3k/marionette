# 0012: Physics on Rapier with written-back state

Date: 2026-11-11. Status: accepted. Proofs and cost measured 2026-11-12.

## Decision

- Rapier 0.36 (`rapier3d`, `rapier2d`) with features `enhanced-determinism` and
  `serde-serialize`. One backend is written once in `crates/marionette-physics/src/backend.rs`
  and expanded for 3D and 2D in `crates/marionette-physics/src/dims.rs`; only conversions differ.
- Physics is on when the scene has the `PhysicsSettings` resource (`mode`: `Dim3` or `Dim2`).
- Bodies are built at load from `RigidBody` + `Collider` + `Transform` in scene-id order. Each
  tick, one exclusive system in `Stage::Physics` does: kinematic targets, character controllers
  (Rapier `KinematicCharacterController`, which pushes dynamic bodies), the step, collision
  events into `EventLog`, then writes `Transform`, `Velocity`, `AngularVelocity` and
  `CharacterState` back to the entities.
- The state hash covers physics through those written-back components. Rapier's internal caches
  (contacts, warm-start impulses, islands) are not hashed but are saved whole (postcard) in
  snapshots, so a restored world continues exactly.
- An entity with a physics body must not have `Position` (error `physics.position_conflict`):
  physics owns its `Transform`.
- 2D uses the XY plane; rotation is about Z, converted from cos/sin with `sqrt` only.

## Why

Rapier is the maintained Rust physics engine with a deterministic mode and serializable state.
Writing results into ordinary components keeps one definition of state (the scene text) for
saving, hashing and observing.

## Cost

`enhanced-determinism` makes Rapier use deterministic software math (Marionette does not turn
on Rapier's SIMD or parallel features, so those are not lost). Measured 2026-11-12 with
`cargo run --release -p marionette-physics --example step_cost -- <boxes>` (a pile of dynamic
boxes falling on a floor, 600 ticks, two runs each, desktop CPU), average ms per tick over the
first 120 ticks while the pile collapses:

| Boxes | With the feature | Without |
| --- | --- | --- |
| 400 | 0.541, 0.546 | 0.538, 0.527 |
| 1000 | 1.803, 1.764 | 1.688, 1.792 |

The difference is inside run-to-run noise (at most about 5 percent), against a frame budget of
11.1 ms. Keep the feature. The example scenes (`examples/playground3d.scene.json`, 5 bodies)
cost far less than either.

## Proofs

`crates/marionette-cli/tests/line6_proofs.rs`: the 3D and 2D examples give the same hash twice
and match pinned hashes; snapshot at tick 500, restore and run to 1,000 equals the
uninterrupted run (twice); replaying a recording gives the original hash. Build errors:
`crates/marionette-physics/tests/build_errors.rs`.

## Character jumps

A jump starts on the tick the jump action is first pressed, only if the character stood on
something at the end of the previous tick. There is no coyote time or jump buffering yet:
walking off an edge and pressing jump one tick later does nothing (`grounded` is false). A
"jump while held" rule was tried and dropped: Rapier's controller can report `grounded` while
the character rises along a moving box, which made a held jump fire on several ticks in a row.
Coyote time and buffering need extra fields in `CharacterState`; add them with the first
platformer proof scene (plan line 9) if it needs them.
