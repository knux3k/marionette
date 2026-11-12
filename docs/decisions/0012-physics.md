# 0012: Physics on Rapier with written-back state

Date: 2026-11-11. Status: accepted (proofs pending, see `docs/ai/STATE.md`).

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

`enhanced-determinism` turns off SIMD and parallelism in Rapier and uses software math. The
frame-time cost has not been measured yet; measure it on the line 6 example scenes.
