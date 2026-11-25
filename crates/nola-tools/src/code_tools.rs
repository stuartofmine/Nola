// SPDX-License-Identifier: Apache-2.0
//! High-productivity code & workspace tools:
//! - `multi_edit_file`: atomic multi-hunk replacements on one file
//! - `move_file` & `append_file` & `read_many_files`
//! - `notebook`: read/edit Jupyter `.ipynb` cells cleanly
//! - `repo_map`: fast multi-language symbol outline & symbol search
//! - `workspace_checkpoint`: create/list/restore file snapshots for safe rollback

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use ignore::WalkBuilder;
use nola_types::{
    sha256_hex, CapabilityDomain, ContentVersion, ResourceEffect, ResourceOperation,
    ToolExecutionResult, ToolSchema, ToolSpec, ToolTier, VersionTransition,
};
use regex::Regex;
use serde_json::{json, Value};

use crate::file_pipeline::{
    apply_text_edit, clip_read_line, looks_binary, path_within_workspace, resolve_path,
    FileMutationPipeline,
};
use crate::{ToolContext, ToolHandler};

// --- Workspace Checkpoint Manager ---

#[derive(Default)]
pub struct CheckpointManager {
    snapshots: Mutex<BTreeMap<String, BTreeMap<PathBuf, Vec<u8>>>>,
    next_id: Mutex<u64>,
}

impl CheckpointManager {
    pub fn create(&self, root: &std::path::Path, label: &str) -> Result<(String, usize), String> {
        let mut files = BTreeMap::new();
        let walker = WalkBuilder::new(root)
            .hidden(false)
            .filter_entry(|e| {
                !matches!(
                    e.file_name().to_string_lossy().as_ref(),
                    ".git" | "target" | "node_modules" | "__pycache__" | ".venv"
                )
            })
            .build();
        for dent in walker.flatten() {
            if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            if let Ok(meta) = dent.metadata() {
                if meta.len() > 2 * 1024 * 1024 {
                    continue;
                }
            }
            if let Ok(bytes) = fs::read(dent.path()) {
                let rel = dent
                    .path()
                    .strip_prefix(root)
                    .unwrap_or(dent.path())
                    .to_path_buf();
                files.insert(rel, bytes);
                if files.len() >= 1_000 {
                    break;
                }
            }
        }
        let count = files.len();
        let id = {
            let mut n = self.next_id.lock().unwrap();
            *n += 1;
            let slug: String = label
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .take(24)
                .collect();
            if slug.is_empty() {
                format!("cp_{}", *n)
            } else {
                format!("cp_{}_{slug}", *n)
            }
        };
        self.snapshots.lock().unwrap().insert(id.clone(), files);
        Ok((id, count))
    }

    pub fn list(&self) -> Vec<Value> {
        self.snapshots
            .lock()
            .unwrap()
            .iter()
            .map(|(id, map)| json!({"id": id, "files": map.len()}))
            .collect()
    }

    pub fn restore(&self, ctx: &ToolContext, id: &str) -> Result<Vec<ResourceEffect>, String> {
        let snap = self
            .snapshots
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| format!("unknown checkpoint '{id}'"))?;
        let mut effects = Vec::new();
        // Delete files created after the checkpoint (with same ignore filters)
        let walker = WalkBuilder::new(&ctx.workspace_root)
            .hidden(false)
            .filter_entry(|e| {
                !matches!(
                    e.file_name().to_string_lossy().as_ref(),
                    ".git" | "target" | "node_modules" | "__pycache__" | ".venv"
                )
            })
            .build();
        for dent in walker.flatten() {
            if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let rel = dent
                .path()
                .strip_prefix(&ctx.workspace_root)
                .unwrap_or(dent.path())
                .to_path_buf();
            if !snap.contains_key(&rel) && path_within_workspace(&ctx.workspace_root, dent.path()) {
                if let Ok(eff) = FileMutationPipeline::delete_file(ctx, dent.path()) {
                    effects.push(eff);
                }
            }
        }
        // Restore every snapshotted file
        for (rel, bytes) in snap {
            let target = ctx.workspace_root.join(&rel);
            let eff = FileMutationPipeline::write_bytes(
                ctx,
                &target,
                &bytes,
                ResourceOperation::Edit,
            )?;
            if eff.transition.changed() {
                effects.push(eff);
            }
        }
        Ok(effects)
    }
}

// --- 1. multi_edit_file ---

pub struct MultiEditFileTool;

