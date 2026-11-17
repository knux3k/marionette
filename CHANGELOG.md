# Changelog

Notable changes to Marionette, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Before 1.0, a minor version (0.x) may break scenes,
operations or the Rust API; such changes are listed under **Changed** with what to do.

## [0.1.0] - Unreleased

First public release. Feature levels (slice, usable, proven) are in `docs/ai/STATUS.md`.

### Engine core

- ECS world, canonical JSON scene format with JSON Schema and a validator whose errors carry a
  code, location and suggested fix.
- Deterministic fixed-step simulation: the same seed and inputs give the same state hash on
  Windows and Linux; replays and snapshots.
- More than 100 operations behind one table, run from the CLI (`marionette op`) or as MCP tools
  (`marionette mcp`, with `--tools` groups and `--lean` listings).
- Scenario tests, input replays, a seeded stability suite with time and memory budgets, and a
  shared performance suite.

### Making games

- `marionette make`: interview, brief and a playable, tested maze or platformer (2D or 3D),
  proven winnable by an autopilot.
- TypeScript gameplay scripts with hot reload; parallel TypeScript jobs compiled to WebAssembly;
  game modules in Rust without forking the engine; game WGSL shaders and meshes made in code, skinned meshes bent per copy by joint palettes.
- Physics (Rapier) with joints and character bodies; navigation and gravity on round planets.
- Input with keyboard, mouse and gamepads; windowed play; save slots with autosave and migration.
- Audio files, 3D sound, layered music, a synth and streamed audio.
- Skeletal animation with cross-fades, blend spaces and masked overlays.
- Scene editor (`marionette edit`).
- Headsets through OpenXR (optional), co-op lockstep networking with optional Steam.

### Rendering

- wgpu renderer: PBR glTF models, sprites, screen UI and world-space UI; shadows with cascades,
  sky, HDR environments, reflection probes, bloom, fog, particles; MSAA, mipmaps, BC7 textures,
  culling, level of detail, render scale, foveation, single-pass stereo; optional ray-traced
  reflections.

### Assets and AI

- Asset lookup order: project, owned library, public CC0 pool, online CC0 libraries, generation.
- Image, texture and 3D model generation from recipes (Qwen, Tripo), with caching, provenance,
  a person-set spend cap and person-only approval; an offline mock provider by default.
- Unity and Unreal importers and porting tools (scan, map, scaffold, convert, serde types).
- Skills and handoff files installed into every project for AI agents.

### Shipping

- `marionette package`: a zip per target (Windows x64, Linux x64, Steam Deck, Steam Frame), with
  licenses and a smoke run.
- Prebuilt `marionette` binaries for Windows x64 and Linux x64 (glibc 2.31 or newer).

### Known limits

- No macOS build and no code signing (Windows SmartScreen warns on first start).
- VR, networking, ray tracing, the Unreal importer and world-space UI are first slices.
- "AI while playing" is not built yet.

[0.1.0]: https://github.com/knux3k/marionette/releases/tag/v0.1.0
