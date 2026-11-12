# State

Rule: every session ends by updating this file (status, UNVERIFIED, session log). Plan lines
refer to `docs/DESIGN.md`.

## Status per plan line

| Line | Topic | Status |
| --- | --- | --- |
| 1 | Workspace, CI | Done locally. CI file exists, never run (no remote). |
| 2 | Core | Done. |
| 3 | Renderer | Done. |
| 4 | Scripting spike (TypeScript vs Luau) | Deferred (V8 build size). Not started. |
| 5 | Agent interface (CLI + MCP) | Done. |
| 6 | Input, physics, audio, UI | In progress (see below). |
| 7 | Providers | Not started. |
| 8 | Assets (import, pool, generation) | Not started. DESIGN updated with the owned-asset importer. |
| 9 | Six proof scenes | Not started. |
| 10 | Later (OpenXR, AI while playing, inspector) | Not started. |

## Proofs for done lines (rerun these)

Line 1: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --all -- --check` all pass. Workflow: `.github/workflows/ci.yml`.

Line 2 (core):
- `cargo run -q -p marionette-cli -- run examples/drift.scene.json --headless --ticks 1000 --seed 1`
  prints `7441258fdf64141911d44e0608c48498897564168684fdd86cd244e149926558` (twice in a row).
- Same with `--seed 2` prints `2ae633b60710c744b5aff29d9a534d6732872f5278451ef3351b976558d0af15`.
- Tests: `crates/marionette-core/tests/acceptance.rs` (`same_seed_gives_identical_hash_after_1000_ticks`,
  `different_seed_gives_different_hash`, `load_then_save_is_byte_identical`,
  `diagnostic_positions_are_exact`); `drift_hash_is_unchanged_from_plan_line_2` in
  `crates/marionette-cli/tests/cli.rs`.

Line 3 (renderer):
- Reference images (320x180, 60 ticks, seed 1, MSAA 4), SHA-256 of the committed files:
  - `examples/reference/showcase.png`: `9563bd7152cc2ac32f2aff9119ad9ce8475e07cd96a085fc6c22651e6adab996`
  - `examples/reference/sprites.png`: `4b07882d40928aca85f67d169868b3e1a4bde97de84186c86d2e16c64e97e97d`
  - `examples/reference/ui.png`: `4c7417beb5c10b9bba89ad7ab9721a1a49aa8c566ca2bd4c7e9c36f4fa56df06`
- Tests: `crates/marionette-render/tests/render.rs` (`same_inputs_give_identical_images`,
  `matches_reference_images`, `rendering_does_not_change_simulation_state`).
  `screenshot_is_deterministic_and_leaves_state_alone` in `crates/marionette-cli/tests/cli.rs`.
- Frame time, `examples/showcase.scene.json`, desktop GPU of the RTX 2080 class, Vulkan,
  1920x1080, 4x MSAA, offscreen, 2,000 frames, release build: average 0.27 ms, p99 0.31 to
  0.48 ms, worst 5.7 to 6.5 ms over three runs (budget 11.1 ms). Command:
  `cargo run --release -q -p marionette-cli -- bench examples/showcase.scene.json --frames 2000 --size 1920x1080`.
- State hashes at 60 ticks, seed 1 (`run ... --headless --ticks 60 --seed 1`): showcase
  `b95606f6c44132e9b2fd82caad0e37a7d056da1b267e5e4ec24be117de832c08`, sprites
  `3c90975a249c2cb5a44bb38e83d2116ba77c804b61352db3858c3dc93baaf3d1`.

Line 5 (agent interface): `crates/marionette-cli/tests/agent_e2e.rs`
(`cli_only_sequence_succeeds_and_is_deterministic`, `mcp_sequence_matches_cli_state_hash`,
`mcp_read_only_refuses_mutations_with_structured_error`,
`invalid_edits_are_refused_and_leave_the_file_alone`, `cli_read_only_refuses_mutations`).

Repository rules: `crates/marionette-cli/tests/repo_hygiene.rs` (no identifying details, every
path in `docs/ai/` exists, generated reference is current).

## Line 6 in progress

Done (committed, builds, all tests pass):
- Module system and stages: `crates/marionette-core/src/module.rs`, decision 0011.
- Input: `crates/marionette-input/src/lib.rs` (action maps, events files, recording), decision 0013.
- Physics: `crates/marionette-physics/` (3D and 2D, character controller, events, raycast,
  snapshots), decision 0012.
- Audio: `crates/marionette-audio/src/lib.rs` (triggers, headless sound log, kira player,
  built-in sounds; 3 unit tests), decision 0014.
- UI: `crates/marionette-core/src/ui.rs`, UI pass in `crates/marionette-render/src/renderer.rs`,
  font `crates/marionette-assets/src/font.rs`, scene `examples/ui.scene.json` with reference
  image, decision 0015.
- Operations: `session_snapshot`, `session_restore`, `session_record_save`, `observe_raycast`,
  `observe_events`; `scene_run` takes `input`, `events`, `record`; `session_start` takes `input`;
  `session_step` takes `events`; scenarios take `input` and an `event` assertion.

Remaining (in this order):
1. Example scenes: a 3D one (character controller walks on a floor, pushes dynamic boxes, jumps)
   and a 2D one with the same idea, each with an action map (`*.actions.json`), a scripted input
   file (`*.inputs.json`) and sounds on jump and collision.
2. Proof tests: replaying a recorded input file gives the original state hash (both scenes,
   twice); the 3D physics scene run 1,000 ticks twice gives the same hash; snapshot at tick 500,
   restore, run to 1,000 equals the uninterrupted run; the sound log lists the expected sounds
   at the expected ticks.
3. Window integration in `crates/marionette-render/src/window.rs` and
   `crates/marionette-cli/src/render.rs`: live keyboard and mouse into the input queue,
   `--record <file>`, `marionette_audio::Player` playing new sound events, `UiButton` clicks.
4. Measure the frame-time cost of Rapier's `enhanced-determinism` on the 3D example; record it
   in decision 0012.
5. Unit tests for action-map parsing errors and physics build errors.

## UNVERIFIED

- CI has never run (no remote); the state hash has never been compared on Linux.
- Windowed mode (`marionette run <scene>` without `--headless`) has never been run on a screen.
- MCP over the handshake-free protocol revision 2026-07-28 (the SDK handles it; tests use the
  2025-11-25 handshake). No real Claude Code client has connected; the `claude mcp add` line in
  `README.md` is unchecked.
- Reference-image tolerance on AMD, Intel, Apple GPUs and on Mesa lavapipe.
- OGG decoding (enabled in kira; no OGG file has been decoded).
- Physics determinism, snapshot/restore exactness and replay equality: code exists, the proof
  tests are line 6 remaining work.
- Live gamepad input (format supported, no backend).
- Cost of `enhanced-determinism`.
- History content: older commits still hold earlier versions of a few files with identifying
  details (decision 0016, known gap).

## Session log

| Date | What changed | Commits |
| --- | --- | --- |
| 2026-11-11 | Lines 1 and 2: workspace, core, CLI, acceptance tests, decisions 0001 to 0003. | `ee6663d`..`eaaf11c` |
| 2026-11-11 | Line 3: renderer, offscreen capture, examples, reference images, decisions 0004 to 0007. | `4513604`..`7f793b8` |
| 2026-11-11 | Line 5: agent operations, MCP server, e2e tests, decisions 0008 to 0010. | `699d177`..`a9ce4fe` |
| 2026-11-11 | DESIGN asset importer; line 6 partial: modules, input, physics, audio, UI, ops. | `14e592d`..`a3c83b0` |
| 2026-11-11 | Scrub of identifying details, neutral commit identity (history rewritten), hygiene test, `docs/ai/`, `marionette docs`, decisions 0011 to 0016. | `3d2b5d3`..HEAD |
