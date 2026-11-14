# 0281: Cut-out and see-through materials from glTF alpha modes; double-sided materials

Date: 2026-11-14. Status: accepted. Focus 4 (graphics), decision 0060.

## Context

glTF `alphaMode`, `alphaCutoff` and `doubleSided` were not read: leaves, fences and hair drew
as solid quads, glass and water could not be drawn, and every surface culled its back faces
(a single leaf plane vanished from behind). The audit of 2026-11-14 listed this as a gap.

## Decision

1. **Import**: `MaterialData` gains `alpha_mode` (`Opaque`, `Mask { cutoff }`, `Blend`) and
   `double_sided`, read from glTF.
2. **Pipeline variants**: lit pipelines exist per variant, bit 0 double-sided (no culling), bit
   1 blended (alpha blending, no depth writes), for static and skinned parts and for the ray
   tracing path. Variants other than the plain one are built the first time a frame draws them
   (`renderer/variants.rs`), so scenes without such materials pay nothing.
3. **Cut-outs** need no variant: the material uniform carries the cutoff (`emissive.w`) and the
   fragment shader discards below it. They cast cut-out shadows through a second shadow
   pipeline that samples the base color. The ray tracing depth prepass leaves them to the lit
   pass. Skinned cut-outs cast solid shadows (not alpha tested yet).
4. **See-through** parts draw after the opaque ones and after the sky, one instance per draw,
   farthest from the first view first (ties keep instance order, so frames stay reproducible).
   They cast no shadows. Blended game shaders draw after them. Skinned see-through parts draw
   in the same phase, unsorted.
5. **`Material` overrides** keep the part's sidedness, have no cut-out, and blend when their
   base color's alpha is below 1 (no example scene used such a color before).
6. The group key of instanced draws includes the look, so instancing, culling and bulk draws
   are unchanged.

## Alternatives

- Alpha to coverage for cut-outs: smoother edges under MSAA, but needs a pipeline per variant
  and does nothing without MSAA; discard plus coverage-preserving mipmaps (decision 0280) keeps
  foliage solid at distance. Can be added as a variant later.
- Order-independent transparency (weighted blended): no sorting, but approximate colors and an
  extra target; per-instance sorting is exact for separate objects and cheap at the counts
  games use.

## Consequences

- New example `examples/materials.scene.json` (built by `materials.ops.json`) with a reference
  image, stability mode `render-3d-materials`, test models made by
  `crates/marionette-render/examples/make_materials.rs`.
- Tests in `crates/marionette-render/tests/materials.rs`: holes show through from both sides
  and in shadows, panes mix and sort back to front whatever their scene order, and the
  reference scene fails when its metallic and roughness channels are swapped or its normal
  map's green channel is flipped (the audit's undetected mutations).
