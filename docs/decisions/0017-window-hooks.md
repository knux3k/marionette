# 0017: The window hands input and frames to its caller through hooks

Date: 2026-11-12. Status: accepted.

## Decision

- `crates/marionette-render/src/window.rs` defines `WindowHooks` with three methods, all
  optional: `device` (a key or mouse button changed: `key:<KeyCode>`, `mouse:Left`), `action`
  (an action set directly, used for `UiButton` clicks), and `after_ticks` (once per frame after
  the simulation caught up).
- The window turns winit events into input names (`key_name`, `mouse_name`) and finds the
  clicked button (`button_at`: top-most `UiNode` with `UiButton` by layer, then scene id). A
  click sets the button's action to 1 at the next tick to run and back to 0 one tick later.
  Key auto-repeat is ignored; Escape still closes the window.
- `crates/marionette-cli/src/render.rs` implements the hooks: live events go into the same
  input queue as scripts, at tick `sim.tick()` (the next tick to run), through
  `marionette_agent::engine::queue_live`; `after_ticks` plays new `sound` events with
  `marionette_audio::Player` (re-exported as `engine::SoundPlayer`). Problems (an audio device
  that cannot open, a sound that cannot load) are collected and reported in the run's result,
  not fatal.
- `marionette run <scene>` takes `--input` (a script or recording), `--record <file>` (written
  when the window closes) and `--mute` (no audio device). `--input` and `--record` also work
  with `--headless` (they map to `scene_run`).
- The window and `marionette bench` load scenes with every engine module
  (`engine::load_file`), as `scene_run` does. Before this they ran the core systems only, so a
  physics scene did not move in a window.

## Why

The crate map forbids the renderer from depending on input or audio. Hooks keep that rule and
keep one path for live input: a live key press becomes the same queued event a script or a
recording would be, so a windowed session can be recorded and replayed headless with the same
hash. Queuing at the next tick (never the current one) keeps ticks complete and deterministic.

## Not verified

The window has not been opened on a screen yet (owner approval needed, `docs/ai/RULES.md`
rule 3). The hook logic has unit tests (`window::tests`); the input path behind it is covered
by the replay proofs.
