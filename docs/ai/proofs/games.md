# Proofs: Proof scenes (plan line 9)

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

## Game starter (decision 0150)

- `cargo test -p marionette-cli --test starter`: every template from a prompt is won by its
  autopilot and passes its own tests; the same brief gives byte-identical files; refresh keeps
  edits outside CONFIG and refuses a script without markers; bad briefs list every problem.
- `cargo test -p marionette-starter`: mazes are connected, platformer gaps are at most 3 wide,
  same seed same level, prompt reading.
- Demo games (in `proof_scenes.rs` and the stability suite): `games/gearbug-garden` (3D maze),
  `games/comet-hopper` (platformer); `examples/starter-maze` (2D maze from a prompt, placeholder
  art).
- First-try autopilot wins on sampled seeds (container, 2026-11-13): mazes 32 of 32; platformer
  about 90% (spikes, edges and walking enemies cause the rest); `game_make` tries further seeds.
