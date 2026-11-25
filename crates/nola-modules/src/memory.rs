// SPDX-License-Identifier: Apache-2.0
//! Long-term FTS5 Memory & Codebase Memory module.

use std::process::Command;
use std::sync::Arc;

use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, CapabilityDomain, MemoryItem, ToolExecutionResult, ToolSchema, ToolSpec,
    ToolTier,
};
use serde_json::{json, Value};

use crate::ModulesSuite;

/// Background memory distillation worker saving concise turn summaries into FTS5.
pub struct MemoryWorker;

impl MemoryWorker {
    pub fn distill_turn(
        suite: &ModulesSuite,
        namespace: &str,
        user_prompt: &str,
        assistant_answer: &str,
    ) -> Result<Option<MemoryItem>, nola_store::StoreError> {
        let answer = assistant_answer.trim();
        if answer.chars().count() < 80 {
            return Ok(None);
        }
        let clean_prompt = nola_llm::PrivacyRedactor::redact_text(user_prompt);
        let clean_answer = nola_llm::PrivacyRedactor::redact_text(answer);
        let title: String = clean_prompt
            .lines()
            .next()
            .unwrap_or("Session insight")
            .chars()
            .take(80)
            .collect();
        let content: String = clean_answer.chars().take(600).collect();
        let ts = now_ms();
        let item = MemoryItem {
            id: new_id(),
            namespace: namespace.to_string(),
            title,
            content,
            tags: vec!["auto_distilled".into()],
            created_at_ms: ts,
            updated_at_ms: ts,
        };
        suite.store.upsert_memory(&item)?;
        Ok(Some(item))
    }
}

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(MemorySaveTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(MemorySearchTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(CodeMemorySummaryTool {
        suite: Arc::clone(suite),
    }));
}

struct MemorySaveTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for MemorySaveTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "memory_save".into(),
                description: "Persist a durable fact, decision, or note into FTS5-indexed long-term memory.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "namespace": { "type": "string", "description": "Default 'default'" },
                        "title": { "type": "string" },
                        "content": { "type": "string" },
                        "tags": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["title", "content"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(title) = args.get("title").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'title'", None);
        };
        let Some(content) = args.get("content").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'content'", None);
        };
        let ts = now_ms();
        let item = MemoryItem {
            id: args
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(new_id),
            namespace: args
                .get("namespace")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string(),
            title: title.to_string(),
            content: content.to_string(),
            tags: args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            created_at_ms: ts,
            updated_at_ms: ts,
        };
        match self.suite.store.upsert_memory(&item) {
            Ok(()) => ToolExecutionResult::ok_structured(
                serde_json::to_value(&item).unwrap_or(json!({})),
                format!("Saved memory '{}' ({})", item.title, item.id),
            ),
            Err(e) => ToolExecutionResult::err_structured("memory_error", e.to_string(), None),
        }
    }
}

struct MemorySearchTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for MemorySearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "memory_search".into(),
                description: "Search persisted long-term memories using SQLite FTS5 BM25 ranking.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "namespace": { "type": "string" },
                        "limit": { "type": "integer", "description": "Default 10" }
                    },
                    "required": ["query"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let ns = args.get("namespace").and_then(|v| v.as_str());
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .clamp(1, 100) as usize;
        match self.suite.store.search_memory(query, ns, limit) {
            Ok(items) => {
                let count = items.len();
                ToolExecutionResult::ok_structured(
                    json!({"query": query, "count": count, "items": items}),
                    format!("Found {count} memory item(s)"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("memory_error", e.to_string(), None),
        }
    }
}

struct CodeMemorySummaryTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for CodeMemorySummaryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "code_memory_snapshot".into(),
                description: "Capture and index current repository HEAD, changed files, and recent commits into Code-Memory.".into(),
                parameters: json!({ "type": "object", "properties": {} }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, _args: Value) -> ToolExecutionResult {
        let root = if ctx.workspace_root.exists() {
            &ctx.workspace_root
        } else {
            &self.suite.workspace_root
        };
        let index_stats = crate::code_memory::index_workspace(&self.suite.store, root, false).ok();
        let head = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "none".into());
        let log = Command::new("git")
            .args(["log", "--oneline", "-n", "10"])
            .current_dir(root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let (total_files, total_symbols, duration_ms) = index_stats
            .as_ref()
            .map(|s| (s.total_files, s.total_symbols, s.duration_ms))
            .unwrap_or((0, 0, 0));
        let summary_content = if log.is_empty() {
            format!("Indexed {total_files} source file(s) with {total_symbols} symbol(s) in {duration_ms}ms.")
        } else {
            format!(
                "Indexed {total_files} source file(s) with {total_symbols} symbol(s) in {duration_ms}ms.\nRecent commits:\n{log}"
            )
        };
        let ts = now_ms();
        let item = MemoryItem {
            id: format!("code_memory:{head}"),
            namespace: "code_memory".into(),
            title: format!("Repository snapshot @ {head}"),
            content: summary_content,
            tags: vec!["git".into(), "code_memory".into(), "symbols".into()],
            created_at_ms: ts,
            updated_at_ms: ts,
        };
        if let Err(e) = self.suite.store.upsert_memory(&item) {
            return ToolExecutionResult::err_structured("memory_error", e.to_string(), None);
        }
        ToolExecutionResult::ok_structured(
            json!({
                "head": head,
                "recent_commits": log,
                "total_files": total_files,
                "total_symbols": total_symbols,
                "duration_ms": duration_ms
            }),
            format!(
                "Indexed Code-Memory snapshot @ {head} ({total_files} files, {total_symbols} symbols)"
            ),
        )
    }
}
