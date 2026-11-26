// SPDX-License-Identifier: Apache-2.0
//! Unified MCP client manager supporting Stdio, Streamable HTTP, and SSE transports,
//! resources, prompts, pagination, list_changed invalidation, progress/cancellation,
//! plus OAuth 2.1 PKCE/DCR token lifecycle wired to `nola-store::Vault`.

pub mod oauth;
pub mod transport;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use nola_store::Vault;
use nola_tools::{ToolContext, ToolHandler};
use nola_types::{
    sha256_hex, CapabilityDomain, McpServerConfig, ToolExecutionResult, ToolSchema, ToolSpec,
    ToolTier,
};
use serde_json::{json, Value};
use thiserror::Error;

pub use oauth::{
    discover_oauth_metadata, exchange_authorization_code, register_dynamic_client,
    run_interactive_oauth_flow, run_interactive_oauth_flow_with_options,
    DynamicClientRegistration, McpOAuthManager, OAuthMetadata, OAuthTokenSet, PkceChallenge,
    DEFAULT_OAUTH_CALLBACK_TIMEOUT,
};
pub use transport::{
    McpConnection, McpPrompt, McpPromptArgument, McpPromptMessage, McpResource,
    McpResourceContent, McpResourceTemplate, ProgressCallback,
};

#[derive(Debug, Error)]
pub enum McpError {
    #[error("mcp transport error: {0}")]
    Transport(String),
    #[error("mcp json-rpc error ({code}): {message}")]
    Rpc { code: i64, message: String },
    #[error("mcp oauth error: {0}")]
    OAuth(String),
    #[error("mcp request cancelled: {0}")]
    Cancelled(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Canonical digest identifying an MCP server connection.
/// Always includes `transport` so switching HTTP vs SSE invalidates cached handles!
pub fn connection_digest(cfg: &McpServerConfig) -> String {
    let serialized = serde_json::to_vec(&json!({
        "id": cfg.id,
        "transport": cfg.transport,
        "command": cfg.command,
        "args": cfg.args,
        "env": cfg.env,
        "url": cfg.url,
        "headers": cfg.headers,
        "oauth_client_id": cfg.oauth_client_id,
        "oauth_scopes": cfg.oauth_scopes,
    }))
    .unwrap_or_default();
    sha256_hex(&serialized)
}

/// Discovered MCP tool metadata.
#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub server_id: String,
    pub original_name: String,
    pub qualified_name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Manages active pooled MCP connections and OAuth token refreshes.
pub struct McpManager {
    connections: Mutex<BTreeMap<String, (String, Arc<McpConnection>)>>,
    connect_locks: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
    disconnected: Mutex<BTreeSet<String>>,
    oauth: Option<Arc<McpOAuthManager>>,
}

impl McpManager {
    pub fn new(vault: Option<Arc<Vault>>) -> Self {
        Self {
            connections: Mutex::new(BTreeMap::new()),
            connect_locks: Mutex::new(BTreeMap::new()),
            disconnected: Mutex::new(BTreeSet::new()),
            oauth: vault.map(|v| Arc::new(McpOAuthManager::new(v))),
        }
    }

    pub fn oauth(&self) -> Option<Arc<McpOAuthManager>> {
        self.oauth.clone()
    }

    pub fn evict(&self, server_id: &str) {
        self.connections.lock().unwrap().remove(server_id);
    }

    /// Explicitly disconnect an MCP server and mark it disconnected so tool calls
    /// do NOT silently auto-reconnect until `connect()` is called explicitly.
    pub fn disconnect(&self, server_id: &str) {
        self.disconnected
            .lock()
            .unwrap()
            .insert(server_id.to_string());
        self.connections.lock().unwrap().remove(server_id);
    }

    /// Returns true if `server_id` currently has a healthy active connection and
    /// has not been explicitly disconnected.
    pub fn is_connected(&self, server_id: &str) -> bool {
        if self.disconnected.lock().unwrap().contains(server_id) {
            return false;
        }
        self.connections
            .lock()
            .unwrap()
            .get(server_id)
            .map(|(_, c)| c.is_healthy())
            .unwrap_or(false)
    }

    fn is_explicitly_disconnected(&self, server_id: &str) -> bool {
        self.disconnected.lock().unwrap().contains(server_id)
    }

    /// Look up an already-active healthy connection by `server_id`.
    pub fn get_connection(&self, server_id: &str) -> Result<Arc<McpConnection>, McpError> {
        if self.is_explicitly_disconnected(server_id) {
            return Err(McpError::Transport(format!(
                "MCP server '{server_id}' was explicitly disconnected"
            )));
        }
        let guard = self.connections.lock().unwrap();
        match guard.get(server_id) {
            Some((_, conn)) if conn.is_healthy() => Ok(Arc::clone(conn)),
            _ => Err(McpError::Transport(format!(
                "MCP server '{server_id}' is not connected"
            ))),
        }
    }

    /// Connect (or reuse healthy pooled connection matching `connection_digest`) to an MCP server.
    pub fn connect(&self, cfg: &McpServerConfig) -> Result<Arc<McpConnection>, McpError> {
        let srv_lock = {
            let mut locks = self.connect_locks.lock().unwrap();
            Arc::clone(
                locks
                    .entry(cfg.id.clone())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let _guard = srv_lock.lock().unwrap();
        self.disconnected.lock().unwrap().remove(&cfg.id);

        let digest = connection_digest(cfg);
        {
            let guard = self.connections.lock().unwrap();
            if let Some((existing_digest, conn)) = guard.get(&cfg.id) {
                if existing_digest == &digest && conn.is_healthy() {
                    return Ok(Arc::clone(conn));
                }
            }
        }

        let conn = Arc::new(McpConnection::open(cfg, self.oauth.clone())?);
        conn.initialize()?;
        self.connections
            .lock()
            .unwrap()
            .insert(cfg.id.clone(), (digest, Arc::clone(&conn)));
        Ok(conn)
    }

    /// Invoke a tool on an active pooled connection by `server_id`.
    pub fn call_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        args: Value,
    ) -> Result<Value, McpError> {
        self.call_tool_with_options(server_id, tool_name, args, None, None)
    }

    /// Invoke a tool on an active pooled connection with optional cancellation and progress callback.
    pub fn call_tool_with_options(
        &self,
        server_id: &str,
        tool_name: &str,
        args: Value,
        cancel: Option<Arc<AtomicBool>>,
        on_progress: Option<ProgressCallback>,
    ) -> Result<Value, McpError> {
        let conn = self.get_connection(server_id)?;
        let res = conn.call_tool_with_options(tool_name, args, cancel, on_progress);
        if matches!(res, Err(McpError::Transport(_))) {
            self.evict(server_id);
        }
        res
    }

    /// List all resources exposed by an MCP server across `nextCursor` pages.
    pub fn list_resources(&self, cfg: &McpServerConfig) -> Result<Vec<McpResource>, McpError> {
        let conn = self.connect(cfg)?;
        conn.list_resources()
    }

    /// List all resource templates exposed by an MCP server across `nextCursor` pages.
    pub fn list_resource_templates(
        &self,
        cfg: &McpServerConfig,
    ) -> Result<Vec<McpResourceTemplate>, McpError> {
        let conn = self.connect(cfg)?;
        conn.list_resource_templates()
    }

    /// Read a specific resource URI from an MCP server.
    pub fn read_resource(
        &self,
        cfg: &McpServerConfig,
        uri: &str,
    ) -> Result<Vec<McpResourceContent>, McpError> {
        let conn = self.connect(cfg)?;
        conn.read_resource(uri)
    }

    /// List all prompt templates exposed by an MCP server across `nextCursor` pages.
    pub fn list_prompts(&self, cfg: &McpServerConfig) -> Result<Vec<McpPrompt>, McpError> {
        let conn = self.connect(cfg)?;
        conn.list_prompts()
    }

    /// Render a prompt template from an MCP server.
    pub fn get_prompt(
        &self,
        cfg: &McpServerConfig,
        name: &str,
        arguments: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<McpPromptMessage>, McpError> {
        let conn = self.connect(cfg)?;
        conn.get_prompt(name, arguments)
    }

    /// Create a deferred `ToolHandler` exposing MCP resources (`list` / `templates` / `read`).
    pub fn resource_tool(self: &Arc<Self>, cfg: &McpServerConfig) -> Arc<dyn ToolHandler> {
        Arc::new(McpResourceTool::new(Arc::clone(self), cfg.clone()))
    }

    /// Create a deferred `ToolHandler` exposing MCP prompts (`list` / `get`).
    pub fn prompt_tool(self: &Arc<Self>, cfg: &McpServerConfig) -> Arc<dyn ToolHandler> {
        Arc::new(McpPromptTool::new(Arc::clone(self), cfg.clone()))
    }

    /// List tools exposed by an MCP server and return `ToolHandler` adapters for `ToolRegistry`.
    /// Also registers deferred `McpResourceTool` / `McpPromptTool` adapters when the server
    /// advertises `resources` or `prompts` capabilities during `initialize`.
    pub fn discover_tools(
        self: &Arc<Self>,
        cfg: &McpServerConfig,
    ) -> Result<Vec<Arc<dyn ToolHandler>>, McpError> {
        let conn = self.connect(cfg)?;
        let raw_tools = conn.list_tools()?;
        let mut out: Vec<Arc<dyn ToolHandler>> = Vec::new();
        for tool in raw_tools {
            let orig = tool
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if orig.is_empty() {
                continue;
            }
            let info = McpToolInfo {
                server_id: cfg.id.clone(),
                qualified_name: format!("mcp_{}_{}", sanitize_ident(&cfg.id), sanitize_ident(&orig)),
                original_name: orig,
                description: tool
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("MCP remote tool")
                    .to_string(),
                input_schema: tool
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
            };
            out.push(Arc::new(McpToolAdapter {
                manager: Arc::clone(self),
                config: cfg.clone(),
                info,
            }));
        }
        if conn.supports_resources() {
            out.push(self.resource_tool(cfg));
        }
        if conn.supports_prompts() {
            out.push(self.prompt_tool(cfg));
        }
        Ok(out)
    }
}

fn sanitize_ident(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Deferred tool allowing agents to discover and read MCP resources and resource templates.
pub struct McpResourceTool {
    manager: Arc<McpManager>,
    config: McpServerConfig,
}

impl McpResourceTool {
    pub fn new(manager: Arc<McpManager>, config: McpServerConfig) -> Self {
        Self { manager, config }
    }
}

impl ToolHandler for McpResourceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: format!("mcp_{}_resources", sanitize_ident(&self.config.id)),
                description: format!(
                    "[MCP:{}] List resources, list resource templates, or read a resource URI",
                    self.config.name
                ),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["list", "templates", "read"],
                            "description": "Operation to perform on MCP resources"
                        },
                        "uri": {
                            "type": "string",
                            "description": "Resource URI to read (required when action is 'read')"
                        }
                    },
                    "required": ["action"],
                    "additionalProperties": false
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Mcp,
            read_only: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        if self.manager.is_explicitly_disconnected(&self.config.id) {
            return ToolExecutionResult::err_structured(
                "mcp_disconnected",
                format!(
                    "MCP server '{}' was explicitly disconnected; reconnect before accessing resources",
                    self.config.id
                ),
                None,
            );
        }
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        match action {
            "list" => match self.manager.list_resources(&self.config) {
                Ok(resources) => ToolExecutionResult::ok_structured(
                    json!({ "resources": resources }),
                    format!("Listed {} MCP resources", resources.len()),
                ),
                Err(e) => ToolExecutionResult::err_structured("mcp_resource_error", e.to_string(), None),
            },
            "templates" => match self.manager.list_resource_templates(&self.config) {
                Ok(templates) => ToolExecutionResult::ok_structured(
                    json!({ "resourceTemplates": templates }),
                    format!("Listed {} MCP resource templates", templates.len()),
                ),
                Err(e) => ToolExecutionResult::err_structured("mcp_resource_error", e.to_string(), None),
            },
            "read" => {
                let Some(uri) = args
                    .get("uri")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                else {
                    return ToolExecutionResult::err_structured(
                        "invalid_arguments",
                        "action 'read' requires non-empty 'uri'",
                        None,
                    );
                };
                match self.manager.read_resource(&self.config, uri) {
                    Ok(contents) => ToolExecutionResult::ok_structured(
                        json!({ "uri": uri, "contents": contents }),
                        format!("Read MCP resource {uri}"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("mcp_resource_error", e.to_string(), None),
                }
            }
            _ => ToolExecutionResult::err_structured(
                "invalid_arguments",
                "action must be one of 'list', 'templates', or 'read'",
                None,
            ),
        }
    }
}

/// Deferred tool allowing agents to list and render MCP prompt templates.
pub struct McpPromptTool {
    manager: Arc<McpManager>,
    config: McpServerConfig,
}

impl McpPromptTool {
    pub fn new(manager: Arc<McpManager>, config: McpServerConfig) -> Self {
        Self { manager, config }
    }
}

impl ToolHandler for McpPromptTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: format!("mcp_{}_prompts", sanitize_ident(&self.config.id)),
                description: format!(
                    "[MCP:{}] List available prompt templates or render a prompt by name",
                    self.config.name
                ),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["list", "get"],
                            "description": "Operation to perform on MCP prompts"
                        },
                        "name": {
                            "type": "string",
                            "description": "Prompt template name (required when action is 'get')"
                        },
                        "arguments": {
                            "type": "object",
                            "additionalProperties": { "type": "string" },
                            "description": "Optional prompt template arguments"
                        }
                    },
                    "required": ["action"],
                    "additionalProperties": false
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Mcp,
            read_only: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        if self.manager.is_explicitly_disconnected(&self.config.id) {
            return ToolExecutionResult::err_structured(
                "mcp_disconnected",
                format!(
                    "MCP server '{}' was explicitly disconnected; reconnect before accessing prompts",
                    self.config.id
                ),
                None,
            );
        }
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        match action {
            "list" => match self.manager.list_prompts(&self.config) {
                Ok(prompts) => ToolExecutionResult::ok_structured(
                    json!({ "prompts": prompts }),
                    format!("Listed {} MCP prompts", prompts.len()),
                ),
                Err(e) => ToolExecutionResult::err_structured("mcp_prompt_error", e.to_string(), None),
            },
            "get" => {
                let Some(name) = args
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                else {
                    return ToolExecutionResult::err_structured(
                        "invalid_arguments",
                        "action 'get' requires non-empty 'name'",
                        None,
                    );
                };
                let prompt_args = args.get("arguments").and_then(Value::as_object).map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect::<BTreeMap<String, String>>()
                });
                match self.manager.get_prompt(&self.config, name, prompt_args) {
                    Ok(messages) => ToolExecutionResult::ok_structured(
                        json!({ "name": name, "messages": messages }),
                        format!("Rendered MCP prompt {name}"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("mcp_prompt_error", e.to_string(), None),
                }
            }
            _ => ToolExecutionResult::err_structured(
                "invalid_arguments",
                "action must be 'list' or 'get'",
                None,
            ),
        }
    }
}

