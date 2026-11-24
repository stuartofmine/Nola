// SPDX-License-Identifier: Apache-2.0
//! Unified `FileMutationPipeline` + `FileAccessState` + all file tools:
//! `read_file`, `write_file`, `edit_file`, `apply_patch`, `delete_file`.
//!
//! Compile-time invariants:
//! - Every mutation computes `before` and `after` `ContentVersion::Sha256` and returns an
//!   exhaustive `VersionTransition` (`Created`, `Modified`, `Deleted`, `Unchanged`).
//! - `path_within_workspace` canonicalizes nearest existing ancestor to block symlink escapes.
//! - `edit_file` supports tolerant indentation matching + optional `expected_replacements`.
//! - `apply_patch` defaults `dry_run = false` and applies unified diffs in pure Rust (zero `git` dep!).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);
const MAX_READ_TEXT_BYTES: u64 = 16 * 1024 * 1024;

use nola_types::{
    sha256_hex, CapabilityDomain, ContentVersion, ResourceEffect, ResourceOperation,
    ToolExecutionResult, ToolSchema, ToolSpec, ToolTier, VersionTransition,
};
use serde_json::{json, Value};

use crate::binary_docs::try_read_binary_document;
use crate::{ToolContext, ToolHandler};

/// Tracks SHA-256 digests of files observed via `read_file` or written by tools.
#[derive(Default)]
pub struct FileAccessState {
    digests: Mutex<BTreeMap<PathBuf, String>>,
}

impl FileAccessState {
    pub fn record(&self, path: &Path, sha256: String) {
        if let Ok(mut g) = self.digests.lock() {
            g.insert(path.to_path_buf(), sha256);
        }
    }

    pub fn last_seen(&self, path: &Path) -> Option<String> {
        self.digests.lock().ok().and_then(|g| g.get(path).cloned())
    }
}

/// Resolve `raw_path` relative to `workspace_root` cleanly.
pub fn resolve_path(workspace_root: &Path, raw_path: &str) -> PathBuf {
    let p = Path::new(raw_path);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    };
    normalize_lexically(&joined)
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonicalize the nearest existing ancestor + remaining tail so symlink escape checks
/// work even for not-yet-created files OR paths containing `..` after a symlink!
pub fn canonicalize_nearest_ancestor(path: &Path) -> PathBuf {
    let normalized = normalize_lexically(path);
    let mut ancestor = normalized.as_path();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = fs::canonicalize(ancestor) {
            let mut full = real;
            for seg in tail.into_iter().rev() {
                full.push(seg);
            }
            return normalize_lexically(&full);
        }
        let Some(name) = ancestor.file_name() else {
            return normalized;
        };
        tail.push(name.to_os_string());
        let Some(parent) = ancestor.parent() else {
            return normalized;
        };
        ancestor = parent;
    }
}

/// Returns true iff `target` (after resolving symlinks on its nearest existing ancestor)
/// resides inside `workspace_root`.
pub fn path_within_workspace(workspace_root: &Path, target: &Path) -> bool {
    let root = fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let resolved = canonicalize_nearest_ancestor(target);
    resolved.starts_with(&root)
}

/// Single authoritative pipeline for every file write, edit, patch, or delete.
pub struct FileMutationPipeline;

