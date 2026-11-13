# Proofs

Commands and tests that prove each done part of the engine. Read only the section for the
part you are changing. Moved out of STATE.md (decision 0061); add new proofs here.

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

Opt-in looks (decision 0063):
- Example `examples/lighting.scene.json`, built by `examples/lighting.ops.json`
  (`marionette op batch_run '{"file":"examples/lighting.ops.json"}'` after deleting the scene).
  `run examples/lighting.scene.json --headless --ticks 60 --seed 1` prints
  `5c7cb06da50ec51e24fafdaea3ba1a00c05902d7ecc570c2ef6535c6e8c0cecb` (ground widened to 300 m
  on 2026-11-13 so fog hides its edge, decision 0066).
- Reference image `examples/reference/lighting.png` (320x180, 60 ticks, seed 1, MSAA 4, made on
  Mesa lavapipe): `2e870ee61d22184d0e725de331ae613d9fe149dd17fb381948176565aec1787e`.
- Tests: `opt_in_looks_change_the_image_only_when_present` and the lighting entries of
  `matches_reference_images` and `same_inputs_give_identical_images` in
  `crates/marionette-render/tests/render.rs`; stability mode `render-3d-lighting`.

More looks (decisions 0064 to 0067):
- Example `examples/looks.scene.json` (bloom, a street lamp with a glTF emissive lantern and a
  shadow-casting point light, a shadow-casting spot light, mirror, brushed and glossy balls,
  environment from `SkyLight` colors with sky background, fog), built by
  `examples/looks.ops.json`. `run examples/looks.scene.json --headless --ticks 60 --seed 1`
  prints `8ef0a4c6887a8796e767348f6176bacf231f761c81155244ddb80e55ac7042e4`.
- Reference image `examples/reference/looks.png` (320x180, as above, Mesa lavapipe):
  `c338965a9568ec8fb3ec7a464fb2dd64057ef97967b68047dd9bc98161f56b16`.
- Tests in `crates/marionette-render/tests/render.rs`:
  `point_and_spot_lights_cast_shadows_only_with_cast_shadows`,
  `bloom_spreads_bright_light_and_only_with_bloom`,
  `environment_lights_reflects_and_draws_the_sky`, `imports_gltf_emissive`, the looks entries
  of the reference, determinism and GPU-object tests; unit tests in `post.rs` and
  `environment.rs`. Existing reference images (showcase, sprites, ui) unchanged; lighting
  within tolerance before its ground change. Stability mode `render-3d-looks` (two eyes).
- Desktop GPU (RTX 2080 class, Vulkan, 1920x1080, MSAA 4, 2,000 frames, release, two runs
  each): lighting 0.45 ms avg (p99 0.69); looks 1.01 ms (p99 1.35); looks two eyes 1.67 ms
  (p99 2.14); looks without `Bloom` 0.63 ms, without `CastShadows` 0.86 ms, without
  `Environment` 0.97 ms. So bloom costs about 0.38 ms, two shadow-casting lights (seven maps)
  0.15 ms, the environment 0.05 ms. Worst frames 5 to 7 ms in every case, baseline included.
  GPU tests passed there (`MARIONETTE_REQUIRE_GPU=1`), release stability `render-3d-looks`
  passed with frame p99 1.21 ms (budget 11.1 ms). Command:
  `cargo run --release -q -p marionette-cli -- bench examples/looks.scene.json --frames 2000 --size 1920x1080 [--eyes 2]`.

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

Stability suite (decision 0031, `docs/ai/STABILITY.md`):
- `cargo run --release -q -p marionette-cli -- stability` passes every mode (twenty-seven when first written)
  (core-headless, physics-3d, physics-2d, scripted, scripted-spawning, render-3d,
  render-3d-play, render-2d, render-2d-play, render-ui, animation, vr-headset, factory-sim,
  factory-vr, proof-lantern-keeper, proof-cloudstep, proof-ember-run, proof-crystal-ward,
  proof-dust-circuit, proof-signal-yard (each proof game also `-render`),
  agent-session); `--long --seeds 1` passed for every mode up to proof-ember-run (the last
  three proof games have the fast suite only so far) (Linux container, lavapipe, release;
  factory-vr frame p99 12.4 ms at 256x256 per eye on the software GPU, under the CI scale).
