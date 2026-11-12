# 0021: Scripts spawn and destroy entities and set timers; all of it is saved state

Date: 2026-11-12. Status: accepted. Extends decision 0022 (script API) and closes the
spawning, `OnDestroy` and timer gaps that decision 0020 (porting) listed.

## Decision

Scripts can add and remove entities and schedule calls. Every piece of this lives in
components and resources, so the state hash covers it and snapshots restore it exactly.

- **Spawning.** `engine.spawn(components, name?)` and `engine.clone(source, overrides?,
  name?)` add a scene entity during the script call and return it, so the script can keep
  changing it. Every component is checked against the registry before anything is spawned.
  A clone copies every registered component except `ScriptState`, `ScriptTimers` and
  `DestroyAt`, so its script starts fresh. Its script's first turn (`start`, then `update`) is
  on the next tick (the tick's work list is fixed when the stage starts, like Unity running
  `Start` before the object's first `Update`). Physics gives new bodies to new entities in the
  same tick's physics step (`sync` in `crates/marionette-physics/src/backend.rs`). A script
  that only spawned entities use is loaded at the end of the stage; if it fails, one
  `script_error` event is recorded and its entities do nothing until the next reload.
- **Ids.** New ids come from the `SpawnIds { next }` resource and are never reused, also
  after an entity from the scene file is destroyed. An id kept in `self.state` can never point
  at a newer entity. `SpawnIds` appears only once a script spawns or destroys something, so
  scenes that never do keep their hashes.
- **Destroying.** `engine.destroy(entity, seconds?)` adds `DestroyAt { tick }`; the earliest
  removal wins. At the end of every script stage, entities whose tick has come get
  `onDestroy(self)` (id order), are removed, and a `destroyed` event is recorded. An
  `onDestroy` may destroy more entities; they go in the next round, at most 64 rounds per tick,
  then the next tick. Removal waits for the end of the stage, like Unity's end-of-frame
  `Destroy`: every entity still gets its turn in that tick. `DestroyAt` also works from scene
  files (a fixed lifetime) without any script. Physics removes the body in the same tick.
- **Timers.** `engine.after(seconds, fn, arg?)`, `engine.afterTicks(n, fn, arg?)`,
  `engine.every(seconds, fn, arg?, firstSeconds?)`, `engine.cancel(handle | fn)` and
  `engine.pending(...)`. A timer belongs to the entity whose hook set it. `fn` must be an
  exported function of that script (passed as the function or its name), so a timer is plain
  data: `{ id, at, every, call, arg }` in the `ScriptTimers` component (JSON text), with `arg`
  stored as JSON. Seconds become whole ticks with `Math.round(seconds / dt)`, at least 1. Due
  timers fire in (tick, id) order in the entity's turn, after the contact hooks and before
  `update`. A one-shot timer is removed before it is called, so a callback that throws is
  reported once and not called again.

## Alternatives not taken

- **Generator coroutines** (`function*` with `yield engine.wait(1)`). Rejected: a suspended
  generator lives inside the JavaScript engine and cannot be written into a snapshot or the
  state hash. Snapshot and restore would silently change behaviour, which breaks the
  determinism rules. Re-running the generator to the saved point on restore cannot work either:
  its side effects would run twice and its reads would see the current world. Coroutines are
  ported as exported step functions chained with `engine.after` (`docs/ai/PORTING.md`).
- **Closures as timer callbacks** (`engine.after(1, () => ...)`). Rejected for the same
  reason: a closure cannot be saved. A name plus a JSON argument can be saved.
- **Immediate removal** inside `engine.destroy`. Rejected: entities later in the id order
  would see the world change halfway through a tick in a way that depends on id order, and
  `onDestroy` would run in the middle of another script's call.
- **Reusing the lowest free id or `max + 1`.** Rejected: it lets a stale id in `self.state`
  point at a new entity.
- **Prefab files** (`engine.spawn("bullet.prefab.json")`). Deferred: cloning a template entity
  or spawning from a components object in the script covers porting today. Prefab files can
  be added later without changing anything here.

## Proofs

- `crates/marionette-script/tests/lifecycle.rs`: spawn timing, `onDestroy`, ids never reused,
  replay gives the same hash and a different seed gives a different one, snapshot with pending
  timers and removals continues exactly, timer order, cancel and pending, timer errors, clone
  and lazy script loading, `onDestroy` chains in one tick.
- `crates/marionette-cli/tests/script_e2e.rs`
  (`spawned_crates_get_bodies_land_and_leave_physics_when_destroyed`): `examples/spawner.scene.json`
  gives the recorded hash twice; spawned crates get physics bodies and land; after they are
  destroyed nothing is left and rays hit the ground; snapshot and restore in a live session
  continue exactly.
- Existing hashes are unchanged (drift, playgrounds, scripted example).
