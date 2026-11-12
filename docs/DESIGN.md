# Marionette: design baseline

Confirmed by the project owner on 2026-11-11. This file is the source of truth for what the engine is and
in what order it is built. Current status and how to continue: `docs/ai/START-HERE.md`.

## Brief (confirmed)

1. What: a general-purpose open source game engine, written from scratch, whose main user is a coding AI. Everything is text files. It runs with no window, describes itself, takes its own screenshots and reports errors as data.
2. Performance: Rust core on wgpu, data-oriented and multithreaded. Game logic is hot-reloaded scripts; anything slow can be moved into Rust.
3. Who drives it: designed for Claude, works with any coding agent through plain files, a CLI with JSON output and MCP. The agent reads a protocol file the project's user writes and follows it, including handing work to other agents.
4. Game types: 3D and 2D on a flat screen. VR is never required; it is an optional OpenXR module, and the renderer must be built so adding it is not a rewrite.
5. AI jobs: Claude designs, writes and checks. Qwen makes images only. Tripo makes 3D models. Rigging: Tripo auto-rig first for humanoids and common animals, Claude-scripted Blender for the rest and for repairs; the engine checks every rig by numbers.
6. Providers: cloud APIs by default, each user brings their own keys, local models allowed. One config file and one command switch any provider, by the person or by the AI.
7. AI while playing: off by default. A game can turn it on as an optional module with budgets and a scripted fallback.
8. Assets: search in this order: the project's own files, the user's owned library (a per-user folder shared across projects), the CC0 pool, known free CC0 libraries online (Poly Haven, ambientCG, Kenney, Quaternius), and only then generate with Qwen and Tripo (spending credits). Only CC0 goes in the pool, with the source recorded. (Updated 2026-11-11.)
8a. Importing owned assets: an importer brings in assets the user owns from Unity and Unreal projects. Unity first, by reading project files and `.unitypackage` archives; Unreal through the Unreal Editor's glTF/FBX export. Imported assets record their license, stay private to the user, and are blocked from the CC0 pool. (Added 2026-11-11.)
9. Platforms and builds: Windows and Linux. Builds and headless tests on every change. Graphics-card and headset checks run on a desktop machine with a real GPU. Other coding agents help wherever they can do the work at the same quality.
10. Name and license: Marionette. MIT OR Apache-2.0 (both offered). Asset pool is CC0. Two future repositories: the engine repository and the assets repository. The bare `marionette` crate name belongs to Mozilla, so crates are named `marionette-*` (for example `marionette-core`, `marionette-cli`).
11. Proof, part 1: six small playable scenes built by Claude with no hand edits: an FPS, a racing game, a platformer, a top-down game in 3D, a top-down game in 2D and a side-scroller in 2D.
12. Proof, parts 2 and 3: each scene uses pooled CC0 assets or Qwen -> Tripo assets rebuilt from recipes; each runs headless with scripted input, passes its tests twice with identical results, and holds 90 frames per second on a mid-range desktop GPU.

## Plan

