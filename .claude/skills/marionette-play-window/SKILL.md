---
name: marionette-play-window
description: Play a scene in a real window and make it feel right - present mode, frame queue, frame cap, fullscreen, mouse look, gamepads - and measure frame pacing and input latency. Use when a game should run on a screen, feels laggy or juddery, needs controller or fullscreen support, or a windowed run needs numbers.
---

# Play in a window and measure it

Opening a window or sound on a person's machine needs their OK first.

## Run

`marionette run <scene> --seconds 10 --mute --frame-stats frames.json`

- Without `--seconds` or `--ticks` it runs until closed (Escape, or the close button).
- `--keep-escape` gives Escape to the game (bind `key:Escape` in the action map for a menu).
- `--grab-cursor` hides and locks the cursor for mouse look; it is freed on focus loss.
- `--fullscreen` starts borderless fullscreen; F11 or Alt+Enter toggles it any time.
- `--run-in-background` keeps simulating while minimized (default: pause).
- `--record` writes every input change; `--input` replays it exactly, with no window needed.

## Feel

| Want | Flags |
| --- | --- |
| No tearing, default | `--present vsync` |
| Lower input latency with vsync | `--frame-latency 1` (light scenes; GPU-bound scenes may lose frames) |
| Lowest latency, tearing allowed | `--present immediate --fps-cap 240` |
| Fast but tear-free | `--present mailbox` |

A mode the driver lacks falls back; `present_mode` in the output says which one ran.

## Controllers

Connected gamepads feed `pad:<Button>` (0..1, triggers analog: `pad:LeftTrigger2`) and
`axis:<Axis>` (-1..1, stick up and right +: `axis:LeftStickX`, `axis:LeftStickY`). Bind them
in the action map next to the keys. All pads drive one player. A build without the `gamepad`
cargo feature has none.

## Read the numbers

The run prints them under `frames` (same as `--frame-stats`):

- `interval`: time between presents (16.7 ms at 60 Hz vsync); `missed_refreshes` counts
  intervals over 1.5 refreshes.
- `cpu`: main-thread work per frame (with vsync it includes the wait for the display).
- `input_latency`: OS event to the present of the first frame that simulated it. The display
  adds about `frame_latency` refreshes on top.
- `ticks_per_frame`: with refresh equal to the tick rate, anything outside `1` is judder.
  Above the tick rate most frames run 0 ticks and show the same state.
- `warmup_ms`: the first 30 frames, kept out of the histograms.

Compare runs of the same scene, size and display only; close other GPU work first.
