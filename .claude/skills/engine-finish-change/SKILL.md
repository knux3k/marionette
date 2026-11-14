---
name: engine-finish-change
description: Close out an engine change with format, lint, tests, drift hash, regenerated reference docs, decision record, state, session log and hygiene rules. Use when about to commit or open a pull request in the engine repository, or when a hygiene, doc budget or generated-docs test fails.
---

# Finish a change (engine work)

## Checks, in this order

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -q -p marionette-cli -- run examples/drift.scene.json --headless --ticks 1000 --seed 1
```

The last line must print the drift hash in `CLAUDE.md`. If operations, components or CLI
flags changed: `cargo run -q -p marionette-cli -- docs`, and commit `docs/ai/reference/`.

## Records

1. A decision record for every choice between real alternatives: next free number in
   `docs/decisions/` (check the default branch right before choosing; other work takes
   numbers too), sections as in `docs/ai/HOWTO.md` "Add a decision record", one line in
   `docs/ai/DECISIONS.md`.
2. `docs/ai/STATE.md`: status per plan line and the UNVERIFIED list only. Proofs (hashes,
   test names, measurements) go in the area file under `docs/ai/proofs/` (index and rules:
   `docs/ai/PROOFS.md`).
   A feature's level (slice, usable, proven) and its evidence go in `docs/ai/STATUS.md`:
   add or move its row when the change adds a feature or new evidence (decision 0274).
3. One row in `docs/ai/SESSION-LOG.md`.
4. Mark gaps you closed in `docs/ai/FOCUSES.md`.
5. A skill that names what you changed is updated in the same change
   (`crates/marionette-cli/tests/skills.rs` fails on stale names).

## Hygiene (tests enforce these)

- No personal names, identifying details, machine details or absolute paths in any file or
  commit message. Say "the project owner", "the engine repository", a generic hardware class.
  Patterns: `docs/ai/forbidden-patterns.txt`.
- Commit identity `Marionette Contributors`; AI credit is a plain `Co-Authored-By: Claude`
  trailer with no address and no links.
- The always-read docs stay under their size budget
  (`crates/marionette-cli/tests/doc_budget.rs`): move detail out instead of raising a limit.
- Nothing measured on one machine is stated as general: say where a number came from, and
  list what was not checked as UNVERIFIED.

## Needs the owner first

Pushing, publishing, spending, deleting outside the repository, opening windows or sound on
a user's machine. Design choices do not: decide, record, continue.

## When a test fails

| Test | Usual cause |
| --- | --- |
| `generated_reference_docs_are_current` | run the docs command above |
| `every_scene_has_a_stability_mode` | skill `engine-add-scene-or-mode` |
| `always_read_docs_fit_the_budget` | a doc grew; split it by task |
| the skills tests | a skill names an operation, flag, type or path that no longer exists |
