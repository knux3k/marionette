# Proofs: game meshes (decision 0180)

- `cargo test -p marionette-render --test game_mesh` (GPU; lavapipe in the container):
  `a_mesh_built_in_code_draws_updates_and_goes_away` (unknown name is `render.asset`, vertex
  colors tint, same-size update in place, resize, removal), `removed_meshes_free_their_gpu_copy`,
  `without_vertex_colors_the_material_color_is_used_and_meshes_cast_shadows`.
- Cost: `cargo test --release -p marionette-render --test game_mesh large_mesh -- --ignored
  --nocapture`. Container, lavapipe: store 0.9 ms, rewrite adds about 1.9 ms to the frame
  (131,072 triangles).
- `cargo test -p marionette-core mesh` (checks, normals, grid, versions).
- Reference images unchanged (`matches_reference_images`) with the new vertex color attribute.
