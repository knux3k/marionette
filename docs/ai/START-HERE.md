# START HERE

Entry point for any agent working on Marionette, written for an AI reader. Every claim below can
be checked against a repo-relative path, a command or a test name. Read in this order:

1. [RULES.md](RULES.md): what you may decide alone and what needs the owner. Read fully.
2. [STATE.md](STATE.md): what is done (with proof commands), in progress, next, and UNVERIFIED.
3. [WORKSTREAMS.md](WORKSTREAMS.md): pick your workstream; it names the files you own.
4. [ARCHITECTURE.md](ARCHITECTURE.md): crates, the tick, determinism, the state hash.
5. [HOWTO.md](HOWTO.md): step-by-step recipes (component, system, operation, crate, scene, test).
6. [ENVIRONMENT.md](ENVIRONMENT.md): toolchain, exact commands, durations, GPU-less setup.
7. As needed: [DECISIONS.md](DECISIONS.md) (index of `docs/decisions/`),
   [ASSETS-AND-LICENSES.md](ASSETS-AND-LICENSES.md) (normative asset rules),
   [PERFORMANCE.md](PERFORMANCE.md) (budgets and how to measure),
   [GLOSSARY.md](GLOSSARY.md), [PORTING.md](PORTING.md) (bringing Unity or Unreal code over),
   [GENERATION.md](GENERATION.md) (generating images, textures and models),
   and the generated reference in `docs/ai/reference/`
   ([operations](reference/operations.md), [components](reference/components.md),
   [CLI](reference/cli.md), [porting map](reference/porting-map.md)).

`docs/DESIGN.md` is the product definition (brief, plan lines 1 to 10, principles). These docs
describe how far the code has come and how to continue.

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
3. Updating [STATE.md](STATE.md): status per plan line, UNVERIFIED list, and one session-log row.
4. A decision record in `docs/decisions/` for every choice between real alternatives.
5. Small commits; then `cargo clean` if disk space matters.
