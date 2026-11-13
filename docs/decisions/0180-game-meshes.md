# 0180: Meshes built in game code (`GameMeshes`, `mesh:<name>` assets)

Date: 2026-11-13. Status: accepted. Focuses 1 (modularity, custom code), 3 (performance) and
4 (graphics), decision 0060.

## Context

Models came only from files (glTF) or three built-in shapes. Games that generate terrain,
planets, caves, roads or props procedurally had no way to hand the renderer a mesh made in
Rust, let alone change it while the game runs. Workarounds (writing a glTF file and loading
it) would break the no-file-reads-during-a-tick rule and cost a decode per change.

## Decision

1. **Store**: resource `marionette_core::mesh::GameMeshes`, name to `Arc<GameMesh>` plus a
   version number. `GameMesh` is an indexed triangle list: positions and indices required;
   normals (computed smooth and area-weighted when empty, once at `set`), UVs (zero when
   empty), vertex colors (white when empty) and up to two game-made reduced levels of detail
   (index lists over the same vertices). `set` checks sizes and index ranges and returns the
   problem as text. `GameMesh::grid` makes a height-field patch to start from.
2. **Drawing**: any `Model` or `BulkDraw` whose asset is `mesh:<name>` draws the mesh. The
   renderer turns it into an ordinary one-part GPU model, so instancing, frustum culling,
   level of detail (decision 0080), shadows, lights, `Material` overrides and bulk draws all
   apply without new code paths. An unknown name is an error as data (`render.asset`).
3. **Updates**: the renderer re-uploads a mesh when its version changes. A new version with
   the same buffer sizes is written into the existing buffers (`queue.write_buffer`), so a
   game can rewrite a patch every frame without allocating. Removed meshes free their GPU
   copy on the next frame.
4. **Not simulation state**: like bulk draw hooks, game meshes are render data. They are not
   in scenes, snapshots or the state hash; a game rebuilds them from its own saved state
   (seed, heightmap) when a scene loads. The frame copy is one reference count (the map is an
   `Arc`, copied on write only while a frame still holds the old one).
5. **Vertex colors for every mesh**: `MeshVertex` gains an `Unorm8x4` color (36 bytes per
   vertex, was 32). The shader multiplies it into the base color only when the instance's
   `params.w` bit 2 is set (bit 1 is the old normal-map flag), so existing models and
   reference images are unchanged: interpolating a constant 1.0 is not guaranteed to be
   exactly 1.0 on every GPU, so the multiply is skipped rather than relied on. Custom shaders
   (decision 0181) may read the color as four free numbers, such as terrain layer weights.
   glTF `COLOR_0` is not read yet (it would change the look of existing models).

## Alternatives

- **A new draw path just for procedural meshes**: duplicate culling, shadows and instancing;
  rejected (modularity, and every future renderer feature would need doing twice).
- **Meshes as scene components**: would put megabytes of vertices in scene text, the state
  hash and snapshots; rejected (tokens and performance). Scenes keep naming meshes by
  `mesh:<name>`.
- **A second vertex buffer for colors only on game meshes**: saves 4 bytes per vertex on
  glTF models but needs two pipeline layouts for every mesh pipeline; rejected for now.

## Consequences

- Measured (Linux container, Mesa lavapipe, release): storing a 256x256-cell patch (66,049
  vertices, 131,072 triangles) with normal computation takes 0.9 ms; the frame that writes the
  new version into the existing buffers costs about 1.9 ms more than one that does not
  (`large_mesh_rewrite_cost`, ignored test). Hardware GPU numbers: UNVERIFIED.
- Automatic reduced levels (meshoptimizer) run at upload; for meshes that change often a
  game should give its own levels, or rewrites pay for simplification each time.
- Physics colliders from game meshes and a TypeScript API to build meshes are open.
