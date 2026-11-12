# 0027: FBX to glTF conversion inside the importer

Date: 2026-11-12. Status: accepted.

## Context

The renderer loads glTF only. Unity packs (and Unreal exports, workstream W6) are mostly FBX, so
imported models were kept as `source.fbx` with `engine_ready: false` (decision 0018).

## Decision

- **Own reader, no new dependency.** `crates/marionette-import/src/fbx/` reads FBX 2011 and newer
  (file version 7000+), binary (32-bit and 7.5 64-bit node headers, zlib arrays via `flate2`) and
  ASCII, into one node tree (`fbx/tree.rs`), then writes binary glTF (`fbx/mod.rs`). Math is
  `glam` (already in the workspace). Rejected: the Autodesk FBX SDK (proprietary, not
  redistributable under MIT OR Apache-2.0), calling Blender or Assimp at import time (external
  programs, not on every user's machine), `ufbx` (C, needs a C build and bindings),
  `fbxcel`/`fbxcel-dom` (binary only, DOM layer incomplete for skins and animation). Older FBX
  (6.x) is refused with a message asking for a re-export.
- **Coordinates.** The file's axis system (GlobalSettings `UpAxis`, `FrontAxis`, `CoordAxis` and
  signs) and unit (`UnitScaleFactor`, centimeters) are converted to glTF (Y up, +Z front,
  right-handed, meters) by conjugating every node transform, vertex and bind matrix with the
  change of basis. No extra root node; winding is flipped when the basis or a geometric transform
  mirrors.
- **Nodes.** Every `Model` becomes a node. Pivots, offsets and pre/post rotation (when
  `RotationActive`) and the rotation order fold into glTF translation/rotation/scale exactly
  (pivots only move the translation). The geometric transform is baked into the vertices.
  Property templates from `Definitions` are honored.
- **Meshes.** Polygons are fan-triangulated; normals and the first UV set (V flipped) from layer
  0 with all FBX mapping/reference modes; one primitive per material slot; vertices de-duplicated
  by (control point, normal, UV).
- **Materials.** Diffuse color (white factor when a diffuse texture exists, as Unity and Blender
  do), opacity, emissive color times factor, roughness from the Phong exponent
  (`sqrt(2 / (n + 2))`), metallic 0. Diffuse, normal and emissive textures; bump maps and other
  slots are listed as left out.
- **Textures and licenses.** Embedded media first. Otherwise the importer looks in the source:
  the written relative path, beside the model, a `Textures` folder beside it or in a parent
  folder (Unity's habit), then a unique file of that name anywhere in the source. A file from the
  source is bundled into `model.glb` only if it passes the license gate under the same
  declaration as the model; otherwise it is left out with the refusal in the model's notes (it
  is still imported or refused as its own asset). The same rule now applies to the buffers and
  images a `.gltf` file references. PNG is kept, JPEG and TGA are converted to PNG.
- **Skins.** Clusters become joints; a cluster's `Transform` is the inverse bind matrix
  (mesh node space to bone space; `TransformLink` is used only when `Transform` is missing). Up
  to four weights per vertex, largest kept, normalized.
- **Animation.** One glTF animation per animation stack, first layer only. Translation,
  rotation and scale are sampled at the union of the node's key times and interpolated linearly
  (constant keys honored); cubic curves are approximated by their keys, which is exact for
  baked exports (the usual case). Clips start at time 0 (the stack's `LocalStart`).
- **Pipeline.** The import writes `model.glb` (main) and keeps the original as `source.fbx`;
  the record notes summarize the conversion and list everything approximated or left out. If
  conversion fails the old behaviour stays (`source.fbx`, not engine-ready, with the reason).
- **Not converted yet:** blend shapes, vertex colors, extra UV sets, texture tiling/offset,
  extra animation layers, cameras and lights (empty nodes), FBX 6.x. The renderer does not skin
  yet, so skinned models draw in their bind pose.

## Proof

`crates/marionette-import/tests/fbx.rs` with fixtures made for the project in
`crates/marionette-import/tests/fixtures/fbx/` (CC0, generator script beside them). Blender's
own glTF export of the same scenes is the reference: corner positions within 0.1 mm, equal
signed volume (winding) and area for the static crate, the Z-up/meters variant, the FBX 7.5
file from Assimp (against Blender's reading of it) and the skinned, animated column at five
times. ASCII is checked by writing the binary trees as ASCII and converting both. Broken and
truncated files return errors, never panic. The outputs pass the Khronos glTF validator with no
errors (one warning: a skinned mesh node keeps its local transform so non-skinning renderers
show the bind pose).
