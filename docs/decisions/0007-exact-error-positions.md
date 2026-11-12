# 0007: Exact line and column for scene diagnostics

Date: 2026-11-11. Status: accepted (replaces the best-guess locator from plan line 2).

## Decision

After `serde_json` accepts the scene text, `json_span::spans` scans it once more and records the
1-based line and column of every value, keyed by JSON Pointer (`/entities/2/components/Follow`).
Object members are located at their key. Diagnostics use the pointer of what they are about:
the component key, the misspelled field key, the duplicate `id`, the bad top-level key.

## Why

`serde_json::Value` drops positions, and the old locator searched for text, which failed on
minified files and could match the wrong entity. A small exact scanner (about 100 lines, with
tests) is cheaper and more legible than switching to a span-preserving JSON parser crate.
