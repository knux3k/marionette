# 0061: Small always-read docs, a size test, and selectable MCP tools

Date: 2026-11-12. Status: accepted. Focus 2 (decision 0060).

## Context

An agent starting a session followed START-HERE.md's read order: RULES, STATE, WORKSTREAMS,
ARCHITECTURE, HOWTO, ENVIRONMENT, about 75 KB (roughly 19,000 tokens) before any task. STATE.md
alone was 40 KB, most of it proof commands and a session log that a given task rarely needs.
An MCP client of `marionette mcp` received all 72 operations with input and output schemas in
`tools/list` and kept them in context for the whole conversation. `marionette ops` printed the
same schemas, and every JSON answer was indented.

## Decision

1. STATE.md keeps only status per plan line and UNVERIFIED. Proof commands and test names moved
   to `docs/ai/PROOFS.md` (read one section when needed); the session log moved to
   `docs/ai/SESSION-LOG.md`.
2. START-HERE.md names an always-read set (RULES, FOCUSES, STATE) and a table of what to read
   per task. Large generated references are searched, not read whole.
3. `crates/marionette-cli/tests/doc_budget.rs` enforces the sizes: always-read set (with
   CLAUDE.md and START-HERE.md) at most 24 KB, STATE.md at most 12 KB, any hand-written doc in
   `docs/ai/` at most 32 KB. Raising a limit needs a decision record.
4. `marionette mcp --tools <groups>` lists only some operations. A group is an operation name
   up to its first `_`; `core` is project, scene, entity, component, resource, session,
   observe, test and batch; `all` is the default so existing setups keep working. Calls to a
   tool that is not listed fail with `agent.not_enabled`. `--lean` leaves output schemas out
   of the listing; results are unchanged. README recommends `--tools core --lean`.
5. `marionette ops --brief` gives name, mutating flag and summary only; `--tools` filters it
   the same way.
6. CLI JSON is indented only when stdout is a terminal; piped output (agents, scripts) is one
   compact line.

Measured when this landed (tests `mcp_tool_groups_and_lean_listing_shrink_the_context` and
`ops_brief_is_small_and_filters_by_group` print them):

| What | Before | After |
| --- | --- | --- |
| Always-read docs | about 75 KB (START-HERE order) | 18.7 KB (CLAUDE.md, START-HERE, RULES, FOCUSES, STATE) |
| MCP `tools/list` | 158 KB, all 72 tools | 27.6 KB with `--tools core --lean` (all tools: unchanged) |
| `marionette ops` | 155 KB | 14.6 KB with `--brief`, 4.6 KB with `--brief --tools core` |

## Alternatives

- A meta-tool server (one `call_operation` tool plus `describe_operation`): smallest listing,
  but clients lose typed tool schemas and per-tool read-only annotations. Groups keep both.
- Default to `core`: smaller for new users, but silently drops tools from existing setups.
  The README recommends it instead.
- Shorten docs by rewriting them tersely: helps less than not loading them, and loses detail.

## Consequences

Threads that used to append proofs or session rows to STATE.md now append to PROOFS.md and
SESSION-LOG.md. A doc that outgrows its limit must be split by task.
