---
name: marionette-round-worlds
description: Movement and navigation on round planets, with point gravity, characters whose up follows position (walk, jump, jetpack, swim) and crowd pathfinding on changing terrain. Use when a game has spherical worlds or agents that need paths.
---

# Round worlds: movement and navigation

All opt-in components; set the scene's `PhysicsSettings` gravity to zero when wells are used.

## Gravity and characters

1. On each planet or moon entity (its `Transform` translation is the centre): a
   `GravityWell` (`surface_gravity`, `radius`, `range`; 0 range pulls everywhere). Wells add
   up, so crossing from one body to another just works. Dynamic bodies fall toward them.
2. Optional `SphereFluid` on the same entity: an ocean (`radius`, `buoyancy`, `drag`).
   Optional `GravityZone` (`radius`, `gravity`): uniform gravity inside a sphere, for a flat
   area round a landing site where things stand straight up; wells are ignored inside it.
3. A walking character: `CharacterController` plus `PlanetCharacter` (`align_speed` in rad/s,
   `thrust` for a jetpack with `thrust_action`, `climb_speed` above 0 for a hover pack that
   eases to a steady climb with gravity off, `face_movement` for AI). It walks in its own
   tangent plane, jumps along up, swims inside a fluid. Turn the player by yawing its
   `Transform` rotation about its up; read `PlanetState` (`up`, `gravity`, `swimming`) for the
   camera.
3a. Ground made in code (a game mesh, `mesh:<name>`): give the planet entity a `Collider`
   with `TriMesh { model: "mesh:<name>" }`. The body appears once the mesh is stored and
   follows it when the mesh is stored again after an edit.
4. Scripts or AI steer it with `MoveIntent` (world velocity, `jump`) instead of actions.
5. VR: put `XrCharacter` on the rig with this body (skill `marionette-make-vr`).
6. Moves placed by game code (a climb, a grapple swing, a vault): insert `CharacterPlaced` and
   set the `Transform` each tick; remove it to hand the body back (works on flat worlds too).

## Navigation

1. `NavSurface` on the planet entity: `resolution` cells per cube face edge (cells are about
   radius × 1.6 / resolution metres; 64 to 256 is typical), `max_grade` (1 = 45°), `probe`
   metres above and below `radius` for the ground ray cast (needs colliders; 0 = smooth
   sphere), `sea_level`, `clearance`.
2. Things agents must walk round: `NavObstacle` (`surface`, `radius`). Moving, adding or
   removing one updates only its cells; only agents whose route crosses them plan again.
3. Agents: `NavAgent` (`goal` in world space, `speed`, `acceleration`, `radius`, `height`,
   `arrive_distance`, `drive`). With drive Direct, navigation moves the agent along the ground
   (use it for crowds: no physics body); with drive Intent it writes `MoveIntent` for a
   `PlanetCharacter`. Change `goal` to send an agent elsewhere; `active` false stops it.
4. Read progress from `NavAgentState` (`status`: Waiting, Moving, Partial, Arrived,
   Unreachable).
5. Tune with the `NavSettings` resource (searches per tick, neighbour distance) only when
   measured.

Terrain made in Rust: call `marionette_nav::bake_with` with the height function, and
`marionette_nav::rebake` after an edit.

## Check it

- `nav_agents` with `ticks` set: counts per status; everyone should reach Arrived.
- `nav_path` between two points: corners and `complete`.
- `perf_sim` on the scene before and after adding agents; crowds of thousands should stay
  well under the tick budget.
- Add the scene to the stability suite (`marionette-test-game`).
