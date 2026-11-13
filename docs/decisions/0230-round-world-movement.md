# 0230: Movement on round worlds: gravity wells and planet characters

Date: 2026-11-13. Status: accepted.

## Context

Physics had one uniform gravity vector and a character controller that walks in X/Z with up
fixed at +Y. Games with small planets, moons or ships need gravity toward a centre, characters
whose up follows their position, jumping and jetpacks along that up, swimming in oceans, and
crossing from one body to another. Existing scenes and hashes must not change.

## Decision

- **Opt-in components in physics, in their own file** (`crates/marionette-physics/src/planet.rs`):
  `GravityWell` (surface gravity, radius, range) on a body's entity, `SphereFluid` (radius,
  buoyancy, drag), `PlanetCharacter` next to a `CharacterController`, and the written
  `PlanetState` (gravity, up, swimming). Without them the backend runs exactly as before.
- **Wells add up** (`g · (radius / r)²` outside, linear to zero inside, nothing beyond
  `range`), so moving between bodies needs no switching rule: a character's up turns toward
  minus the summed gravity, at most `align_speed` rad/s.
- **Dynamic bodies** get the well force (times their gravity scale, buoyancy and drag in a
  fluid) as a per-step force; the scene's uniform gravity still applies, so planet scenes set
  it to zero.
- **Planet characters reuse Rapier's controller** with its `up` set per step and the body
  rotated so the collider stands upright; slopes and ground snapping are measured against the
  local up. Walking is in the character's tangent plane; facing is the `Transform` rotation,
  which scripts yaw about up, or `face_movement` turns it toward the walk.
- **`MoveIntent` in core** (world velocity and a jump request) is how navigation, scripts or a
  game system drive a character without depending on physics, and physics reads it without
  depending on navigation.
- **Determinism**: only `+ - * /` and `sqrt` per tick; quaternion helpers moved to
  `marionette_core::vmath` and shared with navigation.

## Alternatives

- Rotating the whole world so the player stays upright: breaks multiplayer, many characters
  and physics bodies on several planets.
- A separate planet controller crate: it needs Rapier's controller and the physics world, so it
  belongs in physics; a separate file and opt-in components keep it apart.
- Switching to the nearest well instead of summing: needs hysteresis rules and gives jumps in
  up at the switch.

## Consequences

- Swimming is simple (buoyancy, drag, thrust or jump to rise); full 3D swimming direction from
  the camera is left to the game through `MoveIntent`.
- Characters on fast-spinning bodies do not inherit the body's motion (no moving platforms yet).
