# 0161: Handoff files and work claims in every game project

Date: 2026-11-13. Status: accepted. Focus 2 (token use), focus 1 (modularity).

## Context

The engine repository hands work between sessions through `docs/ai/` (START-HERE, STATE,
SESSION-LOG, DECISIONS). A game project got only `PROTOCOL.md` and `AGENTS.md`: nothing told
the next agent what exists, what is unverified or what to do next. Games built by many agents
at once had no way to say who is changing what. The engine's own practice shows the weak
points: every parallel branch appends to the same session log and decision index and they
conflict on merge, and decision numbers collide unless blocks are handed out by hand.

## Decision

1. `project_init` writes, and `handoff_init` adds to an existing project without
   overwriting: `docs/ai/START-HERE.md`, `docs/ai/STATE.md`, `docs/ai/claims/`,
   `docs/ai/sessions/`, `docs/decisions/`.
2. One file per unit, so parallel branches do not touch the same lines:
   - a claim is `docs/ai/claims/<topic>.claim.json` (topic, date, summary, areas, branch,
     reserved decision numbers);
   - a session note is `docs/ai/sessions/<date>-<topic>.md` (done, not done, unverified, next);
   - a decision is `docs/decisions/NNNN-title.md`; there is no index file.
   `STATE.md` is the one shared file: agents change only the rows of their area.
3. Operations (group `handoff`, in the MCP `core` preset):
   - `handoff_status`: what to read first, open claims, the newest five notes, the next free
     decision number, problems (missing files, claims that overlap in area or decision
     numbers, duplicate decision numbers, always-read files over 16 KB together);
   - `handoff_claim`: refused with `handoff.overlap` when an area is another claim's area or
     inside it; reserves a block of decision numbers after every recorded and reserved one;
   - `handoff_note`: writes the note; `release: true` removes the claim.
   Dates are inputs: operations do not read the clock.
4. The skill `marionette-handoff` carries the procedure; the project `AGENTS.md` template and
   the MCP server instructions tell every agent to start with `handoff_status`.

## Why

- A handoff that depends on an agent remembering a convention is skipped. Operations make the
  right thing one call, and `handoff_status` makes a broken handoff visible at the start of
  the next session.
- Claims as data let the engine refuse a collision instead of documenting how to avoid one.
- Reserved number blocks are what the engine's own parallel work converged on by hand.

## Rejected

- One `CLAIMS.md` and one `SESSION-LOG.md` table, as in the engine repository: the files that
  record parallel work would be the files parallel work conflicts on.
- Locks outside the repository (a server, a lock file in a temp folder): agents on different
  machines share only the repository.
- Reading the clock for dates and stale-claim warnings: operations stay reproducible; the
  date in a claim lets a reader judge its age.

## Not done yet

- A claim protects only once other agents can see it (committed and on the shared branch).
  Two branches that claim at the same moment both succeed locally; `handoff_status` reports
  the overlap after the merge, and the later claim renumbers. Nothing checks the remote.
- The engine repository itself still uses its table files; moving it to this layout is
  separate work, because every open branch edits those files.
- No game has been run through this by several real agents yet.
