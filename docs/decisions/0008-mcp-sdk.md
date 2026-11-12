# 0008: MCP server on the official Rust SDK (`rmcp`)

Date: 2026-11-11. Status: accepted.

## Context

`marionette mcp` must speak the Model Context Protocol over stdio. The protocol is moving: the
SDK source lists revisions 2024-11-05, 2025-03-26, 2025-06-18, 2025-11-25 and 2026-07-28, and the
newest one (SEP-2567) replaces the `initialize` handshake with per-request `_meta`.

## Decision

Use `rmcp` 3.9 (the official SDK, modelcontextprotocol/rust-sdk) with only the `server` and
`transport-io` features. `McpServer` implements `ServerHandler::list_tools` and `call_tool` by
hand (no macros), because the tools come from the agent's operation table at run time.

## Why

- Version negotiation across five revisions, including one without a handshake, is exactly the
  part that is easy to get subtly wrong by hand. The SDK's default
  `supported_protocol_versions` covers all known revisions.
- The parts we use were read from the 3.9.0 source: `ServerHandler`, `Tool::new` with raw input
  and output schemas, `ToolAnnotations` (read-only and destructive hints), `CallToolResult::
  structured` / `structured_error`, `serve_server` with `transport::stdio()`.
- Cost: tokio (current-thread runtime only) and the SDK's own dependencies. Operations are
  blocking (GPU readback, file I/O), so each call runs in `spawn_blocking`; one `Agent` behind a
  mutex keeps calls sequential, which is what live sessions need anyway.

## Rejected

A hand-written JSON-RPC subset (initialize, tools/list, tools/call, ping) would be about 200
lines with no new dependencies, but would have to track protocol changes ourselves.

## Consequences

- Tool errors are tool results with `isError: true` and `structuredContent` =
  `{"ok": false, "diagnostics": [...]}`; JSON-RPC errors are reserved for protocol problems.
- Verified by the stdio test client with the 2025-11-25 handshake. The handshake-free
  2026-07-28 path is handled by the SDK and not tested here.
