# Proofs: Core and workspace (plan lines 1 and 2)

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

Parent and child transforms (decision 0272):
- `crates/marionette-core/tests/hierarchy.rs`: `children_are_placed_at_load_and_follow_every_tick`,
  `links_changed_while_running_are_followed`, `a_cycle_is_refused_with_its_own_code`,
  `local_is_the_inverse_of_compose`, `snapshots_and_reruns_give_the_same_hash`.
- `crates/marionette-physics/tests/children.rs`: `a_collider_on_a_child_moves_with_the_parent`,
  `a_child_moved_by_physics_is_refused`, `a_child_of_a_falling_body_is_on_it_in_the_same_tick`.
- `crates/marionette-agent/tests/hierarchy.rs` (`entity_attach` in a file and a session, a
  parent moved by a script) and the `ChildOf` steps in `tests/indexed_edits.rs`.
- `cargo test --release -p marionette-cli --test engine_budgets -- --ignored hierarchy_budgets`:
  10,000 children under 2,500 moving parents, 1.3 ms per tick (release, 4-core container).
