---
name: engine-add-operation
description: Add or change an engine operation, component, resource or simulation system with schema, registration, tests and regenerated docs. Use when engine work (crates under crates/) needs a new operation, MCP tool, component type, resource or system.
---

# Add an operation, component or system (engine work)

Full recipes: `docs/ai/HOWTO.md` "Add an operation", "Add a component (or resource)",
"Add a simulation system". Read only the one you need. This page is the short path and the
traps.

## Operation

1. Pick the module in `crates/marionette-agent/src/` by group (the name up to the first `_`).
   A new optional part is a cargo feature (decision 0062).
2. Input and output structs with doc comments (they become the schema); inputs deny unknown
   fields. Handler `fn(&mut Agent, In) -> Result<Out, Errors>`; diagnostics with stable codes
   and a `fix`; never panic on input.
3. Register in that module's `ops()`. `mutating` is true when it writes files or changes a
   session.
4. Output stays small (focus 2): paged lists, paths instead of pixels, no echo of the input.
5. Extend `crates/marionette-cli/tests/agent_e2e.rs`.

## Component or resource

1. Derive `Component` or `Resource`, `Reflect`, `Debug`, `Clone`, `PartialEq`; doc comment on
   the type and on fields with units.
2. Register: core types in `crates/marionette-core/src/builtin.rs`, module types in the
   module's `register`.
3. A new visual feature is opt-in through scene data, so no existing reference image changes.

## System

1. Add it to a stage in the owning crate. Follow the determinism list in
   `docs/ai/ARCHITECTURE.md` "Determinism".
2. Test: run a scene twice with the same seed and compare state hashes.

## Always, after any of the three

1. `cargo run -q -p marionette-cli -- docs` and commit `docs/ai/reference/`.
2. If a skill under `.claude/skills/` names what you changed, update it in the same change;
   a new feature that an agent would use gets a skill (skill `engine-add-skill`).
3. Finish with skill `engine-finish-change`.
