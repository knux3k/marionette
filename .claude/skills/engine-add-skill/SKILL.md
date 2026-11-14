---
name: engine-add-skill
description: Add or update an agent skill that ships with the engine. Use when an engine feature, operation group or workflow lands, a skill is out of date or fails its test, or when deciding whether knowledge belongs in a skill or in docs/ai.
---

# Add or update a skill (engine work)

Skills are how the engine tells an agent what to do for one kind of task. The agent always
sees every description and loads a body only when the task matches, so descriptions are the
permanent cost and bodies are free until used (decision 0160).

## Where things go

| Knowledge | Home |
| --- | --- |
| A procedure for one kind of task | a skill here |
| Rules every session needs | `CLAUDE.md` and `docs/ai/RULES.md` |
| Exact fields, flags, signatures | generated reference (`docs/ai/reference/`), never copied into a skill |
| Why something is the way it is | a decision record |

## Two kinds

- `marionette-*`: for people's games. Installed into game projects by `skills_install` and
  `project_init`, so the body may name operations, CLI commands and component types but no
  path in the engine repository (it would not exist there).
- `engine-*`: for work on the engine itself. May name repository paths and doc sections.

## Steps

1. Create `.claude/skills/<name>/SKILL.md`. Front matter: `name` (the folder name) and a
   one-line `description` that says what it does and then `Use when ...` with the words a
   task would contain. The description decides whether the skill is ever loaded.
2. Body: the shortest path through the task, the traps, and the checks that prove it worked.
   Imperative steps, real commands. Point at reference instead of repeating it.
3. Register it in `crates/marionette-agent/src/skills.rs` (one line; name the cargo feature
   it needs, if any).
4. Add it to the index in `AGENTS.md`.
5. `cargo test -p marionette-cli --test skills`. It checks every operation, CLI command and
   flag, component type, repository path and quoted doc heading the skill names, the size
   limits, and that the index and registry match the folders.

## Limits (tested)

Description at most 400 characters; body at most 4,000 bytes; all descriptions together at
most 6,000 bytes. A skill that needs more is two skills.

## Writing rules

- Name things exactly and in backticks: the test only checks what is in backticks.
- No personal names, machine details or private project details (`docs/ai/RULES.md` rule 8).
- Say what only a person may do (approvals, license declarations, spending).
- A removed feature removes its skill in the same change.
