# 0290: Physics, audio, animation and XR are cargo features; the factory module is an example

Date: 2026-11-14. Status: accepted. Extends 0062; moves the crate of 0026.

## Context

Decision 0062 made import, pool, generation, porting, job TypeScript and later save, nav,
netplay, editor and ui3d optional, but left physics, audio, animation and XR gameplay as fixed
dependencies of `marionette-agent`, and also linked `marionette-factory`, a factory-game
simulation, into every engine. A game that has no physics still built Rapier; a game with its
own factory types collided with the engine's (`FactoryState`, `FactoryMachine` are registered
short names in every scene); and the engine's component reference listed a genre's types as if
they were engine types. An outside review listed this as a modularity gap (focus 1).

## Decision

- **New features on `marionette-agent`**: `physics` (Rapier module, `observe_raycast`),
  `audio` (sound module, `audio_*` and `synth_*` operations, `engine::SoundPlayer`,
  `engine::synth`), `anim` (animation module, `anim_*` operations), `xr` (XR gameplay module;
  needs `physics`). `script` needs `anim` (`playClip`), `nav` needs `physics`. All four are on
  by default, so the default engine, its operation list and every state hash are unchanged.
  `marionette-cli` forwards them; its `xr` feature (OpenXR runtime) turns on the agent's `xr`,
  and the `audio-check` command exists only with `audio`. Without `audio`,
  `engine::SoundPlayer::new` reports that the build has no audio, as on a machine without a
  device, and windowed play runs silent.
- **The factory module moves to `examples/factory`** (package `marionette-example-factory`),
  built the way a game adds its own module (`Engine::builder().module(FactoryModule)`). It
  carries what only it uses: the `perf_factory_scene` operation, the timing budgets
  (`tests/perf_budgets.rs`), the thread-independence and draw-call proofs
  (`tests/perf_proofs.rs`, same recorded hash), its two stability modes in
  `examples/factory/stability.json`, and the `factory-bench` binary (the engine CLI with the
  factory added). CI's performance job runs the budgets from there.
- **`perf_suite` cases can name a generating operation** (`Source::Generated { op, scale }`)
  instead of linking a game's code. The `sim/factory-*` and `render/factory-*` cases keep their
  names, so old reports still compare; an engine without `perf_factory_scene` reports them as
  skipped. Run the full suite with `factory-bench` to include them.

## Alternatives

- A `factory` feature on `marionette-agent`, off by default: smaller diff, but the engine
  would still own a genre's simulation and its names. Rejected.
- Make `input` optional too: actions and the input queue are part of the scene format and of
  recorded replays every mode uses, and the crate is small. Left fixed.
- Splitting `perf_suite` into an engine suite and a factory suite: two report formats and two
  baselines to keep. Rejected in favour of skipping cases whose operation is missing.

## Consequences

- A game picks `default-features = false` and lists the parts it uses; a pure-logic or 2D game
  without sound now builds without Rapier and kira.
- `docs/ai/reference/` no longer lists the factory components or `perf_factory_scene`.
- The factory benchmark numbers in `docs/ai/proofs/performance.md` stay valid: same scene,
  same hash, same module code.
