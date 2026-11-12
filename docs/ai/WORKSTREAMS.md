# Workstreams and roadmap

Each workstream is one thread of work for one fresh session. Start with START-HERE.md, then the
docs listed. "Owns" lists what the workstream may edit; touching another workstream's files
needs care (small, additive edits only, mentioned in the session log). Shared files that several
workstreams may append to: `crates/marionette-agent/src/lib.rs` (one line per new ops module),
`crates/marionette-agent/src/engine.rs` (one line per new module), root `Cargo.toml` (members),
`docs/ai/STATE.md`, `docs/ai/DECISIONS.md`, `docs/ai/reference/` (regenerated).

## Order and parallelism

| ID | Workstream | Can start | Runs alongside |
| --- | --- | --- | --- |
| W1 | Finish line 6 (input, physics, audio, UI) | now | W2, W5, W7 |
| W2 | Assets A: local import path and Unity importer | now | W1, W5, W7 |
| W3 | Assets B: CC0 pool and CC0 library search | after W2 | W1, W5, W7 |
| W4 | Assets C: generation with Qwen and Tripo | after W3 and W5 | W6, W7 |
| W5 | Providers (line 7) | now | W1, W2, W3, W7 |
| W6 | Assets D: Unreal importer (glTF/FBX exports) | after W2 | W4, W5, W7 |
| W7 | Scripting spike, TypeScript vs Luau (line 4) | deferred until disk space allows a V8 build | any |
| W8 | Six proof scenes (line 9) | after W1 (assets from W3/W4 help) | W6, W7 |
| W9 | Later (line 10): OpenXR, AI while playing, inspector | after W8 | - |
| W0 | Owner: create the engine and assets repositories on GitHub | owner | - |

After W0: push (with the owner's approval), run CI, verify the drift hash on Linux, update the
UNVERIFIED list in STATE.md.

## W1: finish line 6

- Scope: the "Remaining" list in STATE.md, section "Line 6 in progress".
- Read: ARCHITECTURE.md (tick, determinism, hash), decisions 0011 to 0015.
- Owns: `crates/marionette-input/`, `crates/marionette-physics/`, `crates/marionette-audio/`,
  `crates/marionette-render/src/window.rs`, `crates/marionette-cli/src/render.rs`, new example
  scenes and their input files under `examples/`, new tests in `crates/marionette-cli/tests/`.
- Acceptance: replaying a recorded input file gives the original state hash for the 3D and 2D
  examples, twice; the 3D physics example run 1,000 ticks twice gives the same hash; snapshot at
  tick 500, restore and run to 1,000 equals the uninterrupted run; the headless sound log lists
  the expected sounds at the expected ticks; the UI scene matches its reference (done); build,
  test, clippy, fmt pass; drift hash and existing references unchanged.

## W2: assets A, local import path and Unity importer

- Scope: the owned library (per-user folder shared across projects, path from configuration,
  never inside the engine repository), the project's own files, and a Unity importer that reads
  Unity project folders and `.unitypackage` archives (models, textures, materials as far as they
  map to glTF PBR). Follow ASSETS-AND-LICENSES.md exactly.
- Read: ASSETS-AND-LICENSES.md, `docs/DESIGN.md` brief item 8a and plan line 8.
- Owns: new modules in `crates/marionette-assets/` (or a new crate marionette-import if it grows
  large; record the choice), a new ops module in `crates/marionette-agent/src/`.
- Acceptance: importing a sample owned asset records its license; engine-locked content and
  undeclared licenses are refused with reasons in the import report; imported assets are never
  accepted by the pool; operations and tests exist for each path.

## W3: assets B, CC0 pool and library search

- Scope: the CC0 pool (lives in the assets repository; CC0 only, source recorded), search of
  the project files, owned library and pool in that order, then free CC0 libraries online (Poly
  Haven, ambientCG, Kenney, Quaternius) with license and source recorded.
- Owns: the pool and search modules in `crates/marionette-assets/`, matching operations.
- Acceptance: search returns results in the required order; the pool refuses non-CC0 and owned
  assets; every pooled asset has a source record.

## W4: assets C, generation

- Scope: recipes (a generated asset is a recipe plus a cached output keyed by content hash),
  Qwen for images only, Tripo for 3D models, rigging (Tripo auto-rig first for humanoids and
  common animals, agent-scripted Blender otherwise), numeric rig checks, post-processing,
  validation, import.
- Depends on providers (W5). Spending needs the user's approval and a spend cap; a generated
  image is approved before 3D credit is spent.
- Acceptance (plan line 8): three assets rebuilt from recipes, each with license and source.

## W5: providers (line 7)

- Scope: language, image and 3D provider interfaces; Claude, Qwen and Tripo plug-ins behind
  feature flags; one config file, one switch command; keys from environment variables only;
  local models allowed; a spend cap.
- Owns: a new crate marionette-providers, a new ops module in `crates/marionette-agent/src/`.
- Acceptance: one command swaps a provider; the engine builds and all tests pass with no
  provider configured and no network.

## W6: assets D, Unreal importer

- Scope: import from the Unreal Editor's glTF/FBX exports with the same license rules as W2.
- Acceptance: as W2, for an Unreal export.

## W7: scripting spike (line 4, deferred)

- Scope: one-week spike, TypeScript against Luau, measured on startup, call cost and reload
  time; the winner gets typed bindings generated from the schema.
- Acceptance: the numbers, and a script change shows in a running session in under 1 second.

## W8: six proof scenes (line 9)

- Scope: an FPS, a racing game, a platformer, a top-down 3D game, a top-down 2D game and a 2D
  side-scroller, each built through operations only.
- Acceptance per scene: deterministic headless run with scripted input (same hash twice), its
  tests pass twice with identical results, uses pooled CC0 or recipe-generated assets, holds
  90 frames per second on a mid-range desktop GPU.

## W9: later (line 10)

OpenXR module (VR optional, never required; the renderer's per-view design allows it), an
optional AI-while-playing module (off by default, with budgets and a scripted fallback), a
read-only inspector.
