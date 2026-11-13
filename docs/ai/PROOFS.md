# Proofs

Commands and tests that prove each done part of the engine, one file per area in
`docs/ai/proofs/`. Read only the file for the part you are changing.

| Area | File |
| --- | --- |
| Core and workspace (plan lines 1 and 2) | [proofs/core.md](proofs/core.md) |
| Renderer and looks (plan line 3) | [proofs/render.md](proofs/render.md) |
| Scripting and job TypeScript (plan line 4) | [proofs/scripting.md](proofs/scripting.md) |
| Agent interface, game Rust modules, skills and handoffs (plan line 5) | [proofs/agent.md](proofs/agent.md) |
| Input, physics, audio, UI and animation (plan line 6) | [proofs/play.md](proofs/play.md) |
| World-space UI and text (decisions 0200, 0201) | [proofs/ui3d.md](proofs/ui3d.md) |
| Providers, import, pool and generation (plan lines 7 and 8) | [proofs/assets.md](proofs/assets.md) |
| Proof scenes (plan line 9) | [proofs/games.md](proofs/games.md) |
| Performance, porting and saving | [proofs/performance.md](proofs/performance.md) |
| Co-op networking, Steam, achievements, cloud saves | [proofs/networking.md](proofs/networking.md) |

## Adding a proof

- Add it to the file of its area, under a heading that names the feature and its decision.
  Say what was run, on what class of machine, and what was not checked.
- A new area gets a new file `docs/ai/proofs/<area>.md` and a row here. Do not create other
  overflow files in `docs/ai/`.
- Each file stays under the size limit in `crates/marionette-cli/tests/doc_budget.rs`; when
  one fills up, split it by feature.
- Measurement tables that a doc explains (budgets, frame cost) stay with that doc; the proof
  entry links to it.
