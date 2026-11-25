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
const PREVIEW_HEAD_CHARS: usize = 2_400;
const PREVIEW_TAIL_CHARS: usize = 1_600;
const MAX_LINE_CHARS: usize = 2_000;
const MAX_READ_RESPONSE_BYTES: usize = 24 * 1024;

fn clip_line(line: &str) -> String {
    let char_count = line.chars().count();
    if char_count <= MAX_LINE_CHARS {
        line.to_string()
    } else {
        let prefix: String = line.chars().take(MAX_LINE_CHARS).collect();
        format!("{prefix}... [line truncated: {char_count} chars]")
    }
}

/// Convert a `ToolModelPayload` into clean multi-line text for line-oriented `tool_output`
/// inspection, plus preserve key scalar metadata fields directly in the compact spilled envelope.
fn render_spillable_text(payload: &ToolModelPayload) -> (String, serde_json::Map<String, Value>) {
    let mut preserved = serde_json::Map::new();
    match payload {
        ToolModelPayload::Text(s) => (s.clone(), preserved),
        ToolModelPayload::Structured { model_data, .. } => {
            if let Some(obj) = model_data.as_object() {
                for key in [
                    "exit_code",
                    "timed_out",
                    "duration_ms",
                    "path",
                    "count",
                    "total_lines",
                    "truncated",
                    "status",
                    "error",
                    "message",
                    "command",
                ] {
                    if let Some(v) = obj.get(key) {
                        if !v.is_object() && !v.is_array() {
                            preserved.insert(key.to_string(), v.clone());
                        }
                    }
                }
                // Extract dominant multi-line text fields with REAL unescaped newlines!
                let mut sections = Vec::new();
                for key in ["stdout", "stderr", "content", "output"] {
                    if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                        if !s.is_empty() {
                            sections.push(format!("=== {key} ===\n{s}"));
                        }
                    }
                }
                if !sections.is_empty() {
                    return (sections.join("\n\n"), preserved);
                }
            }
            let pretty = serde_json::to_string_pretty(model_data)
                .unwrap_or_else(|_| payload.to_model_string());
            (pretty, preserved)
        }
    }
}

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
        let serialized_len = payload.to_model_string().len();
        if serialized_len <= threshold_bytes {
            return None;
        }
        let (full, preserved) = render_spillable_text(payload);
        let meta = self.store_text(tool_name, &full).ok()?;
        let head: String = full.chars().take(PREVIEW_HEAD_CHARS).collect();
        let total_chars = full.chars().count();
        let tail: String = if total_chars > PREVIEW_HEAD_CHARS {
            full.chars()
                .skip(total_chars.saturating_sub(PREVIEW_TAIL_CHARS))
                .collect()
        } else {
            String::new()
        };
        let summary = payload.ui_summary().to_string();
        let mut envelope = serde_json::Map::new();
        envelope.insert("spilled".into(), Value::Bool(true));
        envelope.insert("output_id".into(), Value::String(meta.output_id.clone()));
        envelope.insert("tool".into(), Value::String(tool_name.to_string()));
        envelope.insert("total_bytes".into(), json!(meta.total_bytes));
        envelope.insert("total_lines".into(), json!(meta.total_lines));
        for (k, v) in preserved {
            envelope.entry(k).or_insert(v);
        }
        envelope.insert(
            "hint".into(),
            Value::String(
                "Use tool_output (action=read|tail|search) with this output_id to inspect full content."
                    .into(),
            ),
        );
        envelope.insert("head_preview".into(), Value::String(head));
        if !tail.is_empty() {
            envelope.insert("tail_preview".into(), Value::String(tail));
        }
        let compact = ToolModelPayload::structured(
            Value::Object(envelope),
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
        let total_lines = lines.len();
        let start = start_line.max(1);
        let requested_end = end_line
            .max(start)
            .min(start.saturating_add(399))
            .min(total_lines.max(1));

        let mut rendered = Vec::new();
        let mut bytes_used = 0usize;
        let mut actual_end = start.saturating_sub(1);
        let mut byte_capped = false;
        if total_lines > 0 && start <= total_lines {
            for (i, line) in lines
                .iter()
                .enumerate()
                .skip(start.saturating_sub(1))
                .take(requested_end.saturating_sub(start).saturating_add(1))
            {
                let formatted = format!("{}: {}", i + 1, clip_line(line));
                if !rendered.is_empty() && bytes_used + formatted.len() + 1 > MAX_READ_RESPONSE_BYTES {
                    byte_capped = true;
                    break;
                }
                bytes_used += formatted.len() + 1;
                actual_end = i + 1;
                rendered.push(formatted);
            }
        }
        Ok(json!({
            "output_id": output_id,
            "start_line": start,
            "end_line": actual_end,
            "total_lines": total_lines,
            "truncated": byte_capped || actual_end < total_lines,
            "content": rendered.join("\n")
        }))
    }

    pub fn read_tail(&self, output_id: &str, tail_lines: usize) -> Result<Value, String> {
        let path = self.resolve_output_path(output_id)?;
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read stored output: {e}"))?;
        let total = text.lines().count();
        let count = tail_lines.clamp(1, 400);
        let start = total.saturating_sub(count).saturating_add(1).max(1);
        self.read_range(output_id, start, total.max(1))
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
        let mut bytes_used = 0usize;
        let limit = max_matches.clamp(1, 200);
        let mut truncated = false;
        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                let clipped = clip_line(line);
                if !hits.is_empty() && bytes_used + clipped.len() + 32 > MAX_READ_RESPONSE_BYTES {
                    truncated = true;
                    break;
                }
                bytes_used += clipped.len() + 32;
                hits.push(json!({"line": i + 1, "text": clipped}));
                if hits.len() >= limit {
                    truncated = true;
                    break;
                }
            }
        }
        Ok(json!({
            "output_id": output_id,
            "pattern": pattern,
            "count": hits.len(),
            "truncated": truncated,
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
                description: "Inspect spilled oversized tool outputs (action: list|read|tail|search).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["list", "read", "tail", "search"] },
                        "output_id": { "type": "string" },
                        "start_line": { "type": "integer", "description": "1-based start line for read" },
                        "end_line": { "type": "integer", "description": "1-based inclusive end line for read" },
                        "tail_lines": { "type": "integer", "description": "Number of trailing lines for action=tail (default 100)" },
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
            "tail" => {
                let Some(id) = args.get("output_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'output_id'", None);
                };
                let tail_lines = args.get("tail_lines").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
                match ctx.output_store.read_tail(id, tail_lines) {
                    Ok(v) => ToolExecutionResult::ok_structured(v, format!("Read last {tail_lines} lines of {id}")),
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
