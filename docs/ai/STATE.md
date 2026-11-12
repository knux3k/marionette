# State

Rule: every session ends by updating this file (status, UNVERIFIED, session log). Plan lines
refer to `docs/DESIGN.md`.

## Status per plan line

| Line | Topic | Status |
| --- | --- | --- |
| 1 | Workspace, CI | Done. First CI run (main at `6cbe22d`): Windows passed; Linux failed at build because the runner lacked the ALSA library. Workflow fixed (installs `libasound2-dev`); Linux CI result comes with the next run. |
| 2 | Core | Done. |
| 3 | Renderer | Done. |
| 4 | Scripting (TypeScript vs Luau) | Done (W7): TypeScript chosen, binding landed (decision 0022). |
| 5 | Agent interface (CLI + MCP) | Done. |
| 6 | Input, physics, audio, UI | Done headless; windowed mode never opened on a screen. |
| 7 | Providers | Done offline (W5, decision 0019): interface, mock default, Claude/Qwen/Tripo plug-ins tested against recorded shapes, one-command switch, spend cap. Network transport behind the `net` feature, keys files and per-provider caps added with W4 (decision 0045); no live call made yet. |
| 8 | Assets (import, pool, generation) | In progress: owned library, Unity importer, FBX to glTF conversion and Unity prefab import done (W2, decisions 0027, 0040); Unreal importer done (W6, decision 0029); generation pipeline done offline (W4, decision 0045: recipes, cache, provenance, approval, PNG/GLB post-processing); CC0 pool and asset search done (W3, decision 0041, online adapters unverified live). |
| 9 | Six proof scenes | In progress (W8, decision 0050): 3 of 6 games done (`games/lantern-keeper`, `games/ember-run`, `games/cloudstep`). Gaps found: `docs/ai/PROOF-SCENES.md`. |
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

Line 4 (scripting, decision 0022):
- Spike numbers and how to rerun them: decision 0022, harness `spikes/script-spike` (not a
  workspace member).
- `cargo run -q -p marionette-cli -- run examples/scripted.scene.json --headless --ticks 1000 --seed 1`
  prints `4450bc3cb97805f59584f11c13b999a13f8ac42ca67752dc7d1cc247575251d3` (script
  `examples/scripts/bouncer.ts`: gravity, bounces, `engine.find`, `engine.rand`, `Math.sin`).
- Tests: `crates/marionette-script/tests/script.rs` (`same_seed_same_hash_and_seed_matters`,
  `scripts_change_components_and_keep_state`, `snapshot_then_restore_continues_exactly`,
  `clock_and_unseeded_randomness_are_gone_and_math_is_libm`, `load_errors_are_data`,
  `runtime_errors_become_events_and_the_tick_goes_on`, `reload_swaps_behavior_fast_and_keeps_state`,
  `typings_cover_every_component`); `crates/marionette-cli/tests/script_e2e.rs`
  (`scripted_example_gives_the_same_hash_twice_and_matches_the_recorded_hash`,
  `script_change_shows_in_a_live_session_in_under_a_second`,
  `script_types_writes_declarations_from_the_schema`,
  `physics_contacts_reach_collision_hooks_one_tick_later`); unit tests in `src/strip.rs`.
- Typings: `docs/ai/reference/marionette.d.ts` (regenerated by `marionette docs`).
  `examples/scripts/bouncer.ts` passes `tsc --strict` against it (checked by hand; `tsc` is not
  part of the test suite).
- Spawning, destroying and timers (decision 0021): `engine.spawn`, `engine.clone`,
  `engine.destroy` (component `DestroyAt`), `onDestroy`, `engine.after`/`afterTicks`/`every`/
  `cancel`/`pending` (component `ScriptTimers`), resource `SpawnIds`; physics adds and removes
  bodies of spawned and destroyed entities. Tests: `crates/marionette-script/tests/lifecycle.rs`
  (`spawned_entities_run_next_tick_and_are_destroyed_with_on_destroy`,
  `spawning_destroying_and_timers_replay_identically_and_follow_the_seed`,
  `snapshot_with_pending_timers_and_removals_continues_exactly`,
  `timers_after_every_cancel_and_pending`, `timer_errors_are_events_and_fire_once`,
  `clone_copies_components_but_not_script_state_and_loads_new_scripts`,
  `on_destroy_can_destroy_more_in_the_same_tick`); `crates/marionette-cli/tests/script_e2e.rs`
  (`spawned_crates_get_bodies_land_and_leave_physics_when_destroyed`).
