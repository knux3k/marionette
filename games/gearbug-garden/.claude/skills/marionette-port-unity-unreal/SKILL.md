---
name: marionette-port-unity-unreal
description: Port Unity C# or Unreal C++ and Blueprint gameplay code to Marionette with the engine's scan, map, scaffold and convert tools. Use when converting a MonoBehaviour, Actor, ActorComponent, Burst job, prefab or whole game from Unity or Unreal.
---

# Port Unity or Unreal code

A GameObject or Actor becomes an entity. Engine components become data components. A
MonoBehaviour or Actor class becomes one TypeScript script; its fields become `self.state`.
There is no frame-rate update: everything runs in fixed ticks and must be deterministic.

## Steps

1. Scan (read-only): `marionette op port_scan '{"source": "<Assets or Source folder>"}' --json`.
   Read `summary` first: counts by status, `actions`, `hazards`, `top_gaps`. Then only the
   classes you are porting.
2. Scaffold: `port_scaffold` with `source`, `out` and `scene` writes one compiling `.ts`
   skeleton per class, with the original code as comments and a `->` line under every engine
   call naming its equivalent, plus `PORT-NOTES.md`. Use `"dry_run": true` to see the list.
   Burst jobs get a Rust skeleton instead (`jobs.rs`).
   Saved data (`[Serializable]` classes read by Unity JsonUtility): `port_serde` with `source`
   and `out` writes Rust serde types that read and write the same JSON; read its `notes`
   (fields Unity does not save, unknown types kept raw, untranslated initializers).
3. Build the scene with operations (skill `marionette-build-scene`): one entity per object,
   the components the class report lists, and its `Script`.
4. Convert every position, rotation, scale and length with `port_convert`
   (`{"engine": "unity", "kind": "position", "value": [...]}`). Axes and units differ; never
   copy numbers across by hand.
5. Define an input action for every name in `summary.actions`.
6. Replace the comments with TypeScript, following the `->` lines
   (skill `marionette-write-script`). One API in doubt: `port_map` by name.
7. Prove it: same seed, same input, same `scene_run` hash twice; a scenario per behaviour.
8. Record what could not be ported at the end of `PORT-NOTES.md`.

## Statuses

| Status | Do |
| --- | --- |
| `ready` | port as data (a component, an action, a file) |
| `script` | translate the call directly |
| `rewrite` | follow the note: timers, coroutines, tags, input bindings take another shape |
| `gap` | use the note's workaround and record what is left |
| `hazard` | breaks determinism; must be rewritten as the note says |
| `unknown` | not in the tables; look it up |

## Rules that bite most often

- Delta time is `engine.dt`. Wall-clock time and unseeded randomness are hazards: use
  `engine.rand()`.
- Coroutines split at each wait into exported step functions chained with `engine.after`.
- Component access returns copies: get, change, set.
- No parenting: use separate entities with `Follow` or `LookAt`, or copy positions.
- Pushing a dynamic body from a script is a gap: use a kinematic body or
  `CharacterController`.
- Burst jobs become systems of the game's own Rust module (skill
  `marionette-add-rust-code`), or a job file when they are plain number loops
  (skill `marionette-write-job`).
- Assets do not come through these tools: skill `marionette-find-assets`
  (`library_import`, `prefab_instantiate`).
