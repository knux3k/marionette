# Working rules for agents (Marionette)

This file and `CLAUDE.md` hold the same rules.

**Open `docs/ai/START-HERE.md` first.** It names the small always-read set (rules, the four
focuses, current state) and what to read per task. `docs/DESIGN.md` defines what the
engine is.

## Always

- Build and check before every commit:
  `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- The drift hash must stay
  `7441258fdf64141911d44e0608c48498897564168684fdd86cd244e149926558`
  (`cargo run -q -p marionette-cli -- run examples/drift.scene.json --headless --ticks 1000 --seed 1`).
- Change scenes through operations (`marionette op <name> '<json>'`, `marionette ops`) or the
  MCP server (`marionette mcp`); both run the same code.
- Follow the determinism rules in `docs/ai/ARCHITECTURE.md` and the memory and crash rules in
  `docs/ai/MEMORY-AND-CRASHES.md` (children via `marionette_host::spawn`, temp files via
  `marionette_host::temp_dir`, every log or cache bounded).
- Record every real choice in `docs/decisions/` and index it in `docs/ai/DECISIONS.md`.
- Regenerate `docs/ai/reference/` with `cargo run -q -p marionette-cli -- docs` when operations,
  components or CLI flags change.
- No names, identifying details or machine details in any file or commit message
  (`docs/ai/RULES.md`, decision 0016).
- Ask the owner before pushing, publishing, spending, deleting outside the repository, or
  opening windows or sound on a user's machine.
- A task with a procedure has a skill in `.claude/skills/` (index below); a feature an agent
  would use gets one (`engine-add-skill`).
- Weigh every change against the four focuses in `docs/ai/FOCUSES.md`: modularity (including
  custom code), limited context and token use, performance, graphics.
- End every session by updating `docs/ai/STATE.md` and adding a row to `docs/ai/SESSION-LOG.md`.

## Skills

A skill is the tested procedure for one kind of task. Before a task, open the one that
matches: `.claude/skills/<name>/SKILL.md` (agents that read skills load them by themselves;
`marionette op skills_get '{"name": "<name>"}'` prints one). New skill: `engine-add-skill`.

| Skill | Task |
| --- | --- |
| `engine-add-operation` | add an operation, component, resource or system to the engine |
| `engine-add-scene-or-mode` | add an example or proof scene with its stability mode and reference image |
| `engine-add-skill` | add or update a skill |
| `engine-finish-change` | checks, records and hygiene before every commit |
| `marionette-add-rust-code` | a game's own Rust module and operations |
| `marionette-animate-model` | play and blend animation clips |
| `marionette-build-scene` | create or change a scene through operations |
| `marionette-find-assets` | find, import or pool an asset in the required order |
| `marionette-find-slowdown` | measure and fix slow ticks, frames, hitches |
| `marionette-generate-assets` | generate images, textures and models from recipes |
| `marionette-handoff` | start, claim and hand off work in a game project |
| `marionette-light-scene` | lighting and post-processing |
| `marionette-make-vr` | headsets through OpenXR, grab and throw, VR movement |
| `marionette-multiplayer` | co-op lockstep, lobbies, Steam, achievements, cloud saves |
| `marionette-package-game` | package a game for Windows, Linux, Steam Deck, Steam Frame |
| `marionette-port-unity-unreal` | port Unity or Unreal code |
| `marionette-synth-audio` | synth voices, streamed music, generated samples, audio checked as numbers |
| `marionette-test-game` | scenarios, replays, stability soaks |
| `marionette-tune-render-cost` | level of detail, render scale, foveation, anti-aliasing |
| `marionette-world-ui` | panels, buttons and text in the 3D world, pointer and touch input |
| `marionette-write-job` | hot loops in job TypeScript |
| `marionette-write-script` | gameplay scripts in TypeScript |