- `cargo run -q -p marionette-cli -- run examples/spawner.scene.json --headless --ticks 1000 --seed 1`
  prints `a04911705b0d9f1ce6b8ebc66858c7054c72aa081cff416e227d881650c4aaa8` (scripts
  `examples/scripts/spawner.ts` and `crate.ts`: a repeating timer drops 40 physics crates plus 4
  clones; each removes itself three seconds later). Seed 2 prints
  `96a90ff4615992a93a5c71cef3c2dbbdb45188b0c3d8b29edfc32f12aa556a50`.
- `examples/scripts/*.ts` pass `tsc --strict --lib es2022` against the generated typings (checked
  by hand); the porting skeleton test runs `tsc` against the same typings.
- Not covered yet: `import` between script files, window-mode file watching for hot reload, a
  faster component bridge, prefab files, removing one component from a script
  (`Destroy(component)`), generator coroutines (rejected, decision 0021), and the
  `entity_create` operation on a live session still picks `max id + 1` instead of `SpawnIds`.

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

Line 7, providers (W5, decision 0019):
- `crates/marionette-cli/tests/providers_e2e.rs`: `one_command_swaps_a_provider_and_mock_needs_nothing`
  (`marionette provider use image qwen` swaps; generation then stops at the missing key; swapping
  back gives the same bytes), `bad_switches_are_refused_and_leave_the_file_alone`,
  `provider_list_names_every_plugin_without_showing_keys`.
- `crates/marionette-providers/tests/providers.rs`: `mock_is_deterministic_and_pinned_across_platforms`
  (mock PNG `f9049f7ad383f5b97363c83ee7711822ea04b9aba608958ae44a323be995bbbf`, GLB
  `658a8b15dcc62bf38e65bde6c267b411c0cf2c57abe5c9cb13cdd0a97f1308ad`), `mock_outputs_load_in_the_engine`,
  `config_defaults_to_mock_and_refuses_keys`, `checks_run_in_order_before_any_request_is_sent`,
  `claude_request_shape_cost_and_ledger`, `claude_refusal_and_http_errors_are_data_without_the_key`,
  `qwen_submits_polls_and_downloads`, `tripo_text_and_image_jobs`,
  `unreadable_ledger_refuses_paid_calls_but_not_free_ones`,
  `spend_cap_comes_from_the_environment_or_the_user_config_only`,
  `every_offered_kind_has_a_default_model_and_keys_are_env_names`.
- The whole suite builds and passes with no `providers.json`, no keys and no network.

Line 8, owned library and Unity import (W2, decision 0018):
- `crates/marionette-import/tests/import.rs`: `owned_file_with_declaration_is_imported_with_its_license`,
  `undeclared_assets_are_refused_with_reasons`, `engine_locked_licenses_are_refused`,
  `engine_content_in_a_unity_project_is_refused_without_override`,
  `the_most_specific_declaration_applies`, `unitypackage_brings_in_textures_models_and_materials`,
  `materials_resolve_textures_from_earlier_imports`, `pool_refuses_owned_assets_and_copies_of_them`,
  `library_location_rules_hold`, `dry_run_writes_nothing_and_bad_archives_are_errors`; plus unit
  tests in `src/tar.rs`, `src/license.rs`, `src/unity.rs`.
- `crates/marionette-cli/tests/library_e2e.rs`: `library_operations_import_list_and_gate`,
  `operations_never_write_license_declarations`.
