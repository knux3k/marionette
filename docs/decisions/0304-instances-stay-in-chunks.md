# 0304: Instances stay in their chunks (compact shadow copies tried and dropped)

Date: 2026-11-15. Status: accepted. Second optimization block (0300-0309), focus 3.

## Context

After 0303 the realistic benchmark is still bound by recording on the reference PC (about
4.6 ms). Two copies remained on that path:

- The prepare chunks (0300, 0303) were appended into one array before the upload gathered
  them again in buffer order (0108), so every static instance (112 bytes) was copied twice.
- A sun map whose casters in a draw are not one unbroken run gets copies of them after the main
  instances (0102). Those copies were full instances, though the depth pipelines that draw
  them read only the world matrix.

## Decision

- No merge copy. A buffer place names its instance by one `u32`: the source in the top 5 bits
  (a chunk, or the bulk draws' array last) and the index in that source below (`Sources`). The
  upload reads each instance from where it was prepared. 16 chunks and the bulk array fit in
  5 bits (a compile-time check); 27 index bits are more instances than any device buffer holds.

Every pass draws the same instances in the same order with the same matrices.

## Proof

Realistic scene, cloud container, lavapipe, 20 frames per run, runs alternated:

- No merge copy: median record 4.60 ms before, 4.23 ms after (6 pairs, all faster).
- RTX 2080, 1920x1080, MSAA 4, 2 frames in flight, three alternated runs of 1000 frames:
  recording 4.14 to 3.61 ms; frame p50 5.72 to 5.61 ms, p99 6.79-8.25 to 6.67-6.68 ms. The
  frame is now bound by the GPU (about 5.5 ms), so recording saved shows up as GPU wait.

A 1280x720 screenshot is byte-identical to main's. Render tests and the drift hash are
unchanged.

## Rejected

- Compact sun shadow copies (only the world matrix, 64 bytes instead of 112, in a second buffer
  with two pipelines that read it): 0.07 ms in the cloud (inside noise) and nothing measurable
  on the RTX 2080 against this change alone, CPU or GPU. Removed: two pipelines and a buffer
  for no gain.
