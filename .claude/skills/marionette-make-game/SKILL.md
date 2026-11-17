---
name: marionette-make-game
description: Interview the person, then make a playable, tested Marionette game (maze or platformer) from an idea or brief and add art. Use when asked to make a game, or change a made game's brief or art.
---

# Make a game from an idea

`game_make` writes a whole project: scene, gameplay script, input, tests, art recipes and
handoff files. Its template's autopilot (F1) plays the level headless; a seed it cannot win is
rerolled, so a made game is always winnable.

## Steps

1. Interview the person before building: `game_interview` with `prompt` returns the open
   questions (genre, view, setting, player, pickups, enemy, goal, style, difficulty, length,
   title, extra), each with choices or examples and the guess used if unanswered. Ask them in
   your own words, several at once, offering the guess as the default. Raise its `notes` (an
   idea no template fits). Only if the person says to just build it, set `skip_interview`.
2. Draft the design: `game_brief` with `prompt` and `answers` (replies by question id). It
   writes nothing and returns a brief plus its art slots.
3. Improve the brief if needed: title, pitch, `style`, one `describe` per slot, `words`,
   `colors`. Point a slot at existing art with `use` (a project path or `pool:<id>`) or give it
   `search` words for the lookup.
4. Make it: `game_make` with `path` and `brief` (or `prompt` plus `answers`). CLI:
   `marionette make "<idea>"` asks on a terminal; else `--answer id=value` or `--yes`.
   Read the output: `autopilot.won`, `tests_passed`, `tests_failed`, `pending` recipes, and
   `wishes` (the person's `extra`: build it next with `marionette-write-script`).
5. Art: slots without art get placeholders and recipes. Generating spends money, so follow
   `marionette-generate-assets`; then `game_make` with `path` and `refresh` (or `generate`
   to do both in one call).
6. Play: `marionette run scenes/main.scene.json --project <path>`.

## Changing a made game

- Design (words, colors, art, difficulty, seed, and for a maze `pickup_count` 1 to 12 and
  `enemy_count` 0 to 6): edit `brief.json`, then `game_make` with
  `refresh`. Refresh rewrites only the CONFIG block of the gameplay script and re-pins the tests.
- Gameplay: edit the script outside its CONFIG markers; refresh keeps it. Never hand-edit
  inside CONFIG: refresh stops with `starter.config_edited` (`overwrite_config` replaces it and
  keeps `.config.backup`). Extra scene
  operations go in `custom.ops.json`, which every rebuild appends.
- A newer template: delete the gameplay script (your edits go with it) and refresh.

## Traps

- `game_make` refuses an existing folder unless `refresh` is set, and a bare prompt with no
  `answers` (`starter.interview_needed`).
- `interview.json` in the game holds what the person asked for; check later changes against it.
- Refresh refuses a script whose CONFIG markers were removed.
- A level the autopilot cannot win after `tries` seeds fails with `starter.unwinnable`: change
  difficulty or size.
- Check with `test_run` after any edit: the autopilot test must still win and the idle test
  must still lose.
