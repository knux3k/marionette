# Cloudstep (3D platformer)

Hop across floating islands to the beacon on the highest one. Time the jump onto the sliding
ferry, take the spring up the cliff, climb the stepping stones. Golden seeds float along the
way (some off the straight path). Falling sends you back to the last checkpoint; the clock in
the corner is the score.

Controls: WASD or arrows to walk, Space (gamepad South) to jump.

| File | What it is |
| --- | --- |
| `build.ops.json` | The recipe: operations that build `scenes/main.scene.json` (`batch_run`). |
| `scenes/scripts/hero.ts` | Seeds, the spring (sets the controller's vertical speed), checkpoints, falls and respawn, the beacon. |
| `scenes/scripts/course.ts` | The ferry (kinematic, scripted), spinning and bobbing seeds, the chase camera, the HUD. |
| `scenes/main.actions.json` | Action map (`move_x`, `move_forward`, `jump`). |
| `tests/play.test.json` | A full run (`tests/play.inputs.json`): ferry, spring, stones, beacon in about 11 s, no falls. |
| `tests/plunge.test.json` | Holds forward and never jumps: falls off the first island again and again. |

Proves: 3D physics, the character controller (walk, jump, step onto ledges), a kinematic
moving platform, sensors for pickups, checkpoints, a spring and the goal, scripts writing
`CharacterState` and teleporting the character, a perspective chase camera with `LookAt`,
lights, jump sounds, UI text.
