# 0009: Live sessions and scene files are separate targets

Date: 2026-11-11. Status: accepted.

## Decision

- A live session (`session_start`) is a copy of a scene file loaded into memory, with its own
  tick counter and RNG. It lives as long as the process that holds it (the MCP server).
- Every edit names exactly one target: `scene` (a file) or `session` (a live world).
  - File edits are validated, written atomically in canonical form, and never change running
    sessions. Start a new session (or call `session_start` again) to see them.
  - Session edits are validated and applied to the live world only. Tick and RNG state are kept
    (`Sim::reload`), so the session keeps running deterministically. The file is not touched.
  - `session_save` is the only way a live world reaches disk: it writes the current world (at
    its current tick) as a canonical scene, by default over the file it was loaded from.
- Observation takes a `session`, or a `scene` plus `seed` and `ticks` (loaded, simulated,
  discarded). The CLI is one process per call, so it uses the second form.

## Why

- No hidden coupling: an agent always knows which copy it is changing, and a file on disk only
  changes when an operation that names it succeeds.
- Hot-reloading a running world from file changes would make results depend on when the
  reload happened, which breaks reproducibility.
- Keeping tick and RNG across a session edit means "step 100, edit, step 100" gives the same
  result every time it is replayed.

## Consequences

- Session state is lost when the server stops unless saved.
- Saving mid-run writes moved positions into the file; save to a new file to keep the original
  scene as authored.
