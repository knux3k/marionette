//! MCP server over stdio, built on the official Rust SDK (`rmcp`). Every operation in the
//! agent's table becomes one tool; input and output schemas come from the same types.
//! Operation errors are returned as tool results with `isError: true` and the diagnostics as
//! structured content, so the calling model can read and fix them.

use crate::Agent;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, InitializeResult, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

const INSTRUCTIONS: &str = "Marionette game engine. Before working, read the project's PROTOCOL.md \
(the owner's rules) and AGENTS.md. Loop: project_validate, edit with entity_*/component_* (validated \
before writing), run with scene_run or session_start/session_step, observe with observe_*, check with \
test_run. Errors come back as diagnostics with a code, a JSON pointer and a suggested fix. This server \
may list only some operation groups (`marionette mcp --tools`); `marionette ops --brief` lists them all.";

/// Operation groups in the `core` preset: the edit, run, observe and test loop.
pub const CORE_GROUPS: &[&str] = &[
    "project",
    "scene",
    "entity",
    "component",
    "resource",
    "session",
    "observe",
    "test",
    "batch",
];

/// Which tools the server lists (decision 0061). Every listed tool costs the client's context
/// on every request, so a client that only edits and runs scenes can ask for less.
#[derive(Debug, Clone, Default)]
pub struct McpOptions {
    /// Groups (`scene`, `observe`, ...: an operation's name up to its first `_`), operation
    /// names, `core` (see [`CORE_GROUPS`]) or `all`. Empty means all.
    pub tools: Vec<String>,
    /// Leave output schemas out of `tools/list`. Calls return the same results.
    pub lean: bool,
}

/// The group of an operation: its name up to the first `_` (`observe_tree` is in `observe`).
pub fn group_of(name: &str) -> &str {
    name.split('_').next().unwrap_or(name)
}

/// Resolves a `--tools` list to operation names, in table order. Unknown entries are an
/// error naming the groups that exist.
pub fn select_operations(agent: &Agent, spec: &[String]) -> Result<Vec<&'static str>, String> {
    let names: Vec<&'static str> = agent.operations().map(|o| o.name).collect();
    if spec.is_empty() || spec.iter().any(|s| s == "all") {
        return Ok(names);
    }
    let groups: BTreeSet<&str> = names.iter().map(|n| group_of(n)).collect();
    let mut wanted: BTreeSet<&str> = BTreeSet::new();
    for item in spec.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        if item == "core" {
            wanted.extend(CORE_GROUPS.iter().copied());
        } else if groups.contains(item) || names.contains(&item) {
            wanted.insert(item);
        } else {
            return Err(format!(
                "unknown tool group or operation `{item}`; use all, core, an operation name or a group: {}",
                groups.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    Ok(names
        .into_iter()
        .filter(|n| wanted.contains(n) || wanted.contains(group_of(n)))
        .collect())
}

/// The MCP handler: shares one [`Agent`] (and its live sessions) across calls.
#[derive(Clone)]
pub struct McpServer {
    agent: Arc<Mutex<Agent>>,
    tools: Arc<Vec<Tool>>,
    enabled: Arc<BTreeSet<&'static str>>,
}

fn object(v: Value) -> JsonObject {
    match v {
        Value::Object(m) => m,
        _ => JsonObject::new(),
    }
}

impl McpServer {
    /// Every operation as a tool, with output schemas.
    pub fn new(agent: Agent) -> Self {
        Self::with_options(agent, &McpOptions::default()).expect("the default options are valid")
    }

    /// Only the tools `options` select. Errors name an unknown group or operation.
    pub fn with_options(agent: Agent, options: &McpOptions) -> Result<Self, String> {
        let read_only = agent.read_only();
        let enabled: BTreeSet<&'static str> =
            select_operations(&agent, &options.tools)?.into_iter().collect();
        let tools = agent
            .operations()
            .filter(|o| !(read_only && o.mutating) && enabled.contains(o.name))
            .map(|o| {
                let tool = Tool::new(o.name, o.description, Arc::new(object(o.input_schema.clone())));
                let tool = if options.lean {
                    tool
                } else {
                    tool.with_raw_output_schema(Arc::new(object(o.output_schema.clone())))
                };
                tool.with_annotations(
                    ToolAnnotations::new()
                        .read_only(!o.mutating)
                        .destructive(o.mutating),
                )
            })
            .collect();
        Ok(Self {
            agent: Arc::new(Mutex::new(agent)),
            tools: Arc::new(tools),
            enabled: Arc::new(enabled),
        })
    }

    /// The listed tools (what `tools/list` returns).
    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    /// Runs one tool call; shared by the MCP handler and tests. A tool left out by
    /// `--tools` is refused like an unknown operation.
    pub fn call(&self, name: &str, arguments: Value) -> CallToolResult {
        if !self.enabled.contains(name) {
            let diagnostics = marionette_core::Errors::from(
                marionette_core::Diagnostic::error(
                    crate::codes::NOT_ENABLED,
                    format!("`{name}` is not one of this server's tools"),
                )
                .fix("restart the server with `marionette mcp --tools all` or name its group in --tools"),
            );
            return CallToolResult::structured_error(json!({ "ok": false, "diagnostics": diagnostics }));
        }
        let mut agent = self.agent.lock().unwrap_or_else(|p| p.into_inner());
        match agent.call(name, arguments) {
            Ok(v) => CallToolResult::structured(v),
            Err(e) => CallToolResult::structured_error(json!({ "ok": false, "diagnostics": e })),
        }
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("marionette", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.to_string();
        let arguments = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let server = self.clone();
        // Operations block (GPU readback, file I/O); keep them off the async reactor.
        let result = tokio::task::spawn_blocking(move || server.call(&name, arguments))
            .await
            .map_err(|e| ErrorData::internal_error(format!("operation panicked: {e}"), None))?;
        Ok(result.into())
    }
}

/// Serves MCP on stdin/stdout until the client disconnects.
pub fn serve_stdio(agent: Agent, options: &McpOptions) -> Result<(), String> {
    let server = McpServer::with_options(agent, options)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start async runtime: {e}"))?;
    runtime.block_on(async {
        let running = rmcp::serve_server(server, rmcp::transport::stdio())
            .await
            .map_err(|e| format!("MCP initialization failed: {e}"))?;
        running
            .waiting()
            .await
            .map_err(|e| format!("MCP server stopped with an error: {e}"))?;
        Ok(())
    })
}