- Tests: `crates/marionette-cli/tests/stability.rs` (`fast_suite_passes_for_every_mode`,
  `drift_mode_reaches_the_pinned_drift_hash`, `injected_panic_is_reported_with_tick_and_backtrace`,
  `injected_hang_is_detected`, `nondeterminism_is_caught_and_the_crash_report_replays`,
  `budget_miss_is_caught`, `the_cli_writes_crash_reports_and_exits_1`, `memory_growth_is_caught`,
  `every_scene_has_a_stability_mode`, `bad_suites_and_modes_are_refused_as_data`).
- CI: job `stability` in `.github/workflows/ci.yml` (fast, release, budgets x2);
  `.github/workflows/stability-soak.yml` (nightly long soak, 3 seeds).
- Hardware GPU (2026-11-13, main at `e8814b5`, NVIDIA GeForce RTX 2080 on Vulkan, Windows,
  release, budget scale 1, timing enforced): all 31 fast modes passed, frame budgets checked
  (`frame_budget_checked: true`) in every render mode; no panic, hang or driver warning. Tick
  p99 (budget 2 ms): highest `proof-signal-yard` 1.19 ms, `proof-crystal-ward` 0.90,
  `proof-ember-run` 0.84, `agent-session` 0.69 (budget 4), `factory-sim` 0.036. Frame p99
  (budget 11.1 ms, size per mode in the suite): `proof-crystal-ward-render` 1.61 ms,
  `proof-ember-run-render` 1.45, `proof-signal-yard-render` 1.38, `proof-cloudstep-render`
  1.32, `render-3d-looks` (two eyes) 1.18, `job-swarm-render` 1.00, `animation` 0.91,
  `factory-vr` (two eyes) 0.50, `vr-headset` (two eyes) 0.42, the rest 0.40 to 0.69. Worst
  single frames after warm-up were over budget in two modes, `proof-crystal-ward-render` 42 ms
  and `proof-signal-yard-render` 32.5 ms: a script spawns a model no earlier frame drew and it
  was decoded inside the frame (about 28 ms). Fixed by decision 0068. Windows had no memory
  check in that run (resident size only on Linux); decision 0069 adds one.
- Late model streaming (decision 0068), lavapipe, release, seed 1, worst frame after warm-up
  before and after: `proof-crystal-ward-render` 61.1 to 29.5 ms (p50 about 19 ms, the
  software rasterizer), `proof-signal-yard-render` 48.8 to 20.5 ms.
- The same on the hardware GPU (RTX 2080, branch head `9a4b679`): the full fast suite passed
  again, every mode with the heap probe (growth 0 to 0.24 MiB, `factory-sim` 4.0,
  `factory-vr` 1.06, job modes about 1.0). Worst frame after warm-up, in the suite and three
  more runs each: `proof-crystal-ward-render` 5.1 to 6.2 ms (was 42), `proof-signal-yard-render`
  5.4 to 7.3 ms (was 32.5); p99 1.9 to 5.7 ms. In one of four `proof-signal-yard-render` runs the
  heap grew 12.6 MiB (limit 64; others 0.15): a streamed model that finished decoding after the
  warm-up sample is counted as growth. One-time, not a leak.
- Heap probe (decision 0069), `factory-sim` seed 1, debug: counted heap +4.0 MiB after warm-up
  at 300 and at 1,500 ticks (resident +33.8 and +23.6 MiB): no leak.
- Known finding: `scripted` at 216,000 ticks grows 48 MiB (unbounded `EventLog`, about 670
  bytes per event); the nightly soak fails on it until the event log is bounded.
