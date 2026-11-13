# 0171: Focus loss releases input, hidden windows sleep, vsync frame times snap

Date: 2026-11-13. Status: accepted.

## Context

Three faults of the window loop that a virtual display never shows: a key held while the
player alt-tabs stays pressed forever (its release goes to another window); a minimized
window kept requesting redraws, spinning a core; and with a display refresh equal to the tick
rate, timer noise made some frames run 0 ticks and the next 2 (visible judder).

## Decision

- The window tracks the keys and mouse buttons it saw pressed. On focus loss it sends a 0
  for each through `WindowHooks::device` (same tick queue as live input, so recordings replay
  the release) and calls the new `WindowHooks::focus`, where hooks release what they own
  (gamepads, decision 0172).
- Minimized (zero size) or `Occluded`: no frames, `ControlFlow::Wait`. The simulation pauses
  and does not owe the hidden time on return, unless `run_in_background`, which keeps ticking
  at the tick rate without drawing.
- Under a vsync present mode, a measured frame time within 5% of the monitor's reported
  refresh period is snapped to it (`snap_to_refresh`) before it feeds the fixed-step
  accumulator. Longer frames are not touched, so a missed refresh is still caught up.

## Consequences

Simulation results never depend on any of this: wall time only decides how many ticks run
per frame (ARCHITECTURE.md). Snapping can drift from wall time by the error of the reported
refresh rate (for 59.94 Hz reported as 60, 0.1%); the 8-tick catch-up cap bounds it.
