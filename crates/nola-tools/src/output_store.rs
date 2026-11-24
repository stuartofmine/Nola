// SPDX-License-Identifier: Apache-2.0
//! Externalized Tool Output Store (`OutputStore`) + `tool_output` pagination/search tool.
//! Automatically spills oversized tool payloads (> 24 KiB) to disk while giving the model
//! a compact head+tail preview plus `output_id` for on-demand offset/regex inspection.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use nola_types::{
    sha256_hex, CapabilityDomain, ToolExecutionResult, ToolModelPayload, ToolSchema, ToolSpec,
    ToolTier,
};
use regex::RegexBuilder;
use serde_json::{json, Value};

use crate::{ToolContext, ToolHandler};

pub const DEFAULT_SPILL_THRESHOLD_BYTES: usize = 24 * 1024;
const PREVIEW_HEAD_CHARS: usize = 3_000;
const PREVIEW_TAIL_CHARS: usize = 1_500;

#[derive(Debug, Clone)]
pub struct StoredOutputMeta {
    pub output_id: String,
    pub tool_name: String,
    pub total_bytes: usize,
    pub total_lines: usize,
    pub path: PathBuf,
}

/// Content-addressed spill store for large tool outputs.
pub struct OutputStore {
    root_dir: PathBuf,
    entries: Mutex<BTreeMap<String, StoredOutputMeta>>,
}

impl Default for OutputStore {
    fn default() -> Self {
        let dir = std::env::temp_dir().join(format!("nola-v2-outputs-{}", std::process::id()));
        Self::new(dir)
    }
}

impl OutputStore {
    pub fn new(root_dir: PathBuf) -> Self {
        Self {
            root_dir,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn store_text(&self, tool_name: &str, text: &str) -> Result<StoredOutputMeta, String> {
        fs::create_dir_all(&self.root_dir)
            .map_err(|e| format!("failed to create output store dir: {e}"))?;
        let hash = sha256_hex(text.as_bytes());
        let output_id = format!("out_{}", &hash[..16]);
        let path = self.root_dir.join(format!("{output_id}.txt"));
        fs::write(&path, text.as_bytes())
            .map_err(|e| format!("failed to write spilled output: {e}"))?;
        let meta = StoredOutputMeta {
            output_id: output_id.clone(),
            tool_name: tool_name.to_string(),
            total_bytes: text.len(),
            total_lines: text.lines().count(),
            path,
        };
        self.entries.lock().unwrap().insert(output_id, meta.clone());
        Ok(meta)
    }

    /// Spill oversized `ToolModelPayload` if serialized bytes exceed `threshold_bytes`.
    pub fn maybe_spill_payload(
        &self,
        tool_name: &str,
        payload: &ToolModelPayload,
        threshold_bytes: usize,
    ) -> Option<(ToolModelPayload, StoredOutputMeta)> {
        let full = payload.to_model_string();
        if full.len() <= threshold_bytes {
            return None;
        }
        let meta = self.store_text(tool_name, &full).ok()?;
        let head: String = full.chars().take(PREVIEW_HEAD_CHARS).collect();
        let total_chars = full.chars().count();
        let tail: String = full
            .chars()
            .skip(total_chars.saturating_sub(PREVIEW_TAIL_CHARS))
            .collect();
        let summary = payload.ui_summary().to_string();
        let compact = ToolModelPayload::structured(
            json!({
                "spilled": true,
                "output_id": meta.output_id,
                "tool": tool_name,
                "total_bytes": meta.total_bytes,
                "total_lines": meta.total_lines,
                "hint": "Use tool_output (action=read or search) with this output_id to inspect full content.",
                "head_preview": head,
                "tail_preview": tail
            }),
            format!("{summary} [spilled {} bytes -> {}]", meta.total_bytes, meta.output_id),
        );
        Some((compact, meta))
    }

    pub fn list(&self) -> Vec<Value> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .map(|m| {
                json!({
                    "output_id": m.output_id,
                    "tool_name": m.tool_name,
                    "total_bytes": m.total_bytes,
                    "total_lines": m.total_lines
                })
            })
            .collect()
    }

    pub fn read_range(&self, output_id: &str, start_line: usize, end_line: usize) -> Result<Value, String> {
        let path = self.resolve_output_path(output_id)?;
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read stored output: {e}"))?;
        let lines: Vec<&str> = text.lines().collect();
        let start = start_line.max(1);
        // Cap single read to at most 400 lines so tool_output cannot blow up context!
        let end = end_line
            .max(start)
            .min(start.saturating_add(399))
            .min(lines.len().max(1));
        let slice = lines
            .iter()
            .enumerate()
            .skip(start.saturating_sub(1))
            .take(end.saturating_sub(start).saturating_add(1))
            .map(|(i, l)| format!("{}: {l}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(json!({
            "output_id": output_id,
            "start_line": start,
            "end_line": end.min(lines.len()),
            "total_lines": lines.len(),
            "truncated": end < lines.len(),
            "content": slice
        }))
    }