impl ToolHandler for MultiEditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "multi_edit_file".into(),
                description: "Atomically apply an ordered list of old_text -> new_text replacements to one file.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "old_text": { "type": "string" },
                                    "new_text": { "type": "string" }
                                },
                                "required": ["old_text", "new_text"]
                            }
                        }
                    },
                    "required": ["path", "edits"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::FilesWrite,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(raw_path) = args.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'path'", None);
        };
        let Some(edits) = args.get("edits").and_then(|v| v.as_array()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'edits' array", None);
        };
        let path = resolve_path(&ctx.workspace_root, raw_path);
        let mut content = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => return ToolExecutionResult::err_structured("read_error", e.to_string(), None),
        };
        for (idx, edit) in edits.iter().enumerate() {
            let Some(old_text) = edit.get("old_text").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
                return ToolExecutionResult::err_structured(
                    "missing_param",
                    format!("edit[{idx}] missing non-empty 'old_text'"),
                    None,
                );
            };
            let Some(new_text) = edit.get("new_text").and_then(|v| v.as_str()) else {
                return ToolExecutionResult::err_structured(
                    "missing_param",
                    format!("edit[{idx}] missing 'new_text'"),
                    None,
                );
            };
            match apply_text_edit(&content, old_text, new_text, false, Some(1)) {
                Ok((updated, _, _, _)) => content = updated,
                Err(mut err_res) => {
                    let mut val = err_res.payload.to_model_value();
                    if let Some(obj) = val.as_object_mut() {
                        obj.insert("failed_edit_index".into(), json!(idx));
                        obj.insert(
                            "rollback".into(),
                            Value::String("all edits rolled back atomically".into()),
                        );
                    }
                    err_res.payload = nola_types::ToolModelPayload::structured(
                        val,
                        format!("edit[{idx}] failed; rolled back atomically"),
                    );
                    return err_res;
                }
            }
        }
        match FileMutationPipeline::write_bytes(ctx, &path, content.as_bytes(), ResourceOperation::Edit) {
            Ok(eff) => ToolExecutionResult::ok_structured(
                json!({"path": path.display().to_string(), "applied_edits": edits.len(), "transition": eff.transition}),
                format!("Applied {} atomic edits to {}", edits.len(), path.display()),
            )
            .with_effects(vec![eff]),
            Err(e) => ToolExecutionResult::err_structured("write_error", e, None),
        }
    }
}

// --- 2. move_file ---

pub struct MoveFileTool;

impl ToolHandler for MoveFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "move_file".into(),
                description: "Move or rename a file within the workspace and record SHA-256 version receipts.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "source": { "type": "string" },
                        "destination": { "type": "string" },
                        "overwrite": { "type": "boolean", "description": "Allow replacing an existing destination file (default false)" }
                    },
                    "required": ["source", "destination"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::FilesWrite,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(src_raw) = args.get("source").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'source'", None);
        };
        let Some(dst_raw) = args.get("destination").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'destination'", None);
        };
        let overwrite = args
            .get("overwrite")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let src = resolve_path(&ctx.workspace_root, src_raw);
        let dst = resolve_path(&ctx.workspace_root, dst_raw);
        if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
            && (!path_within_workspace(&ctx.workspace_root, &src)
                || !path_within_workspace(&ctx.workspace_root, &dst))
        {
            return ToolExecutionResult::err_structured(
                "workspace_escape",
                "move_file source or destination escapes workspace boundary",
                None,
            );
        }
        if src == dst {
            return ToolExecutionResult::ok_structured(
                json!({"source": src.display().to_string(), "destination": dst.display().to_string(), "unchanged": true}),
                "Source and destination are identical",
            );
        }
        if !overwrite && dst.symlink_metadata().is_ok() {
            return ToolExecutionResult::err_structured(
                "destination_exists",
                format!(
                    "destination '{}' already exists; set overwrite=true to replace it",
                    dst.display()
                ),
                Some(json!({
                    "source": src.display().to_string(),
                    "destination": dst.display().to_string()
                })),
            );
        }
        let bytes = match fs::read(&src) {
            Ok(b) => b,
            Err(e) => return ToolExecutionResult::err_structured("read_error", e.to_string(), None),
        };
        let dst_before = fs::read(&dst)
            .ok()
            .map(|b| ContentVersion::Sha256(sha256_hex(&b)));
        let sha_hex = sha256_hex(&bytes);
        let sha = ContentVersion::Sha256(sha_hex.clone());
        if let Some(parent) = dst.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                return ToolExecutionResult::err_structured("mkdir_error", e.to_string(), None);
            }
        }
        // Support cross-device moves (EXDEV) cleanly via atomic write + delete fallback!
        if let Err(rename_err) = fs::rename(&src, &dst) {
            if let Err(e) = FileMutationPipeline::write_bytes(ctx, &dst, &bytes, ResourceOperation::Move) {
                return ToolExecutionResult::err_structured(
                    "move_error",
                    format!("rename failed ({rename_err}) and copy fallback failed: {e}"),
                    None,
                );
            }
            if let Err(e) = fs::remove_file(&src) {
                return ToolExecutionResult::err_structured("move_cleanup_error", e.to_string(), None);
            }
        }
        ctx.file_state.record(&src, String::new());
        ctx.file_state.record(&dst, sha_hex);
        let dst_transition = match dst_before {
            Some(before) => VersionTransition::Modified {
                before,
                after: sha.clone(),
            },
            None => VersionTransition::Created { after: sha.clone() },
        };
        let effects = vec![
            ResourceEffect {
                uri: src.display().to_string(),
                operation: ResourceOperation::Delete,
                transition: VersionTransition::Deleted { before: sha },
            },
            ResourceEffect {
                uri: dst.display().to_string(),
                operation: ResourceOperation::Move,
                transition: dst_transition,
            },
        ];
        ToolExecutionResult::ok_structured(
            json!({"source": src.display().to_string(), "destination": dst.display().to_string()}),
            format!("Moved {} -> {}", src.display(), dst.display()),
        )
        .with_effects(effects)
    }
}

