# State

Rule: every session ends by updating this file (status, UNVERIFIED) and adding one row to
[SESSION-LOG.md](SESSION-LOG.md). Plan lines refer to `docs/DESIGN.md`. Proof commands and test
names for every done part are in [PROOFS.md](PROOFS.md); read only the section you need.
Standing priorities: [FOCUSES.md](FOCUSES.md).

## Status per plan line

| Line | Topic | Status |
| --- | --- | --- |
| 1 | Workspace, CI | Done. First CI run (main at `6cbe22d`): Windows passed; Linux failed at build because the runner lacked the ALSA library. Workflow fixed (installs `libasound2-dev`); Linux CI result comes with the next run. |
| 2 | Core | Done. |
| 3 | Renderer | Done. Opt-in shadows, point and spot lights, sky light, emissive, tone mapping and fog (decision 0063); bloom, point and spot light shadows, environment lighting with sky background, glTF emissive (decisions 0064 to 0067). |
| 4 | Scripting (TypeScript vs Luau) | Done (W7): TypeScript chosen, binding landed (decision 0022). Scenes and scripts run job TypeScript over saved job arrays (decision 0070). |
| 5 | Agent interface (CLI + MCP) | Done. Games add their own Rust modules and operations and pick optional parts by cargo feature (decision 0062). |
| 6 | Input, physics, audio, UI | Done headless; windowed mode never opened on a screen. 3D sound (distance curves, rear muffle, Doppler, head pose) and layered music (decision 0075). |
| 7 | Providers | Done offline (W5, decision 0019): interface, mock default, Claude/Qwen/Tripo plug-ins tested against recorded shapes, one-command switch, spend cap. Network transport behind the `net` feature, keys files and per-provider caps added with W4 (decision 0045); live Qwen (personal plan address, decision 0046) and Tripo calls made on the owner's PC. |
| 8 | Assets (import, pool, generation) | In progress: owned library, Unity importer, FBX to glTF conversion and Unity prefab import done (W2, decisions 0027, 0040); Unreal importer done (W6, decision 0029); generation pipeline done (W4, decisions 0045-0048: recipes, cache, provenance, approval with an owner waiver, PNG/GLB post-processing with model slimming); 19 generated models for the 3D proof games are in `games/*/assets/generated`; CC0 pool and asset search done (W3, decision 0041, online adapters unverified live). |
| 9 | Six proof scenes | Done (W8, decision 0050): all 6 games (`games/lantern-keeper`, `games/ember-run`, `games/cloudstep`, `games/crystal-ward`, `games/dust-circuit`, `games/signal-yard`) with stability modes, each using its generated images or models (models drawn through `ModelPlacement`). Gaps found: `docs/ai/PROOF-SCENES.md`. |
| 10 | Later (OpenXR, AI while playing, inspector) | Not started. |

## UNVERIFIED

- Opt-in looks (decision 0063): checked offscreen on one desktop GPU (numbers in the decision);
  not yet in a real headset through OpenXR, nor on AMD or Intel GPUs.
- Bloom, local light shadows, environment and glTF emissive (decisions 0064 to 0067): checked
  on Mesa lavapipe and one desktop GPU offscreen (numbers in PROOFS.md); not in a real headset,
  nor on AMD or Intel GPUs.
- CC0 pool online adapters (Poly Haven, ambientCG, Kenney and Quaternius archives, decision 0041;
  cgbookcase, decision 0071)
  are tested only against scripted answers shaped like the real APIs as read on 2026-11-12; the
  build machine's network blocks those sites, so no live search or download has run.

- Unity prefab import (decision 0040) is tested only on hand-written prefab files in Unity's text
  format, not on prefabs saved by the Unity editor or on a real pack; the model-root override
  rule, the nested file-id rule and the half turn for converted models are from knowledge of
  Unity's serializer, not checked in the editor.
- Windows side of `marionette-host` (job object, run-folder locks, kill tests): compiles and
  passes clippy for a Windows target; first run is the CI run of its merge.

- Linux CI passes on `main` since the providers merge (ALSA fix in the workflow); Windows CI
  passes, including the GPU tests on WARP. In a Linux container without a GPU the drift hash,
  the seed-2 hash, the playground hashes (`line6_proofs.rs`) and the scripted example hash
  match; GPU tests skipped there.
- `marionette-script` (rquickjs/QuickJS-ng and oxc) on Windows MSVC: see the CI run of the
  scripting merge; not built on the owner's desktop yet.