impl FileMutationPipeline {
    pub fn write_bytes(
        ctx: &ToolContext,
        path: &Path,
        new_bytes: &[u8],
        op_hint: ResourceOperation,
    ) -> Result<ResourceEffect, String> {
        if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
            && !path_within_workspace(&ctx.workspace_root, path)
        {
            return Err(format!(
                "write target '{}' escapes workspace boundary",
                path.display()
            ));
        }
        let existing_perms = fs::metadata(path).ok().map(|m| m.permissions());
        let before = fs::read(path)
            .ok()
            .map(|b| ContentVersion::Sha256(sha256_hex(&b)));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("failed to create parent dir: {e}"))?;
        }
        // Thread-safe unique sibling temp file preserving executable bits!
        let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = PathBuf::from(format!(
            "{}.nola.tmp.{}.{seq}",
            path.display(),
            std::process::id()
        ));
        fs::write(&tmp, new_bytes).map_err(|e| format!("failed to write temp file: {e}"))?;
        if let Some(perms) = existing_perms {
            let _ = fs::set_permissions(&tmp, perms);
        }
        fs::rename(&tmp, path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("failed to commit file {}: {e}", path.display())
        })?;
        let after_sha = sha256_hex(new_bytes);
        ctx.file_state.record(path, after_sha.clone());
        let after = ContentVersion::Sha256(after_sha);

        let (operation, transition) = match before {
            None => (ResourceOperation::Create, VersionTransition::Created { after }),
            Some(b) if b == after => (op_hint, VersionTransition::Unchanged { version: after }),
            Some(b) => (
                if op_hint == ResourceOperation::Create {
                    ResourceOperation::Edit
                } else {
                    op_hint
                },
                VersionTransition::Modified { before: b, after },
            ),
        };
        Ok(ResourceEffect {
            uri: path.display().to_string(),
            operation,
            transition,
        })
    }

    pub fn delete_file(ctx: &ToolContext, path: &Path) -> Result<ResourceEffect, String> {
        if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
            && !path_within_workspace(&ctx.workspace_root, path)
        {
            return Err(format!(
                "delete target '{}' escapes workspace boundary",
                path.display()
            ));
        }
        let before_bytes = fs::read(path).map_err(|e| format!("cannot read file before delete: {e}"))?;
        let before = ContentVersion::Sha256(sha256_hex(&before_bytes));
        fs::remove_file(path).map_err(|e| format!("failed to delete file: {e}"))?;
        ctx.file_state.record(path, String::new());
        Ok(ResourceEffect {
            uri: path.display().to_string(),
            operation: ResourceOperation::Delete,
            transition: VersionTransition::Deleted { before },
        })
    }
}

// --- 1. read_file ---

pub struct ReadFileTool;

impl ToolHandler for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "read_file".into(),
                description: "Read a UTF-8 text file with 1-based line numbers (supports start_line/end_line) or inspect spreadsheets (.xlsx/.xls/.ods), PDFs (.pdf), audio, and images.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path relative to workspace or absolute" },
                        "start_line": { "type": "integer", "description": "1-based start line (default 1)" },
                        "end_line": { "type": "integer", "description": "1-based inclusive end line (default start_line + 399)" },
                        "page": { "type": "integer", "description": "Optional 1-based page number for PDFs" }
                    },
                    "required": ["path"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::FilesRead,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(raw_path) = args.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'path'", None);
        };
        let path = resolve_path(&ctx.workspace_root, raw_path);
        if !path.exists() {
            return ToolExecutionResult::err_structured(
                "not_found",
                format!("file does not exist: {}", path.display()),
                None,
            );
        }
        if path.is_dir() {
            return ToolExecutionResult::err_structured(
                "is_directory",
                format!("path is a directory, use list_dir instead: {}", path.display()),
                None,
            );
        }
        let page = args.get("page").and_then(|v| v.as_u64()).map(|n| n as usize);
        if let Some(binary_res) = try_read_binary_document(&path, page, 200) {
            return binary_res;
        }
        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() > MAX_READ_TEXT_BYTES {
                return ToolExecutionResult::err_structured(
                    "file_too_large",
                    format!(
                        "file '{}' is {} bytes (exceeds {} byte text limit)",
                        path.display(),
                        meta.len(),
                        MAX_READ_TEXT_BYTES
                    ),
                    None,
                );
            }
        }
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                return ToolExecutionResult::err_structured(
                    "io_error",
                    format!("failed to read {}: {e}", path.display()),
                    None,
                )
            }
        };
        let digest = sha256_hex(&bytes);
        ctx.file_state.record(&path, digest.clone());

        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        let total_lines = lines.len();
        let start = args
            .get("start_line")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).max(1))
            .unwrap_or(1);
        let clamped_start = if total_lines == 0 {
            1
        } else {
            start.min(total_lines)
        };
        let end = args
            .get("end_line")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).max(clamped_start))
            .unwrap_or_else(|| clamped_start.saturating_add(399))
            .min(total_lines.max(1));

        let numbered = if total_lines == 0 || start > total_lines {
            String::new()
        } else {
            lines
                .iter()
                .enumerate()
                .skip(clamped_start.saturating_sub(1))
                .take(end.saturating_sub(clamped_start).saturating_add(1))
                .map(|(idx, line)| format!("{}: {}", idx + 1, line))
                .collect::<Vec<_>>()
                .join("\n")
        };

        ToolExecutionResult::ok_structured(
            json!({
                "path": path.display().to_string(),
                "sha256": digest,
                "start_line": start,
                "end_line": end.min(total_lines),
                "total_lines": total_lines,
                "truncated": end < total_lines,
                "content": numbered
            }),
            format!(
                "Read {} (lines {}-{} of {})",
                path.display(),
                start,
                end.min(total_lines),
                total_lines
            ),
        )
    }
}

