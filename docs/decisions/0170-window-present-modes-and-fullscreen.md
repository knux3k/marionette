# 0170: Windowed play picks its present mode, frame queue and fullscreen

Date: 2026-11-13. Status: accepted.

## Context

The window always used `AutoVsync` with wgpu's default frame queue, had no fullscreen, no way
to keep Escape for a game's own menu and no mouse-look cursor lock. Games need to trade input
latency against smoothness per player, and a desktop build is expected to toggle fullscreen.

## Decision

- `WindowOptions` gains `present` (`Vsync` default, `Mailbox`, `Immediate`), `frame_latency`
  (1 to 3, default 2), `fps_cap` (only when not waiting for the display), `fullscreen`,
  `fullscreen_keys`, `escape_exits`, `grab_cursor`, `run_in_background` and `max_seconds`.
  `WindowOptions::new` fills the defaults, so new options never break callers again.
- Present modes fall back in a fixed order (`choose_present`): Mailbox to Fifo (stay
  tear-free), Immediate to Mailbox to Fifo. The mode in use is reported, not assumed.
- Fullscreen is borderless on the window's monitor (no mode switch, fast alt-tab). F11 and
  Alt+Enter toggle it and are not passed to the game; a game can turn the keys off.
- The cursor lock uses `Locked`, falling back to `Confined`, and is released on focus loss.
- CLI: `marionette run` flags `--present`, `--frame-latency`, `--fps-cap`, `--fullscreen`,
  `--keep-escape`, `--grab-cursor`, `--run-in-background`, `--seconds`, `--frame-stats`.

## Consequences

Default behaviour is unchanged apart from a fixed Fifo instead of `AutoVsync` (identical on
every backend that supports vsync). Exclusive fullscreen is not offered: borderless costs
nothing measurable on current Windows compositors with flip-model swapchains, which wgpu uses.
