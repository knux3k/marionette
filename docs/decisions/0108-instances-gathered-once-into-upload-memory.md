# 0108: Instances are gathered once, straight into the upload memory

Date: 2026-11-14. Status: accepted. Part of the optimization pass (decision 0100), focus 3.

## Context

Recording wrote every mesh instance (112 bytes) three times after building it: scattered into
group order, copied again for each sun shadow map it touches (decision 0102), then copied by
`queue.write_buffer` into wgpu's upload memory. On the realistic benchmark scene (28,673
instances, 59,097 places in the buffer with the shadow map copies) those three passes took
about 3 of the 8.5 ms of recording.

## Decision

Only `u32` indices move: `src_of[place]` names the frame-order instance for each place in the
instance buffer, shadow map copies append indices, and the see-through sort reads positions
through `src_of`. `write_gathered` then copies each instance once, in buffer order, into the
memory `queue.write_buffer_with` hands out. Ray-traced frames also read the instances on the
CPU, so only they still gather a vector (`scratch_rt`). The buffer's contents and the draw
calls are the same as before, byte for byte.

## Proof

Realistic scene, cloud container, lavapipe, three alternated runs of 20 frames: median record
8.36 / 8.50 / 8.53 ms before, 6.68 / 8.05 / 7.70 ms after (median of runs 8.50 to 7.70, about
9 percent less). A 160x90 4x MSAA screenshot of the scene is byte-identical before and after,
and the render tests pass with the same reference images.

## Rejected

- Mapping the instance buffer directly (`MAP_WRITE`): needs a ring of buffers and a wait for
  each to be free; `write_buffer_with` already gives upload memory without that.
- Smaller instances (3x4 matrix, no emissive): saves 14 percent of the bytes but changes every
  mesh shader and the ray tracing material read; kept for later if uploads still show.