- FBX to glTF (decision 0027): `crates/marionette-import/tests/fbx.rs`
  (`static_meshes_match_blenders_own_gltf`, `embedded_textures_are_used`,
  `other_axis_systems_and_units_give_the_same_world`,
  `fbx_7500_from_a_second_writer_matches_blenders_reading`,
  `skinned_animation_matches_blenders_own_gltf`, `ascii_fbx_converts_like_binary`,
  `conversion_is_deterministic`, `broken_files_are_errors_not_panics`,
  `blend_shapes_become_morph_targets_and_match_blender`,
  `unity_project_fbx_is_converted_with_its_texture`,
  `textures_outside_the_models_license_are_not_bundled`); unit tests in `src/fbx/`. One-off
  conversion: `cargo run -p marionette-import --example fbx2glb -- in.fbx out.glb`.
- Not covered yet: OBJ/blend to glTF conversion (kept as `source.<ext>`, `engine_ready:
  false`); FBX blend shape in-betweens and normal offsets, vertex colors, extra UV sets, FBX 6.x
  (decision 0027); skinning in
  the renderer (skinned FBX models draw in bind pose), Unity scenes, animations and audio (skipped and listed), binary-serialized
  `.mat` files (skipped and listed).
- Unity prefabs (decision 0040): `crates/marionette-import/tests/prefab.rs`
  (`a_prefab_becomes_entities_with_mapped_components`,
  `nested_prefabs_and_models_follow_their_overrides`,
  `variants_remove_components_and_override_materials`,
  `broken_recursive_and_unlicensed_prefabs_are_reported`,
  `conversion_is_deterministic_and_dry_runs_write_nothing`); unit tests in `src/unity_yaml.rs` and
  `src/prefab.rs`; `crates/marionette-cli/tests/prefab_e2e.rs`
  (`import_list_and_place_a_unity_prefab`: import, `library_list` kind `prefab`,
  `prefab_instantiate` twice, `scene_run`). Not covered: sub-mesh selection inside a model,
  overrides on model parts, point/spot lights, audio sources, physic materials, X/Z capsules,
  compound bodies, Unity `.unity` scenes (same reader would serve them).
- CC0 pool (W3, decision 0041): `cargo test -p marionette-pool`:
  `crates/marionette-pool/tests/pool.rs` (`seed_pool_matches_its_generator`,
  `made_assets_need_a_method_and_cc0_license_files`, `generated_assets_need_a_persons_dedication`,
  `owned_assets_and_copies_of_them_are_refused`,
  `online_assets_need_a_known_library_its_domain_and_license_file`,
  `poly_haven_textures_and_models_are_ingested_with_checks`,
  `ambientcg_materials_are_ingested_from_their_archives`, `search_follows_the_required_order`);
  unit tests in `src/net.rs` (MD5) and `src/zip.rs`; `crates/marionette-cli/tests/pool_e2e.rs`
  (`pool_operations_search_fetch_add_and_refuse`). Seed set in `pool/` rebuilt by
  `cargo run -p marionette-pool --example make_seed -- <folder>`. Operations `asset_search`,
  `pool_info`, `pool_list`, `pool_get`, `pool_add`, `pool_ingest`, `pool_fetch`, `pool_index`.
  The pool moves to its own repository once one exists; until then `pool/` is the pool.

Line 8, Unreal importer (W6, decision 0029):
- `crates/marionette-import/tests/unreal.rs`: `unreal_project_imports_exports_and_refuses_engine_content`
  (project with exported FBX, PNG, TGA, T3D material instance and material graph, Starter
  Content, mannequins, an Epic plugin, packages, a `.pak`, code, generated folders),
  `unreal_assets_follow_the_license_rules`, `a_content_folder_from_a_vault_cache_is_an_unreal_source`;
  `unreal_convention_fbx_lands_in_meters_y_up` in `tests/fbx.rs` (fixture
  `tests/fixtures/unreal/crate_unreal_cm.fbx`, made by `make_fixtures.py` there); unit tests in
  `src/unreal.rs`.
- Not covered yet: reading `.uasset`/`.umap` (diagnosed only, by design), levels and actor
  placement, Blueprints (port_scan reads their text), audio, material graphs beyond direct
  connections, material functions, zip downloads (unzip first), separate roughness and
  metallic textures (listed, not packed).

