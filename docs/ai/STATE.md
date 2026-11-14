# State

Rule: every session ends by updating this file (status, UNVERIFIED) and adding one row to
[SESSION-LOG.md](SESSION-LOG.md). Plan lines refer to `docs/DESIGN.md`. Proof commands and test
names for every done part are in [PROOFS.md](PROOFS.md); read only the section you need.
Standing priorities: [FOCUSES.md](FOCUSES.md).

## Status per plan line

"Landed" means in main with tests, not that a game can lean on it. How far each feature goes
(slice, usable, proven, with the evidence) is in [STATUS.md](STATUS.md) (decision 0274):
check a feature there before building on it, and update its row when your work moves it.

| Line | Topic | Status |
| --- | --- | --- |
| 1 | Workspace, CI | Landed: fast check on pull requests, full suite on main, no merge queue on a personal repository (0260). |
| 2 | Core | Landed: save games (0076), indexed edits (0270), parent and child transforms (0272). |
| 3 | Renderer | Landed: opt-in looks (0063 to 0067), culling and level of detail (0080), cascades, environments, probes, ray tracing (0090 to 0093), mipmaps, alpha, particles, BC7, stereo (0280 to 0285). |
| 4 | Scripting | Landed: TypeScript (0022, 0271), job TypeScript from scenes and scripts (0070). |
| 5 | Agent interface (CLI + MCP) | Landed: skills (0160), handoffs and claims (0161), a game's own Rust modules and cargo features (0062, 0290). |
| 6 | Input, physics, audio, UI | Landed: windowed play (0170 to 0173), 3D sound and music layers (0075), world-space UI (0200, 0201), synth (0220). |
| 7 | Providers | Landed (0019, 0045, 0046); live Qwen and Tripo calls made on the owner's PC. |
| 8 | Assets (import, pool, generation) | Landed: owned library, Unity and Unreal importers, FBX conversion, prefabs (0027, 0029, 0040), generation (0045 to 0048), CC0 pool and search (0041). |
| 9 | Six proof scenes | Landed (0050): six games under `games/` with stability modes. Gaps found: `docs/ai/PROOF-SCENES.md`. |
| 10 | Later | OpenXR (0190), inspector (0130), co-op networking, achievements, cloud saves, Steam (0210 to 0213) landed; rest not started. |
| - | Game packages | Landed (0140): Windows, Linux, Steam Deck, Steam Frame (ARM64). |

## UNVERIFIED

- Indexed edits, script speed, children (0270 to 0272): timed in a cloud container only.
- Game packages (decision 0140): never run on a Deck, a Frame, a headset or a screen; Frame binaries
  ran headless on generic ARM64 Linux (CI) only, not in the Steam runtime.
- Starter (0150): made games not played by a person.
- Networking and Steam (0210-0213): loopback and 127.0.0.1 UDP only; the Steam backend has
  never run against a Steam client (first check: `steam_check`, app id 480, owner login).
- Synth and streamed audio (decision 0220): rendered and checked as numbers only; played through a real
  device twice (64 and 128 voices clean on a 96-frame buffer, 256 and up miss deadlines; see
  proofs/play.md); no one has listened; the device buffer size is not configurable yet. OGG streaming untested (WAV is).
- World-space UI (decisions 0200, 0201): checked offscreen on Mesa lavapipe and by CPU timing only;
  not on the RTX 2080, in a headset, with a TrueType font, or with hand-tracked touch.
- Game handoffs (decision 0161): tested with two claims in one working tree; no game has
  been worked on by several real agents through it, and claims are not checked against a
  remote.
- Skills (decision 0160): the format and two agents' project skill folders were read from
  their documentation; one agent listed all 15 from the repository unprompted in the session that wrote them; no
  session has been watched choosing one for a task, and other agents reading `.agents/skills` is assumed from the open format.
- Headsets (0190): only a simulated headset; never worn (SteamVR, hands, haptics, 90 Hz).
- Game meshes, shaders, 0280-0285 renderer work (0180-0181): lavapipe only.
- Opt-in looks (0063): checked offscreen on one desktop GPU (numbers in the decision);
  not on AMD or Intel GPUs.
- Bloom, local light shadows, environment, glTF emissive (0064-0067): checked
  on Mesa lavapipe and one desktop GPU offscreen (numbers in PROOFS.md); not on AMD or Intel GPUs.
- Render cost controls (0080-0082): one desktop GPU offscreen only; not in a headset or
  on AMD/Intel. Scale plus foveation resolve not re-timed.
- Ray tracing (0093): lavapipe and one desktop GPU; skinned models are not traced.
- CC0 pool online adapters (Poly Haven, ambientCG, Kenney and Quaternius archives, decision 0041;
  cgbookcase, decision 0071)
  are tested only against scripted answers shaped like the real APIs as read on 2026-11-12; the
  build machine's network blocks those sites, so no live search or download has run.

- Unity prefab import (decision 0040) is tested only on hand-written prefab files in Unity's text
  format, not on prefabs saved by the Unity editor or on a real pack; the model-root override
  rule, the nested file-id rule and the half turn for converted models are from knowledge of
  Unity's serializer, not checked in the editor.

- Job TypeScript speed on the owner's desktop and on ARM CPUs (only a 4-core x86-64 cloud
  container measured; the full test suite passes on ARM64 in CI, decision 0140). Job schedules, `engine.jobs` and the swarm hash on Windows: first seen
  in the CI run of decision 0070; `JobDraw` drawn only on Mesa lavapipe so far.
- Window (proofs/window.md; editor too): gamepads on hardware, high-refresh screens, clicks, speakers,
  Linux and macOS screens.
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
  Mesa lavapipe. `bench --headset` emulates two eyes offscreen.
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

- `port_serde` (0240): field order and float text unchecked against real JsonUtility files.
