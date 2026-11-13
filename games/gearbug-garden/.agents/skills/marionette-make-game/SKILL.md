---
name: marionette-make-game
description: Turn a game idea into a playable, tested Marionette game project from one prompt or a short brief (maze in 2D or 3D, side-scrolling platformer), then swap in generated or pooled art. Use when asked to make, start or prototype a new game, to change a made game's design through its brief, or to refresh its art and tests.
---

# Make a game from an idea

`game_make` writes a whole project: scene, gameplay script, input, tests, art recipes and
handoff files. Its template's autopilot (F1) plays the level headless; a seed it cannot win is
rerolled, so a made game is always winnable.

## Steps

1. Draft the design: `game_brief` with `prompt` (and `template` `maze` | `platformer`, `view`
   `2d` | `3d`). It writes nothing and returns a brief plus its art slots.
2. Improve the brief before making: title, pitch, `style`, one `describe` per slot, `words`,
   `colors`. Point a slot at existing art with `use` (a project path or `pool:<id>`) or give it
   `search` words for the lookup.
3. Make it: `game_make` with `path` and `brief` (or `prompt`). CLI: `marionette make "<idea>"`.
   Read the output: `autopilot.won`, `tests_passed`, `tests_failed`, `pending` recipes.
4. Art: slots without art get placeholders and recipes. Generating spends money, so follow
   `marionette-generate-assets`; then `game_make` with `path` and `refresh` (or `generate`
   to do both in one call).
5. Play: `marionette run scenes/main.scene.json --project <path>`.

## Changing a made game

- Design (words, colors, art, difficulty, seed): edit `brief.json`, then `game_make` with
  `refresh`. Refresh rewrites only the CONFIG block of the gameplay script and re-pins the tests.
- Gameplay: edit the script outside its CONFIG markers; refresh keeps it. Extra scene
  operations go in `custom.ops.json`, which every rebuild appends.
- A newer template: delete the gameplay script (your edits go with it) and refresh.

## Traps

- `game_make` refuses an existing folder unless `refresh` is set.
- Refresh refuses a script whose CONFIG markers were removed.
- A level the autopilot cannot win after `tries` seeds fails with `starter.unwinnable`: change
  difficulty or size.
- Check with `test_run` after any edit: the autopilot test must still win and the idle test
  must still lose.
