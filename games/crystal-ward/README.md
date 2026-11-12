# Crystal Ward (top-down 3D)

Crawlers come out of the dark in five waves and walk to the crystal in the middle of the
arena. You are the warden: walk anywhere, and while you hold fire your bolts seek the nearest
crawler in range. Every crawler that reaches the ward ring cracks the crystal; ten cracks and
it shatters. Survive the fifth wave to win.

Controls: WASD or arrows (left stick) to walk, Space, left mouse or right trigger to fire.

| File | What it is |
| --- | --- |
| `build.ops.json` | The recipe: operations that build `scenes/main.scene.json` (`batch_run`). |
| `scenes/scripts/ward.ts` | The director: waves on a repeating timer, crawler spawns at seeded random gates, crystal health and score from messages, HUD. |
| `scenes/scripts/warden.ts` | The player: turns to face the input and walks with `CharacterInput`; fires bolts at the nearest crawler and sends each its heading. |
| `scenes/scripts/crawler.ts` | An enemy: faces the crystal and walks with `CharacterInput` (wall sliding around pillars); breaches the ward ring. |
| `scenes/scripts/bolt.ts` | A projectile: flies along the heading it was sent, kills a crawler, breaks on walls, expires. |
| `tests/play.test.json` | Holds fire and moves around (`tests/play.inputs.json`): all five waves, at least 15 kills, the crystal holds. |
| `tests/idle.test.json` | Does nothing: ten breaches, the crystal shatters in wave 3. |

Proves: script-driven characters (player and AI), entities spawned and destroyed in volume,
repeating and one-shot timers, messages between scripts (`engine.send`/`onMessage`), sensors
between kinematic bodies, `DestroyAt` lifetimes, sounds on spawn, perspective top-down camera,
UI text.
