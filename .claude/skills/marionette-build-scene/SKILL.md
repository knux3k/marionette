---
name: marionette-build-scene
description: Create or change a Marionette scene (entities, components, resources, input actions) through validated operations, then run and look at it. Use when adding objects, a level, a camera, physics bodies, UI, sounds or controls to a game built on Marionette, or when a scene edit is refused.
---

# Build or change a scene

Never hand-edit `*.scene.json`. Every edit goes through an operation, which checks it against
the schema, writes canonical JSON and returns a diff. A refused edit changes nothing and
returns diagnostics (code, JSON pointer, entity, suggested fix): apply the fix and retry.

Run operations as `marionette op <name> '<json>' --json`, or as MCP tools of the same name.

## Loop

1. `project_validate` before and after a batch of edits.
2. Look up a type before using it: `project_schema` with `{"types": ["Collider"]}` returns its
   JSON Schema. All fields are required; references to other entities are entity ids.
3. Edit:
   - `scene_create` (`template`: `empty` or `basic` with camera, light, ground).
   - `entity_create` with `components`, `entity_update` to set and remove several at once,
     `component_set`, `component_remove`, `entity_delete`.
   - `resource_set` for scene-wide settings (`FixedStep`, `PhysicsSettings`, `AmbientLight`).
4. Run: `scene_run` gives a state hash for scene, seed and ticks. For repeated looks keep a
   world loaded: `session_start`, then `session_step`; edits with `session` instead of `scene`
   change only the live world, and `session_save` writes it back.
5. Observe: `observe_tree`, `observe_query`, `observe_entity`, `observe_events`,
   `observe_raycast`, and `observe_screenshot` (returns a PNG path; open the file only when
   the picture is the question). Lists are paged: pass `offset` for more.

Many edits at once: write them as a `*.ops.json` file and run `batch_run`. The file is also
the recipe that rebuilds the scene, so keep it beside the scene.

## Common pieces

| Need | Components |
| --- | --- |
| Something visible in 3D | `Transform`, `Model` (`primitive:cube`, `primitive:sphere`, `primitive:plane` or a glTF path), optional `Material` |
| Glass, leaves, fences | glTF `alphaMode` `BLEND` (see-through) or `MASK` (cut-out), `doubleSided` for both faces; or a `Material` whose `base_color` alpha is below 1 (see-through) |
| Something visible in 2D | `Transform`, `Sprite` |
| A solid object | `Collider`; add `RigidBody` to make it move |
| A player | `CharacterController` plus the actions it names |
| Controls | resource `InputConfig` naming an action map file (`*.actions.json`); scripts read actions, never keys; gamepads bind as `pad:South`, `axis:LeftStickX` (up and right +) |
| Sound | `AudioSource` (heard through an `AudioListener`) |
| Screen UI | `UiNode` with `UiRect`, `UiText`, `UiImage`, `UiButton` |
| Behaviour | `Script` (skill `marionette-write-script`) |

Paths inside components are relative to the scene file.

## Checks before you finish

- `project_validate` is clean.
- The same scene, seed and ticks give the same `scene_run` hash twice.
- A scenario covers what you built (skill `marionette-test-game`).
