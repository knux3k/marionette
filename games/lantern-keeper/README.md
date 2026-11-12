# Lantern Keeper (top-down 2D)

A maze at night. Walk onto every lantern to light it; when all five burn, the gate opens and
reaching it wins. Two ghosts drift through the corridors at random (seeded); touching one costs
a life and sends you back to the start. Three lives.

Controls: arrow keys or WASD (stick on a gamepad).

| File | What it is |
| --- | --- |
| `build.ops.json` | The recipe: operations that build `scenes/main.scene.json` (`batch_run`). |
| `scenes/scripts/keeper.ts` | All gameplay: the text map, tile-by-tile movement, ghosts, lanterns, lives, HUD. |
| `scenes/main.actions.json` | Action map (`move_x`, `move_y`). |
| `tests/play.test.json` | Plays a winning route (`tests/play.inputs.json`): five lanterns, gate, win, no life lost. |
| `tests/idle.test.json` | Stands still: both ghosts are released, catch the player three times, game over. |

Proves: 2D sprites and orthographic camera, UI text, scripts spawning about 100 entities from
a map at start, timers (`engine.after` releases ghosts, `engine.every` flickers lanterns),
seeded randomness, input actions, emitted events checked by tests.

```bash
marionette --project games/lantern-keeper op test_run '{}'
marionette --project games/lantern-keeper run scenes/main.scene.json --input tests/play.inputs.json
```
