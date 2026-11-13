---
name: marionette-write-script
description: Write or fix gameplay logic as a deterministic TypeScript script on an entity (hooks, engine API, timers, spawning, input actions). Use when adding behaviour, handling collisions or triggers, reading input, or debugging a script error or a state hash that changes between runs.
---

# Write a game script

A script is an ES module next to the scene, attached with a `Script` component
(`component_set` with `{"path": "<name>.ts"}`, relative to the scene file). It runs once per
fixed tick. Run `marionette op script_types '{}'` once to write `marionette.d.ts`: read that
file for the exact API instead of guessing.

```ts
export function start(self: Entity): void {
  self.state.speed = 2;
}
export function update(self: Entity): void {
  const t = self.get("Transform");
  if (!t) return;
  t.translation.x += engine.action("move_x") * self.state.speed * engine.dt;
  self.set("Transform", t);
}
```

Optional hooks: `onCollisionEnter`, `onCollisionExit`, `onTriggerEnter`, `onTriggerExit`
(each `(self, other)`), `onDestroy`. A trigger is a `Collider` with `sensor: true`.

## Rules

- State lives in `self.state` or components, never in module-level variables (those reset
  on reload and restore, and are not saved).
- `self.get(...)` returns a copy: change it, then `self.set(...)`.
- Time is `engine.dt` and `engine.tick`. No wall clock.
- Random numbers come from `engine.rand()`. `Math.random` does not exist.
- Input is actions: `engine.action(name)`, `engine.pressed(name)`, `engine.justPressed(name)`.
- Timers: `engine.after(seconds, fn, arg)`, `engine.every(...)`, `engine.cancel(...)`; `fn` is
  an exported function of the same script. No generators or async.
- Other entities: `engine.find(name)`, `engine.entity(id)`, `engine.query("Collider")`,
  `engine.spawn(components)`, `engine.clone(template)`, `engine.destroy(entity)`,
  `engine.send(target, kind, detail)`.
- `sin`, `cos`, `pow` are the same on every machine here; iteration order of plain objects
  must still never decide game state.
- A loop over thousands of items every tick belongs in a job (skill `marionette-write-job`).

## Loop

1. Edit the file. In a live session `script_reload` swaps it in and keeps state.
2. Load errors come back as `script.*` diagnostics with line and column. Runtime throws are
   events of kind script error: read them with `observe_events`, logs with `observe_logs`.
3. Optional type check: `tsc --noEmit --strict --target es2023 --lib es2023 --module esnext
   marionette.d.ts <name>.ts`.

## Checks

Run the scene twice with the same seed (`scene_run`): the hashes match. Add a scenario for
the behaviour (skill `marionette-test-game`).
