# Proofs: save games (decision 0076)

- `cargo test -p marionette-save --test save`: exact continuation after load (same hash as a
  run that never saved), background writer equals `Sim::save_state` and drop finishes every
  save, waiting saves of one slot are replaced, cut-off and flipped-byte files load the
  backup, a crash between the two renames stays loadable and the temp file is swept, slot
  menus read headers only, bad slot names and large meta refused, data plus Rust migrations
  from version 1 to 3, the scene's migrations file, newer and other-game saves refused with
  the world untouched, autosave rotation and `save_request` slots.
- `cargo test -p marionette-agent --test save_ops`: `save_write` then `save_load` into a
  fresh session continues with the same hash for `playground3d` (physics, input),
  `playground2d` and `scripted`; `save_check`, `save_list`, `save_delete`; read-only mode.
- Leaks: `save_write` and `save_load` run in every round of the session scenario of
  `crates/marionette-agent/tests/leaks.rs`.
- `cargo test --release -p marionette-cli --test perf_budgets -- --ignored save_game_budget`:
  4x factory, main-thread part of a whole save 1.06 ms (budget 2 ms); save thread 68 ms for
  2.3 MB; load 0.4 s (container).
- Windowed `--saves` (autosave, F5, F9): compiled and wired only; never run on a screen.
- `a_script_asking_for_a_save_gets_its_own_slot` in `crates/marionette-agent/tests/save_ops.rs`:
  a TypeScript `engine.emit("save_request", { slot, label })` makes the autosaver write that
  slot with the label.
- Skill `marionette-save-games`: checked by `cargo test -p marionette-cli --test skills`.
