# Ember Run (2D side-scroller)

A spark runs right across night rooftops to light a beacon. Collect embers on the way, jump
the gaps, time the jump onto the sliding lift, and dodge the rain that falls over the two wet
stretches. A drop or a fall costs a life and sends you back to the last checkpoint. Three lives.

Controls: A/D or arrows to run, Space/W/Up (gamepad South) to jump.

| File | What it is |
| --- | --- |
| `build.ops.json` | The recipe: operations that build `scenes/main.scene.json` (`batch_run`). |
| `scenes/scripts/runner.ts` | The runner: pickups, checkpoints, the beacon and rain hits (trigger hooks), falls, respawn (teleport), lives. |
| `scenes/scripts/level.ts` | Camera follow, parallax hills, the lift, rain (a repeating timer spawning falling sensors), HUD. |
| `scenes/main.actions.json` | Action map (`move_x`, `jump`). |
| `tests/play.test.json` | A full run (`tests/play.inputs.json`): three checkpoints, at least 14 of 15 embers, wins with at most two hits. |
| `tests/pit.test.json` | Holds right and never jumps: falls into the first gap three times, game over. |

Proves: 2D physics (Rapier 2D), the character controller (run, jump, ground), sensors as
pickups and hazards, a kinematic moving platform, dynamic bodies spawned by a timer and
removed by `DestroyAt`, teleporting a character from a script, jump sounds, orthographic
camera follow and parallax, UI text.
