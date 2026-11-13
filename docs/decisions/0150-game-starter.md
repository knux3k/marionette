# 0150: The game starter: one prompt to a playable, tested game

Status: accepted (2026-11-13)

## Context

The engine is built so that an AI turns an idea into a game. Until now that meant writing a
build recipe, a TypeScript game script, an input map, tests and generation recipes by hand for
every game (the six proof games, decision 0050). The owner asked for one prompt that takes an
idea to a playable scene with generated art, tying together the asset lookup order (decision
0041), generation (0045) and the proof-game way of building (0050).

Options considered:

1. **A language model writes the whole game per prompt** (scene, script, tests). Most flexible,
   but every game costs a large amount of tokens, results vary run to run, and nothing proves
   the result is playable.
2. **Genre templates driven by a small brief** (chosen). The model writes about 1 KB of JSON;
   tested template code does the rest.
3. **A provider call inside the engine** to turn the prompt into the brief. The agent calling
   the engine is already the language model, so this would pay twice; it stays possible later.

## Decision

- A **brief** (`marionette.brief/1`, crate `marionette-starter`): template, view, title, pitch,
  style, difficulty, size, seed, HUD words, two colors, and one `describe` (plus optional `use`
  or `search`) per asset slot. `game_brief` makes one from a prompt with a keyword reader (no
  model needed) and lists the slots; agents are expected to improve it.
- **Templates**: `maze` (2D sprites or 3D models with shadows, sky light, bloom) and
  `platformer` (2D, scrolling). Each is one TypeScript script with a `CONFIG` block, an input map,
  a scene recipe and asset slots. Levels are generated from the seed (integer-only SplitMix64).
- **Every template has an autopilot** (F1). `game_make` lets it play each level headless; a
  level it cannot win is replaced by the next seed (default 8 tries), and the seed used is
  written back to the brief. The tests pin a win by the autopilot and no win when idle.
- **Assets** follow the lookup order: the brief's `use`, an already built recipe output, project
  files and the CC0 pool (when the slot has `search` words), otherwise a placeholder shape plus
  generation recipes (sprites with a keyed magenta background, seamless tiles, a backdrop, or a
  reference image and a model). Sprites are cropped to their visible pixels (`SpriteRegion`).
  `generate: true` runs `generate_run` first.
- **Refresh**: `game_make` with `refresh` re-reads `brief.json`, rewrites only the script's
  `CONFIG` block (edits elsewhere are kept; a script without the markers is refused), rebuilds
  the scene (plus the game's own `custom.ops.json`) and re-pins the tests. This is how generated
  art replaces the placeholders.
- **Handoffs**: each made game gets the AI handoff files and skills through `handoff_init` and
  `skills_install` (decisions 0160 and 0161); a build without them reports it.
- CLI: `marionette make "<idea>"` (or `--brief file`, `--refresh --path`, `--generate`).
- Cargo feature `starter` (on by default, needs `script`, `pool`, `generate`).

## Consequences

- Focus 1 (modularity): a new crate behind a feature; templates are ordinary game files the
  agent then edits; `custom.ops.json` and code outside `CONFIG` survive refreshes.
- Focus 2 (tokens): a game costs a ~1 KB brief instead of a full recipe and script; outputs are
  summaries (slots, pending recipes, autopilot time).
- Focus 3 (performance): template scripts run on the fixed tick with tile maps; both demo games
  are in the stability suite.
- Focus 4 (graphics): the 3D maze opts into shadows, sky light, tone mapping and bloom; pickups
  glow.
- Only two genres so far; more templates are the obvious next step (top-down shooter, racer).
  The keyword reader is crude by design; an agent-written brief is the normal path.
