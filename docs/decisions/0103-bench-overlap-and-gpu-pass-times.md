# 0103: The benchmark can overlap CPU and GPU and time each pass on the GPU

Date: 2026-11-14. Status: accepted. Focus 3 (performance), decision 0060. Part of the
optimization pass (decision 0100).

## Context

`marionette bench` waited for the GPU after every frame, so recording and GPU time added up
(19.3 ms on the realistic scene: 8.0 recording + 10.5 waiting). Unity's player queues frames,
so its CPU and GPU overlap; the two numbers were not the same kind of measurement. And the
bench gave one GPU number per frame, which cannot say which pass to optimize.

## Decision

- `Offscreen::set_frames_in_flight(n)` (1 to 3): a render returns once the frame `n - 1`
  renders back is done (`PollType::Wait` on that submission). 1, the default, is the old
  behavior, so earlier reports stay comparable. `marionette bench --in-flight 2` measures the
  steady frame time of a loop with a frame queue; comparisons against Unity use it.
- `Renderer::set_pass_timing(true)` times every main pass with timestamp queries (sun shadow
  maps one by one, local shadow maps, main, HDR, single-pass stereo, output) where the device
  has `TIMESTAMP_QUERY`; the device asks for that feature when the adapter has it.
  `marionette bench --gpu-passes` reports the median per pass as `gpu_passes_p50_ms` (null
  where unsupported) and forces `--in-flight 1`.
- Timing never feeds the simulation or the picture.

## Limits

- Passes not listed (bloom, render scale resolve inside other passes) count in no label; the
  frame's GPU total is still `gpu_wait` at `--in-flight 1`.
- Timestamps on tiled or software GPUs can be coarse; they are for comparing passes on one
  machine, not across machines.