1. Repos and workspace. A Cargo workspace of small crates, CI on Windows and Linux. Proof: first build passes on both.
2. Core. ECS world, schema registry of every type, text scene format, fixed-step loop, errors as data, a CLI that runs with no window. Proof: a scene runs 1,000 steps twice with an identical state hash, and saving a loaded scene gives back the same bytes.
3. Renderer. 3D glTF rendering and a 2D sprite path on wgpu, offscreen screenshots. Proof: reference screenshots match in CI and on a desktop GPU; test scene under 11.1 ms a frame on a mid-range desktop GPU.
4. Scripting. One-week spike of TypeScript against Luau measured on startup, call cost and reload time; the winner gets typed bindings generated from the schema. Proof: the numbers, and a script change shows in under 1 second.
5. Agent interface. CLI and MCP server share one set of operations; repo carries CLAUDE.md, AGENTS.md and the user's protocol file. Proof: a CLI-only session creates, runs, screenshots and tests a scene.
6. Physics, input, sound, UI. Rapier, recorded input, kira, minimal UI. Proof: replaying recorded input gives the same state hash.
7. Providers. Language, image and 3D interfaces with Claude, Qwen and Tripo plug-ins, one config file, one switch command, a spend cap. Proof: one command swaps a provider; the engine builds and passes tests with none configured.
8. Assets. Recipe: search project files, the owned library, the pool, CC0 libraries, then generate via Qwen and Tripo; post-process, validate, import. The local import path (owned library, Unity project files and `.unitypackage`, Unreal glTF/FBX exports) is built before the generation step. Proof: three assets rebuilt from recipes, each with license and source recorded; one owned asset imported with its license recorded and refused by the pool.
9. Proof scenes. The six scenes in brief line 11.
10. Later. OpenXR module, optional AI-while-playing module, read-only inspector.

## Design principles

1. Text is the source of truth. Scenes, prefabs, input maps, materials and asset recipes are plain text with stable ordering and stable ids. Binary files are build outputs.
2. Headless first. Every feature works without a window. The windowed build is the same runtime plus a presentation layer.
3. The engine describes itself. One command emits a machine-readable schema (JSON Schema) of every component, resource, system, event and script API. Script typings and tool definitions are generated from it.
4. Deterministic by default. Fixed timestep, seeded randomness, recorded input. No wall-clock time, no hash-map iteration order, no thread-timing dependence in simulation state.
5. Errors are data. Every failure has a stable code, file and line where known, the entity involved, and a suggested fix, as JSON.
6. Observation is cheap. Screenshots, scene tree dumps, entity queries and frame statistics are single calls, available mid-run.
7. Fast edit loop. Game logic, scenes and assets hot reload; changing a script must not trigger a native rebuild.
8. Reproducible assets. A generated asset is a recipe plus a cached output keyed by content hash.
9. Small, legible core. Each crate should fit in an agent's working context. Few plain concepts, no deep hierarchies.
10. Human in control. Spending money, publishing and deleting need explicit permission.

## Architecture

- `marionette-core`: world (ECS), schema registry, scene format and validator, fixed-step loop, seeded RNG, structured errors and logs. No graphics, no network.
- `marionette-render`: wgpu renderer (3D PBR forward, 2D sprites), glTF loading, offscreen capture. Stereo-ready.
- `marionette-script`: scripting host and typings generator (language decided by the spike).
- `marionette-physics`, `marionette-audio`, `marionette-input`: Rapier, kira, live/scripted/recorded input.
- `marionette-agent`: the operation set shared by CLI and MCP server: project, edit, run, observe, test, assets.
- `marionette-assets`: recipes, cache, post-processing, validation, import (including owned Unity and Unreal assets into a per-user library, never into the pool), CC0 pool and library search.
- `marionette-providers`: language, image and 3D traits; Claude, Qwen, Tripo behind feature flags; keys from env only.
- `marionette-ai-runtime`: optional, off by default.
- `marionette-cli`: the single binary, named `marionette`. Every command accepts `--json`.

Tech choices: ECS is `bevy_ecs` used standalone (reflection feeds the schema) unless it proves too heavy, in
which case `hecs` plus a hand-written registry. Scene format is RON or JSON with a published schema; pick one
in the core milestone and record why in `docs/decisions/`. Models are glTF 2.0 / GLB only.

## Scene format requirements

- One file per scene, listing entities with a stable id, an optional name, and components keyed by type name.
- Deterministic serialization: sorted keys, fixed float formatting, so load then save is byte-identical.
- Validation against the schema with errors as data (unknown component, wrong field type, dangling reference).

## State hash requirement

`marionette run <scene> --headless --ticks N --seed S --json` prints a hash of all simulation state after N
ticks. Two runs with the same inputs must print the same hash on the same machine, and should across
Windows and Linux.
