# Marionette

An open source game engine built to be driven by coding AIs. Everything is text, it runs
with no window, it describes itself as JSON Schema, and it reports errors as data.

Status: early. Plan lines 1 (workspace) and 2 (core) of [`docs/DESIGN.md`](docs/DESIGN.md)
are in place: an ECS world, a schema registry, a canonical JSON scene format with a validator,
a deterministic fixed-step loop, and the `marionette` CLI. No renderer yet.

## Quick start

Requires stable Rust (MSVC toolchain on Windows).

```bash
cargo build
cargo run -p marionette-cli -- run examples/drift.scene.json --headless --ticks 1000 --seed 1 --json
```

Commands (all accept `--json`):

| Command | What it does |
| --- | --- |
| `marionette schema` | JSON Schema of the scene format and every component and resource |
| `marionette validate <scene>` | Checks a scene; errors have a code, line, entity and suggested fix |
| `marionette run <scene> --headless --ticks N --seed S` | Simulates and prints a SHA-256 state hash |
| `marionette scene fmt <scene> [--check]` | Rewrites a scene in canonical form |

## For agents

Read [`AGENTS.md`](AGENTS.md) (same as [`CLAUDE.md`](CLAUDE.md)) for build steps, design
principles and determinism rules. Decisions are recorded in [`docs/decisions/`](docs/decisions/).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise, any
contribution intentionally submitted for inclusion in this work, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or conditions.
