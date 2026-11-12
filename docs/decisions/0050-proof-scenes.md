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
