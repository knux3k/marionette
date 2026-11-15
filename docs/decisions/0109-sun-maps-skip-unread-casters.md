# 0109: Sun shadow maps skip casters whose shadow no pixel can read

Date: 2026-11-14. Status: accepted. Part of the optimization pass (decision 0100), focus 3.

## Context

On the RTX 2080 (1080p, 4x MSAA, realistic scene) the farthest sun cascade took 2.9 ms of the
7.0 ms of GPU work, nearly as much as the main pass. Decision 0102 already leaves a caster out
of a map whose volume it misses. But each map's volume is a square around a bounding sphere of
its slice of the view, and much of that square lies beside the view, where no pixel reads it.

## Decision

Per sun map, `cull::Receivers` projects the corners of every view's frustum along the light
into the map's clip xy and takes their convex hull. A pixel can only read the map inside that
hull, widened by the filter taps, the normal offset and one more texel (`pad`). A caster goes
into the map only when its circle in that plane reaches the hull. The test is conservative:
every texel a pixel can read still gets every caster that covers it, so images do not change.
Non-orthographic maps and frames that keep everything (ray tracing) skip the test.

## Proof

Realistic scene: 30,421 shadow instances before, 26,695 after (12 percent fewer), 786 draw calls
down to 742. Screenshots at 160x90 (4x MSAA) and 1280x720 are byte-identical before and after,
and the render tests pass with the same reference images. `cull` unit tests cover the hull and a
caster beside the view. The GPU saving needs the PC run (lavapipe cannot time passes).

## Later

Decision 0300 runs the receiver test only for casters outside every view, and reuses one
projection of the center for all maps. As first merged, the test cost more recording time in
the cloud than it saved.

## Rejected

- Lower detail or fewer casters in the far cascade by size: changes shadow edges.
- Caching the far cascade between frames: wrong with moving casters (the scene has 3,200).
