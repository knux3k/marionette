# 0304: Instances stay in their chunks; sun shadow copies carry only the world matrix

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
- Compact shadow copies. Copies for draws the plain or one-sided depth pipelines draw (no game
  shader, not cut out) go to a second vertex buffer of world matrices (64 bytes each), drawn
  by two pipelines with the same vertex math (`vs_shadow_compact`). Cut-out and game shader
  draws, which read more of the instance, keep full copies in the instance buffer.

Every pass draws the same instances in the same order with the same matrices.

## Proof

Realistic scene, cloud container, lavapipe, 20 frames per run, runs alternated:

- No merge copy: median record 4.60 ms before, 4.23 ms after (6 pairs, all faster).
- Compact copies, on top: median 4.21 to 4.14 ms, inside the cloud's noise. They take about
  0.6 MB a frame off the upload, which matters more on a discrete GPU; kept until the next PC
  run says otherwise (`docs/ai/proofs/performance.md`).

A 1280x720 screenshot is byte-identical to main's. Render tests and the drift hash are
unchanged.
