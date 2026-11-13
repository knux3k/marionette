# Start here

Entry point for any agent working on this game. Written by agents, for agents. Keep it short:
it is read at the start of every session.

## Every session starts by

1. Reading `PROTOCOL.md` (the owner's rules), this file and [STATE.md](STATE.md).
2. Running `handoff_status`: work in progress (claims), the latest session notes, the next free
   decision number, and problems to fix first.
3. Claiming the work with `handoff_claim` before changing anything, when anyone else (a person
   or another agent) may be working here too.

## Every session ends by

1. `project_validate` and `test_run` passing.
2. `handoff_note`: what was done, what was not, what is unverified, the next step. Pass
   `release: true` when the claimed work is finished.
3. Updating your own rows of [STATE.md](STATE.md).
4. A decision record in `docs/decisions/` for every choice between real alternatives.

## Where things are

| What | Where |
| --- | --- |
| What exists now, what is unverified, what is next | [STATE.md](STATE.md) |
| Who is working on what right now | `docs/ai/claims/` (one file per piece of work) |
| What each session did | `docs/ai/sessions/` (one note per session, newest last) |
| Why something was chosen | `docs/decisions/` (`NNNN-title.md`) |
| How to do a task | the skills (`skills_list`), starting with `marionette-handoff` |

## This game

Knight And The Coins: Collect all the coins, then reach the goal. From the prompt: "a spooky dungeon maze where you play as a little knight who collects gold coins and avoids skeletons"

- `brief.json`: the whole design (template, words, colors, one line per asset slot, seed). Change it, then run game_make with `refresh`.
- `scenes/scripts/game.ts`: the game. game_make rewrites only its CONFIG block; the rest is yours.
- `build.ops.json`: the scene recipe, rebuilt by game_make. Own scene operations go in `custom.ops.json`.
- `assets/generated/*.recipe.json`: art still to generate (`marionette generate run`, then game_make `refresh`).
- `tests/autoplay.test.json` (the autopilot wins) and `tests/idle.test.json` (doing nothing does not win); refresh re-pins them.