    pub fn search(&self, output_id: &str, pattern: &str, max_matches: usize) -> Result<Value, String> {
        let path = self.resolve_output_path(output_id)?;
        let re = RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .map_err(|e| format!("invalid regex: {e}"))?;
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read stored output: {e}"))?;
        let mut hits = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                hits.push(json!({"line": i + 1, "text": line}));
                if hits.len() >= max_matches.clamp(1, 200) {
                    break;
                }
            }
        }
        Ok(json!({
            "output_id": output_id,
            "pattern": pattern,
            "count": hits.len(),
            "matches": hits
        }))
    }

    /// Resolve `output_id` from in-memory map OR safely from disk (supports `--resume` across processes).
    fn resolve_output_path(&self, output_id: &str) -> Result<PathBuf, String> {
        if let Some(meta) = self.entries.lock().unwrap().get(output_id) {
            return Ok(meta.path.clone());
        }
        if output_id.is_empty()
            || !output_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!("invalid output_id '{output_id}'"));
        }
        let disk_path = self.root_dir.join(format!("{output_id}.txt"));
        if disk_path.is_file() {
            return Ok(disk_path);
        }
        Err(format!("unknown output_id '{output_id}'"))
    }
}

pub struct ToolOutputTool;

impl ToolHandler for ToolOutputTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "tool_output".into(),
                description: "Inspect spilled oversized tool outputs (action: list|read|search).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["list", "read", "search"] },
                        "output_id": { "type": "string" },
                        "start_line": { "type": "integer", "description": "1-based start line for read" },
                        "end_line": { "type": "integer", "description": "1-based inclusive end line for read" },
                        "pattern": { "type": "string", "description": "Regex pattern for search" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("list") {
            "list" => {
                let items = ctx.output_store.list();
                let count = items.len();
                ToolExecutionResult::ok_structured(
                    json!({"count": count, "outputs": items}),
                    format!("Listed {count} stored tool output(s)"),
                )
            }
            "read" => {
                let Some(id) = args.get("output_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'output_id'", None);
                };
                let start = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let end = args
                    .get("end_line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(start.saturating_add(199) as u64) as usize;
                match ctx.output_store.read_range(id, start, end) {
                    Ok(v) => ToolExecutionResult::ok_structured(v, format!("Read lines {start}-{end} of {id}")),
                    Err(e) => ToolExecutionResult::err_structured("output_read_error", e, None),
                }
            }
            "search" => {
                let Some(id) = args.get("output_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'output_id'", None);
                };
                let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'pattern'", None);
                };
                match ctx.output_store.search(id, pattern, 100) {
                    Ok(v) => {
                        let c = v["count"].as_u64().unwrap_or(0);
                        ToolExecutionResult::ok_structured(v, format!("Found {c} matches in {id}"))
                    }
                    Err(e) => ToolExecutionResult::err_structured("output_search_error", e, None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported tool_output action '{other}'"),
                None,
            ),
        }
    }
}
