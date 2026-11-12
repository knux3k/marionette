# 0010: One operation table for CLI and MCP

Date: 2026-11-11. Status: accepted.

## Decision

- `marionette-agent` holds one table of operations. Each is a Rust function from a typed input
  struct to a typed output struct; JSON Schemas for both are generated from those structs with
  `schemars`. The CLI (`marionette op <name> '<json>'`, `marionette ops`) and the MCP server
  (`tools/list`, `tools/call`) both dispatch through `Agent::call`. The older named commands
  (`validate`, `run --headless`, `screenshot`, `scene fmt`, `schema`) are shortcuts that call the
  same operations.
- Names are `snake_case` with a group prefix (`entity_create`, `observe_query`), which every MCP
  client accepts as a tool name.
- Each operation is marked mutating or not. Read-only mode (`--read-only`) refuses mutating
  operations with `agent.read_only`, hides them from MCP `tools/list`, and only lets screenshots
  be written under `.marionette/`.
- Paths: relative paths resolve against the project root (`--project`) and may not contain `..`;
  absolute paths are used as given.
- Compact results: lists are paged (`offset`, `limit`, default 50, max 500, `next_offset`),
  screenshots return a file path and hash, whole scenes are never returned unless asked.
- Refused edits report the JSON pointer of the problem in the candidate document instead of a
  line number, because the text was never written.

## Why

A single table makes it impossible for the CLI and MCP to drift: there is no second
implementation to update. Generated schemas cannot disagree with the parser that reads the
input.

## Not done yet

Scripted input events (`session_input`) wait for the input system (plan line 6).
