# 0105: No `unwrap`, `expect` or `panic!` outside tests

Date: 2026-11-14. Status: accepted. Follows decision 0104 (the owner's code rules, rule 2).
Focus 1 (modularity: a game's bad data must never take the engine down).

## Context

After 0104, non-test code still held 170 `unwrap`/`expect`/`panic!` sites in 22 crates (a
clippy survey with `unwrap_used`, `expect_used` and `panic` on). Most guarded something the
code had just checked; some were reachable: a script calling an engine function from module
top-level code panicked (caught by the script guard, so it surfaced only as a load error with
a panic message), and a registry with two types under one name panicked while building.

## Decision

1. Every crate root (`src/lib.rs`, `src/main.rs`) carries
   `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`; `clippy.toml` allows
   them in `#[test]` code. The `repo_hygiene` test fails on a crate root without the line.
   A crate-root attribute, not `[workspace.lints]`, so integration tests, benches and
   examples (test code) keep their asserts and unwraps.
2. Behaviour that changed, all from a panic to a named error:
   - Scripts: engine calls outside a hook throw `script.no_world` (a load error at module top
     level, as before, now with that code). Inserting a component that does not fit throws
     `script.bad_value`.
   - Core: a duplicate registry name is a `registry.duplicate` error from `Sim::with_modules`;
     a component that cannot be inserted while loading a scene is `scene.insert`.
   - Agent: an operation output that does not serialize is `agent.internal`.
   - Providers: a missing key or wrong request kind reaching a provider is
     `provider.key_missing` / `provider.kind_unsupported`.
   - Jobs: a failed WebAssembly engine or watchdog start is a `jobts.internal` error on every
     job instead of a crash.
   Everything else that "cannot happen" now skips or falls back (`let .. else { continue }`,
   `get_or_insert_with`, `Option::next_if`, `from_utf8_lossy`).
3. Serializing a `serde_json::Value` or a struct of plain fields cannot fail; those use
   `.unwrap_or_default()`. Other serialization returns an error.
4. A deliberate panic (the stability harness's injected one) takes `#[allow(clippy::panic)]`
   with a comment.

Along the way: the scene diff and `observe_query` borrow entities instead of cloning them,
and `session::get_mut` no longer copies every session name on each call.

## Proof

Drift hash unchanged; every test passes (release, lavapipe GPU); clippy clean on the newest
stable Rust. New tests: `engine_calls_at_load_name_the_missing_world` (script),
`built_in_names_are_unique` (agent), `every_crate_root_denies_panicking_shortcuts`.

## Rejected

- `[workspace.lints.clippy]`: applies to test targets too, which would need an `allow` in
  every integration test file.
- Keeping `expect` with a reason string: still a crash in a shipped game for a state the
  engine can report.
