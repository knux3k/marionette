# Proofs: game meshes and shaders (decisions 0180, 0181)

## Meshes (0180)

- `cargo test -p marionette-render --test game_mesh` (GPU; lavapipe in the container):
  `a_mesh_built_in_code_draws_updates_and_goes_away` (unknown name is `render.asset`, vertex
  colors tint, same-size update in place, resize, removal), `removed_meshes_free_their_gpu_copy`,
  `without_vertex_colors_the_material_color_is_used_and_meshes_cast_shadows`.
- Cost: `cargo test --release -p marionette-render --test game_mesh large_mesh -- --ignored
  --nocapture`. Container, lavapipe: store 0.9 ms, rewrite adds about 1.9 ms to the frame
  (131,072 triangles).
- `cargo test -p marionette-core mesh` (checks, normals, grid, versions).
- Reference images unchanged (`matches_reference_images`) with the new vertex color attribute.

## Shaders (0181)

- `cargo test -p marionette-render --test game_shader` (GPU): surface shader keeps engine
  lighting and receives shadows; `vertex` moves the mesh; additive unlit draws after opaque;
  unknown name, compile error and unknown option are `render.shader`; file shader with a
  texture; game time animates; bulk draws take a shader.
- Example `examples/game-shader.scene.json` (built by `examples/game-shader.ops.json`),
  reference `examples/reference/game-shader.png`, stability mode `render-3d-game-shaders`.
- Existing pictures unchanged: every reference image regenerated on lavapipe before and after
  the shader changes, compared byte for byte, identical.
- `cargo test -p marionette-core shader` (options, `has_fn`, stored sources).
