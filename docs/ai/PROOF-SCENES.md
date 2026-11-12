# Proof scenes (plan line 9, W8)

Six small original games that prove the engine end to end, built the way an agent builds a
game: scenes through operations (a recipe in `build.ops.json`), gameplay in TypeScript, tests
that play the game with scripted input. Decision 0050. Proof test:
`crates/marionette-cli/tests/proof_scenes.rs`.

## Games

| Game | Genre | Directory | Status |
| --- | --- | --- | --- |
| Lantern Keeper | top-down 2D | `games/lantern-keeper/` | done: wins with scripted input; idle run loses all lives |
| Ember Run | 2D side-scroller | `games/ember-run/` | done: wins with scripted input (14 of 15 embers, two hits); running into the first gap loses all lives |
| Cloudstep | 3D platformer | `games/cloudstep/` | done: reaches the beacon in about 11 s with scripted input, no falls; walking straight ahead falls again and again |
| Crystal Ward | top-down 3D | `games/crystal-ward/` | done: survives five waves with scripted input (crystal at 5 of 10); idle loses in wave 3 |
| Dust Circuit | racing | `games/dust-circuit/` | done: three laps against an AI rival; the scripted drive (analog steering) wins in about 24.6 s with no wall contact; idle loses to the rival |
| Signal Yard | first-person shooter | `games/signal-yard/` | done: an agent played it through the MCP server (closed loop, aim speed capped) and its recording clears three drone waves in about 24.6 s, taking 3 hits; idle loses to wave 1 |

## Recipe for a new proof game

1. `marionette op project_init '{"path": "games/<name>"}'`, keep `marionette.json`, remove the
   template scene and test.
2. Write `build.ops.json` (format `marionette.ops`): `scene_create`, `resource_set`
   (`FixedStep`, `InputConfig`, `PhysicsSettings`, `AmbientLight`), `entity_create` per entity,
   last `scene_validate`. Run it: `marionette --project games/<name> op batch_run '{"file": "build.ops.json"}'`.
   To change the scene later, change the recipe and rebuild (delete the scene file first), or
   run edit operations and add them to the recipe.
3. Gameplay in `scenes/scripts/*.ts`; keep changing state in `self.state`.
4. Play it: write `tests/play.inputs.json`, check what happened with `observe_events`
   (`"input": "tests/play.inputs.json"`), then write `tests/*.test.json` with event counts,
   `ScriptState.data.*` fields and the `state_hash`.
5. Add the name to `GAMES` in `proof_scenes.rs`, a row above, and a stability mode.

## Generated assets