Memory and crash hygiene (decision 0032, rules in `docs/ai/MEMORY-AND-CRASHES.md`):
- `crates/marionette-agent/tests/leaks.rs` (`memory_stays_flat_under_repeated_work`: session
  load/unload, a long run past the event-log bound, script hot reload, entity churn,
  screenshots; heap growth under 64 KB per scenario, counted by `marionette_host::alloc`).
- `gpu_objects_come_back_after_repeated_setups_and_trims` in
  `crates/marionette-render/tests/render.rs`.
- `crates/marionette-host/tests/crash.rs` (`children_die_when_the_engine_is_killed`,
  `a_killed_process_leaves_a_run_folder_that_the_next_start_removes`,
  `shutdown_removes_the_run_folder_and_temp_dirs_clean_up_on_drop`);
  `crates/marionette-cli/tests/crash.rs` (`a_run_killed_mid_simulation_is_cleaned_up_by_the_next_start`,
  `a_clean_exit_leaves_no_run_folder`).
- `crates/marionette-script/tests/misbehaving.rs` (endless loop, out of memory, deep recursion,
  engine call at load); `a_panicking_operation_is_an_error_and_the_agent_keeps_serving` in
  `crates/marionette-agent/src/lib.rs`; `the_event_log_is_bounded_and_positions_stay_absolute`
  in `crates/marionette-core/src/module.rs`.
- CI job `leaks (valgrind)`: core, host and script tests under valgrind, no definite leaks
  (checked locally on Linux).

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
  `crates/marionette-pool/tests/pool.rs` (`the_committed_seed_pool_verifies`, `seed_pool_matches_its_generator`,
  `made_assets_need_a_method_and_cc0_license_files`, `generated_assets_need_a_persons_dedication`,
  `owned_assets_and_copies_of_them_are_refused`,
  `online_assets_need_a_known_library_its_domain_and_license_file`,
  `poly_haven_textures_and_models_are_ingested_with_checks`,
  `ambientcg_materials_are_ingested_from_their_archives`, `search_follows_the_required_order`);
  unit tests in `src/net.rs` (MD5) and `src/zip.rs`; `crates/marionette-cli/tests/pool_e2e.rs`
  (`pool_operations_search_fetch_add_and_refuse`). Seed set in `pool/` rebuilt by
  `cargo run -p marionette-pool --example make_seed -- <folder>`. Operations `asset_search`,
  `pool_info`, `pool_list`, `pool_get`, `pool_add`, `pool_ingest`, `pool_fetch`, `pool_index`,
  `pool_verify` (the e2e test also breaks a pool on purpose). The public pool is the repository
  `knux3k/marionette-assets`, whose CI runs `pool_verify`.

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
- Not yet: root motion, animation events, IK, state machines; FBX blend shapes (additive
  layers and bone masks: decision 0074, next section)
  (the FBX converter of the FBX branch writes skins and clips; blend shapes not yet).

## Blend spaces, overlays, 3D sound, music layers (decisions 0074, 0075)

- Scene `examples/blend-and-sound.scene.json`, built by
  `cargo run -q -p marionette-cli -- op batch_run '{"file":"examples/blend-and-sound.ops.json"}'`
  (scripts `examples/scripts/strider.ts`, `orbit.ts`, `conductor.ts`).
- `cargo run -q -p marionette-cli -- run examples/blend-and-sound.scene.json --headless --ticks 600 --seed 1`
  prints `d344027414b14c0f555f9de4b443777a25a1b6f07886cda765a49bfd0938e236`.
