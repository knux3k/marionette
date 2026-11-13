# 0213: Achievements, stats and cloud files behind interfaces, with an offline backend and a three-way save sync (`marionette-platform`)

Date: 2026-11-13. Status: accepted.

## Context

Games want achievements, stats and cloud saves on Steam, and the same game code must run
without a store (development, CI, other stores later). Save files come from the save game
system (decision 0076, crate `marionette-save`), which writes slots into a folder.

## Decision

- Crate `marionette-platform` (no engine dependencies): traits `Achievements` (unlock, check,
  clear, stats as `Int` or `Float`, progress pop-ups, `store`) and `CloudFiles` (enabled, write,
  read, delete, list).
- Definitions file `*.achievements.json` (format `marionette.achievements`): ids are store API
  names (ASCII letters, digits, `_`), unique; an achievement may unlock when a stat reaches
  `at`; stats have a kind and an optional maximum. `Tracker` checks every call against it,
  clamps stats and unlocks stat-driven achievements. Operation `achievements_check`.
- `OfflinePlatform`: both traits in a local folder, written by temporary file and rename.
- `sync_dir(dir, suffixes, cloud)`: three-way sync per file (local, cloud, last synced base kept
  in the folder; the cloud keeps a hash manifest). One side changed: copy or delete across.
  Both changed: nothing is overwritten; the cloud copy is written as `<name>.cloud-conflict`
  and reported. It works on the save system's slot folder without depending on its crate.

## Alternatives

- **Rely on Steam Auto-Cloud only**: no code, configured on the store's site, and fine for many
  games (the docs say so). But it does nothing offline or on other stores and cannot report
  conflicts to the game. Both remain possible.
- **Last-writer-wins by timestamp**: simpler, but clocks differ between machines and a
  handheld that played offline would silently lose or overwrite progress. Rejected.
- **Put achievements in the scene**: they are per player, not per world, and must not enter the
  state hash. Rejected.

## Consequences

- Game code calls the same traits on Steam (`marionette-steam`) and offline.
- Conflicts need a game decision (ask the player); the engine never picks one silently.
- Not yet wired to scripts (TypeScript cannot unlock an achievement); a game's Rust module
  does it. A script binding is a follow-up when a game needs it.