/// Bridges an MCP tool into `nola-tools::ToolRegistry` as a `ToolTier::Deferred` tool.
struct McpToolAdapter {
    manager: Arc<McpManager>,
    config: McpServerConfig,
    info: McpToolInfo,
}

impl ToolHandler for McpToolAdapter {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: self.info.qualified_name.clone(),
                description: format!("[MCP:{}] {}", self.config.name, self.info.description),
                parameters: self.info.input_schema.clone(),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Mcp,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        if self.manager.is_explicitly_disconnected(&self.config.id) {
            return ToolExecutionResult::err_structured(
                "mcp_disconnected",
                format!(
                    "MCP server '{}' was explicitly disconnected; reconnect before invoking tools",
                    self.config.id
                ),
                None,
            );
        }
        let conn = match self.manager.connect(&self.config) {
            Ok(c) => c,
            Err(e) => {
                return ToolExecutionResult::err_structured("mcp_connect_error", e.to_string(), None)
            }
        };
        match conn.call_tool(&self.info.original_name, args) {
            Ok(result) => {
                let is_error = result
                    .get("isError")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if is_error {
                    ToolExecutionResult::err_structured(
                        "mcp_tool_error",
                        format!("MCP tool '{}' returned error", self.info.original_name),
                        Some(result),
                    )
                } else {
                    ToolExecutionResult::ok_structured(
                        result,
                        format!("Executed MCP tool {}", self.info.qualified_name),
                    )
                }
            }
            Err(e) => {
                if matches!(e, McpError::Transport(_)) {
                    self.manager.evict(&self.config.id);
                }
                ToolExecutionResult::err_structured("mcp_call_error", e.to_string(), None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_types::McpTransport;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn connection_digest_distinguishes_http_and_sse_transports() {
        let mut cfg = McpServerConfig {
            id: "docs".into(),
            name: "Docs".into(),
            transport: McpTransport::StreamableHttp,
            enabled: true,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some("https://mcp.example.com/v1".into()),
            headers: BTreeMap::new(),
            oauth_client_id: None,
            oauth_scopes: vec![],
        };
        let d_http = connection_digest(&cfg);
        cfg.transport = McpTransport::ServerSentEvents;
        let d_sse = connection_digest(&cfg);
        assert_ne!(
            d_http, d_sse,
            "transport change must invalidate pooled connection digest"
        );
    }

    #[test]
    fn explicit_disconnect_prevents_silent_auto_reconnect_in_tool_adapter() {
        let mgr = Arc::new(McpManager::new(None));
        let cfg = McpServerConfig {
            id: "srv1".into(),
            name: "Srv1".into(),
            transport: McpTransport::Stdio,
            enabled: true,
            command: Some("/nonexistent/mcp-bin".into()),
            args: vec![],
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
            oauth_client_id: None,
            oauth_scopes: vec![],
        };
        assert!(!mgr.is_connected("srv1"));
        mgr.disconnect("srv1");
        assert!(!mgr.is_connected("srv1"));

        let adapter = McpToolAdapter {
            manager: Arc::clone(&mgr),
            config: cfg,
            info: McpToolInfo {
                server_id: "srv1".into(),
                original_name: "ping".into(),
                qualified_name: "mcp_srv1_ping".into(),
                description: "Ping".into(),
                input_schema: json!({"type": "object"}),
            },
        };
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext::new(dir.path());
        let res = adapter.execute(&ctx, json!({}));
        assert!(res.is_error);
        assert!(res.payload.to_model_string().contains("mcp_disconnected"));
    }

    #[test]
    fn discover_tools_exposes_deferred_resource_and_prompt_tools_and_manager_call_with_options() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(8) {
                let mut stream = stream.unwrap();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let header_end = loop {
                    let n = stream.read(&mut tmp).unwrap();
                    if n == 0 {
                        break buf.len();
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let hdr = String::from_utf8_lossy(&buf[..header_end]);
                let cl = hdr
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                while buf.len() < header_end + cl {
                    let n = stream.read(&mut tmp).unwrap();
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let req: Value =
                    serde_json::from_slice(&buf[header_end..header_end + cl]).unwrap_or(Value::Null);
                let id = req.get("id").cloned();
                let body = match req["method"].as_str().unwrap_or("") {
                    "initialize" => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": { "tools": {}, "resources": {}, "prompts": {} }
                        }
                    })
                    .to_string(),
                    "notifications/initialized" => String::new(),
                    "tools/list" => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": { "tools": [{"name": "echo", "inputSchema": {"type": "object"}}] }
                    })
                    .to_string(),
                    "resources/list" => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": { "resources": [{"uri": "doc://guide", "name": "Guide"}] }
                    })
                    .to_string(),
                    "resources/read" => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": { "contents": [{"uri": "doc://guide", "text": "Guide Body"}] }
                    })
                    .to_string(),
                    "prompts/list" => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": { "prompts": [{"name": "greet", "arguments": []}] }
                    })
                    .to_string(),
                    "prompts/get" => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": { "messages": [{"role": "user", "content": {"type": "text", "text": "Hello!"}}] }
                    })
                    .to_string(),
                    "tools/call" => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": { "content": [{"type": "text", "text": "echoed"}] }
                    })
                    .to_string(),
                    other => panic!("unexpected rpc {other}"),
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });

        let mgr = Arc::new(McpManager::new(None));
        let cfg = McpServerConfig {
            id: "docs".into(),
            name: "Docs".into(),
            transport: McpTransport::StreamableHttp,
            enabled: true,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some(format!("http://{addr}/mcp")),
            headers: BTreeMap::new(),
            oauth_client_id: None,
            oauth_scopes: vec![],
        };

        let tools = mgr.discover_tools(&cfg).unwrap();
        let names: Vec<String> = tools.iter().map(|t| t.spec().schema.name).collect();
        assert_eq!(
            names,
            vec!["mcp_docs_echo", "mcp_docs_resources", "mcp_docs_prompts"]
        );

        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext::new(dir.path());

        let res_tool = tools
            .iter()
            .find(|t| t.spec().schema.name == "mcp_docs_resources")
            .unwrap();
        let listed = res_tool.execute(&ctx, json!({"action": "list"}));
        assert!(!listed.is_error);
        assert!(listed.payload.to_model_string().contains("doc://guide"));
        let read = res_tool.execute(&ctx, json!({"action": "read", "uri": "doc://guide"}));
        assert!(!read.is_error);
        assert!(read.payload.to_model_string().contains("Guide Body"));

        let prompt_tool = tools
            .iter()
            .find(|t| t.spec().schema.name == "mcp_docs_prompts")
            .unwrap();
        let pl = prompt_tool.execute(&ctx, json!({"action": "list"}));
        assert!(!pl.is_error);
        assert!(pl.payload.to_model_string().contains("greet"));
        let pg = prompt_tool.execute(&ctx, json!({"action": "get", "name": "greet"}));
        assert!(!pg.is_error);
        assert!(pg.payload.to_model_string().contains("Hello!"));

        let call_res = mgr
            .call_tool_with_options("docs", "echo", json!({"msg": "hi"}), None, None)
            .unwrap();
        assert_eq!(call_res["content"][0]["text"], "echoed");

        server.join().unwrap();
    }
}
