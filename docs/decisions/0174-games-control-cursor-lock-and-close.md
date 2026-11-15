# 0174: Games control the cursor lock and closing the window; focus is an input

Date: 2026-11-15. Status: accepted.

## Context

The cursor lock was a window option fixed for the whole run (`--grab-cursor`, decision 0170).
A first-person game frees the mouse while a pause menu is open and locks it again on resume,
needs a menu button that quits, and usually pauses or shows a "click to play" card when the
window loses focus. None of that could be done from a scene or a game module.

## Decision

- New built-in resource `WindowControl { lock_cursor, close }` in `marionette_core::ui`. The
  window reads it once per frame after the ticks: with it, `lock_cursor` decides the lock
  (the `grab_cursor` option is only the default for scenes without it); `close` closes the
  window after that frame. The lock is still never held while another window has focus.
- The window feeds focus changes as the device input `window:Focus` (1 focused, 0 not). It
  goes through the same queue as keys, so recordings and replays carry it and a tick never
  reads wall-clock or OS state directly. Action maps accept the `window:` prefix.
- While the cursor is locked, left clicks are not `UiButton` presses (a locked cursor has no
  meaningful screen position).

## Consequences

Headless runs ignore `WindowControl`, so it changes no state hash and no test result; the
drift hash is unchanged. A game writes the resource from a system (or a scene sets it), so
menus stay ordinary game code. Mouse movement still feeds `axis:MouseX/Y` while the cursor is
free; a game that should not turn the camera in a menu ignores the axes while paused.
