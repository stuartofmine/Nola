// SPDX-License-Identifier: Apache-2.0
//! Externalized Tool Output Store (`OutputStore`) + `tool_output` pagination/search/slice tool.
//! Automatically spills oversized tool payloads (> 24 KiB) to session-isolated disk directories
//! (`<root>/<sha256(session_id)[..16]>/`) via atomic temp+rename writes, while giving the model
//! a compact head+tail preview plus `output_id` for on-demand line/byte-offset/regex inspection.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
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

static OUTPUT_TMP_SEQ: AtomicU64 = AtomicU64::new(1);

fn clip_line(line: &str) -> String {
    let char_count = line.chars().count();
    if char_count <= MAX_LINE_CHARS {
        line.to_string()
    } else {
        let prefix: String = line.chars().take(MAX_LINE_CHARS).collect();
        format!("{prefix}... [line truncated: {char_count} chars]")
    }
}

/// Slice a UTF-8 string strictly on character boundaries starting at or after `offset`
/// and returning up to `max_bytes` (guaranteeing forward progress of at least 1 char when `start < len`).
pub(crate) fn utf8_byte_slice(text: &str, offset: usize, max_bytes: usize) -> (&str, usize, usize) {
    let total = text.len();
    if offset >= total {
        return ("", total, total);
    }
    let mut start = offset;
    while start < total && !text.is_char_boundary(start) {
        start += 1;
    }
    if start >= total {
        return ("", total, total);
    }
    let cap = max_bytes.max(1);
    let mut end = start.saturating_add(cap).min(total);
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == start {
        // Single multi-byte char exceeds `cap`: advance across that single char to ensure progress.
        end = (start + 1).min(total);
        while end < total && !text.is_char_boundary(end) {
            end += 1;
        }
    }
    (&text[start..end], start, end)
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

fn session_scope_key(session_id: &str) -> String {
    let sid = if session_id.trim().is_empty() {
        "default"
    } else {
        session_id
    };
    sha256_hex(sid.as_bytes())[..16].to_string()
}

/// Content-addressed, session-isolated spill store for large tool outputs.
pub struct OutputStore {
    root_dir: PathBuf,
    default_session_id: String,
    entries: Mutex<BTreeMap<(String, String), StoredOutputMeta>>,
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
            default_session_id: "default".to_string(),
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Return a session-scoped view rooted at the same directory, isolating all reads & writes
    /// under `<root>/<sha256(session_id)[..16]>/`.
    pub fn store_for_session(&self, session_id: &str) -> Self {
        Self {
            root_dir: self.root_dir.clone(),
            default_session_id: session_id.to_string(),
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    fn session_dir(&self, session_id: &str) -> (String, PathBuf) {
        let scope = session_scope_key(session_id);
        let dir = self.root_dir.join(&scope);
        (scope, dir)
    }

    pub fn store_text(&self, tool_name: &str, text: &str) -> Result<StoredOutputMeta, String> {
        self.save_scoped(&self.default_session_id, tool_name, text)
    }

    pub fn save_scoped(
        &self,
        session_id: &str,
        tool_name: &str,
        text: &str,
    ) -> Result<StoredOutputMeta, String> {
        let (scope, dir) = self.session_dir(session_id);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("failed to create output store dir: {e}"))?;
        let hash = sha256_hex(text.as_bytes());
        let output_id = format!("out_{}", &hash[..16]);
        let path = dir.join(format!("{output_id}.txt"));
        // Atomic write via sibling temp file + rename
        let seq = OUTPUT_TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{output_id}.tmp.{}.{seq}", std::process::id()));
        fs::write(&tmp, text.as_bytes())
            .map_err(|e| format!("failed to write temp spilled output: {e}"))?;
        if let Err(e) = fs::rename(&tmp, &path) {
            let _ = fs::remove_file(&tmp);
            return Err(format!("failed to commit spilled output: {e}"));
        }
        let meta = StoredOutputMeta {
            output_id: output_id.clone(),
            tool_name: tool_name.to_string(),
            total_bytes: text.len(),
            total_lines: text.lines().count(),
            path,
        };
        self.entries
            .lock()
            .unwrap()
            .insert((scope, output_id), meta.clone());
        Ok(meta)
    }

    /// Spill oversized `ToolModelPayload` if serialized bytes exceed `threshold_bytes`.
    pub fn maybe_spill_payload(
        &self,
        tool_name: &str,
        payload: &ToolModelPayload,
        threshold_bytes: usize,
    ) -> Option<(ToolModelPayload, StoredOutputMeta)> {
        self.maybe_spill_payload_scoped(&self.default_session_id, tool_name, payload, threshold_bytes)
    }

    pub fn maybe_spill_payload_scoped(
        &self,
        session_id: &str,
        tool_name: &str,
        payload: &ToolModelPayload,
        threshold_bytes: usize,
    ) -> Option<(ToolModelPayload, StoredOutputMeta)> {
        let serialized_len = payload.to_model_string().len();
        if serialized_len <= threshold_bytes {
            return None;
        }
        let (full, preserved) = render_spillable_text(payload);
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
        envelope.insert("tool".into(), Value::String(tool_name.to_string()));
        envelope.insert("total_bytes".into(), json!(full.len()));
        envelope.insert("total_lines".into(), json!(full.lines().count()));
        for (k, v) in preserved {
            envelope.entry(k).or_insert(v);
        }
        envelope.insert("head_preview".into(), Value::String(head));
        if !tail.is_empty() {
            envelope.insert("tail_preview".into(), Value::String(tail));
        }

        match self.save_scoped(session_id, tool_name, &full) {
            Ok(meta) => {
                envelope.insert("spilled".into(), Value::Bool(true));
                envelope.insert("full_output_available".into(), Value::Bool(true));
                envelope.insert("output_id".into(), Value::String(meta.output_id.clone()));
                envelope.insert(
                    "hint".into(),
                    Value::String(
                        "Use tool_output (action=read|tail|search|slice) with this output_id to inspect full content."
                            .into(),
                    ),
                );
                let compact = ToolModelPayload::structured(
                    Value::Object(envelope),
                    format!(
                        "{summary} [spilled {} bytes -> {}]",
                        meta.total_bytes, meta.output_id
                    ),
                );
                Some((compact, meta))
            }
            Err(err) => {
                envelope.insert("spilled".into(), Value::Bool(false));
                envelope.insert("full_output_available".into(), Value::Bool(false));
                envelope.insert("spill_error".into(), Value::String(err.clone()));
                envelope.insert(
                    "hint".into(),
                    Value::String(
                        "Output exceeded inline threshold and persisting the full spill file failed; only head/tail preview is available."
                            .into(),
                    ),
                );
                let fallback_meta = StoredOutputMeta {
                    output_id: String::new(),
                    tool_name: tool_name.to_string(),
                    total_bytes: full.len(),
                    total_lines: full.lines().count(),
                    path: PathBuf::new(),
                };
                let compact = ToolModelPayload::structured(
                    Value::Object(envelope),
                    format!(
                        "{summary} [truncated {} bytes; full output unavailable: {err}]",
                        full.len()
                    ),
                );
                Some((compact, fallback_meta))
            }
        }
    }

    pub fn list(&self) -> Vec<Value> {
        self.list_scoped(&self.default_session_id)
    }

    pub fn list_scoped(&self, session_id: &str) -> Vec<Value> {
        let scope = session_scope_key(session_id);
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|((s, _), _)| s == &scope)
            .map(|(_, m)| {
                json!({
                    "output_id": m.output_id,
                    "tool_name": m.tool_name,
                    "total_bytes": m.total_bytes,
                    "total_lines": m.total_lines
                })
            })
            .collect()
    }

    pub fn read_range(
        &self,
        output_id: &str,
        start_line: usize,
        end_line: usize,
    ) -> Result<Value, String> {
        self.read_range_scoped(&self.default_session_id, output_id, start_line, end_line)
    }

    pub fn read_range_scoped(
        &self,
        session_id: &str,
        output_id: &str,
        start_line: usize,
        end_line: usize,
    ) -> Result<Value, String> {
        let path = self.resolve_output_path_scoped(session_id, output_id)?;
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
                if !rendered.is_empty() && bytes_used + formatted.len() + 1 > MAX_READ_RESPONSE_BYTES
                {
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

    pub fn read_bytes_scoped(
        &self,
        session_id: &str,
        output_id: &str,
        offset: usize,
        max_bytes: usize,
    ) -> Result<Value, String> {
        let path = self.resolve_output_path_scoped(session_id, output_id)?;
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read stored output: {e}"))?;
        let size_bytes = text.len();
        let cap = max_bytes.clamp(1, MAX_READ_RESPONSE_BYTES);
        let (slice, actual_offset, next_offset) = utf8_byte_slice(&text, offset, cap);
        Ok(json!({
            "output_id": output_id,
            "content": slice,
            "offset": actual_offset,
            "bytes_returned": slice.len(),
            "next_offset": next_offset,
            "size_bytes": size_bytes,
            "has_more": next_offset < size_bytes
        }))
    }

    pub fn read_tail(&self, output_id: &str, tail_lines: usize) -> Result<Value, String> {
        self.read_tail_scoped(&self.default_session_id, output_id, tail_lines)
    }

    pub fn read_tail_scoped(
        &self,
        session_id: &str,
        output_id: &str,
        tail_lines: usize,
    ) -> Result<Value, String> {
        let path = self.resolve_output_path_scoped(session_id, output_id)?;
        let text = fs::read_to_string(&path)
            .map_err(|e| format!("failed to read stored output: {e}"))?;
        let total = text.lines().count();
        let count = tail_lines.clamp(1, 400);
        let start = total.saturating_sub(count).saturating_add(1).max(1);
        self.read_range_scoped(session_id, output_id, start, total.max(1))
    }

    pub fn search(
        &self,
        output_id: &str,
        pattern: &str,
        max_matches: usize,
    ) -> Result<Value, String> {
        self.search_scoped(&self.default_session_id, output_id, pattern, max_matches)
    }

    pub fn search_scoped(
        &self,
        session_id: &str,
        output_id: &str,
        pattern: &str,
        max_matches: usize,
    ) -> Result<Value, String> {
        let path = self.resolve_output_path_scoped(session_id, output_id)?;
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

    /// Resolve `output_id` strictly within `session_id`'s directory (never crosses sessions!).
    fn resolve_output_path_scoped(
        &self,
        session_id: &str,
        output_id: &str,
    ) -> Result<PathBuf, String> {
        let (scope, dir) = self.session_dir(session_id);
        if let Some(meta) = self
            .entries
            .lock()
            .unwrap()
            .get(&(scope, output_id.to_string()))
        {
            return Ok(meta.path.clone());
        }
        if output_id.is_empty()
            || !output_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!("invalid output_id '{output_id}'"));
        }
        let disk_path = dir.join(format!("{output_id}.txt"));
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
                description: "Inspect spilled oversized tool outputs (action: list|read|tail|search|slice).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["list", "read", "tail", "search", "slice"] },
                        "output_id": { "type": "string" },
                        "start_line": { "type": "integer", "description": "1-based start line for read" },
                        "end_line": { "type": "integer", "description": "1-based inclusive end line for read" },
                        "tail_lines": { "type": "integer", "description": "Number of trailing lines for action=tail (default 100)" },
                        "pattern": { "type": "string", "description": "Regex pattern for search" },
                        "offset": { "type": "integer", "description": "0-based UTF-8 byte offset for action=slice" },
                        "max_bytes": { "type": "integer", "description": "Maximum bytes to return for action=slice (default 8192)" }
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
                let items = ctx.output_store.list_scoped(&ctx.session_id);
                let count = items.len();
                ToolExecutionResult::ok_structured(
                    json!({"count": count, "outputs": items}),
                    format!("Listed {count} stored tool output(s)"),
                )
            }
            "read" => {
                let Some(id) = args.get("output_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "missing 'output_id'",
                        None,
                    );
                };
                let start = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let end = args
                    .get("end_line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(start.saturating_add(199) as u64) as usize;
                match ctx
                    .output_store
                    .read_range_scoped(&ctx.session_id, id, start, end)
                {
                    Ok(v) => ToolExecutionResult::ok_structured(
                        v,
                        format!("Read lines {start}-{end} of {id}"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("output_read_error", e, None),
                }
            }
            "slice" => {
                let Some(id) = args.get("output_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "missing 'output_id'",
                        None,
                    );
                };
                let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let max_bytes = args
                    .get("max_bytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(8_192) as usize;
                match ctx
                    .output_store
                    .read_bytes_scoped(&ctx.session_id, id, offset, max_bytes)
                {
                    Ok(v) => {
                        let returned = v["bytes_returned"].as_u64().unwrap_or(0);
                        ToolExecutionResult::ok_structured(
                            v,
                            format!("Sliced {returned} bytes from {id} at offset {offset}"),
                        )
                    }
                    Err(e) => ToolExecutionResult::err_structured("output_read_error", e, None),
                }
            }
            "tail" => {
                let Some(id) = args.get("output_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "missing 'output_id'",
                        None,
                    );
                };
                let tail_lines = args
                    .get("tail_lines")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(100) as usize;
                match ctx
                    .output_store
                    .read_tail_scoped(&ctx.session_id, id, tail_lines)
                {
                    Ok(v) => ToolExecutionResult::ok_structured(
                        v,
                        format!("Read last {tail_lines} lines of {id}"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("output_read_error", e, None),
                }
            }
            "search" => {
                let Some(id) = args.get("output_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "missing 'output_id'",
                        None,
                    );
                };
                let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "missing 'pattern'",
                        None,
                    );
                };
                match ctx
                    .output_store
                    .search_scoped(&ctx.session_id, id, pattern, 100)
                {
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
