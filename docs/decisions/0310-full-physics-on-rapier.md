# 0310: A full physics feature set on Rapier, behind our own components

Date: 2026-11-16. Status: accepted. Focuses 1 (modularity), 3 (performance); extends 0012.

## Context

Physics since 0012 covered rigid bodies, colliders, character controllers, round-world
gravity (0230, 0233) and a single raycast. A game ported from Unity also uses joints
(Fixed, Hinge, Spring, Character/Configurable), layers and the collision matrix,
continuous collision for fast bodies, `AddForce` in its force modes, and the overlap and
sweep queries (`OverlapBox/Sphere/Capsule`, `BoxCast`, `SphereCast`, `RaycastAll`). The owner
asked for a full physics engine.

## Options

1. Write our own solver. Full control over memory layout and determinism, but stable
   stacking, contact solving, CCD and joints take years to get right, and PhysX is heavily
   tuned, so the speed goal (beat Unity, no quality loss) would be at risk for a long time.
2. Another backend (PhysX through C bindings, Jolt through C++ bindings). Mature, but adds
   FFI `unsafe` and a C++ toolchain to every build, against the code rules.
3. Keep Rapier (pure Rust, deterministic mode, serializable, already measured at about
   1.8 ms per tick for 1,000 settling boxes) and expose the missing features as plain scene
   components and Rust functions, so games never name a Rapier type.

## Decision

Option 3. New components, each optional and read where the body is built or every tick:

- `Joint` (`Fixed`, `Ball`, `Hinge` with limits and motor, `Slider` with limits and motor,
  `Rope`, `Spring`) joins an entity's body to another entity's body or to the world (`other`
  0). Joints wait until both bodies exist, are rebuilt when the component changes and go
  away with either body. Motors use a fixed gain (`MOTOR_GAIN`) capped by `motor_force`.
- `CollisionLayers` (32 layer bits, member and collides-with; Rapier interaction groups).
  A character controller on layers moves through what its layers do not touch.
- `ContinuousCollision` (Rapier CCD on a dynamic body).
- `ExternalForce` (every tick while present; a removed force stops at the next step) and
  `ExternalImpulse` (applied once, then physics sets it back to zero).

Layers and CCD are read when the body is built (as Unity's are fixed per collider in
practice); to change them, rebuild the entity.

Queries and game-owned bodies are decision 0311.

## Consequences

- Existing scenes are unchanged bit for bit: nothing new runs unless a scene uses a new
  component (the drift and physics example hashes are unchanged).
- Snapshots append the joint and game-body maps after the Rapier world, so older snapshots
  still load.
- If profiling ever shows a Rapier hot spot, that part can be replaced behind these
  components without touching games.