// --- 3. read_many_files ---

pub struct ReadManyFilesTool;

impl ToolHandler for ReadManyFilesTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "read_many_files".into(),
                description: "Read multiple text files in a single tool call (up to 20 files, bounded per-file lines).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "paths": { "type": "array", "items": { "type": "string" } },
                        "max_lines_per_file": { "type": "integer", "description": "Default 200" }
                    },
                    "required": ["paths"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::FilesRead,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(paths) = args.get("paths").and_then(|v| v.as_array()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'paths'", None);
        };
        let max_lines = args
            .get("max_lines_per_file")
            .and_then(|v| v.as_u64())
            .unwrap_or(200)
            .clamp(10, 1_000) as usize;
        let mut results = Vec::new();
        let mut total_budget_bytes = 48 * 1024usize;
        for p in paths.iter().filter_map(|v| v.as_str()).take(20) {
            let full = resolve_path(&ctx.workspace_root, p);
            if let Ok(meta) = fs::metadata(&full) {
                if meta.len() > 4 * 1024 * 1024 {
                    results.push(json!({"path": p, "error": "file exceeds 4 MiB read_many_files limit"}));
                    continue;
                }
            }
            match fs::read(&full) {
                Ok(bytes) => {
                    if looks_binary(&bytes) {
                        results.push(json!({"path": p, "error": "binary file skipped"}));
                        continue;
                    }
                    let sha = sha256_hex(&bytes);
                    ctx.file_state.record(&full, sha.clone());
                    let text = String::from_utf8_lossy(&bytes);
                    let lines: Vec<&str> = text.lines().collect();
                    let mut rendered = Vec::new();
                    let mut budget_hit = false;
                    for (i, l) in lines.iter().take(max_lines).enumerate() {
                        let formatted = format!("{}: {}", i + 1, clip_read_line(l));
                        if formatted.len() + 1 > total_budget_bytes {
                            budget_hit = true;
                            break;
                        }
                        total_budget_bytes = total_budget_bytes.saturating_sub(formatted.len() + 1);
                        rendered.push(formatted);
                    }
                    results.push(json!({
                        "path": full.display().to_string(),
                        "sha256": sha,
                        "total_lines": lines.len(),
                        "truncated": budget_hit || lines.len() > max_lines,
                        "content": rendered.join("\n")
                    }));
                    if budget_hit {
                        break;
                    }
                }
                Err(e) => results.push(json!({"path": p, "error": e.to_string()})),
            }
        }
        let count = results.len();
        ToolExecutionResult::ok_structured(
            json!({"count": count, "files": results}),
            format!("Read {count} file(s)"),
        )
    }
}

// --- 4. notebook (Jupyter .ipynb read/edit) ---

pub struct NotebookTool;

