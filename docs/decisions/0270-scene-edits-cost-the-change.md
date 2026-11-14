# 0270: Scene edits cost the size of the change, not the size of the scene

Date: 2026-11-14. Status: accepted.

## Context

Every edit operation parsed, validated and rewrote the whole scene twice and compared the two
results, and a live session took two snapshots and built its world again. Building a scene by
edits was therefore quadratic. Measured in a release build on a 4-core Linux container before
this change: `batch_run` with `entity_create` 100 times 1.0 s, 500 times 21.5 s, 1,000 times
89 s; 3,000 single creates in a live session over MCP 423 s. No test used a scene above about
115 entities, so nothing showed it.

## Decision

- `marionette_core::scene_index::SceneIndex` holds a scene that is already canonical, with each
  entity's text and outgoing references beside it. Create, replace, delete and the resource
  edits check and rewrite only what they touch. An edit that is not plainly valid returns
  `None` and changes nothing.
- The seven edit operations (`entity_create`, `entity_update`, `entity_delete`,
  `component_set`, `component_remove`, `resource_set`, `resource_remove`) try the index first
  and fall back to the old whole-scene path. So every refusal and every diagnostic comes from
  the old code, unchanged, and a successful edit gives the same bytes and the same answer.
- Scene files: the agent keeps up to 4 scenes indexed. An entry is used only while the file
  still holds exactly the bytes this process last wrote; anything else drops it. Outside a
  batch the text is written after each edit (assembled from the kept pieces, no parsing).
  Inside `batch_run` it is written once: when the batch ends, or before the next step that is
  not one of the seven edits.
- Live sessions: the session keeps its scene indexed until anything but an edit reaches it.
  Outside a batch each edit still builds the world again, at once, so a module can refuse it
  (a script that does not compile, for example), but without the snapshots, the text and the
  comparison (`Sim::reload_scene`). Inside `batch_run`, edits of one session in a row build the
  world once. If that load fails, nothing was applied yet and the batch runs those steps again
  one at a time, so it stops at the step at fault with that step's own message, as before.
- Positions in JSON text (`json_span`) are found on first use. They are only read when a
  problem is reported, so loading a valid scene no longer scans its text a second and third
  time.
- Timing budgets with 10,000 entities in `engine_budgets` (`scene_edit_budgets`).

## Why

An agent builds and changes scenes through these operations, and a real game has thousands of
entities. The cost has to follow the edit. Keeping the old path as the only source of refusals
means the fast path cannot give a different answer: it either does exactly the plain case or
steps aside.

## Consequences

- Same machine, after: `batch_run` with 1,000 creates 0.55 s (was 89 s); 10,000 creates into a
  file 0.5 to 0.9 s; one edit of a 10,000 entity file 4.5 ms; 10,000 creates into a live session
  in one batch 0.6 to 1.0 s; 3,000 single creates of a file over MCP 5.6 s.
- Loading a 10,000 entity scene went from 0.79 s to 0.44 s (the position scan).
- `Agent::call` writes pending scene text before any operation that is not an edit or a batch,
  so nothing reads a stale file. A second process editing the same file between two edits is
  seen (the bytes differ) and the file is read again.

## Rejected

- Editing the live world in place (spawn or change one entity without building the world
  again). Modules build their state in `init` from the whole world and can refuse a scene
  there; an in-place path would need every module to check and take single changes. That is
  the right end state but a change to the module interface, not to the edit operations.
- Deferring the world load of single session edits until the session is next used. A module's
  refusal would then arrive after the edit had been reported as done.
- Trusting file time and size instead of comparing bytes. Reading the file back is a few
  milliseconds at 10,000 entities and cannot be fooled.

## Not done yet

- A single edit of a live session still costs the size of the world: 130 ms at 10,000 plain
  entities, and 3,000 single creates of scripted entities over MCP took 110 s (was 423 s).
  Use `batch_run` for many live edits until modules can take single changes.
- `prefab_*`, the animation and audio edit operations still use the whole-scene path.
- All numbers are from one cloud container; none are from a desktop.
