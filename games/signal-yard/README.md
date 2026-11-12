# Signal Yard (first-person shooter)

A walled yard at night around a relay mast. Drones hold it: they circle their posts and fire
slow orbs at you whenever they can see you. Clear three waves (3, 4 and 5 drones, two hits
each) before they land five hits on you.

Controls: mouse to look (or Q/E and arrows; right stick), WASD to move (left stick), Space to
jump, left mouse or F (right trigger) to fire. Crates and pylons block orbs and line of sight.

| File | What it is |
| --- | --- |
| `build.ops.json` | The recipe: operations that build `scenes/main.scene.json` (`batch_run`). |
| `scenes/scripts/yard.ts` | The director: waves at seeded random posts, kills and the player's fall from messages, HUD, the pulsing beacon. |
| `scenes/scripts/player.ts` | The player: turns and tilts the view, places camera and gun, shoots with `engine.raycast`, draws tracers, takes hits. Walking and jumping are the character controller's own actions. |
| `scenes/scripts/drone.ts` | An enemy: orbits its post, checks line of sight with a ray cast, fires orbs on a repeating timer, flashes when hit. |
| `scenes/scripts/orb.ts` | A drone's shot: flies along the velocity it was sent, hurts the player, bursts on anything solid. |
| `tests/play.test.json` | Replays `tests/play.inputs.json`, a recording of an agent playing through the MCP server (mouse look, strafing, firing): all three waves cleared, 24 hits, 1 to 4 hits taken. |
| `tests/idle.test.json` | Does nothing: wave 1 lands five hits. |

Proves: first-person camera and mouse look, hit-scan shooting with ray casts, line-of-sight
checks for enemies, projectiles as kinematic sensors, waves with timers and seeded randomness,
messages between scripts, a playthrough recorded from a live session and replayed exactly.
