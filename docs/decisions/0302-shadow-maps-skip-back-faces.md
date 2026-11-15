# 0302: Shadow maps skip the back faces of one-sided materials

Date: 2026-11-15. Status: accepted (the project owner chose it over keeping shadows
identical). Second optimization block (0300-0309), focus 3.

## Context

After 0301 the far sun cascade of the realistic benchmark costs about 2.5 ms on the reference
PC, most of the GPU gap to Unity. It draws about 13M triangles, front and back faces, and is
limited by how many triangles reach the rasterizer: reordering for vertex reuse (0301) and
leaving out casters per cascade (tried: 59 fewer instances, no gain) did not move it.

## Decision

Sun and local shadow maps draw static parts of one-sided materials with back faces culled
(`shadow_one_sided`), as the lit pass already does. Double-sided materials, cut-outs, skinned
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

## Rejected

- Culling back faces for every material: open double-sided surfaces would lose their shadow
  when the sun is behind them.
- Culling front faces (casting from back faces): 8,747 pixels change, and thin parts leak.
- Lower resolution or level of detail for the far cascade: visible softer or moved edges.
