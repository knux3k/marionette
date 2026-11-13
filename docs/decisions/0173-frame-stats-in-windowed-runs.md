# 0173: Every windowed run measures its frame pacing and input latency

Date: 2026-11-13. Status: accepted.

## Context

Play feel was unmeasured: no frame interval numbers, no count of judder frames, no input
latency. Real-screen checks need numbers an agent can compare between runs and machines.

## Decision

- `FrameStats` (`crates/marionette-render/src/frame_stats.rs`) keeps fixed-size histograms
  (0.05 ms buckets to 200 ms; same memory for any run length) of present-to-present
  interval, main-thread frame work, and input latency, plus ticks per frame (0, 1, 2, 3+),
  skipped frames, resizes, focus losses, the present mode in use, frame queue depth, monitor
  refresh and size.
- Input latency is measured from the OS event reaching the window (or a gamepad poll that
  found input) to the `present` call of the first frame that simulated it. The display queue
  after `present` is invisible from inside the process and is not counted; with vsync it adds
  about `frame_latency` refreshes.
- The first 30 presented frames (`WARMUP_FRAMES`: pipeline compiles, first uploads, the
  window manager's first resizes) are left out of the histograms; their total main-thread
  time is `warmup_ms`. `resizes` counts only real size changes.
- `missed_refreshes` counts intervals longer than 1.5 refresh periods.
- `marionette run` (windowed) prints the summary under `frames`; `--frame-stats <file>`
  also writes it.

## Consequences

Cost per frame is three histogram increments. Numbers from a virtual display only show the
code paths work; real-screen numbers are recorded in `docs/ai/proofs/window.md`.
