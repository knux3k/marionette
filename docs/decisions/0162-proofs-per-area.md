# 0162: Proofs in one file per area

Date: 2026-11-13. Status: accepted. Focus 2 (token use).

## Decision

`docs/ai/PROOFS.md` is a short index. Proofs live in `docs/ai/proofs/<area>.md` (core,
render, scripting, agent, play, assets, games, performance). A new area is a new file and a
row in the index. Each file is under the same size limit as other docs
(`crates/marionette-cli/tests/doc_budget.rs` now checks the folder).

## Why

The single file reached the 32 KB limit while several branches were appending to its end, and
branches had begun to add overflow files with names of their own. Area files keep a read
small, give parallel branches different files to edit, and give overflow one known place.

## Rejected

- Raising the limit: the file is read by section, but every append still conflicts.
- One file per decision: too many small files to find a proof in.

## Consequences

A branch that appended to the old `PROOFS.md` moves its section into the area file when it
merges the default branch.