// --- 2. write_file ---

pub struct WriteFileTool;

impl ToolHandler for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "write_file".into(),
                description: "Create or overwrite a UTF-8 file atomically and return its SHA-256 version transition.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::FilesWrite,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(raw_path) = args.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'path'", None);
        };
        let Some(content) = args.get("content").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'content'", None);
        };
        let path = resolve_path(&ctx.workspace_root, raw_path);
        match FileMutationPipeline::write_bytes(ctx, &path, content.as_bytes(), ResourceOperation::Create) {
            Ok(effect) => {
                let bytes = content.len();
                let lines = content.lines().count();
                ToolExecutionResult::ok_structured(
                    json!({
                        "path": path.display().to_string(),
                        "bytes": bytes,
                        "lines": lines,
                        "transition": effect.transition
                    }),
                    format!("Wrote {} ({bytes} bytes, {lines} lines)", path.display()),
                )
                .with_effects(vec![effect])
            }
            Err(e) => ToolExecutionResult::err_structured("write_error", e, None),
        }
    }
}

// --- 3. edit_file ---

pub struct EditFileTool;

impl ToolHandler for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "edit_file".into(),
                description: "Replace exact text (or whitespace-tolerant block) in an existing file. Optional expected_replacements and replace_all.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "old_text": { "type": "string" },
                        "new_text": { "type": "string" },
                        "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)" },
                        "expected_replacements": { "type": "integer", "description": "Optional exact expected match count" }
                    },
                    "required": ["path", "old_text", "new_text"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::FilesWrite,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(raw_path) = args.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'path'", None);
        };
        let Some(old_text) = args.get("old_text").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'old_text'", None);
        };
        let Some(new_text) = args.get("new_text").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'new_text'", None);
        };
        if old_text.is_empty() {
            return ToolExecutionResult::err_structured("invalid_param", "old_text must be non-empty", None);
        }
        let replace_all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
        let expected = args
            .get("expected_replacements")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        let path = resolve_path(&ctx.workspace_root, raw_path);
        let original = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                return ToolExecutionResult::err_structured(
                    "read_error",
                    format!("cannot read {}: {e}", path.display()),
                    None,
                )
            }
        };

        let exact_count = original.matches(old_text).count();
        let (updated, replaced_count, match_mode) = if exact_count > 0 {
            if let Some(exp) = expected {
                if exact_count != exp {
                    return ToolExecutionResult::err_structured(
                        "replacement_count_mismatch",
                        format!("expected {exp} matches but found {exact_count}"),
                        Some(json!({"found": exact_count, "expected": exp})),
                    );
                }
            } else if !replace_all && exact_count > 1 {
                return ToolExecutionResult::err_structured(
                    "ambiguous_match",
                    format!(
                        "old_text matched {exact_count} locations; provide more context or set replace_all=true"
                    ),
                    Some(json!({"found": exact_count})),
                );
            }
            let out = if replace_all || expected.map(|e| e > 1).unwrap_or(false) {
                original.replace(old_text, new_text)
            } else {
                original.replacen(old_text, new_text, 1)
            };
            let count = if replace_all || expected.map(|e| e > 1).unwrap_or(false) {
                exact_count
            } else {
                1
            };
            (out, count, "exact")
        } else if expected.is_none_or(|e| e == 1) {
            if let Some(out) = tolerant_trimmed_replace(&original, old_text, new_text) {
                (out, 1, "trimmed_lines")
            } else {
                return ToolExecutionResult::err_structured(
                    "not_found",
                    "old_text not found in file (checked exact and whitespace-trimmed line match)",
                    None,
                );
            }
        } else {
            return ToolExecutionResult::err_structured(
                "not_found",
                "old_text not found in file",
                None,
            );
        };

        match FileMutationPipeline::write_bytes(ctx, &path, updated.as_bytes(), ResourceOperation::Edit) {
            Ok(effect) => ToolExecutionResult::ok_structured(
                json!({
                    "path": path.display().to_string(),
                    "replacements": replaced_count,
                    "match_mode": match_mode,
                    "transition": effect.transition
                }),
                format!("Edited {} ({replaced_count} replacement(s), {match_mode})", path.display()),
            )
            .with_effects(vec![effect]),
            Err(e) => ToolExecutionResult::err_structured("write_error", e, None),
        }
    }
}

