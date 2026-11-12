# 0002: Scene files are canonical JSON

Date: 2026-11-11. Status: accepted.

## Context

DESIGN.md asks for RON or JSON with a published schema, deterministic serialization (sorted
keys, fixed float formatting) and byte-identical load/save.

## Decision

JSON (`*.scene.json`). Canonical form: object keys sorted, entities sorted by `id`, two-space
indent (`serde_json` pretty printer), floats in shortest round-trip form, `\n` line endings and
a trailing newline. `marionette scene fmt` writes it; `--check` verifies it.

## Why

- `marionette schema` emits JSON Schema, and JSON Schema validates JSON directly. Any agent or
  editor can check a scene with off-the-shelf tools; RON has no schema ecosystem.
- Every coding agent and every language reads and writes JSON reliably; RON is Rust-specific.
- `serde_json` parse errors carry line and column, which feed our diagnostics.
- Downsides accepted: no comments and more quoting than RON. Names (`"name"`) and future
  per-entity notes can carry intent instead of comments.

## Details that make round-trips exact

- Component values are serialized with `serde_json` straight from the reflected value (which
  prints `f32` in shortest form, e.g. `0.1`) and read back into a `serde_json::Value`, so the
  `f32 -> f64` widening inside `Value` never shows up in the text.
- Loading goes through the typed path, so `fmt` normalizes numbers (`1` -> `1.0` for an `f32`).
- `.gitattributes` forces LF so a Windows checkout keeps files byte-identical.