impl ToolHandler for NotebookTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "notebook".into(),
                description: "Read or edit Jupyter .ipynb notebook cells cleanly (action: read|replace_cell|insert_cell|delete_cell).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "action": { "type": "string", "enum": ["read", "replace_cell", "insert_cell", "delete_cell"] },
                        "cell_index": { "type": "integer" },
                        "cell_type": { "type": "string", "enum": ["code", "markdown"] },
                        "source": { "type": "string" }
                    },
                    "required": ["path", "action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::FilesWrite,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(raw_path) = args.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'path'", None);
        };
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("read");
        let path = resolve_path(&ctx.workspace_root, raw_path);
        let raw = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => return ToolExecutionResult::err_structured("read_error", e.to_string(), None),
        };
        let mut nb: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => return ToolExecutionResult::err_structured("invalid_ipynb", e.to_string(), None),
        };
        let Some(cells) = nb.get_mut("cells").and_then(|v| v.as_array_mut()) else {
            return ToolExecutionResult::err_structured("invalid_ipynb", "notebook missing 'cells' array", None);
        };
        match action {
            "read" => {
                let rendered: Vec<Value> = cells
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let src = match c.get("source") {
                            Some(Value::Array(lines)) => lines
                                .iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(""),
                            Some(Value::String(s)) => s.clone(),
                            _ => String::new(),
                        };
                        json!({
                            "index": i,
                            "cell_type": c.get("cell_type").and_then(|v| v.as_str()).unwrap_or("code"),
                            "source": src
                        })
                    })
                    .collect();
                let count = rendered.len();
                ToolExecutionResult::ok_structured(
                    json!({"path": path.display().to_string(), "count": count, "cells": rendered}),
                    format!("Read notebook {} ({count} cells)", path.display()),
                )
            }
            "replace_cell" => {
                let Some(idx) = args.get("cell_index").and_then(|v| v.as_u64()).map(|n| n as usize) else {
                    return ToolExecutionResult::err_structured("missing_param", "replace_cell requires non-negative 'cell_index'", None);
                };
                let Some(source) = args.get("source").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "replace_cell requires 'source'", None);
                };
                let Some(cell) = cells.get_mut(idx) else {
                    return ToolExecutionResult::err_structured("out_of_range", format!("cell {idx} out of range"), None);
                };
                let lines: Vec<Value> = source.split_inclusive('\n').map(|s| Value::String(s.into())).collect();
                cell["source"] = Value::Array(lines);
                write_notebook(ctx, &path, &nb, format!("Updated cell {idx} in {}", path.display()))
            }
            "insert_cell" => {
                let idx = args
                    .get("cell_index")
                    .and_then(|v| v.as_u64())
                    .map(|n| (n as usize).min(cells.len()))
                    .unwrap_or(cells.len());
                let ctype = args.get("cell_type").and_then(|v| v.as_str()).unwrap_or("code");
                let source = args.get("source").and_then(|v| v.as_str()).unwrap_or("");
                let lines: Vec<Value> = source.split_inclusive('\n').map(|s| Value::String(s.into())).collect();
                let cell = if ctype == "markdown" {
                    json!({
                        "cell_type": "markdown",
                        "metadata": {},
                        "source": lines
                    })
                } else {
                    json!({
                        "cell_type": "code",
                        "execution_count": null,
                        "metadata": {},
                        "source": lines,
                        "outputs": []
                    })
                };
                cells.insert(idx, cell);
                write_notebook(ctx, &path, &nb, format!("Inserted {ctype} cell at {idx}"))
            }
            "delete_cell" => {
                let Some(idx) = args.get("cell_index").and_then(|v| v.as_u64()).map(|n| n as usize) else {
                    return ToolExecutionResult::err_structured("missing_param", "delete_cell requires non-negative 'cell_index'", None);
                };
                if idx >= cells.len() {
                    return ToolExecutionResult::err_structured("out_of_range", format!("cell {idx} out of range"), None);
                }
                cells.remove(idx);
                write_notebook(ctx, &path, &nb, format!("Deleted cell {idx}"))
            }
            other => ToolExecutionResult::err_structured("invalid_action", format!("unsupported notebook action '{other}'"), None),
        }
    }
}

fn write_notebook(ctx: &ToolContext, path: &std::path::Path, nb: &Value, summary: String) -> ToolExecutionResult {
    let serialized = match serde_json::to_string_pretty(nb) {
        Ok(s) => s,
        Err(e) => return ToolExecutionResult::err_structured("serialize_error", e.to_string(), None),
    };
    match FileMutationPipeline::write_bytes(ctx, path, serialized.as_bytes(), ResourceOperation::Edit) {
        Ok(eff) => ToolExecutionResult::ok_structured(
            json!({"path": path.display().to_string(), "transition": eff.transition}),
            summary,
        )
        .with_effects(vec![eff]),
        Err(e) => ToolExecutionResult::err_structured("write_error", e, None),
    }
}

// --- 5. repo_map (Multi-language fast symbol outline & symbol search) ---

