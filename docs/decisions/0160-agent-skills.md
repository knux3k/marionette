# 0160: Task skills that agents load by themselves

Date: 2026-11-13. Status: accepted. Focus 2 (token use) first; focus 1 (modularity).

## Context

The engine's goal is to be the better choice for building games with an AI agent. Until now
the engine's know-how reached an agent in two ways: docs under `docs/ai/` that the agent had
to pick from a table and read, and operation descriptions. A game in its own repository got
only the short project template. An agent porting a script or lighting a scene started from
general knowledge instead of the tested path.

Agent Skills are an open format (a folder with a `SKILL.md`: front matter `name` and
`description`, then instructions). A skill-aware agent keeps every description in context and
loads a body only when the task matches. Read on 2026-11-13: the format specification, and
the documentation of two agents. One reads project skills from `.claude/skills`; another reads
`.github/skills`, `.claude/skills` and `.agents/skills`.

## Decision

1. Skills live in `.claude/skills/<name>/SKILL.md` in the engine repository, in the open
   format with only the two required front matter fields.
2. Two kinds, by name. `marionette-*` are for games (hand work on, build a scene, find assets, generate
   assets, write a script, write a job, port Unity or Unreal code, light a scene, animate,
   test, find a slowdown, add Rust code). `engine-*` are for work on the engine (add an
   operation, add a scene or mode, finish a change, add a skill).
3. The files are compiled into `marionette-agent` (`src/skills.rs`), so they travel with the
   engine, including inside a game's own binary:
   - `skills_install` writes the game skills into a project as `.claude/skills` and
     `.agents/skills`; `project_init` does it for a new project. Engine skills are not
     installed. A skill whose cargo feature is left out of the build is not listed.
   - `skills_list` and `skills_get` give the same progressive loading to any agent through
     the CLI or MCP, with no skill support needed. The `skills` group is in the MCP `core`
     preset, and the server instructions name it.
   - `AGENTS.md` (engine and project template) carries a plain index.
4. A test (`crates/marionette-cli/tests/skills.rs`) checks every skill against the engine:
   each backticked operation, CLI command and flag, component or resource type, repository
   path, quoted doc heading and other skill must exist. Game skills may not name engine
   repository paths. Sizes are capped: description 400 characters, body 4,000 bytes, all
   descriptions together 6,000 bytes. A second test proves the checker catches each kind of
   stale name.
5. `docs/ai/` stays the reference and the record. Skills are the procedures and point into it;
   exact fields stay in the generated reference and are not copied.

## Why

- Token use: the permanent cost is the descriptions (about 5 KB for 16 skills). A body
  (1.5 to 3 KB) replaces reading a 6 to 13 KB doc for the common path of a task.
- Games in their own repositories get the same guidance as the engine repository, updated by
  one command after an engine update.
- A procedure that names a removed operation is worse than none, because the agent follows it
  without checking. The test turns that into a build failure in the change that caused it.

## Rejected

- Skills only as docs with an index: agents without the index in context never find them, and
  nothing loads by itself.
- Fetching skills from the network at install time: a game would get skills that do not match
  its engine version.
- Moving all of `docs/ai/HOWTO.md` into skills now: other work edits it at the same time, and
  it is already read by section. Engine skills point to its headings (checked by the test).
- Extra front matter fields of one agent (tool allow lists, path filters): not portable.

## Consequences

- A change that adds a feature an agent would use adds or updates a skill
  (`engine-add-skill`); `engine-finish-change` says so.
- Three more tools in the MCP `core` listing.
- `project_init` writes two more folders (22 small files).

## Not done yet

- Only the two agents above were checked against their documentation. That other agents read
  `.agents/skills` is expected from the open format but unverified.
- No measurement yet of how often an agent picks the right skill from the descriptions.
- A game's own skills for its own operations are written by hand; nothing generates them.
