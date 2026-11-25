// SPDX-License-Identifier: Apache-2.0
//! Unified Nola v2 Tool Runtime:
//! - Single `FileMutationPipeline` guaranteeing `VersionTransition` on all file mutations
//! - Single `ShellExecutionPipeline` with capability-probed OS sandbox (`bwrap` / macOS Seatbelt) & `output_artifacts`
//! - Interactive VT100 `PtyManager` (`pty_exec`)
//! - Content-addressed `OutputStore` (`tool_output`) preventing huge outputs from blowing context
//! - Real JSON-RPC 2.0 `LspPool` (`lsp`) + multi-language `repo_map` + `file_outline` + persistent `workspace_checkpoint`
//! - Full binary document readers (DOCX, spreadsheets, PDFs, audio FFT, images) with magic-byte sniffing
//! - DNS-pinned SSRF-safe `web_fetch` + keyless/API `web_search`
//! - Managed Chromium CDP `BrowserManager` (`browser`) + `NativeBrowserBridge`
//! - Native Desktop Automation (`computer` + `computer_helper.py`)
//! - Bounded QuickJS sandbox (`code_exec`) with optional host `tools` proxy dispatch

pub mod binary_docs;
pub mod browser;
pub mod code_mode;
pub mod code_tools;
pub mod computer;
pub mod file_pipeline;
pub mod git;
pub mod lsp;
pub mod output_store;
pub mod pty;
pub mod sandbox;
pub mod search;
pub mod shell_pipeline;
pub mod web;
pub mod web_search;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nola_types::{
    CapabilityDomain, PermissionProfile, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};

pub use binary_docs::extract_document_text_from_bytes;
pub use browser::{BrowserManager, NativeBrowserBridge};
pub use code_tools::CheckpointManager;
pub use file_pipeline::{
    canonicalize_nearest_ancestor, path_within_workspace, FileAccessState, FileLockRegistry,
    FileMutationPipeline,
};
pub use lsp::{collect_post_edit_diagnostics, LspPool};
pub use output_store::{OutputStore, DEFAULT_SPILL_THRESHOLD_BYTES};
pub use pty::PtyManager;
pub use sandbox::{probe_bwrap_available, probe_seatbelt_available, SandboxMode};
pub use shell_pipeline::{validation_snapshot, ShellExecutionPipeline};

/// Optional host tool dispatch callback used by `code_exec` (`tools.<name>(args)`).
pub type CodeModeDispatcher = Arc<dyn Fn(&str, Value) -> Result<Value, String> + Send + Sync>;

/// Shared execution context passed to every `ToolHandler`.
#[derive(Clone)]
pub struct ToolContext {
    pub workspace_root: PathBuf,
    pub session_id: String,
    pub file_state: Arc<FileAccessState>,
    pub file_locks: Arc<FileLockRegistry>,
    pub shell: Arc<ShellExecutionPipeline>,
    pub pty: Arc<PtyManager>,
    pub output_store: Arc<OutputStore>,
    pub checkpoints: Arc<CheckpointManager>,
    pub browser: Arc<BrowserManager>,
    pub lsp: Arc<LspPool>,
    pub network_allowed: bool,
    pub interactive_user_input: bool,
    pub permission_profile: PermissionProfile,
    pub sandbox_mode: SandboxMode,
    pub code_mode_dispatcher: Option<CodeModeDispatcher>,
    pub available_tool_names: Vec<String>,
}

