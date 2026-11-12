# START HERE

Entry point for any agent working on Marionette, written for an AI reader. Every claim below can
be checked against a repo-relative path, a command or a test name.

Load as little as the task needs (focus 2, decision 0061). The always-read set is small and its
size is checked by `crates/marionette-cli/tests/doc_budget.rs`; everything else is read only
when the task calls for it, and then only the section that matters.

## Always read (about 16 KB)

1. [RULES.md](RULES.md): what you may decide alone and what needs the owner.
2. [FOCUSES.md](FOCUSES.md): the four standing focuses and the open gaps.
3. [STATE.md](STATE.md): status per plan line and the UNVERIFIED list.

## Read when the task needs it

| Task | Read |
| --- | --- |
| Pick or continue a workstream | your section of [WORKSTREAMS.md](WORKSTREAMS.md) |
| Change engine code | the crate map and the relevant section of [ARCHITECTURE.md](ARCHITECTURE.md) |
| Add a component, system, operation, crate, scene, test | the recipe in [HOWTO.md](HOWTO.md) |
| Build, test, toolchain, durations, no GPU | [ENVIRONMENT.md](ENVIRONMENT.md) |
| Rerun or add a proof (hashes, reference images, test names) | the section in [PROOFS.md](PROOFS.md) |
| Find why something was chosen | [DECISIONS.md](DECISIONS.md), then one record in `docs/decisions/` |
| When something changed | [SESSION-LOG.md](SESSION-LOG.md), or `git log` |
| Assets and licenses (normative) | [ASSETS-AND-LICENSES.md](ASSETS-AND-LICENSES.md) |
| Child processes, temp files, logs, leaks (normative) | [MEMORY-AND-CRASHES.md](MEMORY-AND-CRASHES.md) |
| Budgets and measuring | [PERFORMANCE.md](PERFORMANCE.md) |
| Hot loops in TypeScript | [JOBS-TS.md](JOBS-TS.md) |
| Unity or Unreal code | [PORTING.md](PORTING.md) |
| Stability modes | [STABILITY.md](STABILITY.md) |
| Generating images and models | [GENERATION.md](GENERATION.md) |
| The six proof games and their gaps | [PROOF-SCENES.md](PROOF-SCENES.md) |
| Terms | [GLOSSARY.md](GLOSSARY.md) |

Generated reference in `docs/ai/reference/` (large; search it, do not read it whole):
[operations](reference/operations.md), [components](reference/components.md),
[CLI](reference/cli.md), [porting map](reference/porting-map.md), [jobs.d.ts](reference/jobs.d.ts),
[marionette.d.ts](reference/marionette.d.ts). To use one operation, pick it from the summary
table at the top of operations.md (or `marionette ops`), then search for `` ## `<name>` ``.

`docs/DESIGN.md` is the product definition (brief, plan lines 1 to 10, principles).

## First five minutes

```bash
cargo build
cargo test
cargo run -q -p marionette-cli -- run examples/drift.scene.json --headless --ticks 1000 --seed 1
```

The last command must print
`7441258fdf64141911d44e0608c48498897564168684fdd86cd244e149926558`. If it does not, stop and
find out why before changing anything (see the determinism section of ARCHITECTURE.md).

## Every session ends by

1. `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
2. `cargo run -q -p marionette-cli -- docs` if operations, components or CLI flags changed
   (the test `generated_reference_docs_are_current` fails otherwise).
3. Updating [STATE.md](STATE.md) (status per plan line, UNVERIFIED), adding one row to
   [SESSION-LOG.md](SESSION-LOG.md), and new proofs to [PROOFS.md](PROOFS.md).
4. A decision record in `docs/decisions/` for every choice between real alternatives.
5. Marking gaps you closed in [FOCUSES.md](FOCUSES.md).
6. Small commits; then `cargo clean` if disk space matters.