Porting Unity and Unreal code (W10, decision 0020, guide `docs/ai/PORTING.md`):
- `cargo test -p marionette-port`: `crates/marionette-port/tests/port.rs`
  (`unity_classes_roles_hooks_and_components`, `unity_api_uses_actions_gaps_and_hazards`,
  `unreal_classes_definitions_and_bindings`, `blueprint_text_is_read`,
  `reports_and_skeletons_are_deterministic`, `skeletons_keep_every_original_line_and_annotate_calls`,
  `skeletons_type_check_with_tsc` (skips without `tsc`), `missing_and_empty_sources_are_errors`);
  unit tests in `src/lex.rs`, `src/map.rs` (tables valid, no duplicates, every hazard has a note),
  `src/convert.rs` (axis behaviour of Unity and Unreal Euler angles).
- `crates/marionette-cli/tests/port_e2e.rs`: `scan_map_and_convert`,
  `scaffold_writes_skeletons_and_refuses_to_overwrite`.
- Operations `port_scan`, `port_map`, `port_scaffold`, `port_convert`; generated
  `docs/ai/reference/porting-map.md`. Drift hash unchanged.
- Spawning, destroying, `OnDestroy`/`EndPlay`, timers (`Invoke`, `SetTimer`, `SetLifeSpan`)
  and coroutine waits are mapped to the script API (decision 0021); timer and coroutine targets
  become exported functions in skeletons; the tsc test uses the generated
  `docs/ai/reference/marionette.d.ts`.
- Next: map trigger hooks and raycasts when the porting tables are revisited; Unity prefab and
  scene YAML to Marionette scenes (entities, components, converted transforms) would remove
  most of step 3 of the porting recipe.

Line 9, proof scenes (W8, decision 0050, `docs/ai/PROOF-SCENES.md`):
- `crates/marionette-cli/tests/proof_scenes.rs`: `every_game_directory_is_listed`,
  `recipes_rebuild_the_committed_scenes` (each `build.ops.json` replayed by `batch_run` gives
  the committed scene bytes), `scenario_tests_pass_twice_with_identical_results`,
  `each_game_is_played_with_scripted_input_and_pins_its_hash`.
- Engine features added for the games: `crates/marionette-cli/tests/game_api.rs`
  (`resources_are_set_and_removed_through_operations`,
  `batches_stop_at_the_failing_step_and_do_not_nest`,
  `scripts_cast_rays_that_skip_sensors_and_the_caster`,
  `script_driven_characters_face_turn_and_jump_once`,
  `emit_without_detail_assertions_on_script_state_and_event_counts`,
  `observe_operations_replay_scripted_input`). Drift, playground, scripted and spawner hashes
  unchanged.
- Lantern Keeper: `marionette --project games/lantern-keeper op test_run '{}'` (play: wins
  with all five lanterns and three lives; idle: caught three times, game over).
- Ember Run (`games/ember-run`): play wins with 14 of 15 embers and two hits; pit loses all
  lives. Cloudstep (`games/cloudstep`): play reaches the beacon in about 11 s with no falls;
  plunge falls repeatedly. Same `test_run` command with their `--project`.
- Also tested in `game_api.rs`: `characters_pass_through_sensors_trigger_them_and_can_be_teleported`.
- Not yet: 90 fps measured on a mid-range desktop GPU per game; pooled or generated assets.

## Animation (decision 0028)

- `cargo run -q -p marionette-cli -- run examples/animation.scene.json --headless --ticks 600 --seed 1`
  prints `f409e570ea3b9d3c1e92a01adc3070dbbd49b39d147b0dd97caa39297dba6d13` (twice in a row).
- Reference image `examples/reference/animation.png` (320x180, 15 ticks, seed 1, MSAA 4).
- Tests: `crates/marionette-cli/tests/animation.rs` (recorded hash twice, snapshot mid-clip
  and restore, bones move and loop exactly, cross-fade and `anim_finished`, unknown clips
  refused, script `playClip` equals `anim_play`, clip listing, reference image, 64 skinned
  characters in 4 draw calls, morph targets change the mesh); `crates/marionette-anim/tests/puppet.rs`;
  unit tests in `crates/marionette-anim/src/lib.rs`, `crates/marionette-anim/src/sample.rs` and `crates/marionette-assets/src/rig.rs`.