- `marionette-jobs-ts` on Windows: wasmtime/Cranelift on MSVC and the pinned job hash equal to
  Linux are expected from CI on the job TypeScript branch; not yet observed.
- Job TypeScript speed on the owner's desktop and on ARM CPUs (only a 4-core x86-64 cloud
  container measured). Job schedules, `engine.jobs` and the swarm hash on Windows: first seen
  in the CI run of decision 0070; `JobDraw` drawn only on Mesa lavapipe so far.
- Windowed mode (`marionette run <scene>` without `--headless`) has never been run on a screen:
  live keyboard/mouse input, `UiButton` clicks and sound playback through a real audio device
  are untested beyond unit tests.
- MCP over the handshake-free protocol revision 2026-07-28 (the SDK handles it; tests use the
  2025-11-25 handshake). No real Claude Code client has connected; the `claude mcp add` line in
  `README.md` is unchecked.
- Reference-image tolerance on AMD, Intel, Apple GPUs and on Mesa lavapipe.
- OGG decoding (enabled in kira; no OGG file has been decoded).
- Live gamepad input (format supported, no backend).
- Unity importer on real files: checked read-only (dry run plus the readers, nothing written)
  on one Unity project and five `.unitypackage` files (about 5,700 files, 214 materials, 800
  textures). Engine content was refused as expected (TextMesh Pro resources, XR Interaction
  Toolkit samples), every material parsed, the URP Lit and URP Unlit shader GUIDs matched real
  materials, and two JPEGs saved as `.png` led to content sniffing. Still unchecked: the HDRP
  Lit GUID, the Asset Store cache folder layout (no cache on the test machine), a real import
  with a person-written declaration, and binary-serialized materials.
- The pasted-license phrase list catches common wording only; it never grants permission.
- Porting tools against real projects: one real Unity slice (a factory game's Burst jobs, kept
  private, decision 0072) scanned and scaffolded; otherwise small hand-written samples and one
  Blueprint text snippet. No real Unreal project yet. Table entries for Unity and Unreal APIs are from knowledge of
  those engines, not checked against their current documentation. The skeletons type-check
  against the generated script typings (decision 0022) only in tests with `tsc` installed.
- Generation sample answers (`crates/marionette-generate/tests/recorded/`) and the Tripo upload
  endpoint are hand-written from documentation; the network transport has only talked to a
  server on the same machine. Built with `--features net` only in Linux CI and the container.
- Provider plug-ins (`claude`, `qwen`, `tripo`): request and answer formats are written from
  documentation and tested only against recorded shapes; none has been called live
  (`verified: false`). The Qwen and Tripo prices and endpoints are from memory. The mock PNG
  pin depends on the locked `png` encoder version.
- Blend spaces and overlays (decision 0074): poses checked numerically on the generated
  puppet only, not on a real rigged character or a hardware GPU. 3D sound and music layers
  (decision 0075): the `hear` numbers and music levels are tested; kira voices, glides, the
  rear low-pass, Doppler, synced stem start and `Player::set_head` have never played through
  a real audio device or a headset.
- Skinned and morphed characters have been rendered only on Mesa lavapipe (Linux container);
  never on a desktop GPU, never with a real (non-generated) rigged character or a real FBX
  export.
- Performance on a real desktop GPU and headset: all timings come from a Linux container with
  Mesa lavapipe. `bench --headset` emulates two eyes offscreen; no OpenXR runtime has been used.
- FBX conversion has only seen files written by Blender and Assimp (and ASCII made from them).
  FBX from Maya, 3ds Max, MotionBuilder or Unity's own exporter (pivots, pre-rotation, cubic
  curves, `InheritType` other than RSrs) is untested; a real Asset Store pack is the next check.

- Save capture timings (decision 0073) come from a 4-core Linux container only.
- Stability suite: frame budgets checked on one hardware GPU (RTX 2080, Windows; all 31 fast
  modes pass, PROOFS.md); not on AMD, Intel or a headset, and the long soak has not run on a
  hardware GPU. Late model streaming (decision 0068) and the heap memory probe (decision
  0069) checked on that GPU (PROOFS.md); a model still decoding at the warm-up sample can show
  as one-time growth in render modes. CI job `stability` and the nightly
  soak results: see the workflow runs.

- Unreal importer on real files: nothing from a real Unreal project has been imported. T3D
  material text is hand-written from knowledge of the editor's export format; the package
  class and `RelativeFilename` peek is tested on synthetic bytes; FBX from Unreal's own exporter
  is untested (the fixture is Blender writing Unreal's convention: +Z up, centimeters). Next
  check: a dry run over a project the owner has, with their permission.