- Tests: `crates/marionette-cli/tests/blend_and_sound.rs` (recorded hash twice, synced space
  phase runs at the blended cycle length and parameters glide, a space at an edge poses like
  that clip, a masked overlay changes only masked bones and their children, unknown clips in
  spaces and overlays refused, the orbiting hum pans right, muffles behind and pans left
  through `audio_mix`, music layers fade in by intensity and out through `audio_music`); unit
  tests in `crates/marionette-anim/src/blend.rs` (1D and 2D weights, parameter glide,
  quaternion helpers), `crates/marionette-audio/src/lib.rs` (distance curves, rear muffle and
  head turn, Doppler, stems decode) and `crates/marionette-audio/src/music.rs` (levels, stems
  loop without a click).
- Stability mode `blend-and-sound` (render).
- Drift hash and every earlier pinned hash unchanged (new components only).
- Not yet: heard through a real audio device or headset; HRTF, occlusion, reverb.

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
(decision 0030). Guide: `docs/ai/JOBS-TS.md`.

From scenes and scripts (decision 0070): `cargo test -p marionette-cli --test job_scenes`:
`examples/swarm.scene.json` 600 ticks seed 1 gives
`f39ab44c85fbb75a1edcf1182caf6249ad8b069953c616d238dacf4f49286f33` on 1, 2 and 4 threads;
snapshot and restore continue identically; schedule steps run in place every tick; bad steps
fail the load (`jobts.args`, `jobts.no_function`, `jobts.syntax`, with entity and component);
a trapping step records `job_error` and the tick goes on; scripts run jobs on job arrays and
on their own typed arrays and catch `jobts.*` errors; `jobs_reload` swaps a changed file and
keeps the last good one; `JobDraw` gives one batch of 4,096 copies and matches
`examples/reference/swarm.png` (lavapipe). Stability modes `job-swarm` (debug: 0.28 ms per
tick average) and `job-swarm-render`. Speed (release, 4-core container, `perf_sim`): 4,096
particles 0.09 ms per tick; 131,072 particles 2.5 ms on one thread, 0.8 ms on four.

## Game Rust modules and cargo features (decision 0062)

- `cargo test -p marionette-example-rust-module` (built with `script` only): the game module
  is deterministic and settles at its anchor, the built-in engine refuses its component, the
  `spring_set` operation changes a live session and is refused read-only, duplicate or badly
  named parts are refused, and the `spring-game` binary runs scenes and lists only the
  operations of the parts it was built with (`examples/rust-module/tests/spring.rs`).
- `cargo clippy -p marionette-agent -p marionette-cli --all-targets --no-default-features -- -D warnings`
  passes, and with each single feature (`script`, `import`, `pool`, `generate`, `port`,
  `jobs-ts`).
- Linux debug build: example binary 489 linked crates and 103 MB, full `marionette` 559 and
  147 MB, `marionette-cli --no-default-features` 374 crates.

## Porting Burst jobs (decision 0072)

- `cargo test -p marionette-port --test port unity_jobs_partials_and_tuples`: fixture
  `tests/fixtures/unity-jobs` (job struct, data struct, partial class over two files, tuple
  types): role `job`, summary `jobs`, one merged partial class, native-container and `math.*`
  statuses, and the `jobs.rs` skeleton compiles with `rustc` when it is on the path.
- Private slice of a real game (not in the repository): scan without a panic, 3 jobs found,
  `jobs.rs` compiled unchanged; the hand port ran 0.12 ms per tick at 4x and 0.2 ms at 8x
  (release, one thread, container), same state hash with 1 and 4 threads.

## Saving off the tick (decision 0073)

- `cargo test -p marionette-cli --test perf_proofs capture_saved_on_another_thread_equals_snapshot`:
  factory and swarm scenes; the scene built from `Sim::capture` on another thread equals the
  snapshot byte for byte.
- `cargo test --release -p marionette-cli --test perf_budgets -- --ignored save_capture_budget`:
  4x factory capture 0.75 ms (budget 2 ms); the whole snapshot as text 72 ms, 2.3 MB (container).

## Culling and level of detail (decision 0080)

In [RENDER-COST.md](RENDER-COST.md#proofs).
