# Game starter (decision 0150)

One idea to a playable, tested game. Code: `crates/marionette-starter` (brief, levels,
templates in `templates/*.ts`), operations in `crates/marionette-agent/src/starter.rs`.

## Use it

```bash
marionette make "a 3d maze where a moth collects lanterns and avoids bats"   # folder from the title
marionette make --brief my.brief.json --path games/my-game                   # your own brief
marionette generate run --project games/my-game                              # paid art (keys on the machine)
marionette make --refresh --path games/my-game                               # art in, tests re-pinned
```

As operations: `game_brief {prompt}` returns a brief and its slots without writing; improve it
(title, pitch, style, one `describe` per slot, words, colors), then `game_make {path, brief}`.
Output: slots and where their art came from, `pending` recipes, the autopilot's win time and
seed, test counts, handoff status.

## Brief (`brief.json`)

`template` `maze` | `platformer`; `view` `2d` | `3d` (maze only); `difficulty` easy | normal |
hard; `size` small | medium | large; `seed`; `style` (added to every art prompt); `words`
{`pickups`, `win`, `lose`}; `colors` {`background`, `accent`} as `#rrggbb`; `assets.<slot>`
{`describe`, `use` (path or `pool:<id>`), `search` (lookup words)}. Slots: maze 2D player,
enemy, pickup, goal, wall, floor; maze 3D player, enemy, pickup, goal, wall (models); platformer
player, enemy, pickup, goal, spikes, ground, background.

## A made game

`brief.json`, `scenes/scripts/game.ts` (only its CONFIG block is rewritten), `build.ops.json`
(+ your `custom.ops.json`), `scenes/main.actions.json` (F1 autopilot, R restart),
`assets/generated/*.recipe.json`, `tests/autoplay.test.json` and `tests/idle.test.json`,
README.md, providers.json (Qwen images, Tripo models), handoff files.

Refresh never changes code outside the CONFIG block, so a game keeps the template version it was
made with. To take a newer template, delete `scenes/scripts/game.ts` (your edits in it go too)
and refresh.

## Proofs

- `cargo test -p marionette-cli --test starter`: every template from a prompt is won by its
  autopilot and passes its own tests; the same brief gives byte-identical files; refresh keeps
  edits outside CONFIG and refuses a script without markers; bad briefs list every problem.
- `cargo test -p marionette-starter`: mazes are connected, platformer gaps are at most 3 wide,
  same seed same level, prompt reading.
- Demo games (in `proof_scenes.rs` and the stability suite): `games/gearbug-garden` (3D maze),
  `games/comet-hopper` (platformer); `examples/starter-maze` (2D maze from a prompt, placeholder
  art).
- First-try autopilot wins on sampled seeds (container, 2026-11-13): mazes 32 of 32; platformer
  about 90% (spikes, edges and walking enemies cause the rest); `game_make` tries further seeds.
