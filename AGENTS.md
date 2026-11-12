# Working rules for agents (Marionette)

This file and `CLAUDE.md` hold the same rules.

**Open `docs/ai/START-HERE.md` first.** It gives the read order: rules, current state with proof
commands, workstreams, architecture, recipes and environment. `docs/DESIGN.md` defines what the
engine is.

## Always

- Build and check before every commit:
  `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- The drift hash must stay
  `7441258fdf64141911d44e0608c48498897564168684fdd86cd244e149926558`
  (`cargo run -q -p marionette-cli -- run examples/drift.scene.json --headless --ticks 1000 --seed 1`).
- Change scenes through operations (`marionette op <name> '<json>'`, `marionette ops`) or the
  MCP server (`marionette mcp`); both run the same code.
- Follow the determinism rules in `docs/ai/ARCHITECTURE.md`.
- Record every real choice in `docs/decisions/` and index it in `docs/ai/DECISIONS.md`.
- Regenerate `docs/ai/reference/` with `cargo run -q -p marionette-cli -- docs` when operations,
  components or CLI flags change.
- No names, identifying details or machine details in any file or commit message
  (`docs/ai/RULES.md`, decision 0016).
- Ask the owner before pushing, publishing, spending, deleting outside the repository, or
  opening windows or sound on a user's machine.
- End every session by updating `docs/ai/STATE.md`.
