# State

Rule: every session ends by updating this file (status, UNVERIFIED, session log). Plan lines
refer to `docs/DESIGN.md`.

## Status per plan line

| Line | Topic | Status |
| --- | --- | --- |
| 1 | Workspace, CI | Done locally. CI file exists, never run (no remote). |
| 2 | Core | Done. |
| 3 | Renderer | Done. |
| 4 | Scripting spike (TypeScript vs Luau) | Ready to start (workstream W7). Not started. |
| 5 | Agent interface (CLI + MCP) | Done. |
| 6 | Input, physics, audio, UI | Done headless; windowed mode never opened on a screen. |
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

Line 6 (input, physics, audio, UI):
- Example scenes: `examples/playground3d.scene.json` and `examples/playground2d.scene.json`,
  each with an action map (`*.actions.json`) and a scripted input file (`*.inputs.json`): a
  character walks, pushes and climbs dynamic boxes, jumps; jump and collision sounds.
- `cargo run -q -p marionette-cli -- run examples/playground3d.scene.json --headless --ticks 1000 --seed 1 --input examples/playground3d.inputs.json`
  prints `541b991eb45fe2a436c2ec8c0bea9b490c3a84356b51c8740771aa1083547c44`; the 2D scene with
  its input prints `1071e11381ec030e587cc2e241fedbafca679becc1ef13a886ea389dc184e8ac`.
- Tests: `crates/marionette-cli/tests/line6_proofs.rs`
  (`physics_scenes_give_the_same_hash_twice_and_match_the_recorded_hash`,
  `replaying_a_recording_gives_the_original_hash`,
  `snapshot_at_500_then_restore_continues_exactly`,
  `sound_log_lists_the_expected_sounds_at_the_expected_ticks`);
  `crates/marionette-physics/tests/build_errors.rs`; unit tests in
  `crates/marionette-input/src/lib.rs` and `crates/marionette-render/src/window.rs`.
- UI scene matches its reference image (line 3 proofs).
- Window: live keyboard and mouse into the input queue, `UiButton` clicks, sound playback,
  `--input`, `--record`, `--mute` (decision 0017).
- `enhanced-determinism` cost: within noise (decision 0012, measured with
  `cargo run --release -p marionette-physics --example step_cost -- 1000`).
- Modules: decision 0011; input 0013; physics 0012; audio 0014; UI 0015.

## UNVERIFIED

- CI has never run (no remote); the state hash has never been compared on Linux.
- Windowed mode (`marionette run <scene>` without `--headless`) has never been run on a screen:
  live keyboard/mouse input, `UiButton` clicks and sound playback through a real audio device
  are untested beyond unit tests.
- MCP over the handshake-free protocol revision 2026-07-28 (the SDK handles it; tests use the
  2025-11-25 handshake). No real Claude Code client has connected; the `claude mcp add` line in
  `README.md` is unchecked.
- Reference-image tolerance on AMD, Intel, Apple GPUs and on Mesa lavapipe.
- OGG decoding (enabled in kira; no OGG file has been decoded).
- The playground hashes have only been produced on Windows; Linux equality is unchecked.
- Live gamepad input (format supported, no backend).

## Session log

| Date | What changed | Commits |
| --- | --- | --- |
| 2026-11-11 | Lines 1 and 2: workspace, core, CLI, acceptance tests, decisions 0001 to 0003. | `ee6663d`..`eaaf11c` |
| 2026-11-11 | Line 3: renderer, offscreen capture, examples, reference images, decisions 0004 to 0007. | `4513604`..`7f793b8` |
| 2026-11-11 | Line 5: agent operations, MCP server, e2e tests, decisions 0008 to 0010. | `699d177`..`a9ce4fe` |
| 2026-11-11 | DESIGN asset importer; line 6 partial: modules, input, physics, audio, UI, ops. | `14e592d`..`a3c83b0` |
| 2026-11-11 | Scrub of identifying details, neutral commit identity, hygiene test, `docs/ai/`, `marionette docs`, decisions 0011 to 0016. | up to the commit adding `docs/ai/` |
| 2026-11-12 | History rewritten to remove identifying text before first publication (file contents and commit trailers; final tree unchanged). Scripting spike marked ready. | this commit |
| 2026-11-12 | Line 6 finished (W1): playground examples, proofs, window hooks, parse-error tests, enhanced-determinism cost, decisions 0012 update and 0017. | `3e4514b`..this commit |