- Test character `examples/assets/puppet.glb` (CC0, made by
  `cargo run -p marionette-anim --example make_puppet -- examples/assets/puppet.glb`,
  byte-identical every run).
- Not yet: root motion, additive layers, bone masks, animation events, IK; FBX blend shapes
  (the FBX converter of the FBX branch writes skins and clips; blend shapes not yet).

## Performance (workstream WP)

Proofs: `cargo test -p marionette-cli --test perf_proofs` (thread-independent factory hash,
draw calls and instances with 2 eyes); `cargo test -p marionette-jobs`;
`cargo test -p marionette-factory`; timing budgets in release:
`cargo test --release -p marionette-cli --test perf_budgets -- --ignored --test-threads=1`.
Numbers and budgets: `docs/ai/PERFORMANCE.md`.

Line 8, generation (W4, decision 0045, guide `docs/ai/GENERATION.md`):
- `crates/marionette-generate/tests/generate.rs`: `three_assets_rebuild_identically_from_recipes`
  (image, tileable texture, model from the image; rebuilt from the cache offline and from
  scratch with identical bytes; license and source recorded; pool blocked),
  `engine_formats_textures_are_powers_of_two_and_models_stand_on_the_ground`,
  `zero_cap_refuses_before_anything_is_sent`, `a_paid_image_is_paid_once_and_then_comes_from_the_cache`,
  `a_paid_model_needs_a_person_to_approve_its_image`, `a_run_over_budget_is_refused_up_front`,
  `unusable_provider_output_is_refused_and_not_written`, `bad_recipes_are_refused`,
  `generated_assets_and_their_copies_are_blocked_from_the_pool`,
  `per_provider_caps_unlimited_images_and_a_capped_model_provider`; unit tests in
  `src/texture.rs`.
- `crates/marionette-cli/tests/generate_e2e.rs`: `recipes_build_rebuild_and_list_through_the_cli`,
  `paid_generation_without_a_cap_is_refused_clearly`, `only_a_person_approves_an_image`.
- Providers: `a_provider_can_have_its_own_cap_or_none` in `crates/marionette-providers/tests/providers.rs`;
  unit tests in `src/keys.rs`; `cargo test -p marionette-providers --features net --test network`
  (transport against a server on the same machine).
- Not covered yet: live calls (formats `verified: false`), Tripo auto-rig and rig checks,
  decimation, extra texture maps, a cache shared across projects.
## Job TypeScript (workstream WJ)

Proofs: `cargo test -p marionette-jobs-ts` (same hash on 1, 2, 3 and 8 threads; pinned hash
`994155611f9d326a5f21b3676f2604afef1d4afb291c268990b8cc6644b73f0a` in `pinned_hash`;
bit-identical to the Rust job body and to the QuickJS interpreter; errors with code, line,
column and fix; index trap; time limit; hot reload); `cargo test -p marionette-cli --test
jobs_ts_e2e`. Benchmark: `cargo run --release -p marionette-jobs-ts --example jobs_ts_bench`
(decision 0030). Guide: `docs/ai/JOBS-TS.md`. Not yet usable from scenes or scripts (WJ next).

## UNVERIFIED

- CC0 pool online adapters (Poly Haven, ambientCG, Kenney and Quaternius archives, decision 0041)
  are tested only against scripted answers shaped like the real APIs as read on 2026-11-12; the
  build machine's network blocks those sites, so no live search or download has run.

- Unity prefab import (decision 0040) is tested only on hand-written prefab files in Unity's text
  format, not on prefabs saved by the Unity editor or on a real pack; the model-root override
  rule, the nested file-id rule and the half turn for converted models are from knowledge of
  Unity's serializer, not checked in the editor.

- Linux CI passes on `main` since the providers merge (ALSA fix in the workflow); Windows CI
  passes, including the GPU tests on WARP. In a Linux container without a GPU the drift hash,
  the seed-2 hash, the playground hashes (`line6_proofs.rs`) and the scripted example hash
  match; GPU tests skipped there.
- `marionette-script` (rquickjs/QuickJS-ng and oxc) on Windows MSVC: see the CI run of the
  scripting merge; not built on the owner's desktop yet.
