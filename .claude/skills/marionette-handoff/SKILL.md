---
name: marionette-handoff
description: Start, claim and hand off work in a Marionette game project so the next agent, or several at once, can continue without asking or colliding. Use when starting a session, before beginning or after finishing a piece of work, when recording a decision, or when two agents may touch the same files.
---

# Hand work on (and pick it up)

A game is worked on by many sessions and often by several agents at once. None of them
remembers the others. What they share is in the repository:

| File | Holds | Rule |
| --- | --- | --- |
| `docs/ai/START-HERE.md` | entry point, where things are | short; read every session |
| `docs/ai/STATE.md` | what is true now, UNVERIFIED, next work | change only your area's rows |
| `docs/ai/claims/` | one file per piece of work in progress | written and removed by operations |
| `docs/ai/sessions/` | one note per session | never edited after writing |
| `docs/decisions/` | one record per real choice | numbers from your claim's block |

One file per claim and per session, so parallel branches never edit the same lines. A project
without these files gets them from `handoff_init` (it never overwrites).

## Start of a session

1. Read the owner's protocol file, then `docs/ai/START-HERE.md` and `docs/ai/STATE.md`.
2. `marionette op handoff_status '{}' --json`. Fix every entry in `problems` first. Read the
   `latest_sessions` notes that touch your work, not the whole folder.
3. Continuing claimed work: read its claim and its last note, and keep the topic.

## Before starting a piece of work

`handoff_claim` with `topic` (short, lowercase, hyphens), `date` (today, year-month-day),
`summary`, `areas` (the files or folders you will change) and `decisions` (how many decision
numbers to reserve; the block starts at the next free one).

- Keep areas narrow. A claim on a whole folder blocks everyone else's work in it.
- A refusal with code `handoff.overlap` names the other claim. Narrow your areas, pick other
  work, or agree the split first. Never work around it by not claiming.
- A claim only protects once others can see it: commit the claim file by itself and get it to
  the shared branch before the work, as the owner's protocol allows. Pull first.
- Shared files nobody can avoid (the project manifest, the main scene): claim the smallest
  part, change it in one small commit, and merge that early.

## While working

- A choice between real alternatives gets a decision record: title line with the number,
  then Date, Status, Decision, Why, and what was rejected. Use numbers from your block only.
- Build scenes from an operations recipe file (`batch_run`) when several agents add to the
  same game: recipes merge, hand-merged scene files do not.
- Entity ids: take a range per claim and say so in the claim's `summary`, so two branches
  never create the same id.

## End of a session

1. `project_validate` and `test_run` pass, or the note says what fails.
2. `handoff_note` with `topic`, `date`, `done` (each with how it was checked), and as
   needed `unverified` (with what would check it) and `next`. What is unfinished goes in its
   not-done list. Add `"release": true` when the claimed work is finished or given up.
3. Update your rows in `docs/ai/STATE.md`: status, UNVERIFIED, next. State is what is true
   now; history stays in the notes.
4. Commit the note, the state and the work together.

## Writing for the next agent

- Say what was checked and how; say plainly what was not. An unverified claim written as
  fact costs the next agent more than a gap.
- Paths, operation names and commands, not descriptions of them.
- No names of people, machines or private details in any of these files.

## Checks

`handoff_status` shows no `problems`, no claim of yours that is finished, and your note
among `latest_sessions`.
