// SPDX-License-Identifier: Apache-2.0
//! Unified MCP client manager supporting Stdio, Streamable HTTP, and SSE transports
//! plus OAuth 2.1 PKCE token lifecycle wired to `nola-store::Vault`.

pub mod oauth;
pub mod transport;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use nola_store::Vault;
use nola_types::{
    sha256_hex, CapabilityDomain, McpServerConfig, ToolExecutionResult, ToolSchema, ToolSpec,
    ToolTier,
};
use nola_tools::{ToolContext, ToolHandler};
use serde_json::{json, Value};
use thiserror::Error;

pub use oauth::{McpOAuthManager, OAuthTokenSet, PkceChallenge};
pub use transport::McpConnection;

#[derive(Debug, Error)]
pub enum McpError {
    #[error("mcp transport error: {0}")]
    Transport(String),
    #[error("mcp json-rpc error ({code}): {message}")]
    Rpc { code: i64, message: String },
    #[error("mcp oauth error: {0}")]
    OAuth(String),
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

    /// List tools exposed by an MCP server and return `ToolHandler` adapters for `ToolRegistry`.
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
        Ok(out)
    }
}

fn sanitize_ident(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
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
            Err(e) => return ToolExecutionResult::err_structured("mcp_connect_error", e.to_string(), None),
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
        assert_ne!(d_http, d_sse, "transport change must invalidate pooled connection digest");
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
}
