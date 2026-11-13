---
name: engine-add-scene-or-mode
description: Add an example scene, proof scene or game mode to the Marionette engine repository with its stability mode, reference image and proof entry. Use when adding or renaming a .scene.json file or a reference image, or when stability or render tests fail on an unregistered scene.
---

# Add a scene or game mode (engine work)

A scene in the engine repository is not done until the suite knows it. The test
`every_scene_has_a_stability_mode` fails otherwise.

## Steps

1. Build the scene from an operation recipe (`examples/swarm.ops.json` is a model): write
   `<name>.ops.json` and run it with
   `cargo run -q -p marionette-cli -- op batch_run '{"file": "examples/<name>.ops.json"}'`.
   Only CC0 or self-made assets; list each file in `examples/assets/LICENSE.md`.
2. Canonical form: `cargo run -q -p marionette-cli -- scene fmt examples/<name>.scene.json`.
3. Add a mode to `stability/suite.json`: name, description, kind (`sim`, `render` or
   `agent`), scene, `random_input` when it has an action map, `fast_ticks`, `long_ticks`,
   budgets. Fields and failure kinds: `docs/ai/STABILITY.md` "What one mode run checks".
4. Run it: `cargo run --release -q -p marionette-cli -- stability --mode <name>`, then
   again with `--long --seeds 1`. Fix every failure (`docs/ai/STABILITY.md`
   "Fixing a failure (for an agent)").
5. Reference image, for a scene that renders: add the name to the lists in
   `crates/marionette-render/tests/render.rs`, create it with
   `MARIONETTE_UPDATE_REFERENCES=1 cargo test -p marionette-render --test render matches_reference`,
   look at the PNG, and check `git status` shows no other reference changed.
6. Add the name to `every_example_is_valid_and_canonical` in
   `crates/marionette-cli/tests/cli.rs`.
7. Record the state hash and the image hash in the area file under `docs/ai/proofs/`.

## Traps

- A changed hash or reference image of an existing scene must be said in the commit and in
  `docs/ai/PROOFS.md`, with the reason. The drift hash in `CLAUDE.md` never changes.
- Render modes are skipped without a GPU; frame budgets are judged only on a hardware GPU.
- Opening a window or playing sound on someone's machine needs the owner's yes.

Finish with skill `engine-finish-change`.
