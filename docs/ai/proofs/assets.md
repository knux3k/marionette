# Proofs: Providers, import, pool and generation (plan lines 7 and 8)

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