impl ToolContext {
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        let root = std::fs::canonicalize(workspace_root.as_ref())
            .unwrap_or_else(|_| workspace_root.as_ref().to_path_buf());
        Self {
            workspace_root: root,
            session_id: "default".to_string(),
            file_state: Arc::new(FileAccessState::default()),
            file_locks: Arc::new(FileLockRegistry::default()),
            shell: Arc::new(ShellExecutionPipeline::default()),
            pty: Arc::new(PtyManager::default()),
            output_store: Arc::new(OutputStore::default()),
            checkpoints: Arc::new(CheckpointManager::default()),
            browser: Arc::new(BrowserManager::default()),
            lsp: Arc::new(LspPool::default()),
            network_allowed: true,
            interactive_user_input: false,
            permission_profile: PermissionProfile::Cautious,
            sandbox_mode: SandboxMode::Auto,
            code_mode_dispatcher: None,
            available_tool_names: Vec::new(),
        }
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = session_id.into();
        self
    }

    pub fn with_code_mode_dispatcher(
        mut self,
        dispatcher: CodeModeDispatcher,
        available_tool_names: Vec<String>,
    ) -> Self {
        self.code_mode_dispatcher = Some(dispatcher);
        self.available_tool_names = available_tool_names;
        self
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
        // --- Turn-0 Core orthogonal primitives (12 tools) ---
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

        // --- Deferred full-capability tools (tail-activated on demand via `tool_search`) ---
        reg.register(Arc::new(pty::PtyExecTool));
        reg.register(Arc::new(output_store::ToolOutputTool));
        reg.register(Arc::new(code_tools::MultiEditFileTool));
        reg.register(Arc::new(code_tools::MoveFileTool));
        reg.register(Arc::new(code_tools::AppendFileTool));
        reg.register(Arc::new(code_tools::FileOutlineTool));
        reg.register(Arc::new(code_tools::ReadManyFilesTool));
        reg.register(Arc::new(code_tools::NotebookTool));
        reg.register(Arc::new(code_tools::RepoMapTool));
        reg.register(Arc::new(code_tools::WorkspaceCheckpointTool));
        reg.register(Arc::new(search::WorkspaceStatusTool));
        reg.register(Arc::new(lsp::LspDiagnosticsTool));
        reg.register(Arc::new(web::WebFetchTool));
        reg.register(Arc::new(web_search::WebSearchTool));
        reg.register(Arc::new(browser::BrowserTool));
        reg.register(Arc::new(computer::ComputerTool));
        reg.register(Arc::new(code_mode::CodeExecTool));
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
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("").trim();
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(if query.is_empty() { 40 } else { 10 })
                .clamp(1, 50) as usize;
            // Empty browse query lists available tools WITHOUT blindly activating 40 schemas!
            let activate = args
                .get("activate")
                .and_then(|v| v.as_bool())
                .unwrap_or(!query.is_empty());
            let specs: Vec<ToolSpec> = self
                .search_specs(query)
                .into_iter()
                .filter(|s| {
                    s.tier != ToolTier::InteractiveOnly || ctx.interactive_user_input
                })
                .take(limit)
                .collect();
            let activate_names: Vec<String> = if activate {
                specs
                    .iter()
                    .filter(|s| s.tier == ToolTier::Deferred)
                    .take(8)
                    .map(|s| s.schema.name.clone())
                    .collect()
            } else {
                Vec::new()
            };
            // Return compact tool summaries (avoid duplicating full JSON parameter schemas
            // in BOTH tool output history AND prefix_stable_tools!).
            let matches: Vec<Value> = specs
                .iter()
                .map(|s| {
                    let is_activated = activate_names.contains(&s.schema.name);
                    json!({
                        "name": s.schema.name,
                        "tier": s.tier,
                        "read_only": s.read_only,
                        "activated": is_activated,
                        "description": s.schema.description,
                    })
                })
                .collect();
            let count = matches.len();
            return ToolExecutionResult::ok_structured(
                json!({
                    "query": query,
                    "count": count,
                    "activate_names": activate_names,
                    "tools": matches
                }),
                format!(
                    "Found {count} tools matching '{query}' (activated {})",
                    activate_names.len()
                ),
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
                description: "Search deferred, module, and MCP tools by keyword and activate matching tools into your active schema list (empty query lists all without activating).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Keyword(s) to match tool names or descriptions (empty lists all)" },
                        "activate": { "type": "boolean", "description": "Activate matched deferred tools for calling on next turn (default true when query non-empty)" },
                        "limit": { "type": "integer", "description": "Maximum results to return (default 10)" }
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
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        ToolExecutionResult::ok_structured(
            json!({ "query": query, "count": 0, "tools": [] }),
            format!("Tool search query '{query}'"),
        )
    }
}
