# Project protocol

This file belongs to the person who owns this project. Every coding agent (Claude, or any
other) must read it before doing any work here and follow it. If something here conflicts with
an agent's own defaults, this file wins. If something is unclear, ask the owner instead of
guessing.

Edit the sections below to suit your project. `marionette project init` copies this template
into every new project.

## 1. The game

- What the game is (one or two sentences):
- Platforms: Windows and Linux, flat screen. VR: no.
- Look and feel references:

## 2. Ask before building

Before starting a new feature, the agent writes a short plan (what, which files, how it will be
checked) and waits for the owner's yes when the feature:

- changes how the game plays or looks in a way the owner will notice,
- adds a dependency, a new crate, a new asset source or a new provider,
- takes more than about an hour of work.

Small fixes, tests and refactors that do not change behavior can go ahead without asking.

## 3. Needs explicit approval every time

- Spending money: paid API calls (image, 3D, language models), purchases, subscriptions.
- Publishing: pushing to a remote, releases, uploads, posting anywhere public.
- Deleting: files the agent did not create in this session, saves, assets, branches.
- Changing this file.

## 4. Assets

1. Search the project's own CC0 pool first.
2. Then free CC0 libraries (Poly Haven, ambientCG, Kenney, Quaternius).
3. Generate only after 1 and 2 fail, and only within the spend cap below.

Only CC0 or self-made assets go in the project, each with its source recorded next to it.

- Spend cap per day:
- Providers allowed:

## 5. Who does what

- Design and code review:
- Writing code:
- Art direction:
- Hand work to other agents like this (who, what kind of task, how to report back):

## 6. How to work in this project

1. Read this file, then `AGENTS.md` and `docs/ai/START-HERE.md`; run `handoff_status`. Claim
   work with `handoff_claim` when others may be working too; end with `handoff_note`.
2. `marionette op project_validate` before and after changes.
3. Change scenes only through edit operations (`entity_*`, `component_*`, `scene_*`), which
   validate before writing and keep files canonical.
4. Run headless (`session_start`, `session_step`), observe (`observe_query`, `observe_entity`,
   `observe_screenshot`), and record what you checked.
5. Add or update a scenario in `tests/` for every behavior you build; `test_run` must pass.
6. Report: what changed, how it was checked, what is unverified.

## 7. Reporting

- Where to report (chat, file, issue):
- How long reports should be:
- Time zone for times in reports:
