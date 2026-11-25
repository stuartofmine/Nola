// SPDX-License-Identifier: Apache-2.0
//! Unified `FileMutationPipeline` + `FileAccessState` + `FileLockRegistry` + core file tools:
//! `read_file`, `write_file`, `edit_file`, `apply_patch`, `delete_file`.
//!
//! Compile-time invariants:
//! - Every mutation computes `before` and `after` `ContentVersion::Sha256` and returns an
//!   exhaustive `VersionTransition` (`Created`, `Modified`, `Deleted`, `Unchanged`).
//! - `path_within_workspace` canonicalizes nearest existing ancestor to block symlink escapes.
//! - Per-path `FileLockRegistry` prevents parallel tool calls from racing on the same path.
//! - Continuity guards enforce read/write awareness and prevent stale/blind overwrites & edits.
//! - `edit_file` supports tolerant indentation matching + optional `expected_replacements`.
//! - `apply_patch` defaults `dry_run = false`, checks hunk old-line coverage, and applies unified
//!   diffs in pure Rust (zero `git` dep!).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nola_types::{
    sha256_hex, CapabilityDomain, ContentVersion, ResourceEffect, ResourceOperation,
    ToolExecutionResult, ToolSchema, ToolSpec, ToolTier, VersionTransition,
};
use serde_json::{json, Value};

use crate::binary_docs::try_read_binary_document;
use crate::{ToolContext, ToolHandler};

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);
const MAX_READ_TEXT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_READ_LINE_CHARS: usize = 2_000;
const MAX_READ_RESPONSE_BYTES: usize = 32 * 1024;

pub(crate) fn clip_read_line(line: &str) -> String {
    let count = line.chars().count();
    if count <= MAX_READ_LINE_CHARS {
        line.to_string()
    } else {
        let prefix: String = line.chars().take(MAX_READ_LINE_CHARS).collect();
        format!("{prefix}... [line truncated: {count} chars]")
    }
}

pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|&b| b == 0)
}

/// Per-path mutex registry preventing parallel tool calls from racing on the same file.
#[derive(Default)]
pub struct FileLockRegistry {
    locks: Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>>,
}