- `marionette-jobs-ts` on Windows: wasmtime/Cranelift on MSVC and the pinned job hash equal to
  Linux are expected from CI on the job TypeScript branch; not yet observed.
- Job TypeScript speed on the owner's desktop and on ARM CPUs (only a 4-core x86-64 cloud
  container measured).
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
- Porting tools against real projects: fixtures are small hand-written Unity and Unreal samples
  and one Blueprint text snippet. Table entries for Unity and Unreal APIs are from knowledge of
  those engines, not checked against their current documentation. The skeletons type-check
  against the generated script typings (decision 0022) only in tests with `tsc` installed.
- Generation sample answers (`crates/marionette-generate/tests/recorded/`) and the Tripo upload
  endpoint are hand-written from documentation; the network transport has only talked to a
  server on the same machine. Built with `--features net` only in Linux CI and the container.
- Provider plug-ins (`claude`, `qwen`, `tripo`): request and answer formats are written from
  documentation and tested only against recorded shapes; none has been called live
  (`verified: false`). The Qwen and Tripo prices and endpoints are from memory. The mock PNG
  pin depends on the locked `png` encoder version.
- Skinned and morphed characters have been rendered only on Mesa lavapipe (Linux container);
  never on a desktop GPU, never with a real (non-generated) rigged character or a real FBX
  export.
- Performance on a real desktop GPU and headset: all timings come from a Linux container with
  Mesa lavapipe. `bench --headset` emulates two eyes offscreen; no OpenXR runtime has been used.
- FBX conversion has only seen files written by Blender and Assimp (and ASCII made from them).
  FBX from Maya, 3ds Max, MotionBuilder or Unity's own exporter (pivots, pre-rotation, cubic
  curves, `InheritType` other than RSrs) is untested; a real Asset Store pack is the next check.

