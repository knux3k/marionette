# 0274: Status says how far a feature goes: slice, usable, proven

Date: 2026-11-14. Status: accepted.

## Context

`STATE.md` marked every plan line "Done". The word covered three different things: a part
that works on the paths its own tests take, a part a game can be built on, and a part a real
game or real hardware has run. An audit found the difference mattered most where it was
least visible: a reader planning a game could not tell headsets (a simulated device only)
from the simulation core (a real game's logic ported and matched).

## Decision

- Three levels, each with a meaning and named evidence (`docs/ai/STATUS.md`):
  slice (works on its tests' paths), usable (a game can be built on it; gaps listed), proven
  (a real game, real content or real hardware has run it).
- `STATUS.md` has one row per feature: level, evidence, and what the next level needs. It is
  read when choosing what to build on or what to work on, not in every session.
- `STATE.md` keeps the plan lines but says "Landed" and points to `STATUS.md`. The words it
  lost were lists of decisions, which the rows keep as numbers.
- Closing a change includes moving the feature's row when the change adds a feature or new
  evidence (skill `engine-finish-change`).
- A test (`status_rows_have_a_level_and_evidence`) keeps every row to one of the three
  levels with evidence filled in, and "proven" rows from asking for more.

## Why

- A level without evidence is the same claim as "Done". Naming the evidence in the row makes
  the level checkable and makes the next step concrete.
- The table is not always-read: the always-read set stays inside its budget (decision 0061),
  and `STATE.md` got smaller.

## Not done yet

The first levels are one audit's reading of the tests, proofs and the UNVERIFIED list on
2026-11-14. The thread that owns a feature should correct its row.
