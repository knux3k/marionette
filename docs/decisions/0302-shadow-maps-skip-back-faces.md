# 0302: Shadow maps skip the back faces of one-sided materials

Date: 2026-11-15. Status: accepted (the project owner chose it over keeping shadows
identical). Second optimization block (0300-0309), focus 3.

## Context

After 0301 the far sun cascade of the realistic benchmark costs about 2.5 ms on the reference
PC, most of the GPU gap to Unity. It draws about 13M triangles, front and back faces, and is
limited by how many triangles reach the rasterizer: reordering for vertex reuse (0301) and
leaving out casters per cascade (tried: 59 fewer instances, no gain) did not move it.

## Decision

Sun shadow maps (every cascade) draw static parts of one-sided materials with back faces
culled (`shadow_one_sided`), as the lit pass already does. Point and spot light maps keep both
faces: they are small and close to their casters, where a contact shadow shows most. Double-sided materials, cut-outs, skinned
parts and game shaders still cast from both faces. This is also what Unity does by default.

## Cost

Not byte-identical. On the benchmark scene 778 of 921,600 pixels (1280x720) change: a few thin
light slivers inside contact shadows, where the depth bias of a steep front face used to be
covered by the back face behind it. Accepted by the project owner for the time it saves.
A mirrored instance of a one-sided material casts from its inner faces, as it already draws
them in the lit pass.

## Proof

Realistic scene, cloud container, lavapipe, alternated runs: far cascade 1358 / 1355 ms before,
927 / 883 ms after (about 33 percent less), cascade 2 about 40 percent less. Render tests pass
with their reference images unchanged; the drift hash is unchanged.

Reference PC (RTX 2080, 1080p, MSAA 4, alternated A B A B): far cascade 2.53 to 1.69 ms, cascade
2 0.255 to 0.180 ms, all sun maps 2.83 to 1.91 ms, main pass unchanged. Two frames in flight,
1000 frames: 6.61 / 8.77 ms (p50 / p99) before, 5.98 / 7.72 ms after; the GPU wait is now
0.01 ms, so recording (about 5 ms) bounds the frame again.

## Rejected

- Culling back faces for every material: open double-sided surfaces would lose their shadow
  when the sun is behind them.
- Culling front faces (casting from back faces): 8,747 pixels change, and thin parts leak.
- Lower resolution or level of detail for the far cascade: visible softer or moved edges.
