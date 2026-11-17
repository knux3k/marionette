# 0152: Starter counts, honest generated docs, protected CONFIG block

Status: accepted. Follows 0150 and 0151. Found by the cold-start trial (a fresh AI using only
the public docs made a game).

## Problem

The trial asked for "3 gems". The interview and brief had no count, the maze held more
pickups than the README said, a hand edit of the map inside the CONFIG block was silently
erased by `refresh`, and the generated `docs/ai/STATE.md` named tests that do not exist.

## Choice

- Brief fields `pickup_count` (1 to 12) and `enemy_count` (0 to 6), maze only, validated by
  `Brief::check`. Unset means the old defaults from `size` and `difficulty`, so existing briefs
  make the same files.
- The interview asks both for a maze. The guess for the pickup count is a number the prompt
  gives next to the pickups word ("3 gems", "three shiny gems").
- The generated README states the real counts from the final map; STATE.md is filled with the
  real game, test names and open art.
- `game_make` writes the CONFIG block it generated to `.config.generated`. A `refresh` that
  finds a different block stops with `starter.config_edited`; `overwrite_config: true`
  replaces it and keeps the old block in `.config.backup`. Games made before this have no
  `.config.generated`, so they refresh as before.

## Rejected

Refusing every CONFIG difference (brief changes also change the block, so only a difference
from the last generated block counts); a count field for the platformer (its pickups sit on
generated platforms, so an exact count is a separate design).