- Unreal importer on real files: nothing from a real Unreal project has been imported. T3D
  material text is hand-written from knowledge of the editor's export format; the package
  class and `RelativeFilename` peek is tested on synthetic bytes; FBX from Unreal's own exporter
  is untested (the fixture is Blender writing Unreal's convention: +Z up, centimeters). Next
  check: a dry run over a project the owner has, with their permission.

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
| 2026-11-12 | Line 8 part (W2): `marionette-import` crate (owned library, declarations, refusal rules, Unity project and `.unitypackage` import, material mapping, pool gate), six `library_*`/`license_*`/`pool_*` operations, decision 0018. Drift hash unchanged. | branch `w2-owned-library` merged |
| 2026-11-12 | W2 follow-up: read-only check against real Unity files; textures typed by content, not extension. | this commit |
| 2026-11-12 | Line 7 (W5): `marionette-providers` crate (interface, mock, Claude/Qwen/Tripo as HTTP-as-data, `providers.json`, spend cap and ledger), four `provider_*` operations, `marionette provider` CLI, decision 0019. CI: first run checked, Linux fixed (ALSA). Drift hash unchanged. | branch `claude/project-thread-zgz6yk` |
| 2026-11-12 | Line 4 (W7): spike TypeScript against Luau (`spikes/script-spike`), decision 0022 (TypeScript on QuickJS-ng), crate `marionette-script`, `Stage::Script`, `Script`/`ScriptState` components, `script_reload` and `script_types` operations, `marionette.d.ts` in the generated reference, scripted example. Drift and playground hashes unchanged. | branch `claude/project-thread-ylu6qj` |
| 2026-11-12 | W10: `marionette-port` crate (C#/C++/Blueprint-text scanner, Unity and Unreal concept tables, TypeScript skeletons, coordinate conversion), four `port_*` operations, `docs/ai/PORTING.md`, generated porting map, decision 0020. Drift hash unchanged. | branch `claude/project-thread-um7abe` |
| 2026-11-12 | Scripts spawn, clone and destroy entities (`onDestroy`, `DestroyAt`, `SpawnIds`) and set timers (`ScriptTimers`), decision 0021; physics follows spawned and removed bodies; `examples/spawner.scene.json`; porting tables and skeletons map spawning, destroying, timers and coroutine waits. Drift, playground and scripted hashes unchanged. | branch `claude/project-thread-lnrax1` |
| 2026-11-12 | Line 8 part (W2 follow-up): FBX to glTF conversion in `marionette-import` (binary and ASCII FBX 2011+, nodes, meshes, materials, textures under the license gate, skins, animation), Blender-referenced tests, decision 0027. Drift hash unchanged. | branch `claude/project-thread-zbl2wy` |
| 2026-11-12 | FBX blend shapes to glTF morph targets (default weights, `weights` animation channel), Blender-referenced test, decision 0027 updated. Drift hash unchanged. | branch `claude/project-thread-zbl2wy` |
| 2026-11-12 | Performance (WP): instanced multi-view renderer, bulk draws, `Registry::init_world`, `marionette-jobs`, `marionette-factory`, `perf_factory_scene`/`perf_sim`, `bench --headset`, perf proofs and CI budget job, decisions 0023 to 0026, `docs/ai/PERFORMANCE.md`. Drift hash unchanged. | branch `claude/project-thread-6udfc0` |
| 2026-11-12 | Animation: `marionette-anim` crate (`Animator`, cross-fades, deterministic sampling, `Pose`), glTF rigs/skins/clips/morph targets in `marionette-assets`, GPU skinning and blend shapes in the instanced renderer, `anim_*` operations, script `playClip`/`blendClip`/`stopClip`, `examples/animation.scene.json`, self-made `puppet.glb`, reference image, decision 0028. Drift hash unchanged. | branch `claude/project-thread-9hibpc` |
| 2026-11-12 | Line 9 started (W8): `games/lantern-keeper`, proof harness, operations `resource_set`/`resource_remove`/`batch_run`, `engine.raycast`, facing-relative characters, `CharacterInput`, emit fix, scenario paths into script state and event counts, observe with input, decision 0050. Existing hashes unchanged. | branch `claude/project-thread-2vp8na` |
| 2026-11-12 | W8: `games/ember-run` (2D side-scroller), `games/cloudstep` (3D platformer); sensors report every body type and do not block characters, scripts teleport characters and dynamic bodies via `Transform`, `Entity.name`. Existing hashes unchanged. | branch `claude/project-thread-2vp8na` |
| 2026-11-12 | Line 8 part (W6): Unreal importer in `marionette-import` (`src/unreal.rs`: project, plugin and content folders, engine-content rules, package diagnosis, T3D materials with ORM and normal-map handling; shared material writer; BMP textures), decision 0029. Drift hash unchanged. | branch `claude/project-thread-iag2pq` |
| 2026-11-12 | Line 8 part (W4): `marionette-generate` crate (recipes, request-keyed cache, provenance, person-only image approval, PNG and GLB post-processing), operations `generate_recipe`/`generate_plan`/`generate_run`/`generate_list`, `marionette generate`, providers: Tripo local-image upload, `net` feature transport, keys files, per-provider caps; decision 0045, `docs/ai/GENERATION.md`. Drift hash unchanged. | branch `claude/project-thread-3n87gn` |
| 2026-11-12 | Line 8 part: Unity prefab import (Unity YAML reader, nested prefabs, variants, overrides, component mapping, library kind `prefab`, `prefab_instantiate`), decision 0040. Drift hash unchanged. | branch `claude/project-thread-3xcar0` |
| 2026-11-12 | Line 8 part (W3): `marionette-pool` crate (pool layout and per-asset proof, admission rules, owned gate, generated-asset dedication gate, Poly Haven and ambientCG adapters, archive ingest for Kenney and Quaternius, ordered asset search), eight pool operations, seed set in `pool/`, decision 0041. Drift hash unchanged. | branch `claude/project-thread-3xcar0` |
| 2026-11-12 | WJ: job TypeScript, crate `marionette-jobs-ts` (TS subset to WebAssembly, wasmtime runtime on `marionette-jobs`, safety levels, `JobLibrary` hot reload), operations `jobs_check`/`jobs_run`/`jobs_types`, `examples/jobs/particles.job.ts`, benchmark, decision 0030, `docs/ai/JOBS-TS.md`. Drift hash unchanged. | branch `claude/project-thread-p3he01` |
