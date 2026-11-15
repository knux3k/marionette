# 0300: Mesh instances are prepared on several threads

Date: 2026-11-15. Status: accepted. Second optimization block (0300-0309), focus 3.

## Context

With two frames in flight, the realistic benchmark's frame on the reference PC costs what
recording costs (about 9.5 ms). About half of recording was one loop over the mesh draws:
for each part, the world matrix, bounding sphere, culling class, level of detail, sun map bits,
material, look and draw group. The loop reads only the frame and the uploaded models, so each
draw can be done on its own.

## Decision

`Renderer::record` splits `FrameData::meshes` into up to `PREPARE_CHUNKS` (8) chunks of at
least `PREPARE_MIN_DRAWS` (256) draws and prepares them with `std::thread::scope`, the calling
thread taking the first. Each chunk fills its own reused arrays (`Prepared`, kept in
`scratch_chunks`) and numbers its own draw groups. The chunks then run in order on the calling
thread: each chunk's groups are mapped to the frame's groups, and its instances, group ids and
culling bits are appended. The instance order, groups and draw calls are the same as one pass
in draw order, so frames stay reproducible. A chunk works on a local copy of its arrays and
stores them back at the end. Pushing through the shared slice made neighbouring chunks
contend for the same cache lines, which cost half the gain.

The same change makes decision 0109's receiver test run only for casters outside every view.
A caster inside a view throws its shadow where that view reads the map, so the test could only
keep it. The test also reuses one projection of the center for every map. In the cloud this
removed most of the recording time 0109 had added, with the same shadow instance count.

Rendering is not simulation, so these threads are outside the determinism rules for systems.
The output does not depend on thread timing.

## Proof

- Realistic scene, cloud container (4 cores), lavapipe. Mesh loop: 1.8 to 4.3 ms on one thread
  vs about 1.3 to 1.8 ms on the threads, plus 0.5 ms to merge. Three alternated runs of 20
  frames: median record 5.70 / 6.50 / 5.40 ms before vs 5.22 / 5.58 / 5.32 ms after.
- A 1280x720 screenshot of the scene is byte-identical before and after.
- Render, stability and unit tests pass.
- The PC has 8 cores, so the gain there should be larger. The final benchmark will measure it.

## Rejected

- rayon: a new dependency for the render crate, for one loop that scoped threads handle.
- Grouping on the calling thread: about 1 ms of hash lookups. Numbering groups per chunk
  leaves a few hundred lookups.
