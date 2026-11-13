# 0211: Networked input is per-player actions `p<slot>.<action>` and a `NetPlayers` resource

Date: 2026-11-13. Status: accepted.

## Context

The input module (plan line 6) keeps one `ActionState` for the whole sim, fed by one action
map. In a networked session every peer's devices drive a different player, and every peer must
apply exactly the same input at exactly the same tick.

## Decision

- Each peer reads its own devices through its local action map, outside the sim, and passes the
  values to its session (`Host::set_local`, `Client::set_local`). The session sends only
  changes, as (action index, exact `f32` bits).
- `apply_frame` turns a frame into direct action events for the sim's current tick:
  player `n`'s `jump` becomes the action `p<n>.jump`. Scripts read `engine.action("p1.jump")`.
  When a player leaves, all of that player's actions are set to 0 at that tick.
- `NetModule` registers the resource `NetPlayers { slots }` (occupied slots, ascending),
  updated by roster events. It is part of the state hash, so peers that disagree about who is
  playing are caught as a desync.
- Snapshots for joins and resyncs are `Sim::save_state` as bytes (`encode_snapshot`,
  `restore_snapshot`), so module state (input queues, physics) travels too.
- Example `examples/coop.scene.json` (script `coop-player.ts`) with stability mode
  `coop-netplay`.

## Alternatives

- **One `ActionState` per player (a component on a player entity)**: cleaner for scripts, but
  changes the input module, the script API and every scene with input. The prefix keeps the
  existing input path and replays unchanged; a later decision can add sugar on top.
- **Send device events instead of actions**: each peer would need every other peer's action
  map and bindings; actions are smaller and map-independent.

## Consequences

- A single-player scene runs unchanged; a scene made for co-op reads `p<slot>.` actions and
  `NetPlayers`.
- Input recordings of a networked session contain the prefixed actions, so a recording replays
  the whole session offline.
