# 0176: Games change the running scene

Date: 2026-11-16. Status: accepted.

## Context

A title screen's Continue, Load and New game, a level change and a return to a menu all
replace the world a game runs in with another scene. `Sim::reload` could do it, but only from
outside the tick loop (the agent session and the editor). A game module had no way to ask for
it, so a menu could only quit and relaunch the process, losing the window.

## Decision

- New built-in resource `SceneChange { path }` in `marionette_core::module`. A system sets
  `path` to a scene file (absolute, or relative to the current `SceneDir`).
- `Sim::step` honours it right after the tick: the file loads as `Sim::reload` loads text
  (tick count, RNG state and event log kept; every module's `init` runs again), and its
  folder becomes the new `SceneDir`. The request is emptied either way.
- A file that cannot be read or loaded leaves the world unchanged and records a
  `scene_change_failed` event with the path and the error.
- It lives in `Sim::step`, not in the window, so headless runs, tests, replays and windowed
  play behave the same.

Rejected: a field on `WindowControl` (a window-only, copyable flag set; a scene change must
work headless and in tests too), and a process relaunch (loses the window, the audio device
and the GPU caches).

## Consequences

No scene sets it today, so no state hash changes; the drift hash is unchanged. Whatever a
module keeps outside the world (caches in its own struct) must cope with `init` running again
on a new world, as it already must for `Sim::reload`. Tested headless
(`crates/marionette-core/tests/scene_change.rs`). UNVERIFIED: a change during windowed play on
a real screen (renderer and window hooks are expected to follow the new world as they do for
spawned and despawned entities).
