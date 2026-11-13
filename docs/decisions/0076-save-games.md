# 0076: Save games: `marionette-save` (slots, crash-safe files, migrations, autosave)

Date: 2026-11-13. Status: accepted.

## Context

Games need a standard way to save and load progress, and the large VR factory game the
engine targets needs one that costs under 2 ms of main-thread time (decision 0073 made the
copy cheap with `Sim::capture`). Saves must survive a crash or power loss mid-write, and
saves from an older build must load after the game's data changes shape.

## Decision

- New crate `marionette-save` (depends only on core), cargo feature `save` of the agent and
  CLI, on by default. `SaveModule` is in the standard engine and registers resource
  `SaveFormat { game, version, autosave_every, autosave_slots, migrations }`.
- What is saved: `Sim::save_state`'s content (scene text of every registered type, tick, RNG,
  module bytes such as physics internals and held input), so a loaded save continues
  exactly. `Sim::capture_state` copies it on the main thread (`StateCapture`, `Send`);
  `StateCapture::into_snapshot` builds the text elsewhere.
- File `<slot>.msave`: magic line `MARIONETTE-SAVE 1`, one header line of JSON (game,
  version, tick, label, small `meta` up to 16 KB, wall-clock time, sizes, SHA-256 of the
  body), then the scene text and module bytes. A slot menu reads only the header.
- Crash safety (`SaveStore::write`): write `.<slot>.<pid>-<n>.tmp` and flush it, rename the old
  save to `<slot>.msave.bak`, rename the temp file into place, flush the folder (Unix). Reads
  verify size and checksum and fall back to the backup, reporting why. Stale temp files are
  removed when a store opens (older than 60 s). Temp files sit beside the save, not in the
  engine's run folder, because rename is atomic only within one volume.
- Versions: the header carries `SaveFormat.version`. Older saves migrate one version at a
  time: data steps (`rename_type`, `remove_type`, `rename_field`, `set_field`,
  `remove_field`) from the JSON file `SaveFormat.migrations`, then Rust steps from
  `SaveModule::with(Migrations)` (`EngineBuilder::replace`). Newer saves (`save.newer`) and
  other games' saves (`save.other_game`) are refused; a failed load leaves the world as it
  was. A migrated save drops module bytes (`exact: false`); modules rebuild from components.
- Writing off the tick: `SaveWriter` is one background thread with at most 4 waiting saves;
  a newer save of a waiting slot replaces it; dropping the writer finishes every save.
- Autosave: `Autosaver::after_step` queues a save every `autosave_every` ticks into
  `auto-0..auto-(n-1)` (oldest first) and one per `save_request` event (scripts:
  `engine.emit("save_request", ...)`, optional `slot` and `label`). Events are not state, so
  asking changes no hash. `marionette run --saves <dir>` turns this on in the window, with F5
  quick save and F9 quick load (slot `quick`).
- Operations `save_write`, `save_load`, `save_list`, `save_check`, `save_delete` (folder
  default `saves` under the project root).

## Alternatives

- **Whole save as one JSON document** (module bytes as base64): simpler, but a slot menu
  would parse megabytes to show a label, and base64 grows module bytes by a third.
- **Saving only the scene** (no module bytes): smaller, but physics contacts and held input
  would be rebuilt, so loading would not continue exactly. Kept only for migrated saves.
- **Copy-then-write backup** instead of rename: costs a second full write per save; the
  rename gives the same guarantee for free.
- **Compressed body**: the scene text compresses well, but the 4x factory save is 2.3 MB
  and writing it is off the tick already. Can be added as container format 2.
- **Autosave as a simulation system**: writing files from a tick breaks the determinism
  rules and would stall the tick; the runner calls the autosaver after ticks instead.

## Consequences

- Focus 3: main-thread cost of a whole save game of the 4x factory 1.06 ms (budget 2 ms,
  `perf_budgets::save_game_budget`); 68 ms on the save thread for 2.3 MB; load 0.4 s
  (container numbers).
- Focus 1: a game can leave saves out (`default-features = false`) and adds its own
  migrations without editing the engine.
- Focus 2: `save_list` returns headers only; save results are a few fields.
- No state hash or reference image changes; `SaveFormat` is absent from existing scenes.
