# 0183: Colliders built from game meshes

Date: 2026-11-15. Status: accepted. Focuses 1 (modularity, custom code) and 3 (performance),
decision 0060.

## Context

Games build terrain and other geometry in code as game meshes (`GameMeshes`, `mesh:<name>`,
decision 0180). A character walking on that terrain needs a collider of the same shape, but
`Collider` `TriMesh` and `ConvexHull` only read model files and primitives, and physics never
looked at a collider again after building it. A game would have to write the mesh to a glTF
file, or approximate the ground with spheres.

## Options

1. A separate `MeshCollider` component. A second way to say the same thing.
2. Let `TriMesh` and `ConvexHull` name a game mesh with the same `mesh:<name>` string models
   use; physics reads it from `GameMeshes` and follows its version.
3. A physics operation that builds a collider from a mesh once. No updates after an edit.

## Decision

Option 2.

- A body whose collider names a game mesh that is not stored yet waits (no error) and is built
  on the first tick the mesh exists: modules often store meshes after physics has loaded.
- `GameMeshes::version(name)` is new; physics keeps the version each such collider was built
  at (outside the saved state) and gives the collider the new shape when the mesh is stored
  again, then wakes sleeping bodies once. A removed mesh leaves the last shape.
- After a snapshot restore the saved shapes are taken as built at the meshes' current versions
  (no rebuild), so a restored run continues exactly.
- 2D physics still refuses `TriMesh` and `ConvexHull`.

Also: `marionette_cli::main_with_args`, so a game binary can add a command of its own that ends
in an engine command (for example writing a scene for a save, then running it windowed).

## Consequences

- Terrain made in code collides exactly as drawn, and digging or building on it updates the
  collider by storing the mesh again.
- The collider copies the vertices once (Rapier keeps its own scaled arrays); the cost is the
  triangle tree build, paid at load and on each edit, not per tick.
- Tests: `marionette-physics` `tests/game_mesh_collider.rs` (wait, stand, rebuild, restore).