fn tolerant_trimmed_replace(original: &str, old_text: &str, new_text: &str) -> Option<String> {
    let orig_lines: Vec<&str> = original.lines().collect();
    let needle_lines: Vec<&str> = old_text.lines().collect();
    if needle_lines.is_empty() || needle_lines.len() > orig_lines.len() {
        return None;
    }
    let mut hits = Vec::new();
    for i in 0..=(orig_lines.len() - needle_lines.len()) {
        if orig_lines[i..i + needle_lines.len()]
            .iter()
            .zip(needle_lines.iter())
            .all(|(a, b)| a.trim() == b.trim())
        {
            hits.push(i);
        }
    }
    if hits.len() != 1 {
        return None;
    }
    let idx = hits[0];
    let mut out_lines: Vec<String> = Vec::new();
    out_lines.extend(orig_lines[..idx].iter().map(|s| s.to_string()));
    out_lines.extend(new_text.lines().map(|s| s.to_string()));
    out_lines.extend(
        orig_lines[idx + needle_lines.len()..]
            .iter()
            .map(|s| s.to_string()),
    );
    let mut joined = out_lines.join("\n");
    if original.ends_with('\n') {
        joined.push('\n');
    }
    Some(joined)
}

// --- 4. apply_patch (Pure Rust Unified Diff, dry_run = false by default, ZERO `git` dep!) ---

pub struct ApplyPatchTool;

impl ToolHandler for ApplyPatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "apply_patch".into(),
                description: "Apply a unified diff patch in pure Rust (supports multi-file create/modify/delete without requiring git). dry_run defaults to false.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "patch": { "type": "string", "description": "Unified diff text (--- a/... / +++ b/... / @@ ... @@)" },
                        "dry_run": { "type": "boolean", "description": "Validate without writing (default false)" }
                    },
                    "required": ["patch"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::FilesWrite,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(patch) = args.get("patch").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'patch'", None);
        };
        let dry_run = args.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);
        let file_patches = match parse_unified_diff(patch) {
            Ok(p) if !p.is_empty() => p,
            Ok(_) => {
                return ToolExecutionResult::err_structured(
                    "empty_patch",
                    "no file hunks found in patch",
                    None,
                )
            }
            Err(e) => return ToolExecutionResult::err_structured("invalid_patch", e, None),
        };

        let mut planned: Vec<(PathBuf, Option<String>)> = Vec::new();
        for fp in &file_patches {
            let is_delete = fp.new_path == "/dev/null";
            let rel = if is_delete { &fp.old_path } else { &fp.new_path };
            let target = resolve_path(&ctx.workspace_root, rel);
            if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
                && !path_within_workspace(&ctx.workspace_root, &target)
            {
                return ToolExecutionResult::err_structured(
                    "workspace_escape",
                    format!("patch target '{}' escapes workspace boundary", target.display()),
                    None,
                );
            }
            if is_delete {
                if !target.exists() {
                    return ToolExecutionResult::err_structured(
                        "patch_target_missing",
                        format!("cannot delete missing file {}", target.display()),
                        None,
                    );
                }
                planned.push((target, None));
            } else {
                let original = if target.exists() {
                    match fs::read_to_string(&target) {
                        Ok(s) => s,
                        Err(e) => {
                            return ToolExecutionResult::err_structured(
                                "read_error",
                                format!("cannot read {}: {e}", target.display()),
                                None,
                            )
                        }
                    }
                } else {
                    String::new()
                };
                match apply_hunks(&original, &fp.hunks) {
                    Ok(updated) => planned.push((target, Some(updated))),
                    Err(e) => {
                        return ToolExecutionResult::err_structured(
                            "hunk_apply_failed",
                            format!("{}: {e}", target.display()),
                            None,
                        )
                    }
                }
            }
        }

        if dry_run {
            let files: Vec<String> = planned.iter().map(|(p, _)| p.display().to_string()).collect();
            return ToolExecutionResult::ok_structured(
                json!({"dry_run": true, "files": files}),
                format!("Dry-run validated patch for {} file(s)", files.len()),
            );
        }

        let mut effects = Vec::new();
        for (target, maybe_content) in planned {
            let res = match maybe_content {
                Some(content) => FileMutationPipeline::write_bytes(
                    ctx,
                    &target,
                    content.as_bytes(),
                    ResourceOperation::Edit,
                ),
                None => FileMutationPipeline::delete_file(ctx, &target),
            };
            match res {
                Ok(eff) => effects.push(eff),
                Err(e) => return ToolExecutionResult::err_structured("patch_write_error", e, None),
            }
        }
        let count = effects.len();
        ToolExecutionResult::ok_structured(
            json!({"dry_run": false, "files_changed": count, "effects": effects}),
            format!("Applied patch to {count} file(s)"),
        )
        .with_effects(effects)
    }
}

