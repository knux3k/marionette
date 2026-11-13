# 0172: Gamepads through gilrs, in their own crate

Date: 2026-11-13. Status: accepted.

## Context

Action maps already accepted `pad:` and `axis:` inputs, but nothing produced them in windowed
play. Desktop and handheld builds need controllers.

## Decision

- New crate `marionette-gamepad` wraps gilrs 0.11 (MIT OR Apache-2.0; Windows Gaming Input on
  Windows, evdev through udev on Linux). It returns device inputs named after gilrs:
  `pad:<Button>` (0..1, analog for triggers) and `axis:<Axis>` (-1..1, stick up and right +).
  Axis values are rounded to 1/256 so stick noise does not flood the input queue; repeats
  are dropped; unplugging a pad releases its inputs.
- All pads feed the same names (one local player). `poll_by_pad` gives per-pad changes for
  games that want local multiplayer.
- The render crate stays free of it: the CLI's window hooks poll pads in the new
  `WindowHooks::before_ticks` and queue the inputs for the next tick, like keys.
- The CLI feature `gamepad` is on by default; a build without libudev on Linux turns it off.
  When the OS backend cannot start, the run reports a problem and continues without pads.

## Consequences

Linux builders need `libudev-dev` (CI installs it). Rumble and per-pad mapping overrides are
not exposed yet.