Each game has generation recipes in `assets/generated/` and a `providers.json` choosing Qwen
for images and Tripo for models (decision 0045, `docs/ai/GENERATION.md`). The paid run happens
on the machine that holds the keys, and every 3D model waits for a person to approve its
reference image. Every recipe is built and every game uses its outputs. Models are drawn with
`ModelPlacement` (offset to the collider's bottom, turned to face -Z, sized to the old box), so
colliders and gameplay are unchanged.

| Game | Recipes | Use once built |
| --- | --- | --- |
| Crystal Ward | `warden`, `crawler`, `crystal`, `pillar`, `ward_wall` (models, each from a `*_ref` image) | In use: warden, spawned crawlers (glow hotter each wave), crystal (glow fades with damage), pillars; each arena wall is an invisible collider shown by five stone wall segments. |
| Dust Circuit | `car`, `rival_car`, `tyre_stack`, `grandstand`, `marker_tower` (models from `*_ref` images) | In use: both cars, marker towers, two grandstands per stand, tyre stacks inside the curves. |
| Signal Yard | `drone`, `crate`, `pylon`, `relay` (models from `*_ref` images) | In use: spawned drones (hits flash through `Emissive`), crates, pylons and the relay mast. |
| Cloudstep | `hero`, `island`, `seed`, `beacon`, `spring` (models from `*_ref` images) | In use: the hero, islands (also the stepping stones and the ferry, scaled down), seeds, the spring and the beacon. |
| Ember Run | `runner`, `ember` (sprites), `rooftops` (background) | In use: runner and embers (`Sprite.image` plus a `SpriteRegion` cropping each to its subject), and a `sky` backdrop the level script drifts with the camera. |
| Lantern Keeper | `keeper`, `lantern`, `ghost` (sprites), `hedge` (tileable texture) | In use: the keeper, lanterns and ghosts (cropped with `SpriteRegion`) and hedge walls. A lit lantern also gets a warm glow sprite on its tile. |

Estimate from `generate_plan`: 26 images at about $0.05 each and 19 models at about $0.40 each
(about $1.30 and $7.60). The owner lifted the Qwen and Tripo caps (the account balance is the
limit), so props are generated too, not only the characters. The swap is a build-recipe change per game (plus scripts that
spawn models), and the pinned state hashes change with it.

## Engine gaps found

Fixed while building (decision 0050 lists why): operations for scene resources; `batch_run`;
`engine.raycast`; characters face their rotation; `CharacterInput`; `engine.emit` without a
detail threw; scenario paths into `ScriptState.data`; event counts in assertions; `input` on
observe operations; sensors report every body type and do not block characters; scripts
teleport bodies through `Transform`; script messages (`engine.send`); the `RoundCuboid`
collider (a sharp `Cuboid` character snags on flat ground and covers about a third of its
distance; Dust Circuit's cars use rounded boxes); mouse look (`axis:MouseX` / `axis:MouseY` from the
window).

Open (not fixed here; each names the cheapest fix seen):

| # | Gap | Seen in | Possible fix |
| --- | --- | --- | --- |
| G1 | No tile map: a tile game spawns one sprite entity per wall (about 100 in Lantern Keeper). | Lantern Keeper | A `TileMap` component (text rows plus a palette) drawn as one batch. |
| G2 | A script cannot read another entity's `self.state`: `engine.find(..).state` is empty; the only way is parsing `get("ScriptState").data`. Messages (`engine.send`) now cover writes. | Lantern Keeper, Ember Run and Cloudstep HUDs | Make `Entity.state` of other entities a read-only parsed copy. |
| G3 | `entity.set` replaces the whole component; changing one sprite color repeats image, size and layer. | Lantern Keeper | `entity.patch(name, partial)` merging fields. |
| G4 | No parent/child transforms; a camera that follows a body is moved by a script, which runs before physics, so it trails by one tick. | all 3D games | An `Attach { target, offset }` component applied after physics, or a late script hook. |
| G5 | `marionette screenshot` and `run` (window) take no `--input`; `observe_screenshot` does. | Lantern Keeper | Add `--input` to the CLI command. |
| G6 | Fixed: every game uses its generated images or models. Generated sprites come on a padded square canvas, so each needs a hand-measured `SpriteRegion`, and each model needs a hand-measured `ModelPlacement`. | all | `generate_run` could report each sprite's opaque bounds and each model's size and facing for these components. |
| G7 | A character standing on a moving kinematic platform is not carried; the platform slides out from under it. | Ember Run (the lift is a stepping stone) | Add the ground body's velocity to the character's movement in the controller. |
| G8 | Colors are linear; values picked by eye come out pale on screen (0.2 looks like 0.5). | all | Document it in the component description, or accept sRGB colors in `Sprite`/`Material`/`UiText` behind a flag. |
| G9 | No shadows; in a 3D platformer it is hard to judge where a jump lands. | Cloudstep | A shadow map for the first directional light (or a blob shadow under characters). |
| G10 | Built-in meshes are cube, sphere and plane only; the hero is a stretched sphere over a capsule collider. | Cloudstep | Add `primitive:capsule` and `primitive:cylinder` to match the collider shapes. |
| G11 | A character carried sideways by a moving platform keeps the drift after it leaves (contact pushes it); jumps need correcting. | Cloudstep | Part of G7. |
| G12 | A spawned entity cannot be given starting script state; its first `start` sees `{}` (Crystal Ward sends bolts their heading as a message). | Crystal Ward | An optional `state` argument to `engine.spawn`. |
| G13 | Script files cannot `import` each other; small helpers (yaw to quaternion) are copied into each script. | Crystal Ward | Module imports between scripts in the same project. |
| G14 | A character with a sharp `Cuboid` collider drags on flat ground (Rapier's character controller keeps hitting the ground face). `RoundCuboid` fixes it, but nothing warns about the plain box. | Dust Circuit | `scene_validate` warns when a `CharacterController` entity has a `Cuboid` collider. |
| G15 | Curves are built from many straight boxes (48 wall segments in Dust Circuit); there is no curved or spline mesh, and the track surface is the ground. | Dust Circuit | A swept-profile mesh along a path (walls, roads, rails) built from a recipe operation. |
| G16 | A car is one box: without parenting (G4) wheels and a cabin would each need a script copying the body's pose, one tick late. | Dust Circuit | Part of G4; or a generated car model (G6). |
| G17 | The window does not lock or hide the cursor, and live gamepads are not read at all (`pad:` and stick `axis:` inputs only come from input files). Mouse look works but the cursor can leave the window. | Signal Yard | Cursor lock while a scene binds `axis:MouseX`/`axis:MouseY`; a gamepad backend (gilrs) feeding the same input names. |
| G18 | No point or spot lights and no emissive materials: tracers, orbs and the relay beacon are only bright colors, and a night scene cannot be lit locally. | Signal Yard | `PointLight` component and an `emissive` color on `Material`. |
| G19 | No hit feedback beyond what scripts fake: no decals, particles or screen flash. | Signal Yard | A small particle burst component (count, speed, lifetime, color) and a UI flash helper. |
| G20 | Fixed: `texture.remove_background` keys out the flat background and removes the tinted fringe; the 2D sprite recipes use it. | Ember Run, Lantern Keeper | Done. |
| G21 | `Material` overrides a whole model's colour and `Material` has no texture, so a generated model loses its own look as soon as a script tints it (hit flashes, team colours), and primitives cannot be textured. | Signal Yard, Dust Circuit (worked around: flashes use `Emissive`) | A `tint` multiplier that keeps the model's textures, and a `base_color_texture` on `Material`. |