struct FilePatch {
    old_path: String,
    new_path: String,
    hunks: Vec<Hunk>,
}

struct Hunk {
    old_start: usize,
    lines: Vec<(char, String)>,
    new_no_trailing_newline: bool,
}

fn strip_diff_prefix(raw: &str) -> String {
    let trimmed = raw.trim().split('\t').next().unwrap_or("").trim();
    if trimmed == "/dev/null" {
        return "/dev/null".into();
    }
    trimmed
        .strip_prefix("a/")
        .or_else(|| trimmed.strip_prefix("b/"))
        .unwrap_or(trimmed)
        .to_string()
}

fn parse_unified_diff(patch: &str) -> Result<Vec<FilePatch>, String> {
    let lines: Vec<&str> = patch.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].starts_with("--- ") && i + 1 < lines.len() && lines[i + 1].starts_with("+++ ")
        {
            let old_path = strip_diff_prefix(&lines[i][4..]);
            let new_path = strip_diff_prefix(&lines[i + 1][4..]);
            i += 2;
            let mut hunks = Vec::new();
            while i < lines.len()
                && !lines[i].starts_with("--- ")
                && !lines[i].starts_with("diff --git ")
            {
                if lines[i].starts_with("@@ ") {
                    let (old_start, mut old_rem, mut new_rem) = parse_hunk_header(lines[i])?;
                    i += 1;
                    let mut hlines = Vec::new();
                    let mut new_no_trailing_newline = false;
                    let mut last_tag = ' ';
                    while i < lines.len() && (old_rem > 0 || new_rem > 0) {
                        let l = lines[i];
                        if l.starts_with("\\ No newline") {
                            if last_tag == '+' || last_tag == ' ' {
                                new_no_trailing_newline = true;
                            }
                            i += 1;
                            continue;
                        }
                        let mut chars = l.chars();
                        match chars.next() {
                            Some(' ') => {
                                hlines.push((' ', chars.as_str().to_string()));
                                old_rem = old_rem.saturating_sub(1);
                                new_rem = new_rem.saturating_sub(1);
                                last_tag = ' ';
                            }
                            Some('-') => {
                                hlines.push(('-', chars.as_str().to_string()));
                                old_rem = old_rem.saturating_sub(1);
                                last_tag = '-';
                            }
                            Some('+') => {
                                hlines.push(('+', chars.as_str().to_string()));
                                new_rem = new_rem.saturating_sub(1);
                                last_tag = '+';
                            }
                            None => {
                                hlines.push((' ', String::new()));
                                old_rem = old_rem.saturating_sub(1);
                                new_rem = new_rem.saturating_sub(1);
                                last_tag = ' ';
                            }
                            _ => break,
                        }
                        i += 1;
                    }
                    if i < lines.len() && lines[i].starts_with("\\ No newline") {
                        if last_tag == '+' || last_tag == ' ' {
                            new_no_trailing_newline = true;
                        }
                        i += 1;
                    }
                    hunks.push(Hunk {
                        old_start,
                        lines: hlines,
                        new_no_trailing_newline,
                    });
                } else {
                    i += 1;
                }
            }
            out.push(FilePatch {
                old_path,
                new_path,
                hunks,
            });
        } else {
            i += 1;
        }
    }
    Ok(out)
}

fn parse_hunk_header(header: &str) -> Result<(usize, usize, usize), String> {
    let mut parts = header.split_whitespace();
    let _at = parts.next();
    let old_part = parts
        .next()
        .ok_or_else(|| format!("malformed hunk header: {header}"))?;
    let new_part = parts
        .next()
        .ok_or_else(|| format!("malformed hunk header: {header}"))?;
    let (old_start, old_count) = parse_range(old_part.strip_prefix('-').unwrap_or(old_part), header)?;
    let (_new_start, new_count) = parse_range(new_part.strip_prefix('+').unwrap_or(new_part), header)?;
    Ok((old_start, old_count, new_count))
}

