# 0191: An XR rig can ride a character body

Date: 2026-11-15. Status: accepted.

## Context

`XrLocomotion` (0190) moves the play space itself: stick movement slides it over the world's
XZ plane, turns are about world +Y and teleport sets it on a ray hit. Nothing collides, gravity
does not apply and up is always +Y. A game whose player walks on round worlds (`PlanetCharacter`,
0230) or that wants walls and slopes to stop a VR player as they stop a desktop player needs
the play space to ride the same character body physics already moves.

## Decision

- **`XrCharacter { body, follow_head }` on the rig.** With it, the rig's `XrLocomotion` drives
  the body instead of the play space:
  - stick movement becomes a `MoveIntent` velocity along the body's ground (its
    `PlanetState.up`, else its own +Y), aimed by the locomotion's `direction` source;
  - `follow_head`: the head's step away from the body in the room is added to the same intent,
    so the body steps under the head (room-scale walking) and physics stops it at walls;
  - turns (snap or smooth) rotate the rig about the body's up through the head;
  - teleport moves the body to the aimed point (feet on it) and the rig with it;
  - the body faces where the head looks (its rotation, used by nothing that moves it).
- **The rig follows after physics** (`xr_follow`, `Stage::Effects`): it moves by what the body
  walked, minus the share of the room-scale step physics allowed (a blocked step leaves the
  head where it is), turns about the head onto the body's new up, and puts its floor at the
  body's feet (from the capsule, ball or box collider). Tracked entities and held objects
  without a dynamic body are placed again from the moved rig, so the frame shows this tick.
- **`XrSystems { Tick, Follow }`** is public so a game's own systems (hands that read grip
  poses, a pointer for world UI) can run after the rig has followed its body.
- **Flat character controllers read `MoveIntent` too** (it was planet characters only): a
  world velocity capped at the controller's speed, replacing the walk actions; a `jump` in it
  jumps. Jumping and jetpacks otherwise stay the body's own actions.

## Alternatives

- Make the body a child of the rig: the rig would go through walls and gravity would need a
  second path. Physics already owns bodies; the rig follows instead.
- Teleport the body under the head every tick (no collisions): leaning into a wall would put
  the capsule inside it, where the character controller cannot get out.
- Give `XrLocomotion` a `body` field: every scene listing `XrLocomotion` would have to change;
  a separate component keeps them as they are.

## Consequences

- Round-world VR (and flat VR with walls) needs only scene data: a body with
  `CharacterController` (+ `PlanetCharacter`), a rig with `XrCharacter`.
- The share-out of blocked movement is proportional, not exact: partly blocked diagonal moves
  may leave the head a few millimetres off the body; the next tick's room-scale step closes it.
- Tests: `crates/marionette-cli/tests/xr.rs` (`a_rig_riding_a_body_*`,
  `walking_in_the_room_moves_the_body_under_the_head`, `riding_replays_bit_for_bit`).
