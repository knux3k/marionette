# 0201: UI input is a pointer hook any source can fill

Date: 2026-11-13. Status: accepted.

## Context

World-space panels must work with controller lasers, fingertips, a mouse ray, scripts and
tests. The OpenXR module is built in parallel and must not be a dependency.

## Decision

- **`UiPointers`** (resource, level state) holds `UiPointer { id, kind, origin, direction,
  pressed, scroll }`. `Ray`: the nearest interactive panel whose front the ray hits receives
  it; `pressed` is the trigger. `Touch`: `origin` is a fingertip; it hovers within 8 cm in
  front of a panel (5 cm behind) and presses at 1 cm or closer, so pushing in clicks.
- **Whoever owns input writes the list each tick** before `Stage::Logic` (an XR module from
  controller poses, a window from the mouse and camera, a test or the `ui3d_point` operation).
- **Interaction** (in `Stage::Logic`): press starts a grab on the item under the pointer; a
  slider follows a held pointer even off the panel (clamped); release over the same item
  clicks a button, flips a toggle or selects a list row; `scroll` over a list moves it.
  Results are changes to item values plus `UiEvents` (`Click`, `Toggle`, `Slide`, `Select`,
  `Scroll`, at most 256 per tick) and, for a button with an `action`, that input action set
  to 1 for the tick.
- Hover and pressed highlights are runtime state, not saved; values are scene state.
- Several pointers work at once (two hands); each has its own grab.

## Alternatives

- Reading controller input in the UI module: couples it to XR. Rejected.
- Events as input actions only: loses which item and value. Kept only for buttons, for
  parity with the 2D `UiButton`.