pub struct RepoMapTool;

impl ToolHandler for RepoMapTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "repo_map".into(),
                description: "Extract a structured symbol map (functions, structs, classes, traits, interfaces) across the workspace or filter by symbol query.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Optional subdirectory" },
                        "query": { "type": "string", "description": "Optional substring filter on symbol name" },
                        "max_symbols": { "type": "integer", "description": "Default 200" }
                    }
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::FilesRead,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let base = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(&ctx.workspace_root, p))
            .unwrap_or_else(|| ctx.workspace_root.clone());
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let max_symbols = args
            .get("max_symbols")
            .and_then(|v| v.as_u64())
            .unwrap_or(200)
            .clamp(1, 1000) as usize;

        static SYM_RE: OnceLock<Regex> = OnceLock::new();
        let sym_re = SYM_RE.get_or_init(|| {
            Regex::new(
                r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(fn|struct|enum|trait|impl|class|def|interface|type|func|export\s+(?:async\s+)?function)\s+([A-Za-z_][A-Za-z0-9_]*)",
            )
            .unwrap()
        });

        let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut total_count = 0usize;
        let mut truncated = false;
        let walker = WalkBuilder::new(&base)
            .hidden(false)
            .filter_entry(|e| {
                e.depth() == 0
                    || !matches!(
                        e.file_name().to_string_lossy().as_ref(),
                        ".git" | "target" | "node_modules" | "__pycache__" | ".venv"
                    )
            })
            .build();
        'outer: for dent in walker.flatten() {
            if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            if dent.metadata().map(|m| m.len() > 1024 * 1024).unwrap_or(true) {
                continue;
            }
            let ext = dent
                .path()
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if !matches!(
                ext,
                "rs" | "py" | "ts" | "tsx" | "js" | "jsx" | "go" | "swift" | "kt" | "java" | "c" | "cpp" | "h"
            ) {
                continue;
            }
            let Ok(text) = fs::read_to_string(dent.path()) else {
                continue;
            };
            let rel = dent
                .path()
                .strip_prefix(&ctx.workspace_root)
                .unwrap_or(dent.path())
                .display()
                .to_string();
            for (i, line) in text.lines().enumerate() {
                if let Some(caps) = sym_re.captures(line) {
                    let kind = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let name = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                    if query.is_empty() || name.to_ascii_lowercase().contains(&query) {
                        grouped
                            .entry(rel.clone())
                            .or_default()
                            .push(format!("L{}: {kind} {name}", i + 1));
                        total_count += 1;
                        if total_count >= max_symbols {
                            truncated = true;
                            break 'outer;
                        }
                    }
                }
            }
        }
        ToolExecutionResult::ok_structured(
            json!({"count": total_count, "truncated": truncated, "files": grouped}),
            format!("Mapped {total_count} symbol(s) across {} file(s)", grouped.len()),
        )
    }
}

// --- 6. workspace_checkpoint ---

pub struct WorkspaceCheckpointTool;

impl ToolHandler for WorkspaceCheckpointTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "workspace_checkpoint".into(),
                description: "Create, list, or restore workspace file snapshots for safe rollback (action: create|list|restore).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "restore"] },
                        "label": { "type": "string" },
                        "checkpoint_id": { "type": "string" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::FilesWrite,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("list") {
            "create" => {
                let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("snap");
                match ctx.checkpoints.create(&ctx.workspace_root, label) {
                    Ok((id, files)) => ToolExecutionResult::ok_structured(
                        json!({"checkpoint_id": id, "files_captured": files}),
                        format!("Created checkpoint {id} ({files} files)"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("checkpoint_error", e, None),
                }
            }
            "list" => {
                let items = ctx.checkpoints.list();
                let count = items.len();
                ToolExecutionResult::ok_structured(
                    json!({"count": count, "checkpoints": items}),
                    format!("Listed {count} checkpoint(s)"),
                )
            }
            "restore" => {
                let Some(id) = args.get("checkpoint_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'checkpoint_id'", None);
                };
                match ctx.checkpoints.restore(ctx, id) {
                    Ok(effects) => {
                        let count = effects.len();
                        ToolExecutionResult::ok_structured(
                            json!({"checkpoint_id": id, "restored_files": count}),
                            format!("Restored checkpoint {id} ({count} changed file(s))"),
                        )
                        .with_effects(effects)
                    }
                    Err(e) => ToolExecutionResult::err_structured("checkpoint_error", e, None),
                }
            }
            other => ToolExecutionResult::err_structured("invalid_action", format!("unsupported checkpoint action '{other}'"), None),
        }
    }
}
