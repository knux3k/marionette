# Windowed play on a real screen (decisions 0170 to 0173)

How to rerun: build release, then on Windows
`powershell -ExecutionPolicy Bypass -File tools\window-check.ps1 -Exe target\release\marionette.exe -Out <dir>`
(opens six muted 10 s windows of `examples/playground3d.scene.json`; owner's OK needed on a
user's machine). Each run writes `<name>.frames.json` (`FrameStats`), a replayable
`<name>.inputs.json` and `summary.json`. On Linux, `xvfb-run` exercises the same code paths
(software Vulkan; numbers meaningless).

Unit tests: `crates/marionette-render/src/window.rs` (`present_modes_fall_back_in_order`,
`frame_times_snap_to_the_refresh_only_when_close`), `crates/marionette-render/src/frame_stats.rs`,
`crates/marionette-gamepad/src/lib.rs`.

## First run: RTX 2080, Windows 10, one 1920x1080 60 Hz display, release

Commit before the warm-up and minimize fixes (start-up frames still counted). 10 s per run.

| Run | Present mode | Interval mean / p99 / max ms | Frame work mean ms | Input latency mean / max ms | Ticks per frame 0 / 1 / 2 / 3+ |
| --- | --- | --- | --- | --- | --- |
| default | fifo, queue 2 | 16.76 / 24.1 / 66.1 | 16.31 (blocked in vsync) | 17.6 / 42.6 | 8 / 434 / 5 / 1 |
| `--frame-latency 1` | fifo, queue 1 | 16.74 / 17.3 / 66.2 | 16.47 | 16.4 / 17.2 | 4 / 500 / 1 / 1 |
| `--present mailbox` | mailbox | 1.07 / 2.3 / 5.5 | 0.87 | 8.2 / 16.6 | 7635 / 523 / 0 / 0 |
| `--present immediate` | immediate | 1.01 / 2.35 / 6.2 | 0.83 | 8.7 / 17.3 | 8189 / 530 / 0 / 0 |
| immediate, `--fps-cap 144` | immediate | 6.94 / 8.5 / 10.6 | 1.25 | 9.5 / 21.5 | 741 / 528 / 0 / 0 |
| `--fullscreen` | fifo, 1920x1080 | 16.64 / 17.4 / 33.4 | 16.36 | 17.4 / 57.0 | 5 / 501 / 2 / 0 |

- Focus loss: W held, focus moved to the desktop, W released there: the recording shows
  `move_forward` 1 at tick 162 and 0 at tick 175 (released by the engine on focus loss).
  `focus_losses` 2.
- Vsync snapping: 13 of 448 frames ran other than one tick, all at start-up or the focus
  change (the 66 ms max is the first frame).
- Minimized for 2 s: 219 ms of CPU (the loop kept drawing; Windows reports minimizing neither
  as size 0 nor as `Occluded`). Fixed by checking `Window::is_minimized`; not yet confirmed
  (see the second run).
- Exit codes were not read by the script (Windows PowerShell 5.1 needs `$p.Handle`); fixed.
- Offscreen bench of `examples/lighting.scene.json` on the same machine: 0.827 ms avg,
  p99 1.76 ms.
- Not covered: gamepads (none connected), high-refresh displays, the editor and quick
  save/load keys (not on `main` yet).

## Second run: same machine, engine `1ea3ca4`

Warm-up frames now left out (`warmup_ms` 430 to 730 ms for vsync runs, about 30 ms for the
others). All six windows exited with code 0.

| Run | Interval mean / p99 / max ms | Frame work mean ms | missed_refreshes |
| --- | --- | --- | --- |
| default (fifo, queue 2) | 16.67 / 17.3 / 18.6 | 16.47 | 0 |
| `--frame-latency 1` | 16.67 / 17.3 / 17.3 | 16.49 | 0 |
| `--present mailbox` | 0.46 / 0.85 / 2.4 | 0.38 | 0 |
| `--present immediate` | 0.46 / 0.75 / 1.9 | 0.38 | 0 |
| immediate, `--fps-cap 144` | 6.94 / 8.05 / 8.8 | 0.58 | 0 |
| `--fullscreen` | 16.67 / 17.15 / 17.8 | 16.49 | 0 |

- No synthetic input reached the windows: the desktop was in Task View, which kept the
  foreground, so input latency, focus loss, resize and the minimize CPU check were not
  exercised (input counts 0, empty recording). The script now records `foreground` per run
  and says when input did not land.
- Third run (Task View closed): keys reached the mailbox, immediate, capped and fullscreen
  windows (40 events each; input latency mean 8.0 ms mailbox, 9.2 ms immediate, 9.4 ms capped,
  16.5 ms fullscreen vsync), but not the first two windows (vsync, queue depth 1) in either of
  two tries. The script's `foreground` flag did not match delivery, so it now also records
  `recorded_events`. CPU while minimized read 62 and 172 ms per 2 s on two tries.
- Minimized-CPU trace at `a72d4c4` (per-thread CPU over 10 s, starting 2 s after minimizing,
  three runs each of the standard build and a build without `gamepad`): runs 2 and 3 of each
  build used 0 ms, with no thread above 1 ms; the first 2 s straight after minimizing also used
  0 ms. The loop sleeps when minimized; gamepad polling costs nothing there. The first run of each
  freshly built exe kept drawing on the main thread (765 and 125 ms; frame counts show it was
  never minimized): the script's 5 s wait for the window handle ran out while the new exe was
  still starting, so it minimized and focused handle 0. That also explains the first windows
  that missed synthetic input. The script now waits up to 30 s, stops if there is no window,
  and records `minimized` (`IsIconic`) before it measures CPU.
- `resizes` is 2 on a plain windowed run with no resize: Windows sends start-up size
  changes after the first frame. Not yet explained.

## Findings for later work

- Above 60 Hz (mailbox, immediate, high-refresh displays) most frames run 0 ticks and redraw
  the same state: motion stays at the 60 Hz tick rate. Rendering between ticks
  (interpolating transforms by the accumulator fraction) is the fix; not done here.
- Queue depth 1 halves worst-case input latency at no cost for light scenes; the default
  stays 2 because GPU-bound scenes lose throughput with 1.