fn parse_range(spec: &str, header: &str) -> Result<(usize, usize), String> {
    if let Some((a, b)) = spec.split_once(',') {
        let start = a.parse::<usize>().map_err(|_| format!("invalid hunk start in {header}"))?;
        let count = b.parse::<usize>().map_err(|_| format!("invalid hunk count in {header}"))?;
        Ok((start, count))
    } else {
        let start = spec.parse::<usize>().map_err(|_| format!("invalid hunk start in {header}"))?;
        Ok((start, 1))
    }
}

fn apply_hunks(original: &str, hunks: &[Hunk]) -> Result<String, String> {
    let mut lines: Vec<String> = if original.is_empty() {
        Vec::new()
    } else {
        original.lines().map(ToString::to_string).collect()
    };
    let mut offset: isize = 0;
    let mut min_next_idx: usize = 0;
    let mut no_trailing_newline = !original.is_empty() && !original.ends_with('\n');

    for hunk in hunks {
        if hunk.new_no_trailing_newline {
            no_trailing_newline = true;
        }
        let expected_old: Vec<&str> = hunk
            .lines
            .iter()
            .filter(|(c, _)| *c == ' ' || *c == '-')
            .map(|(_, s)| s.as_str())
            .collect();
        let replacement: Vec<String> = hunk
            .lines
            .iter()
            .filter(|(c, _)| *c == ' ' || *c == '+')
            .map(|(_, s)| s.clone())
            .collect();

        if expected_old.is_empty() {
            let insert_at = (hunk.old_start as isize + offset)
                .clamp(min_next_idx as isize, lines.len() as isize)
                as usize;
            let inserted = replacement.len();
            lines.splice(insert_at..insert_at, replacement);
            offset += inserted as isize;
            min_next_idx = insert_at + inserted;
            continue;
        }

        let target_idx = ((hunk.old_start.saturating_sub(1)) as isize + offset)
            .clamp(min_next_idx as isize, lines.len() as isize) as usize;
        let found = find_hunk_position(&lines, &expected_old, target_idx, min_next_idx)
            .ok_or_else(|| format!("hunk at line {} failed to match file context", hunk.old_start))?;
        let old_len = expected_old.len();
        let new_len = replacement.len();
        lines.splice(found..(found + old_len), replacement);
        offset += (found as isize - target_idx as isize) + (new_len as isize - old_len as isize);
        min_next_idx = found + new_len;
    }

    let mut out = lines.join("\n");
    if !out.is_empty() && !no_trailing_newline {
        out.push('\n');
    }
    Ok(out)
}

fn find_hunk_position(lines: &[String], expected: &[&str], hint: usize, min_idx: usize) -> Option<usize> {
    if expected.len() > lines.len() {
        return None;
    }
    let matches_at = |idx: usize| -> bool {
        idx >= min_idx
            && idx + expected.len() <= lines.len()
            && lines[idx..idx + expected.len()]
                .iter()
                .zip(expected.iter())
                .all(|(a, b)| a == b)
    };
    if matches_at(hint) {
        return Some(hint);
    }
    for delta in 1..=50 {
        if let Some(lo) = hint.checked_sub(delta) {
            if matches_at(lo) {
                return Some(lo);
            }
        }
        let hi = hint + delta;
        if matches_at(hi) {
            return Some(hi);
        }
    }
    None
}

// --- 5. delete_file ---

pub struct DeleteFileTool;

impl ToolHandler for DeleteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "delete_file".into(),
                description: "Delete a file and return its authoritative pre-delete SHA-256 transition.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::FilesWrite,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(raw_path) = args.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'path'", None);
        };
        let path = resolve_path(&ctx.workspace_root, raw_path);
        match FileMutationPipeline::delete_file(ctx, &path) {
            Ok(effect) => ToolExecutionResult::ok_structured(
                json!({
                    "path": path.display().to_string(),
                    "transition": effect.transition
                }),
                format!("Deleted {}", path.display()),
            )
            .with_effects(vec![effect]),
            Err(e) => ToolExecutionResult::err_structured("delete_error", e, None),
        }
    }
}
