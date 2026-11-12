# 0050: Proof scenes are small original games, built from operation recipes and tested by play

Date: 2026-11-12. Status: accepted. Plan line 9 (W8). Adds the operations and script API
the games needed (listed below).

## Decision

- **Six small original games**, one per genre named in the brief: a top-down 2D maze, a 2D
  side-scroller, a 3D platformer, a top-down 3D arena, a racing game and a first-person
  shooter. None recreates an existing game: names, rules, levels and look are made for this
  repository. Each lives in `games/<name>/` as an ordinary Marionette project
  (`marionette.json`), laid out the way an agent would build one:
  - `build.ops.json`: the recipe. A `marionette.ops` file run by the new `batch_run`
    operation: `scene_create`, `resource_set`, `entity_create`, ... in order. The scene files
    in `scenes/` are its output and are committed.
  - `scenes/*.scene.json`, `scenes/*.actions.json`: the scene and its action map.
  - `scenes/scripts/*.ts`: gameplay in TypeScript (decision 0022), using spawning, timers and
    contact hooks (decision 0021).
  - `tests/*.test.json` and `tests/*.inputs.json`: scenario tests that play the game with
    scripted input and assert on what happened (events, script state, state hash).
  - `README.md`: the rules, the controls, and what the game proves.
- **The proof** (`crates/marionette-cli/tests/proof_scenes.rs`): for every game, replaying the
  recipe in an empty copy gives byte-identical scene files; its scenario tests pass twice with
  identical results; at least one test plays it with scripted input and pins the state hash.
  Each game also registers a stability mode (decision 0027, stability suite) once that suite
  is on `main`.
- **Assets**: built-in primitives (`primitive:cube`, `primitive:sphere`, `primitive:plane`),
  built-in sprite images and sounds, all generated in code and CC0. The CC0 pool (W3) and
  recipe-generated assets (W4) do not exist yet; when they land, the games swap their
  primitives for pooled assets through operations and the recipes record it.

## Engine changes made for the games

| Change | Why a game needed it |
| --- | --- |
| Operations `resource_set`, `resource_remove` | Physics, input and the tick length are resources; no operation could set them, so a scene that needs physics could not be built through operations. |
| Operation `batch_run` (`*.ops.json` or inline list; stops at the first failing step and names it; no nesting) | A scene recipe that can be replayed and checked, and fewer round trips over MCP. `Agent::call` now shares the operation table through an `Arc` so an operation can call others. |
| `engine.raycast(origin, direction, maxDistance?, exclude?)` in scripts | Shooting, line of sight and ground checks. Physics offers it to scripts through the core resource `RayCaster` (scripts must not depend on physics). Skips sensors and `exclude`; sees the world as the last physics step left it. `observe_raycast` behaves as before. |
| `CharacterController` moves relative to the entity's facing, and its body takes its rotation from `Transform` | A first-person or chase-camera character turns with the mouse or the stick. Unrotated characters move exactly as before (identity rotation is a fast path; existing hashes are unchanged). 2D is unchanged. |
| Component `CharacterInput { move_x, move_forward, jump }` | AI characters and vehicles walk with the controller (wall sliding, slopes, pushing) under script control instead of input actions. `jump` is one request, cleared by physics. |
| `engine.emit(kind)` with no detail | It threw (the native side refused an explicit `undefined`). |
| Scenario `field` paths read into JSON text (`ScriptState.data.score`) | Tests assert on game state kept in `self.state`. |
| Scenario `event` assertions take `at_least` / `at_most` | "Five lanterns lit, never caught" is a count. |
| `input` on every observe operation's `scene` target | Observing a played game (events, entities, rays) needs its input replayed. |
| Sensors report contacts with every body type (`ActiveCollisionTypes::all()` on sensors only); character movement ignores sensors | Pickups and checkpoints: a fixed sensor never saw the (kinematic) character, and the character was stopped by it as if it were a wall. Solid pairs are unchanged. |
| A script that changes the `Transform` of a dynamic body or a character teleports it | Respawning at a checkpoint. Physics compares the Transform with what it wrote back last tick; untouched bodies are equal bit for bit, so existing results are unchanged. |
| `Entity.name` in scripts | Telling pickups, hazards and checkpoints apart in a trigger hook. |
| `engine.send(target, kind, detail)` and the `onMessage(self, message)` hook; waiting messages live in the `ScriptInbox` component | Enemies report breaches and projectiles report kills to a director, and a spawner gives a new projectile its heading. Kept as a component, so messages are in the state hash and in snapshots. Delivered at the start of the target's next turn, after contact hooks and before timers: the same tick if the target runs later in id order, otherwise the next tick. |
| `RoundCuboid { half_extents, border }` collider shape (3D and 2D) | Dust Circuit's cars are box-shaped characters. With a sharp `Cuboid` the character controller snagged on the flat ground and moved about a third of the distance it should; a ball moved freely but does not fit a car. Rounding the edges keeps the box's outer size (the border is cut from the inside) and slides cleanly. A new variant, so existing scenes and hashes are unchanged. |
| Mouse movement as `axis:MouseX` / `axis:MouseY` (window: pixels / 100 per tick, clamped to -1..1, for the one tick after the movement, then 0) | A first-person game needs mouse look; only mouse buttons reached the game. A per-tick value fits the action model (scripts scale it by their own sensitivity) and records and replays like any other input. Live behaviour on a desktop is untested in the cloud. |
| Texture option `remove_background` (generation post-processing) | Generated sprites come back on a flat magenta background. The key is the mean of the four corners; colours within 48 (largest channel difference) are transparent, from 112 opaque, linear between, with the key's share taken out of partly transparent pixels. A dark subject blended with magenta stays opaque by that measure, so a 3-pixel band along the cut is also unmixed by its spill (how far the key's strong channels rise over its weak ones; under 1/8 of the key's own is left alone as the subject's colour). Integer only. Rejected: a model-based matting step (heavy, not deterministic across machines); asking providers for transparency (Qwen returns none). Not part of the cache key, so rebuilding is free. |
| Component `ModelPlacement { offset, rotation, scale }` (render only) | Generated models have their origin at their feet, face +X or +Z, and come at their own size, while the games' entities are scaled boxes and capsules centred on their colliders. `ModelPlacement` moves, turns and sizes the drawn model relative to the entity (its scale replaces the entity's scale for drawing) without touching physics, so colliders, gameplay and test outcomes stay as they were. Rejected: changing each entity's scale and collider (changes physics and every play test); a field on `Model` (changes the serialized form, and so the hash, of every existing scene); parenting (G4, larger). Hit flashes and damage moved from `Material` tints to `Emissive`, which keeps the model's textures (G21). |

## Why

- Games written exactly as an agent would write them find the gaps that matter; every gap
  found is fixed here if small, or listed in `docs/ai/PROOF-SCENES.md` if not.
- A recipe plus a byte-identical check is the only way to show "built through operations"
  without trusting the author.
- Tests that play the game and check outcomes (won, lives, laps) prove more than a hash; the
  hash pins determinism on top.

## Rejected

- Hand-written scene files: no proof they can be made through operations.
- One shared mega-scene for all genres: hides which feature each genre needs.
- A Rust test harness that drives each game tick by tick: the scenario format already does it
  and is what an agent uses.

## Not done yet

- 90 frames per second on a mid-range desktop GPU is measured per game with
  `marionette bench` on such a GPU; see `docs/ai/PROOF-SCENES.md` for the numbers taken so far.
- Pooled or recipe-generated assets (W3, W4).
