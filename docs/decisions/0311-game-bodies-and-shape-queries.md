# 0311: Game-owned bodies and shape queries

Date: 2026-11-16. Status: accepted. Focuses 1 (modularity), 2 (context), 3 (performance).

## Context

Some games keep many objects as plain data, not entities (a factory game's loose items in
one `Vec`, as Unity games pool objects), yet want some of them to be real physics bodies
(thrown, dropped or carried things that hit walls and bounce). Making every such object a
scene entity costs a component set per object and puts game-internal data in the scene.
Games also need overlap and sweep queries with layer masks, and query results must come in a
fixed order to stay deterministic.

## Options

1. Require an entity per physics object. Simple for the engine, heavy for the game, and the
   game's own data and the scene's would both hold the pose.
2. A Rust API for bodies keyed by a game-chosen `u64` (`add_body`, `body_state`,
   `set_body_velocity`, `set_body_pose`, `set_body_gravity_scale`, `push_body`,
   `remove_body`, `body_contacts`), living in the same Rapier world.
3. Let the game build its own Rapier world. Two worlds would not collide with each other.

## Decision

Option 2, in `marionette_physics::game`. Game bodies:

- share the scene's Rapier world, so they hit every scene collider, are pulled by
  `GravityWell`s and zones, and are pushed by character controllers;
- are tagged in Rapier's user data with bit 64 set, so keys never clash with scene ids;
- report contacts to the game (`body_contacts`), not to the event log (which names
  entities only), and are skipped by the script `raycast` (scripts see entities only);
- are saved in snapshots with the rest of the physics world. The game decides when to add
  and remove them (in its own deterministic order) and copies `body_state` into its data.

Queries (`overlap`, `shape_cast`, `raycast_all`, `raycast_first`) take `QueryOptions`
(exclude one target, skip sensors, skip game bodies, layer mask) and return `Target::Entity`
or `Target::Body`, sorted by distance then target. Operations `observe_overlap` and
`observe_shape_cast` expose the first two to agents.

## Consequences

- A game can give physics to a handful of its objects at a time (only those in motion) and
  keep the rest as cheap data, which is what keeps a large factory fast.
- The game owns hashing of what it copies back; the engine hash still covers entities.
