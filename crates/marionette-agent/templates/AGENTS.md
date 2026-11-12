# Working rules for agents

`CLAUDE.md` and `AGENTS.md` hold the same rules.

1. Read `PROTOCOL.md` first and follow it. It is the project owner's rules: what to ask before
   building, what needs approval, who to hand work to.
2. Work through Marionette operations, from the CLI (`marionette op <name> '<json>' --json`,
   list them with `marionette ops`) or the MCP server (`marionette mcp`). Both run the same
   operations.

## The working loop

1. Validate: `project_validate`.
2. Edit through operations only: `scene_create`, `entity_create`, `entity_update`,
   `entity_delete`, `component_set`, `component_remove`. Each edit is checked against the schema
   before anything is written and leaves the file in canonical form. A refused edit changes
   nothing and returns diagnostics (code, JSON pointer, entity, suggested fix).
3. Run: `scene_run` for a one-off state hash, or `session_start` + `session_step` to keep a
   world loaded between calls. Edits with `session` change only that live world;
   `session_save` writes it back to a scene file.
4. Observe: `observe_tree`, `observe_query`, `observe_entity`, `observe_screenshot` (returns a
   PNG path), `observe_logs`, `observe_stats`. Results are paged; ask for more with `offset`.
5. Test: put scenarios in `tests/*.test.json` (scene, seed, ticks, assertions, optional
   reference image) and run `test_run`. Every behavior you build gets a scenario.

## Determinism

Same scene, seed and ticks must give the same state hash. Do not depend on wall-clock time,
hash-map order or thread timing in simulation code.
