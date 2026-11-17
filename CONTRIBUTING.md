# Contributing

Thanks for helping. Marionette is built mostly by AI agents with a person steering, and its
rules are written so any contributor, human or AI, can follow them.

## Before you start

- Read [AGENTS.md](AGENTS.md) and [docs/ai/START-HERE.md](docs/ai/START-HERE.md). They are
  short and say what to read next for your kind of change.
- For a larger change, open an issue first so it can be checked against the design
  ([docs/DESIGN.md](docs/DESIGN.md)) and the open decisions.

## Every change

1. `cargo fmt --all -- --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test`
4. `cargo run -q -p marionette-cli -- run examples/drift.scene.json --headless --ticks 1000 --seed 1`
   must still print `7441258fdf64141911d44e0608c48498897564168684fdd86cd244e149926558`. A change
   that alters a state hash or a reference image on purpose says so and why.
5. `cargo run -q -p marionette-cli -- docs` when operations, components or CLI flags change.
6. A decision record in `docs/decisions/` for every choice between real alternatives.

Rust code follows [docs/ai/CODE-RULES.md](docs/ai/CODE-RULES.md): no new `unsafe` outside the
listed C calls, no `unwrap` or `expect` outside tests, plain data first.

## Privacy and secrets

- No personal names, machine details or private paths in files or commit messages
  (`crates/marionette-cli/tests/repo_hygiene.rs` checks this).
- Never commit API keys. Keys live outside the repository (see
  [docs/CONNECT-YOUR-AI.md](docs/CONNECT-YOUR-AI.md)); CI scans every commit with gitleaks and
  fails on a finding.

## License

By contributing you agree that your work is licensed under MIT OR Apache-2.0, as in the README.
Assets added to the pool must be CC0 with proof of origin
([docs/ai/ASSETS-AND-LICENSES.md](docs/ai/ASSETS-AND-LICENSES.md)).
