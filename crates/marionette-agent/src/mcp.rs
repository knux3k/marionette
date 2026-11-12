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
use std::sync::{Arc, Mutex};

const INSTRUCTIONS: &str = "Marionette game engine. Before working, read the project's PROTOCOL.md \
(the owner's rules) and AGENTS.md. Loop: project_validate, edit with entity_*/component_* (validated \
before writing), run with scene_run or session_start/session_step, observe with observe_*, check with \
test_run. Errors come back as diagnostics with a code, a JSON pointer and a suggested fix.";

/// The MCP handler: shares one [`Agent`] (and its live sessions) across calls.
#[derive(Clone)]
pub struct McpServer {
    agent: Arc<Mutex<Agent>>,
    tools: Arc<Vec<Tool>>,
}

fn object(v: Value) -> JsonObject {
    match v {
        Value::Object(m) => m,
        _ => JsonObject::new(),
    }
}

impl McpServer {
    pub fn new(agent: Agent) -> Self {
        let read_only = agent.read_only();
        let tools = agent
            .operations()
            .filter(|o| !(read_only && o.mutating))
            .map(|o| {
                Tool::new(o.name, o.description, Arc::new(object(o.input_schema.clone())))
                    .with_raw_output_schema(Arc::new(object(o.output_schema.clone())))
                    .with_annotations(
                        ToolAnnotations::new()
                            .read_only(!o.mutating)
                            .destructive(o.mutating),
                    )
            })
            .collect();
        Self {
            agent: Arc::new(Mutex::new(agent)),
            tools: Arc::new(tools),
        }
    }

    /// Runs one tool call; shared by the MCP handler and tests.
    pub fn call(&self, name: &str, arguments: Value) -> CallToolResult {
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
pub fn serve_stdio(agent: Agent) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start async runtime: {e}"))?;
    runtime.block_on(async {
        let running = rmcp::serve_server(McpServer::new(agent), rmcp::transport::stdio())
            .await
            .map_err(|e| format!("MCP initialization failed: {e}"))?;
        running
            .waiting()
            .await
            .map_err(|e| format!("MCP server stopped with an error: {e}"))?;
        Ok(())
    })
}
