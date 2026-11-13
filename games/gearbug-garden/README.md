# Gearbug Garden

Wind through a clockwork hedge maze, gather every golden cog while topiary beasts roam, then slip out the brass gate.

Made with the Marionette game starter (`game_make`, template `maze`, decision 0150).

Arrows or WASD move. R restarts after a win or loss. F1 toggles the autopilot.

## Files

- `brief.json`: the whole design (template, words, colors, one line per asset slot, seed).
- `scenes/scripts/game.ts`: the game. `game_make` rewrites only its CONFIG block.
- `build.ops.json`: the scene recipe, rebuilt by `game_make`. Put your own scene operations in
  `custom.ops.json` (same format); they run after it.
- `assets/generated/*.recipe.json`: art still to generate (`marionette generate run`).
- `tests/`: the autopilot wins (`autoplay.test.json`); doing nothing does not (`idle.test.json`).

## Change it

Edit `brief.json`, then `marionette op game_make '{"path": "<this folder>", "refresh": true}'` from the
parent project. After `marionette generate run`, the same refresh swaps the placeholders for the
generated art and re-pins the tests.
