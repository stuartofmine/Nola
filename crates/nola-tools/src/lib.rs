// SPDX-License-Identifier: Apache-2.0
//! Unified Nola v2 Tool Runtime:
//! - Single `FileMutationPipeline` guaranteeing `VersionTransition` on all file mutations.
//! - Single `ShellExecutionPipeline` with capability-probed `bwrap` sandbox and `output_artifacts`.
//! - Full binary document intelligence (spreadsheets, PDFs, audio FFT, images).
//! - DNS-pinned SSRF-safe `web_fetch`, bounded `rquickjs` code-mode, and pooled LSP diagnostics.

pub mod binary_docs;
pub mod code_mode;
pub mod file_pipeline;
pub mod git;
pub mod lsp;
pub mod sandbox;
pub mod search;
pub mod shell_pipeline;
pub mod web;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nola_types::{
    CapabilityDomain, PermissionProfile, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};

pub use file_pipeline::{path_within_workspace, FileAccessState, FileMutationPipeline};
pub use lsp::LspPool;
pub use sandbox::{probe_bwrap_available, SandboxMode};
pub use shell_pipeline::{validation_snapshot, ShellExecutionPipeline};

/// Shared execution context passed to every `ToolHandler`.
#[derive(Clone)]
pub struct ToolContext {
    pub workspace_root: PathBuf,
    pub file_state: Arc<FileAccessState>,
    pub shell: Arc<ShellExecutionPipeline>,
    pub lsp: Arc<LspPool>,
    pub network_allowed: bool,
    pub permission_profile: PermissionProfile,
    pub sandbox_mode: SandboxMode,
}

impl ToolContext {
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        let root = std::fs::canonicalize(workspace_root.as_ref())
            .unwrap_or_else(|_| workspace_root.as_ref().to_path_buf());
        Self {
            workspace_root: root,
            file_state: Arc::new(FileAccessState::default()),
            shell: Arc::new(ShellExecutionPipeline::default()),
            lsp: Arc::new(LspPool::default()),
            network_allowed: true,
            permission_profile: PermissionProfile::Cautious,
            sandbox_mode: SandboxMode::Auto,
        }
    }
}

/// Trait implemented by every built-in, module, or MCP tool.
pub trait ToolHandler: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult;
}

/// Authoritative tool registry maintaining deterministic registration order
/// (foundation for KV-cache prefix stability).
#[derive(Default)]
pub struct ToolRegistry {
    order: Vec<String>,
    handlers: BTreeMap<String, Arc<dyn ToolHandler>>,
}

impl ToolRegistry {
    pub fn with_builtin_tools() -> Self {
        let mut reg = Self::default();
        // Turn-0 Core orthogonal primitives (~12 tools)
        reg.register(Arc::new(file_pipeline::ReadFileTool));
        reg.register(Arc::new(file_pipeline::WriteFileTool));
        reg.register(Arc::new(file_pipeline::EditFileTool));
        reg.register(Arc::new(file_pipeline::ApplyPatchTool));
        reg.register(Arc::new(file_pipeline::DeleteFileTool));
        reg.register(Arc::new(search::ListDirTool));
        reg.register(Arc::new(search::GlobTool));
        reg.register(Arc::new(search::GrepTool));
        reg.register(Arc::new(shell_pipeline::BashTool));
        reg.register(Arc::new(shell_pipeline::BashControlTool));
        reg.register(Arc::new(git::GitTool));
        reg.register(Arc::new(ToolSearchTool));
        // Deferred tools (activated on demand via `tool_search`, appended at tail)
        reg.register(Arc::new(search::WorkspaceStatusTool));
        reg.register(Arc::new(web::WebFetchTool));
        reg.register(Arc::new(code_mode::CodeExecTool));
        reg.register(Arc::new(lsp::LspDiagnosticsTool));
        reg
    }

    pub fn register(&mut self, handler: Arc<dyn ToolHandler>) {
        let name = handler.spec().schema.name;
        if !self.handlers.contains_key(&name) {
            self.order.push(name.clone());
        }
        self.handlers.insert(name, handler);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ToolHandler>> {
        self.handlers.get(name).cloned()
    }

    pub fn all_specs(&self) -> Vec<ToolSpec> {
        self.order
            .iter()
            .filter_map(|name| self.handlers.get(name).map(|h| h.spec()))
            .collect()
    }

    pub fn search_specs(&self, query: &str) -> Vec<ToolSpec> {
        let q = query.trim().to_ascii_lowercase();
        let terms: Vec<&str> = q.split_whitespace().collect();
        self.all_specs()
            .into_iter()
            .filter(|spec| {
                if terms.is_empty() {
                    return true;
                }
                let hay = format!(
                    "{} {}",
                    spec.schema.name.to_ascii_lowercase(),
                    spec.schema.description.to_ascii_lowercase()
                );
                terms.iter().any(|t| hay.contains(t))
            })
            .collect()
    }

    pub fn execute(&self, name: &str, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        if name == "tool_search" {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let matches: Vec<Value> = self
                .search_specs(query)
                .into_iter()
                .map(|s| {
                    json!({
                        "name": s.schema.name,
                        "tier": s.tier,
                        "domain": s.domain,
                        "read_only": s.read_only,
                        "description": s.schema.description,
                        "parameters": s.schema.parameters,
                    })
                })
                .collect();
            let count = matches.len();
            return ToolExecutionResult::ok_structured(
                json!({ "query": query, "count": count, "tools": matches }),
                format!("Found {count} tools matching '{query}'"),
            );
        }
        match self.handlers.get(name) {
            Some(handler) => handler.execute(ctx, args),
            None => ToolExecutionResult::err_structured(
                "unknown_tool",
                format!("tool '{name}' is not registered"),
                None,
            ),
        }
    }
}

/// Core tool allowing models to discover deferred / MCP / module tools on demand.
struct ToolSearchTool;

impl ToolHandler for ToolSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "tool_search".into(),
                description: "Search available deferred, module, and MCP tools by keyword and return their full schemas.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Keyword(s) to match tool names or descriptions (empty lists all)" }
                    },
                    "required": ["query"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::System,
            read_only: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        // Intercepted by `ToolRegistry::execute`; fallback for direct unit calls.
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        ToolExecutionResult::ok_structured(
            json!({ "query": query, "count": 0, "tools": [] }),
            format!("Tool search query '{query}'"),
        )
    }
}
