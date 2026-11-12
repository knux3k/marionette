# 0004: Render data components live in core; the renderer is a separate crate

Date: 2026-11-11. Status: accepted.

## Decision

- `Transform`, `LookAt`, `Camera`, `Model`, `Material`, `Sprite`, `SpriteRegion`,
  `DirectionalLight` and the `AmbientLight` resource are plain data in
  `marionette-core::spatial`, registered by `register_builtins`.
- All GPU code lives in `marionette-render` (wgpu, winit, gltf). Core has no graphics dependency.
- Placement rule: an entity's translation is its `Position` if it has one (the simulation moves
  `Position`), otherwise `Transform.translation`. `LookAt` overrides `Transform.rotation`.
- Colors are linear RGBA (as glTF material factors are). Conventions: right-handed, +Y up,
  cameras look down local -Z, meters.

## Why

- A headless server, a validator or a test must be able to load any scene, including ones
  with cameras and sprites, without linking a GPU stack. Data components are not graphics.
- One registry means `marionette schema` describes every type a scene can contain.
- `Position` stays because the plan line 2 state hash for `examples/drift.scene.json` must not
  change. Folding `Position` into `Transform` is possible later with a scene migration.

## Consequences

- Struct fields are all required in scene files (no defaults yet), so a `Transform` is
  written in full. A later decision may add defaulted fields with schema defaults.
