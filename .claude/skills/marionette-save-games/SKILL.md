---
name: marionette-save-games
description: Save slots, autosave, checkpoints and migrating old saves. Use when a game needs saves or a save fails to load.
---

# Save games

A save holds the whole simulation: every registered component and resource, tick, random
state and module state (physics, held input). Loading it continues exactly as if the game
never stopped. Anything not in a component or resource is not saved.

## Set up

1. Add resource `SaveFormat` to the game scene (`resource_set`):
   `{"game": "<id>", "version": 1, "autosave_every": 0, "autosave_slots": 3, "migrations": ""}`.
   `autosave_every` is in ticks (3600 = one minute at 60 Hz; 0 = only when asked).
2. Ask for a save at safe moments (checkpoint, level change, never mid-action) from a script:
   `engine.emit("save_request")` for the next autosave slot, or
   `engine.emit("save_request", { slot: "checkpoint", label: "Gate" })` for a named slot.
3. Play with saving on: `marionette run <scene> --saves saves` (autosave, F5 quick save,
   F9 quick load, slot `quick`).

## Save and load from an agent

- `save_write` (`session`, `slot`, optional `label`, `meta`, `dir`) and `save_load` on a live
  session; `save_list` shows slots from headers only; `save_delete` removes a slot and its
  backup. Folder default: `saves` in the project.
- Check exactness: `session_step` after `save_write`, then `save_load` into a fresh session of
  the same scene and step the same ticks: the `state_hash` values must match.
- Slot names: lower-case letters, digits, `_` and `-`. Autosaves use `auto-0`, `auto-1`, ...;
  never write manual saves there.

## Change saved data without breaking old saves

1. Raise `version` in `SaveFormat`.
2. Write the migrations file named by `SaveFormat.migrations` (relative to the scene):
   `{"steps": [{"from": 1, "changes": [{"op": "rename_type", "from": "Gold", "to": "Wallet"}]}]}`.
   Changes: `rename_type`, `remove_type`, `rename_field`, `set_field` (`value`; only where
   missing unless `overwrite`), `remove_field`. Fields may be dotted paths. Step `from: n`
   turns version n into n + 1; one step per version that changed.
3. Keep one save of every released version under `tests/` and run `save_check` against each:
   `ok` must be true. Then `save_load` one and look at the migrated values.
4. Logic that data steps cannot express goes in a Rust step (game's own binary, skill
   `marionette-add-rust-code`).

## When a load fails

- `save.newer`: written by a newer build; update the game.
- `save.other_game`: `SaveFormat.game` differs.
- `save.corrupt`: damaged file; the load already tried `<slot>.msave.bak` (the previous save).
- A validation error after migrating: a type or field changed without a migration step.
- A failed load leaves the running game unchanged. A migrated load reports `exact: false`:
  physics and input state are rebuilt from components.

## Performance

Saving copies state on the tick (about 1 ms for a large factory) and writes on another
thread. Keep `meta` small (slot menu data only, at most 16 KB).
