# 0185: Skinned game meshes, bent per copy by bulk draws, with game shaders

Date: 2026-11-17. Status: accepted. Focuses 1 (modularity, custom code), 3 (performance) and
4 (graphics), decision 0060.

## Context

Game meshes (decision 0180) were rigid. A game that builds its creatures in code (one
continuous skin per body, posed by its own inverse kinematics instead of animation clips) had
two choices: one rigid mesh per moving part, which shows seams at every joint, or rebuilding
and re-uploading every body every frame on the CPU. glTF models already had GPU skinning
(instanced, one joint palette per character), but only from an animated pose, and only with
the standard shader.

## Decision

1. **Skin on game meshes**: `GameMesh` gains `joints` (four per vertex, below
   `MAX_MESH_JOINTS` = 256) and `weights` (finite, not negative). Both empty: a rigid mesh.
   Positions are the bind pose. `joint_count()` is the highest weighted joint plus one.
2. **Palettes from bulk draws**: `BulkDraw::palettes` holds, for copy `i`, entries
   `i * J .. (i + 1) * J` (`J` = `joint_count()`), each the joint's matrix in the mesh's
   model space times its inverse bind matrix, in the 12-float layout of `transforms`. The game
   works out its own poses (inverse kinematics, procedural motion); the engine only blends four
   matrices per vertex on the GPU. Missing palettes draw the bind pose; an entity `Model`
   drawing a skinned game mesh shows the bind pose. `BulkDraw` derives `Default`, so new
   fields do not break hooks that fill the rest with `..Default::default()`.
3. **Same deforming pass**: skinned game meshes go through the glTF skinning path
   (`skinning.rs`): one instanced draw per (mesh, material, game shader), palettes in the
   frame's storage buffer, shadows from the skinned depth pipeline.
4. **Game shaders on skinned draws**: every game shader also gets a skinned variant
   (`vs_skinned_hooked`): the vertex is skinned in model space, then goes through the game's
   `vertex` function and `surface` function as a rigid one does. The skin buffers share group 3
   with the game material (bindings 6 to 8; wgpu guarantees only four groups). The skinned
   variant is compiled with the shader; if it does not compile, skinned draws keep the
   standard skinned shader and rigid draws are unaffected. This also gives glTF skinned
   models a game shader when their entity has a `ShaderMaterial`.
5. **Shared maths**: the deforming code moved to `shaders/skin_core.wgsl`, used by the
   standard skinned shader and the game variant.

## Limits

- Skinned copies are not culled one by one (as skinned glTF parts); a hook draws only the
  copies it wants (near ones skinned, far ones as a rigid rest mesh).
- In shadow passes a skinned game shader's `vertex` function does not run (the shadow
  follows the skin, not the game's extra displacement); shaders whose `vertex` function moves
  vertices skip the depth prepass, as rigid ones do.
- No stereo (multiview) variant: in a stereo pass skinned game-shaded copies use the standard
  skinned shader.
- No morph targets on game meshes.

## Evidence

`crates/marionette-render/tests/skinned_game_mesh.rs`: palettes move half of a strip, with
the standard shader and with a game shader (its colour on the bent part); an entity shows the
bind pose. `crates/marionette-core/src/mesh.rs` tests the skin checks and `joint_count`.
