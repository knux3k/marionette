# 0060: Four standing focuses for every change

Date: 2026-11-12. Status: accepted.

## Context

The project owner named four focuses for the engine: modularity (including custom code),
limiting context memory and token use, performance, and graphics. They were not written
anywhere an agent reads at the start of a session.

## Decision

- `docs/ai/FOCUSES.md` states the four focuses, what each means in this codebase, a check per
  focus, and a table of open gaps from an audit of the code. It is in the always-read set named
  by START-HERE.md, and RULES.md rule 15, CLAUDE.md and AGENTS.md point at it.
- Sessions mark gaps closed in that table; a new gap found while working is added there.
- A decision record that trades one focus against another says so.

## Alternatives

- Only a line in RULES.md: too short to say what each focus means or what is still missing.
- Put the gaps in WORKSTREAMS.md: that file is read per workstream, so a cross-cutting list
  would be missed.

## Consequences

The gap table is the place to look for the next engine-wide work. It must stay short; finished
rows can be removed once SESSION-LOG.md records them.