impl FileLockRegistry {
    pub fn lock_for(&self, path: &Path) -> Arc<Mutex<()>> {
        let key = canonicalize_nearest_ancestor(path);
        let mut map = self.locks.lock().unwrap();
        Arc::clone(map.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileCoverageError {
    Unread,
    ModifiedExternally {
        last_seen_sha256: String,
        current_sha256: String,
    },
    RangeNotRead {
        start_line: usize,
        end_line: usize,
    },
}

#[derive(Clone, Debug)]
struct FileAccessRecord {
    sha256: String,
    full_coverage: bool,
    /// Merged, sorted inclusive 1-based line ranges read for `sha256`.
    line_ranges: Vec<(usize, usize)>,
}

/// Tracks SHA-256 digests and read line-range coverage of files observed via `read_file`
/// or authored by tools within the session.
#[derive(Default)]
pub struct FileAccessState {
    records: Mutex<BTreeMap<PathBuf, FileAccessRecord>>,
}

impl FileAccessState {
    fn key(path: &Path) -> PathBuf {
        canonicalize_nearest_ancestor(path)
    }

    /// Record full observation/authorship of `path` at `sha256` (empty string marks deleted).
    pub fn record(&self, path: &Path, sha256: String) {
        self.record_full_access(path, sha256);
    }

    /// Record `Full` coverage for `path` at `sha256` so subsequent `edit_file` / `apply_patch`
    /// calls succeed immediately without requiring a redundant re-read.
    pub fn record_full_access(&self, path: &Path, sha256: String) {
        let key = Self::key(path);
        if let Ok(mut g) = self.records.lock() {
            if sha256.is_empty() {
                g.insert(
                    key,
                    FileAccessRecord {
                        sha256: String::new(),
                        full_coverage: false,
                        line_ranges: Vec::new(),
                    },
                );
            } else {
                g.insert(
                    key,
                    FileAccessRecord {
                        sha256,
                        full_coverage: true,
                        line_ranges: Vec::new(),
                    },
                );
            }
        }
    }

    /// Record a 1-based inclusive line range `[start_line, end_line]` read for `path` at `sha256`.
    /// When `is_full` is true (e.g. read from line 1 through EOF without truncation), upgrades to `Full`.
    pub fn record_line_range(
        &self,
        path: &Path,
        sha256: String,
        start_line: usize,
        end_line: usize,
        is_full: bool,
    ) {
        if sha256.is_empty() {
            return;
        }
        let key = Self::key(path);
        if let Ok(mut g) = self.records.lock() {
            let entry = g.entry(key).or_insert_with(|| FileAccessRecord {
                sha256: sha256.clone(),
                full_coverage: false,
                line_ranges: Vec::new(),
            });
            if entry.sha256 != sha256 {
                entry.sha256 = sha256;
                entry.full_coverage = is_full;
                entry.line_ranges.clear();
            } else {
                entry.full_coverage |= is_full;
            }
            if !entry.full_coverage && start_line > 0 && end_line >= start_line {
                entry.line_ranges.push((start_line, end_line));
                merge_line_ranges(&mut entry.line_ranges);
            }
        }
    }

    pub fn last_seen(&self, path: &Path) -> Option<String> {
        let key = Self::key(path);
        self.records
            .lock()
            .ok()
            .and_then(|g| g.get(&key).map(|r| r.sha256.clone()))
    }

    /// Verify that `path` was previously observed/authored and has not drifted externally on disk.
    pub fn verify_observed(&self, path: &Path, current_sha: &str) -> Result<(), FileCoverageError> {
        let key = Self::key(path);
        let guard = self.records.lock().unwrap();
        let Some(rec) = guard.get(&key).filter(|r| !r.sha256.is_empty()) else {
            return Err(FileCoverageError::Unread);
        };
        if rec.sha256 != current_sha {
            return Err(FileCoverageError::ModifiedExternally {
                last_seen_sha256: rec.sha256.clone(),
                current_sha256: current_sha.to_string(),
            });
        }
        Ok(())
    }

    /// Verify that `path` was observed at `current_sha` AND either has `Full` coverage or its
    /// recorded read line ranges completely cover `[start_line, end_line]`.
    pub fn covers_line_range(
        &self,
        path: &Path,
        current_sha: &str,
        start_line: usize,
        end_line: usize,
    ) -> Result<(), FileCoverageError> {
        let key = Self::key(path);
        let guard = self.records.lock().unwrap();
        let Some(rec) = guard.get(&key).filter(|r| !r.sha256.is_empty()) else {
            return Err(FileCoverageError::Unread);
        };
        if rec.sha256 != current_sha {
            return Err(FileCoverageError::ModifiedExternally {
                last_seen_sha256: rec.sha256.clone(),
                current_sha256: current_sha.to_string(),
            });
        }
        if rec.full_coverage || start_line == 0 || end_line < start_line {
            return Ok(());
        }
        if rec
            .line_ranges
            .iter()
            .any(|&(s, e)| s <= start_line && e >= end_line)
        {
            return Ok(());
        }
        Err(FileCoverageError::RangeNotRead {
            start_line,
            end_line,
        })
    }
}

fn merge_line_ranges(ranges: &mut Vec<(usize, usize)>) {
    if ranges.len() <= 1 {
        return;
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (s, e) in ranges.drain(..) {
        if let Some(last) = merged.last_mut() {
            if s <= last.1.saturating_add(1) {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    *ranges = merged;
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
        let after_sha = sha256_hex(new_bytes);
        let after = ContentVersion::Sha256(after_sha.clone());
        // Preserve mtime if file already exists with identical content!
        if before.as_ref() == Some(&after) {
            ctx.file_state.record_full_access(path, after_sha);
            return Ok(ResourceEffect {
                uri: path.display().to_string(),
                operation: op_hint,
                transition: VersionTransition::Unchanged { version: after },
            });
        }
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
        // Verify written disk digest matches authored bytes before granting Full coverage.
        let verified_sha = fs::read(path)
            .ok()
            .map(|b| sha256_hex(&b))
            .unwrap_or_else(|| after_sha.clone());
        if verified_sha == after_sha {
            ctx.file_state.record_full_access(path, after_sha);
        }

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
                description: "Read a UTF-8 text file by 1-based line range (start_line/end_line) or UTF-8 safe byte range (start_byte/max_bytes), or inspect DOCX (.docx), spreadsheets (.xlsx/.xls/.ods), PDFs (.pdf), audio, and images.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path relative to workspace or absolute" },
                        "start_line": { "type": "integer", "description": "1-based start line (default 1)" },
                        "end_line": { "type": "integer", "description": "1-based inclusive end line (default start_line + 399)" },
                        "start_byte": { "type": "integer", "description": "Optional 0-based UTF-8 byte offset for byte-range reading" },
                        "max_bytes": { "type": "integer", "description": "Optional maximum bytes to return for byte-range reading" },
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
        if looks_binary(&bytes) {
            return ToolExecutionResult::err_structured(
                "binary_file",
                format!(
                    "file '{}' appears to be binary ({} bytes, sha256={}); use bash (file/hexdump/strings) to inspect",
                    path.display(),
                    bytes.len(),
                    &digest[..16]
                ),
                Some(json!({"path": path.display().to_string(), "bytes": bytes.len(), "sha256": digest})),
            );
        }

        let text = String::from_utf8_lossy(&bytes);

        let start_byte_opt = args.get("start_byte").and_then(|v| v.as_u64()).map(|n| n as usize);
        let max_bytes_opt = args.get("max_bytes").and_then(|v| v.as_u64()).map(|n| n as usize);
        if start_byte_opt.is_some() || max_bytes_opt.is_some() {
            let req_start = start_byte_opt.unwrap_or(0);
            let max_bytes = max_bytes_opt
                .unwrap_or(MAX_READ_RESPONSE_BYTES)
                .clamp(1, MAX_READ_RESPONSE_BYTES);
            let total_bytes = text.len();
            let (slice, actual_start, next_start) =
                crate::output_store::utf8_byte_slice(&text, req_start, max_bytes);
            let is_full = actual_start == 0 && next_start >= total_bytes;
            let start_ln = byte_offset_to_line(&text, actual_start);
            let end_ln = byte_offset_to_line(&text, next_start.saturating_sub(1).max(actual_start));
            ctx.file_state
                .record_line_range(&path, digest.clone(), start_ln, end_ln, is_full);
            return ToolExecutionResult::ok_structured(
                json!({
                    "path": path.display().to_string(),
                    "sha256": digest,
                    "start_byte": actual_start,
                    "bytes_returned": slice.len(),
                    "next_start_byte": next_start,
                    "total_bytes": total_bytes,
                    "truncated": next_start < total_bytes,
                    "content": slice
                }),
                format!(
                    "Read {} (bytes {}-{} of {})",
                    path.display(),
                    actual_start,
                    next_start,
                    total_bytes
                ),
            );
        }

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
        let requested_end = args
            .get("end_line")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).max(clamped_start))
            .unwrap_or_else(|| clamped_start.saturating_add(399))
            .min(total_lines.max(1));

        let mut rendered = Vec::new();
        let mut bytes_used = 0usize;
        let mut actual_end = 0usize;
        let mut byte_capped = false;
        if total_lines > 0 && start <= total_lines {
            for (idx, line) in lines
                .iter()
                .enumerate()
                .skip(clamped_start.saturating_sub(1))
                .take(requested_end.saturating_sub(clamped_start).saturating_add(1))
            {
                let formatted = format!("{}: {}", idx + 1, clip_read_line(line));
                if !rendered.is_empty() && bytes_used + formatted.len() + 1 > MAX_READ_RESPONSE_BYTES {
                    byte_capped = true;
                    break;
                }
                bytes_used += formatted.len() + 1;
                actual_end = idx + 1;
                rendered.push(formatted);
            }
        }
        let numbered = rendered.join("\n");
        let truncated =
            byte_capped || (total_lines > 0 && start <= total_lines && actual_end < total_lines);

        let is_full = total_lines == 0 || (!truncated && start <= 1);
        ctx.file_state.record_line_range(
            &path,
            digest.clone(),
            clamped_start,
            actual_end.max(clamped_start),
            is_full,
        );

        ToolExecutionResult::ok_structured(
            json!({
                "path": path.display().to_string(),
                "sha256": digest,
                "start_line": start,
                "end_line": actual_end,
                "total_lines": total_lines,
                "truncated": truncated,
                "content": numbered
            }),
            format!(
                "Read {} (lines {}-{} of {})",
                path.display(),
                start,
                actual_end,
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
                description: "Create or overwrite a UTF-8 file atomically (guards against blind overwrites of unread/externally-modified files; supports expected_sha256 and dry_run) and return its SHA-256 version transition.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" },
                        "expected_sha256": { "type": "string", "description": "Expected SHA-256 hex digest of existing file when overwriting an unread or externally-modified file" },
                        "dry_run": { "type": "boolean", "description": "Validate and preview transition without writing to disk (default false)" }
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
        let expected_sha256 = args
            .get("expected_sha256")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let path = resolve_path(&ctx.workspace_root, raw_path);
        if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
            && !path_within_workspace(&ctx.workspace_root, &path)
        {
            return ToolExecutionResult::err_structured(
                "write_error",
                format!("write target '{}' escapes workspace boundary", path.display()),
                None,
            );
        }

        let path_lock = ctx.file_locks.lock_for(&path);
        let _guard = path_lock.lock().unwrap();

        let existing_sha = fs::read(&path).ok().map(|b| sha256_hex(&b));
        let new_sha = sha256_hex(content.as_bytes());

        if let Some(expected) = expected_sha256 {
            if existing_sha.as_deref() != Some(expected) {
                return ToolExecutionResult::err_structured(
                    "existing_file_version_mismatch",
                    format!(
                        "existing file version mismatch for {}: expected sha256 '{}', actual '{}'",
                        path.display(),
                        expected,
                        existing_sha.as_deref().unwrap_or("<missing>")
                    ),
                    Some(json!({
                        "path": path.display().to_string(),
                        "expected_sha256": expected,
                        "actual_sha256": existing_sha
                    })),
                );
            }
        } else if let Some(ref disk_sha) = existing_sha {
            // Overwriting an existing file with different content requires either prior observation
            // of this exact disk version in this run OR an explicit matching `expected_sha256`.
            if disk_sha != &new_sha {
                match ctx.file_state.verify_observed(&path, disk_sha) {
                    Ok(()) => {}
                    Err(FileCoverageError::Unread) => {
                        return ToolExecutionResult::err_structured(
                            "existing_file_version_required",
                            format!(
                                "cannot overwrite unread existing file {} without expected_sha256 (current sha256: {disk_sha}); read the file first or pass expected_sha256",
                                path.display()
                            ),
                            Some(json!({
                                "path": path.display().to_string(),
                                "actual_sha256": disk_sha
                            })),
                        );
                    }
                    Err(FileCoverageError::ModifiedExternally {
                        last_seen_sha256,
                        current_sha256,
                    }) => {
                        return ToolExecutionResult::err_structured(
                            "file_modified_externally",
                            format!(
                                "file {} was modified externally since last read/write (current sha256: {current_sha256}); re-read or pass matching expected_sha256",
                                path.display()
                            ),
                            Some(json!({
                                "path": path.display().to_string(),
                                "last_seen_sha256": last_seen_sha256,
                                "actual_sha256": current_sha256
                            })),
                        );
                    }
                    Err(FileCoverageError::RangeNotRead { .. }) => {}
                }
            }
        }

        if dry_run {
            let after = ContentVersion::Sha256(new_sha.clone());
            let transition = match existing_sha {
                None => VersionTransition::Created { after },
                Some(ref b) if b == &new_sha => VersionTransition::Unchanged { version: after },
                Some(b) => VersionTransition::Modified {
                    before: ContentVersion::Sha256(b),
                    after,
                },
            };
            let bytes = content.len();
            let lines = content.lines().count();
            return ToolExecutionResult::ok_structured(
                json!({
                    "path": path.display().to_string(),
                    "dry_run": true,
                    "would_change": transition.changed(),
                    "bytes": bytes,
                    "lines": lines,
                    "sha256": new_sha,
                    "transition": transition
                }),
                format!(
                    "Dry-run write {} ({bytes} bytes, {lines} lines, sha256: {new_sha})",
                    path.display()
                ),
            );
        }

        match FileMutationPipeline::write_bytes(ctx, &path, content.as_bytes(), ResourceOperation::Create) {
            Ok(effect) => {
                let bytes = content.len();
                let lines = content.lines().count();
                ToolExecutionResult::ok_structured(
                    json!({
                        "path": path.display().to_string(),
                        "bytes": bytes,
                        "lines": lines,
                        "sha256": new_sha,
                        "transition": effect.transition
                    }),
                    format!(
                        "Wrote {} ({bytes} bytes, {lines} lines, sha256: {new_sha})",
                        path.display()
                    ),
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
                description: "Replace exact text (or whitespace-tolerant block) in an existing file after reading it. Optional expected_replacements and replace_all.".into(),
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
        if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
            && !path_within_workspace(&ctx.workspace_root, &path)
        {
            return ToolExecutionResult::err_structured(
                "write_error",
                format!("edit target '{}' escapes workspace boundary", path.display()),
                None,
            );
        }

        let path_lock = ctx.file_locks.lock_for(&path);
        let _guard = path_lock.lock().unwrap();

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

        let current_sha = sha256_hex(original.as_bytes());
        match ctx.file_state.verify_observed(&path, &current_sha) {
            Ok(()) => {}
            Err(FileCoverageError::Unread) => {
                return ToolExecutionResult::err_structured(
                    "read_required",
                    format!(
                        "read_file required before editing {} (current sha256: {current_sha})",
                        path.display()
                    ),
                    Some(json!({
                        "path": path.display().to_string(),
                        "actual_sha256": current_sha
                    })),
                );
            }
            Err(FileCoverageError::ModifiedExternally {
                last_seen_sha256,
                current_sha256,
            }) => {
                return ToolExecutionResult::err_structured(
                    "file_modified_externally",
                    format!(
                        "file {} was modified externally since last read (current sha256: {current_sha256}); call read_file again before editing",
                        path.display()
                    ),
                    Some(json!({
                        "path": path.display().to_string(),
                        "last_seen_sha256": last_seen_sha256,
                        "actual_sha256": current_sha256
                    })),
                );
            }
            Err(FileCoverageError::RangeNotRead { .. }) => {}
        }

        let (updated, replaced_count, match_mode, line_hint) =
            match apply_text_edit(&original, old_text, new_text, replace_all, expected) {
                Ok(v) => v,
                Err(err_res) => return err_res,
            };

        match FileMutationPipeline::write_bytes(ctx, &path, updated.as_bytes(), ResourceOperation::Edit) {
            Ok(effect) => {
                let after_sha = sha256_hex(updated.as_bytes());
                let payload = json!({
                    "path": path.display().to_string(),
                    "replacements": replaced_count,
                    "match_mode": match_mode,
                    "changed_lines": line_hint,
                    "sha256": after_sha,
                    "transition": effect.transition
                });
                ToolExecutionResult::ok_structured(
                    payload,
                    format!("Edited {} ({replaced_count} replacement(s), {match_mode})", path.display()),
                )
                .with_effects(vec![effect])
            }
            Err(e) => ToolExecutionResult::err_structured("write_error", e, None),
        }
    }
}

/// Shared 3-tier replacement engine (exact -> CRLF-normalized -> indent-preserving trimmed block)
/// with actionable line-number diagnostics and closest-match snippets on failure!
pub(crate) fn apply_text_edit(
    original: &str,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
    expected: Option<usize>,
) -> Result<(String, usize, &'static str, Value), ToolExecutionResult> {
    let exact_positions: Vec<usize> = original.match_indices(old_text).map(|(i, _)| i).collect();
    let exact_count = exact_positions.len();
    if exact_count > 0 {
        if let Some(exp) = expected {
            if exact_count != exp {
                let match_lines: Vec<usize> = exact_positions
                    .iter()
                    .take(20)
                    .map(|&off| byte_offset_to_line(original, off))
                    .collect();
                return Err(ToolExecutionResult::err_structured(
                    "replacement_count_mismatch",
                    format!("expected {exp} matches but found {exact_count} at line(s) {match_lines:?}"),
                    Some(json!({"found": exact_count, "expected": exp, "match_lines": match_lines})),
                ));
            }
        } else if !replace_all && exact_count > 1 {
            let match_lines: Vec<usize> = exact_positions
                .iter()
                .take(20)
                .map(|&off| byte_offset_to_line(original, off))
                .collect();
            return Err(ToolExecutionResult::err_structured(
                "ambiguous_match",
                format!(
                    "old_text matched {exact_count} locations at line(s) {match_lines:?}; provide more context or set replace_all=true"
                ),
                Some(json!({"found": exact_count, "match_lines": match_lines})),
            ));
        }
        let first_line = byte_offset_to_line(original, exact_positions[0]);
        let new_line_count = new_text.lines().count().max(1);
        let do_all = replace_all || expected.map(|e| e > 1).unwrap_or(false);
        let out = if do_all {
            original.replace(old_text, new_text)
        } else {
            original.replacen(old_text, new_text, 1)
        };
        let count = if do_all { exact_count } else { 1 };
        return Ok((
            out,
            count,
            "exact",
            json!({"start_line": first_line, "end_line": first_line + new_line_count.saturating_sub(1)}),
        ));
    }

    // Tier 2: CRLF-normalized exact match (preserves original file's CRLF line endings!)
    if original.contains("\r\n") && !old_text.contains("\r\n") {
        let crlf_old = old_text.replace('\n', "\r\n");
        let crlf_new = new_text.replace("\r\n", "\n").replace('\n', "\r\n");
        let crlf_positions: Vec<usize> = original.match_indices(&crlf_old).map(|(i, _)| i).collect();
        let crlf_count = crlf_positions.len();
        if crlf_count > 0 {
            if let Some(exp) = expected {
                if crlf_count != exp {
                    let match_lines: Vec<usize> = crlf_positions
                        .iter()
                        .take(20)
                        .map(|&off| byte_offset_to_line(original, off))
                        .collect();
                    return Err(ToolExecutionResult::err_structured(
                        "replacement_count_mismatch",
                        format!("expected {exp} matches but found {crlf_count} at line(s) {match_lines:?}"),
                        Some(json!({"found": crlf_count, "expected": exp, "match_lines": match_lines})),
                    ));
                }
            } else if !replace_all && crlf_count > 1 {
                let match_lines: Vec<usize> = crlf_positions
                    .iter()
                    .take(20)
                    .map(|&off| byte_offset_to_line(original, off))
                    .collect();
                return Err(ToolExecutionResult::err_structured(
                    "ambiguous_match",
                    format!(
                        "old_text matched {crlf_count} locations at line(s) {match_lines:?}; provide more context or set replace_all=true"
                    ),
                    Some(json!({"found": crlf_count, "match_lines": match_lines})),
                ));
            }
            let first_line = byte_offset_to_line(original, crlf_positions[0]);
            let new_line_count = new_text.lines().count().max(1);
            let do_all = replace_all || expected.map(|e| e > 1).unwrap_or(false);
            let out = if do_all {
                original.replace(&crlf_old, &crlf_new)
            } else {
                original.replacen(&crlf_old, &crlf_new, 1)
            };
            let count = if do_all { crlf_count } else { 1 };
            return Ok((
                out,
                count,
                "crlf_normalized",
                json!({"start_line": first_line, "end_line": first_line + new_line_count.saturating_sub(1)}),
            ));
        }
    }

    // Tier 3: Unique whitespace-trimmed block match preserving indentation delta
    if expected.is_none_or(|e| e == 1) {
        if let Some((out, start_line, end_line)) = tolerant_trimmed_replace(original, old_text, new_text) {
            return Ok((
                out,
                1,
                "trimmed_lines",
                json!({"start_line": start_line, "end_line": end_line}),
            ));
        }
    }

    // Failure: compute closest-matching file window so the model can fix the edit in 1 step!
    let hint = find_closest_match_snippet(original, old_text);
    Err(ToolExecutionResult::err_structured(
        "not_found",
        "old_text not found in file (checked exact, CRLF, and whitespace-trimmed line match)",
        hint,
    ))
}

/// Locate a unique byte span `(start_byte, end_byte, replacement_text)` in `original`
/// without mutating `original`, for non-overlapping multi-edit validation.
pub(crate) fn find_unique_edit_span(
    original: &str,
    old_text: &str,
    new_text: &str,
) -> Result<(usize, usize, String), ToolExecutionResult> {
    // Tier 1: Exact match
    let exact: Vec<usize> = original.match_indices(old_text).map(|(i, _)| i).collect();
    if exact.len() == 1 {
        let start = exact[0];
        return Ok((start, start + old_text.len(), new_text.to_string()));
    }
    if exact.len() > 1 {
        let match_lines: Vec<usize> = exact
            .iter()
            .take(20)
            .map(|&off| byte_offset_to_line(original, off))
            .collect();
        return Err(ToolExecutionResult::err_structured(
            "ambiguous_match",
            format!(
                "old_text matched {} locations at line(s) {match_lines:?}; provide more context",
                exact.len()
            ),
            Some(json!({"found": exact.len(), "match_lines": match_lines})),
        ));
    }

    // Tier 2: CRLF-normalized exact match
    if original.contains("\r\n") && !old_text.contains("\r\n") {
        let crlf_old = old_text.replace('\n', "\r\n");
        let crlf_new = new_text.replace("\r\n", "\n").replace('\n', "\r\n");
        let crlf: Vec<usize> = original.match_indices(&crlf_old).map(|(i, _)| i).collect();
        if crlf.len() == 1 {
            let start = crlf[0];
            return Ok((start, start + crlf_old.len(), crlf_new));
        }
        if crlf.len() > 1 {
            let match_lines: Vec<usize> = crlf
                .iter()
                .take(20)
                .map(|&off| byte_offset_to_line(original, off))
                .collect();
            return Err(ToolExecutionResult::err_structured(
                "ambiguous_match",
                format!(
                    "old_text matched {} locations at line(s) {match_lines:?}; provide more context",
                    crlf.len()
                ),
                Some(json!({"found": crlf.len(), "match_lines": match_lines})),
            ));
        }
    }

    // Tier 3: Unique whitespace-trimmed line block
    if let Some((start, end, repl)) = tolerant_trimmed_span(original, old_text, new_text) {
        return Ok((start, end, repl));
    }

    let hint = find_closest_match_snippet(original, old_text);
    Err(ToolExecutionResult::err_structured(
        "not_found",
        "old_text not found in file (checked exact, CRLF, and whitespace-trimmed line match)",
        hint,
    ))
}

pub(crate) fn byte_offset_to_line(text: &str, byte_offset: usize) -> usize {
    let bytes = text.as_bytes();
    bytes[..byte_offset.min(bytes.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

fn leading_ws(s: &str) -> &str {
    let idx = s.find(|c: char| !c.is_whitespace()).unwrap_or(s.len());
    &s[..idx]
}

fn tolerant_trimmed_span(original: &str, old_text: &str, new_text: &str) -> Option<(usize, usize, String)> {
    let line_ending = if original.contains("\r\n") { "\r\n" } else { "\n" };
    // Collect (line_str_without_ending, byte_start, byte_end_without_ending)
    let mut orig_spans: Vec<(&str, usize, usize)> = Vec::new();
    let mut pos = 0usize;
    for chunk in original.split_inclusive('\n') {
        let len = chunk.len();
        let trimmed_eol = chunk.strip_suffix("\r\n").or_else(|| chunk.strip_suffix('\n')).unwrap_or(chunk);
        orig_spans.push((trimmed_eol, pos, pos + trimmed_eol.len()));
        pos += len;
    }
    let needle_lines: Vec<&str> = old_text.lines().collect();
    if needle_lines.is_empty() || needle_lines.len() > orig_spans.len() {
        return None;
    }
    let mut hits = Vec::new();
    for i in 0..=(orig_spans.len() - needle_lines.len()) {
        if orig_spans[i..i + needle_lines.len()]
            .iter()
            .zip(needle_lines.iter())
            .all(|((a, _, _), b)| a.trim() == b.trim())
        {
            hits.push(i);
        }
    }
    if hits.len() != 1 {
        return None;
    }
    let idx = hits[0];
    let orig_indent = orig_spans[idx..idx + needle_lines.len()]
        .iter()
        .find(|(l, _, _)| !l.trim().is_empty())
        .map(|(l, _, _)| leading_ws(l))
        .unwrap_or("");
    let needle_indent = needle_lines
        .iter()
        .find(|l| !l.trim().is_empty())
        .map(|l| leading_ws(l))
        .unwrap_or("");

    let adjusted_new: Vec<String> = new_text
        .lines()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else if orig_indent != needle_indent {
                let stripped = l.strip_prefix(needle_indent).unwrap_or(l);
                format!("{orig_indent}{stripped}")
            } else {
                l.to_string()
            }
        })
        .collect();
    let start_byte = orig_spans[idx].1;
    let end_byte = orig_spans[idx + needle_lines.len() - 1].2;
    Some((start_byte, end_byte, adjusted_new.join(line_ending)))
}

fn tolerant_trimmed_replace(original: &str, old_text: &str, new_text: &str) -> Option<(String, usize, usize)> {
    let (start_byte, end_byte, repl) = tolerant_trimmed_span(original, old_text, new_text)?;
    let start_line = byte_offset_to_line(original, start_byte);
    let new_len = repl.lines().count().max(1);
    let mut out = String::with_capacity(original.len() + repl.len());
    out.push_str(&original[..start_byte]);
    out.push_str(&repl);
    out.push_str(&original[end_byte..]);
    Some((out, start_line, start_line + new_len.saturating_sub(1)))
}

fn find_closest_match_snippet(original: &str, old_text: &str) -> Option<Value> {
    let orig_lines: Vec<&str> = original.lines().collect();
    let needle_lines: Vec<&str> = old_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if orig_lines.is_empty() || needle_lines.is_empty() {
        return None;
    }
    let win = needle_lines.len().clamp(1, 15);
    let mut best_score = 0usize;
    let mut best_start = 0usize;
    for i in 0..orig_lines.len() {
        let end = (i + win).min(orig_lines.len());
        let mut score = 0usize;
        for nl in &needle_lines {
            if orig_lines[i..end].iter().any(|ol| {
                let ot = ol.trim();
                !ot.is_empty() && (ot == *nl || ot.contains(nl) || nl.contains(ot))
            }) {
                score += 2;
            } else {
                // Token overlap fallback for slight typos
                let toks: Vec<&str> = nl.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|t| t.len() >= 3).collect();
                if !toks.is_empty()
                    && orig_lines[i..end]
                        .iter()
                        .any(|ol| toks.iter().filter(|&&t| ol.contains(t)).count() * 2 >= toks.len())
                {
                    score += 1;
                }
            }
        }
        if score > best_score {
            best_score = score;
            best_start = i;
        }
    }
    if best_score == 0 {
        return None;
    }
    let ctx_start = best_start.saturating_sub(2);
    let ctx_end = (best_start + win + 2).min(orig_lines.len());
    let snippet = orig_lines[ctx_start..ctx_end]
        .iter()
        .enumerate()
        .map(|(k, l)| format!("{}: {}", ctx_start + k + 1, clip_read_line(l)))
        .collect::<Vec<_>>()
        .join("\n");
    Some(json!({
        "closest_match": {
            "start_line": ctx_start + 1,
            "end_line": ctx_end,
            "snippet": snippet
        }
    }))
}

// --- 4. apply_patch (Pure Rust Unified Diff, dry_run = false by default, ZERO `git` dep!) ---

pub struct ApplyPatchTool;

impl ToolHandler for ApplyPatchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "apply_patch".into(),
                description: "Apply a unified diff patch in pure Rust (supports multi-file create/modify/delete without requiring git; verifies read coverage of every hunk range). dry_run defaults to false.".into(),
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

        // Acquire per-path locks in deterministic sorted order to prevent deadlocks across parallel calls.
        let mut unique_targets: BTreeSet<PathBuf> = BTreeSet::new();
        for fp in &file_patches {
            let is_delete = fp.new_path == "/dev/null";
            let rel = if is_delete { &fp.old_path } else { &fp.new_path };
            unique_targets.insert(resolve_path(&ctx.workspace_root, rel));
        }
        let lock_arcs: Vec<Arc<Mutex<()>>> = unique_targets
            .iter()
            .map(|p| ctx.file_locks.lock_for(p))
            .collect();
        let mut _guards = Vec::with_capacity(lock_arcs.len());
        for arc in &lock_arcs {
            _guards.push(arc.lock().unwrap());
        }

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
                if let Ok(existing_bytes) = fs::read(&target) {
                    let current_sha = sha256_hex(&existing_bytes);
                    if let Err(err) = ctx.file_state.verify_observed(&target, &current_sha) {
                        return coverage_err_to_result(&target, err, &current_sha);
                    }
                }
                planned.push((target, None));
            } else {
                let original = if target.exists() {
                    let text = match fs::read_to_string(&target) {
                        Ok(s) => s,
                        Err(e) => {
                            return ToolExecutionResult::err_structured(
                                "read_error",
                                format!("cannot read {}: {e}", target.display()),
                                None,
                            )
                        }
                    };
                    let current_sha = sha256_hex(text.as_bytes());
                    if let Err(err) = ctx.file_state.verify_observed(&target, &current_sha) {
                        return coverage_err_to_result(&target, err, &current_sha);
                    }
                    // Verify read coverage of every hunk's old-line range
                    for hunk in &fp.hunks {
                        let (start_line, end_line) = if hunk.old_count > 0 {
                            let s = hunk.old_start.max(1);
                            (s, s.saturating_add(hunk.old_count - 1))
                        } else if hunk.old_start > 0 {
                            (hunk.old_start, hunk.old_start)
                        } else {
                            (0, 0)
                        };
                        if start_line > 0 {
                            if let Err(err) = ctx.file_state.covers_line_range(
                                &target,
                                &current_sha,
                                start_line,
                                end_line,
                            ) {
                                return coverage_err_to_result(&target, err, &current_sha);
                            }
                        }
                    }
                    text
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

fn coverage_err_to_result(
    path: &Path,
    err: FileCoverageError,
    current_sha: &str,
) -> ToolExecutionResult {
    match err {
        FileCoverageError::Unread => ToolExecutionResult::err_structured(
            "read_required",
            format!(
                "read_file required before patching {} (current sha256: {current_sha})",
                path.display()
            ),
            Some(json!({
                "path": path.display().to_string(),
                "actual_sha256": current_sha
            })),
        ),
        FileCoverageError::ModifiedExternally {
            last_seen_sha256,
            current_sha256,
        } => ToolExecutionResult::err_structured(
            "file_modified_externally",
            format!(
                "file {} was modified externally since last read (current sha256: {current_sha256}); re-read with read_file before patching",
                path.display()
            ),
            Some(json!({
                "path": path.display().to_string(),
                "last_seen_sha256": last_seen_sha256,
                "actual_sha256": current_sha256
            })),
        ),
        FileCoverageError::RangeNotRead {
            start_line,
            end_line,
        } => ToolExecutionResult::err_structured(
            "unread_patch_range",
            format!(
                "patch hunk lines {start_line}-{end_line} in {} were not read via read_file; read that range first",
                path.display()
            ),
            Some(json!({
                "path": path.display().to_string(),
                "start_line": start_line,
                "end_line": end_line,
                "actual_sha256": current_sha
            })),
        ),
    }
}

struct FilePatch {
    old_path: String,
    new_path: String,
    hunks: Vec<Hunk>,
}

struct Hunk {
    old_start: usize,
    old_count: usize,
    lines: Vec<(char, String)>,
    new_no_trailing_newline: bool,
}

fn unquote_diff_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('t') => out.push('\t'),
                    Some('n') => out.push('\n'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    } else {
        trimmed.to_string()
    }
}

fn strip_diff_prefix(raw: &str) -> String {
    let before_tab = raw.trim().split('\t').next().unwrap_or("").trim();
    let unquoted = unquote_diff_path(before_tab);
    if unquoted == "/dev/null" {
        return "/dev/null".into();
    }
    unquoted
        .strip_prefix("a/")
        .or_else(|| unquoted.strip_prefix("b/"))
        .unwrap_or(&unquoted)
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
                    let (old_start, old_count, mut new_rem) = parse_hunk_header(lines[i])?;
                    let mut old_rem = old_count;
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
                        old_count,
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
    let line_ending = if original.contains("\r\n") { "\r\n" } else { "\n" };
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
            .ok_or_else(|| {
                let s = target_idx.saturating_sub(2);
                let e = (target_idx + expected_old.len().min(8) + 2).min(lines.len());
                let actual = lines[s..e]
                    .iter()
                    .enumerate()
                    .map(|(k, l)| format!("{}: {}", s + k + 1, l))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "hunk at line {} failed to match file context; actual lines nearby:\n{actual}",
                    hunk.old_start
                )
            })?;
        let old_len = expected_old.len();
        let new_len = replacement.len();
        lines.splice(found..(found + old_len), replacement);
        offset += (found as isize - target_idx as isize) + (new_len as isize - old_len as isize);
        min_next_idx = found + new_len;
    }

    let mut out = lines.join(line_ending);
    if !out.is_empty() && !no_trailing_newline {
        out.push_str(line_ending);
    }
    Ok(out)
}

fn find_hunk_position(lines: &[String], expected: &[&str], hint: usize, min_idx: usize) -> Option<usize> {
    if expected.len() > lines.len() {
        return None;
    }
    let exact_at = |idx: usize| -> bool {
        idx >= min_idx
            && idx + expected.len() <= lines.len()
            && lines[idx..idx + expected.len()]
                .iter()
                .zip(expected.iter())
                .all(|(a, b)| a == b)
    };
    if exact_at(hint) {
        return Some(hint);
    }
    for delta in 1..=100 {
        if let Some(lo) = hint.checked_sub(delta) {
            if exact_at(lo) {
                return Some(lo);
            }
        }
        let hi = hint + delta;
        if exact_at(hi) {
            return Some(hi);
        }
    }
    // Full-file unique exact match when line numbers drifted > 100 lines
    let exact_hits: Vec<usize> = (min_idx..=lines.len().saturating_sub(expected.len()))
        .filter(|&i| exact_at(i))
        .collect();
    if exact_hits.len() == 1 {
        return Some(exact_hits[0]);
    }
    // Whitespace-trimmed fallback within ±100 lines
    let trimmed_at = |idx: usize| -> bool {
        idx >= min_idx
            && idx + expected.len() <= lines.len()
            && lines[idx..idx + expected.len()]
                .iter()
                .zip(expected.iter())
                .all(|(a, b)| a.trim() == b.trim())
    };
    if trimmed_at(hint) {
        return Some(hint);
    }
    for delta in 1..=100 {
        if let Some(lo) = hint.checked_sub(delta) {
            if trimmed_at(lo) {
                return Some(lo);
            }
        }
        let hi = hint + delta;
        if trimmed_at(hi) {
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
        let path_lock = ctx.file_locks.lock_for(&path);
        let _guard = path_lock.lock().unwrap();
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
