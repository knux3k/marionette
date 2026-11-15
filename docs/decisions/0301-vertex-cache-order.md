# 0301: Mesh triangles are put in vertex cache order at load

Date: 2026-11-15. Status: accepted. Second optimization block (0300-0309), focus 3.

## Context

After 0300 the realistic benchmark's frame on the reference PC is bound by the GPU (about 6.5
ms against Unity's 4.3), and the main pass is the largest part (3.7 ms). Model triangles came in
the order their files store them, so the GPU transformed many vertices more than once: on the
benchmark's lump meshes, 2.07 vertex shader runs per vertex (ATVR) and 1.12 per triangle (ACMR).

## Decision

`cull::cache_order` reorders each part's indices with meshoptimizer's vertex cache optimizer
when a model is decoded, and `reduced_levels` does the same for each level of detail. The
triangles, vertices and vertex buffers are unchanged; only the order of the triangles in the
index buffer changes. Without the `lod` feature (no meshopt), the order is left as it is.

## Proof

- Lump grid: ATVR 2.065 to 1.457, ACMR 1.115 to 0.787 (29 percent fewer vertex shader runs).
- Reference PC, RTX 2080, 1080p, MSAA 4, same 14.93M triangles and 742 draws, alternated A B
  A B: main pass 3.69 / 3.69 ms before, 3.25 / 3.25 ms after (12 percent less). Sun shadow
  passes unchanged (cascade 3 2.51 to 2.51 ms).
- Two frames in flight, 1000 frames: 6.60 / 7.24 ms (p50 / p99) before, 6.14 / 6.76 ms after.
  Unity: 11.23 / 16.04.
- A 1280x720 screenshot of the scene is byte-identical, the render tests pass with the same
  reference images, and load time shows no measurable change on the PC.
- lavapipe (cloud) shows no gain: its software vertex stage does not cache, so measure this on
  real GPUs.

## Notes

The far shadow cascade did not gain, so it is limited by something other than vertex shading
(rasterising and depth writes over many small triangles are likely). It is the next target.

## Rejected

- Also reordering vertices for fetch locality (`optimize_vertex_fetch`): rewrites the vertex
  buffers and every index, a larger change; try only if vertex fetch shows up.
- Overdraw ordering (`optimize_overdraw`): changes which coplanar triangle wins inside one mesh,
  so it can change pixels.
