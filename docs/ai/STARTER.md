# Game starter (decision 0150)

One idea to a playable, tested game. Code: `crates/marionette-starter` (brief, levels,
templates in `templates/*.ts`), operations in `crates/marionette-agent/src/starter.rs`.

## Use it

```bash
marionette make "a 3d maze where a moth collects lanterns and avoids bats"   # asks the interview first
marionette make "a moth maze" --answer genre=maze --answer view=3d          # answers up front (or --yes)
marionette make --brief my.brief.json --path games/my-game                   # your own brief
marionette generate run --project games/my-game                              # paid art (keys on the machine)
marionette make --refresh --path games/my-game                               # art in, tests re-pinned
```

## Interview (decision 0151)

Making from a prompt asks the person first. `game_interview {prompt, answers?}` returns the
open questions (`genre`, `view`, `setting`, `player`, `pickups`, `enemy`, `goal`, `style`,
`difficulty`, `length`, `title`, `extra`), each with choices or examples and the guess kept if
unanswered, plus notes for ideas no template fits. Pass the replies as `answers` (strings by id)
to `game_brief` or `game_make`. `game_make` refuses a bare prompt (`starter.interview_needed`)
unless `skip_interview` is set because the person declined. The answers land in
`interview.json`; `extra` becomes a handoff TODO and the output's `wishes`. On a terminal the
CLI asks the questions itself; elsewhere give `--answer id=value` or `--yes`.

## Operations

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

In [proofs/games.md](proofs/games.md#game-starter-decision-0150).
