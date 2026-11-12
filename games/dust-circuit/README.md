# Dust Circuit (racing)

A three-lap race on a dusty oval at dusk against one rival car. The lights count down from
three; then hold the throttle and steer through the two walled curves. Checkpoints keep
you honest: a lap only counts after passing all four in order. Finish first to win.

Controls: W or up (right trigger) to accelerate, S or down (left trigger) to brake and
reverse, A/D or left/right (left stick) to steer.

| File | What it is |
| --- | --- |
| `build.ops.json` | The recipe: operations that build `scenes/main.scene.json` (`batch_run`), including the 48 curve wall segments. |
| `scenes/scripts/race.ts` | The race: countdown, "go" to both cars, finish order from their messages, who leads, the chase camera and the HUD. |
| `scenes/scripts/car.ts` | The player's car: speed, drag and speed-dependent steering kept in script state, applied by turning the body and driving it with `CharacterInput`; lap counting from checkpoint sensors. |
| `scenes/scripts/rival.ts` | The rival: the same car, steered toward a point six metres ahead on the inner line at a steady pace. |
| `tests/play.test.json` | Holds the throttle and steers with an analog stick (`tests/play.inputs.json`): three laps on the outer line, first across the line. |
| `tests/idle.test.json` | Never touches the controls: the rival finishes three laps and wins. |

Proves: vehicle handling written in script on top of the character controller, an AI driver,
analog input in scripted tests, ordered checkpoint sensors, messages between scripts, a chase
camera, the `RoundCuboid` collider, UI text.
