// SPDX-License-Identifier: Apache-2.0
//! High-productivity code & workspace tools:
//! - `multi_edit_file`: atomic non-overlapping multi-hunk replacements on one file
//! - `move_file`, `append_file`, `read_many_files`, `file_outline`
//! - `notebook`: read/edit Jupyter `.ipynb` cells cleanly
//! - `repo_map`: fast multi-language symbol outline & symbol search
//! - `workspace_checkpoint`: persistent disk-backed checkpoints (`create`/`list`/`diff`/`restore`/`undo`/`redo`)

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ignore::WalkBuilder;
use nola_types::{
    now_ms, sha256_hex, CapabilityDomain, ContentVersion, ResourceEffect, ResourceOperation,
    ToolExecutionResult, ToolSchema, ToolSpec, ToolTier, VersionTransition,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::file_pipeline::{
    byte_offset_to_line, clip_read_line, find_unique_edit_span, looks_binary,
    path_within_workspace, resolve_path, FileCoverageError, FileMutationPipeline,
};
use crate::{ToolContext, ToolHandler};

// --- Persistent Disk-Backed Workspace Checkpoint Manager ---

const CHECKPOINTS_REL_DIR: &str = ".nola/checkpoints";
const MAX_CHECKPOINT_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CHECKPOINT_FILES: usize = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointFileEntry {
    sha256: String,
    bytes: usize,
    /// Relative path beneath `<workspace>/.nola/checkpoints/` where content blob is stored.
    blob_rel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointManifest {
    id: String,
    label: String,
    created_at_ms: i64,
    #[serde(default)]
    internal: bool,
    files: BTreeMap<String, CheckpointFileEntry>,
    /// Files explicitly recorded as absent in this checkpoint's transition scope (only deleted on restore/undo).
    #[serde(default)]
    absent_files: Vec<String>,
    #[serde(default)]
    scope_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CheckpointIndex {
    next_seq: u64,
    undo_stack: Vec<String>,
    redo_stack: Vec<String>,
}

/// Persistent workspace checkpoint manager storing manifests and deduplicated blobs under
/// `<workspace>/.nola/checkpoints/<id>/` so checkpoints and undo/redo stacks survive restarts.
#[derive(Default)]
pub struct CheckpointManager {
    op_lock: Mutex<()>,
    last_root: Mutex<Option<PathBuf>>,
}

impl CheckpointManager {
    fn base_dir(root: &Path) -> PathBuf {
        root.join(CHECKPOINTS_REL_DIR)
    }

    fn load_index(root: &Path) -> CheckpointIndex {
        let idx_path = Self::base_dir(root).join("index.json");
        fs::read_to_string(idx_path)
            .ok()
            .and_then(|s| serde_json::from_str::<CheckpointIndex>(&s).ok())
            .unwrap_or_default()
    }

    fn save_index(root: &Path, index: &CheckpointIndex) -> Result<(), String> {
        let base = Self::base_dir(root);
        fs::create_dir_all(&base).map_err(|e| format!("failed to create checkpoint dir: {e}"))?;
        let data = serde_json::to_string_pretty(index)
            .map_err(|e| format!("failed to serialize checkpoint index: {e}"))?;
        fs::write(base.join("index.json"), data)
            .map_err(|e| format!("failed to write checkpoint index: {e}"))
    }

    fn load_manifest(root: &Path, id: &str) -> Result<CheckpointManifest, String> {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            return Err(format!("invalid checkpoint id '{id}'"));
        }
        let path = Self::base_dir(root).join(id).join("manifest.json");
        let raw = fs::read_to_string(&path)
            .map_err(|_| format!("unknown checkpoint '{id}'"))?;
        serde_json::from_str::<CheckpointManifest>(&raw)
            .map_err(|e| format!("corrupt checkpoint manifest '{id}': {e}"))
    }

    fn save_manifest(root: &Path, manifest: &CheckpointManifest) -> Result<(), String> {
        let cp_dir = Self::base_dir(root).join(&manifest.id);
        fs::create_dir_all(&cp_dir).map_err(|e| format!("failed to create checkpoint dir: {e}"))?;
        let data = serde_json::to_string_pretty(manifest)
            .map_err(|e| format!("failed to serialize checkpoint manifest: {e}"))?;
        fs::write(cp_dir.join("manifest.json"), data)
            .map_err(|e| format!("failed to write checkpoint manifest: {e}"))
    }

    fn existing_blobs_by_sha(root: &Path) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        let base = Self::base_dir(root);
        let Ok(entries) = fs::read_dir(&base) else {
            return map;
        };
        for ent in entries.flatten() {
            let mpath = ent.path().join("manifest.json");
            let Ok(raw) = fs::read_to_string(&mpath) else {
                continue;
            };
            let Ok(manifest) = serde_json::from_str::<CheckpointManifest>(&raw) else {
                continue;
            };
            for file in manifest.files.values() {
                if !map.contains_key(&file.sha256) && base.join(&file.blob_rel).is_file() {
                    map.insert(file.sha256.clone(), file.blob_rel.clone());
                }
            }
        }
        map
    }

    fn store_blob(
        root: &Path,
        cp_id: &str,
        sha: &str,
        bytes: &[u8],
        known_blobs: &mut BTreeMap<String, String>,
    ) -> Result<String, String> {
        if let Some(existing_rel) = known_blobs.get(sha) {
            if Self::base_dir(root).join(existing_rel).is_file() {
                return Ok(existing_rel.clone());
            }
        }
        let blob_rel = format!("{cp_id}/blobs/{sha}");
        let full = Self::base_dir(root).join(&blob_rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("failed to create blob dir: {e}"))?;
        }
        fs::write(&full, bytes).map_err(|e| format!("failed to write checkpoint blob: {e}"))?;
        known_blobs.insert(sha.to_string(), blob_rel.clone());
        Ok(blob_rel)
    }

    fn collect_workspace_files(root: &Path, scope_paths: &[String]) -> BTreeMap<String, Vec<u8>> {
        let mut files = BTreeMap::new();
        if !scope_paths.is_empty() {
            for raw in scope_paths {
                let target = resolve_path(root, raw);
                if !path_within_workspace(root, &target) || !target.is_file() {
                    continue;
                }
                if let Ok(meta) = fs::metadata(&target) {
                    if meta.len() > MAX_CHECKPOINT_FILE_BYTES {
                        continue;
                    }
                }
                if let Ok(bytes) = fs::read(&target) {
                    let rel = target
                        .strip_prefix(root)
                        .unwrap_or(&target)
                        .to_string_lossy()
                        .replace('\\', "/");
                    files.insert(rel, bytes);
                }
            }
            return files;
        }
        let walker = WalkBuilder::new(root)
            .hidden(false)
            .filter_entry(|e| {
                e.depth() == 0
                    || !matches!(
                        e.file_name().to_string_lossy().as_ref(),
                        ".git" | ".nola" | "target" | "node_modules" | "__pycache__" | ".venv"
                    )
            })
            .build();
        for dent in walker.flatten() {
            if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            if let Ok(meta) = dent.metadata() {
                if meta.len() > MAX_CHECKPOINT_FILE_BYTES {
                    continue;
                }
            }
            if let Ok(bytes) = fs::read(dent.path()) {
                let rel = dent
                    .path()
                    .strip_prefix(root)
                    .unwrap_or(dent.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(rel, bytes);
                if files.len() >= MAX_CHECKPOINT_FILES {
                    break;
                }
            }
        }
        files
    }

    fn capture_scoped_snapshot(
        root: &Path,
        index: &mut CheckpointIndex,
        prefix: &str,
        label: &str,
        internal: bool,
        tracked_rel_paths: &BTreeSet<String>,
    ) -> Result<String, String> {
        index.next_seq += 1;
        let id = format!("{prefix}_{}", index.next_seq);
        let mut known_blobs = Self::existing_blobs_by_sha(root);
        let mut files = BTreeMap::new();
        let mut absent_files = Vec::new();
        for rel in tracked_rel_paths {
            let full = root.join(rel);
            if !path_within_workspace(root, &full) {
                continue;
            }
            if full.is_file() {
                if let Ok(bytes) = fs::read(&full) {
                    let sha = sha256_hex(&bytes);
                    let blob_rel = Self::store_blob(root, &id, &sha, &bytes, &mut known_blobs)?;
                    files.insert(
                        rel.clone(),
                        CheckpointFileEntry {
                            sha256: sha,
                            bytes: bytes.len(),
                            blob_rel,
                        },
                    );
                }
            } else {
                absent_files.push(rel.clone());
            }
        }
        let manifest = CheckpointManifest {
            id: id.clone(),
            label: label.to_string(),
            created_at_ms: now_ms(),
            internal,
            files,
            absent_files,
            scope_paths: tracked_rel_paths.iter().cloned().collect(),
        };
        Self::save_manifest(root, &manifest)?;
        Ok(id)
    }

    fn apply_manifest(
        ctx: &ToolContext,
        manifest: &CheckpointManifest,
    ) -> Result<Vec<ResourceEffect>, String> {
        let base = Self::base_dir(&ctx.workspace_root);
        let mut effects = Vec::new();

        // Only delete files explicitly recorded as absent in this checkpoint's scope;
        // NEVER blindly delete unrelated pre-existing workspace files!
        for rel in &manifest.absent_files {
            let target = ctx.workspace_root.join(rel);
            if target.is_file() && path_within_workspace(&ctx.workspace_root, &target) {
                let eff = FileMutationPipeline::delete_file(ctx, &target)?;
                effects.push(eff);
            }
        }

        // Restore every file recorded in the manifest
        for (rel, entry) in &manifest.files {
            let target = ctx.workspace_root.join(rel);
            let blob_path = base.join(&entry.blob_rel);
            let bytes = fs::read(&blob_path).map_err(|e| {
                format!(
                    "missing checkpoint blob '{}' for '{}': {e}",
                    entry.blob_rel, rel
                )
            })?;
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

    pub fn create(&self, root: &Path, label: &str) -> Result<(String, usize), String> {
        self.create_scoped(root, label, &[])
    }

    pub fn create_scoped(
        &self,
        root: &Path,
        label: &str,
        scope_paths: &[String],
    ) -> Result<(String, usize), String> {
        let _guard = self.op_lock.lock().unwrap();
        *self.last_root.lock().unwrap() = Some(root.to_path_buf());

        let mut index = Self::load_index(root);
        index.next_seq += 1;
        let slug: String = label
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(24)
            .collect();
        let id = if slug.is_empty() {
            format!("cp_{}", index.next_seq)
        } else {
            format!("cp_{}_{slug}", index.next_seq)
        };

        let collected = Self::collect_workspace_files(root, scope_paths);
        let mut known_blobs = Self::existing_blobs_by_sha(root);
        let mut entries = BTreeMap::new();
        for (rel, bytes) in collected {
            let sha = sha256_hex(&bytes);
            let blob_rel = Self::store_blob(root, &id, &sha, &bytes, &mut known_blobs)?;
            entries.insert(
                rel,
                CheckpointFileEntry {
                    sha256: sha,
                    bytes: bytes.len(),
                    blob_rel,
                },
            );
        }
        let count = entries.len();
        let manifest = CheckpointManifest {
            id: id.clone(),
            label: label.to_string(),
            created_at_ms: now_ms(),
            internal: false,
            files: entries,
            absent_files: Vec::new(),
            scope_paths: scope_paths.to_vec(),
        };
        Self::save_manifest(root, &manifest)?;
        index.undo_stack.push(id.clone());
        index.redo_stack.clear();
        Self::save_index(root, &index)?;
        Ok((id, count))
    }

    pub fn list(&self) -> Vec<Value> {
        let root_opt = self.last_root.lock().unwrap().clone();
        match root_opt {
            Some(root) => self.list_in(&root),
            None => Vec::new(),
        }
    }

    pub fn list_in(&self, root: &Path) -> Vec<Value> {
        let _guard = self.op_lock.lock().unwrap();
        *self.last_root.lock().unwrap() = Some(root.to_path_buf());
        let base = Self::base_dir(root);
        let Ok(entries) = fs::read_dir(&base) else {
            return Vec::new();
        };
        let mut manifests = Vec::new();
        for ent in entries.flatten() {
            let mpath = ent.path().join("manifest.json");
            if let Ok(raw) = fs::read_to_string(&mpath) {
                if let Ok(m) = serde_json::from_str::<CheckpointManifest>(&raw) {
                    if !m.internal {
                        manifests.push(m);
                    }
                }
            }
        }
        manifests.sort_by(|a, b| a.created_at_ms.cmp(&b.created_at_ms).then(a.id.cmp(&b.id)));
        manifests
            .into_iter()
            .map(|m| {
                json!({
                    "id": m.id,
                    "label": m.label,
                    "created_at_ms": m.created_at_ms,
                    "files": m.files.len()
                })
            })
            .collect()
    }

    pub fn diff(&self, root: &Path, id: &str) -> Result<Value, String> {
        let _guard = self.op_lock.lock().unwrap();
        *self.last_root.lock().unwrap() = Some(root.to_path_buf());
        let manifest = Self::load_manifest(root, id)?;
        let mut modified = Vec::new();
        let mut deleted = Vec::new();
        let mut unchanged_count = 0usize;

        for (rel, entry) in &manifest.files {
            let full = root.join(rel);
            match fs::read(&full) {
                Ok(cur_bytes) => {
                    let cur_sha = sha256_hex(&cur_bytes);
                    if cur_sha == entry.sha256 {
                        unchanged_count += 1;
                    } else {
                        modified.push(json!({
                            "path": rel,
                            "checkpoint_sha256": entry.sha256,
                            "current_sha256": cur_sha,
                            "checkpoint_bytes": entry.bytes,
                            "current_bytes": cur_bytes.len()
                        }));
                    }
                }
                Err(_) => {
                    deleted.push(rel.clone());
                }
            }
        }

        Ok(json!({
            "checkpoint_id": manifest.id,
            "label": manifest.label,
            "modified": modified,
            "deleted": deleted,
            "unchanged_count": unchanged_count
        }))
    }

    pub fn restore(&self, ctx: &ToolContext, id: &str) -> Result<Vec<ResourceEffect>, String> {
        let _guard = self.op_lock.lock().unwrap();
        *self.last_root.lock().unwrap() = Some(ctx.workspace_root.clone());

        let target_manifest = Self::load_manifest(&ctx.workspace_root, id)?;
        let mut index = Self::load_index(&ctx.workspace_root);

        let mut scope: BTreeSet<String> = target_manifest.files.keys().cloned().collect();
        scope.extend(target_manifest.absent_files.iter().cloned());

        // Push current state of affected files onto undo_stack before mutating so restore is undoable!
        let pre_id = Self::capture_scoped_snapshot(
            &ctx.workspace_root,
            &mut index,
            "auto_undo",
            &format!("before restore {id}"),
            true,
            &scope,
        )?;
        let effects = Self::apply_manifest(ctx, &target_manifest)?;
        index.undo_stack.push(pre_id);
        index.redo_stack.clear();
        Self::save_index(&ctx.workspace_root, &index)?;
        Ok(effects)
    }

    pub fn undo(&self, ctx: &ToolContext) -> Result<(String, Vec<ResourceEffect>), String> {
        let _guard = self.op_lock.lock().unwrap();
        *self.last_root.lock().unwrap() = Some(ctx.workspace_root.clone());

        let mut index = Self::load_index(&ctx.workspace_root);
        let Some(target_id) = index.undo_stack.pop() else {
            return Err("nothing to undo (undo stack is empty)".into());
        };
        let target_manifest = Self::load_manifest(&ctx.workspace_root, &target_id)?;
        let mut scope: BTreeSet<String> = target_manifest.files.keys().cloned().collect();
        scope.extend(target_manifest.absent_files.iter().cloned());

        let redo_id = Self::capture_scoped_snapshot(
            &ctx.workspace_root,
            &mut index,
            "auto_redo",
            &format!("redo after undo {target_id}"),
            true,
            &scope,
        )?;
        let effects = Self::apply_manifest(ctx, &target_manifest)?;
        index.redo_stack.push(redo_id);
        Self::save_index(&ctx.workspace_root, &index)?;
        Ok((target_id, effects))
    }

    pub fn redo(&self, ctx: &ToolContext) -> Result<(String, Vec<ResourceEffect>), String> {
        let _guard = self.op_lock.lock().unwrap();
        *self.last_root.lock().unwrap() = Some(ctx.workspace_root.clone());

        let mut index = Self::load_index(&ctx.workspace_root);
        let Some(target_id) = index.redo_stack.pop() else {
            return Err("nothing to redo (redo stack is empty)".into());
        };
        let target_manifest = Self::load_manifest(&ctx.workspace_root, &target_id)?;
        let mut scope: BTreeSet<String> = target_manifest.files.keys().cloned().collect();
        scope.extend(target_manifest.absent_files.iter().cloned());

        let undo_id = Self::capture_scoped_snapshot(
            &ctx.workspace_root,
            &mut index,
            "auto_undo",
            &format!("undo after redo {target_id}"),
            true,
            &scope,
        )?;
        let effects = Self::apply_manifest(ctx, &target_manifest)?;
        index.undo_stack.push(undo_id);
        Self::save_index(&ctx.workspace_root, &index)?;
        Ok((target_id, effects))
    }

    pub fn stack_depths(&self, root: &Path) -> (usize, usize) {
        let idx = Self::load_index(root);
        (idx.undo_stack.len(), idx.redo_stack.len())
    }
}

// --- 1. multi_edit_file ---

pub struct MultiEditFileTool;

impl ToolHandler for MultiEditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "multi_edit_file".into(),
                description: "Atomically apply a list of non-overlapping old_text -> new_text replacements to one file (validated against the original snapshot before mutating).".into(),
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
        if edits.is_empty() {
            return ToolExecutionResult::err_structured("invalid_param", "'edits' array must be non-empty", None);
        }
        let path = resolve_path(&ctx.workspace_root, raw_path);
        if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
            && !path_within_workspace(&ctx.workspace_root, &path)
        {
            return ToolExecutionResult::err_structured(
                "write_error",
                format!("multi_edit_file target '{}' escapes workspace boundary", path.display()),
                None,
            );
        }

        let path_lock = ctx.file_locks.lock_for(&path);
        let _guard = path_lock.lock().unwrap();

        let original = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => return ToolExecutionResult::err_structured("read_error", e.to_string(), None),
        };
        let current_sha = sha256_hex(original.as_bytes());
        match ctx.file_state.verify_observed(&path, &current_sha) {
            Ok(()) => {}
            Err(FileCoverageError::Unread) => {
                return ToolExecutionResult::err_structured(
                    "read_required",
                    format!(
                        "read_file required before multi_edit_file on {} (current sha256: {current_sha})",
                        path.display()
                    ),
                    Some(json!({"path": path.display().to_string(), "actual_sha256": current_sha})),
                );
            }
            Err(FileCoverageError::ModifiedExternally {
                last_seen_sha256,
                current_sha256,
            }) => {
                return ToolExecutionResult::err_structured(
                    "file_modified_externally",
                    format!(
                        "file {} was modified externally since last read (current sha256: {current_sha256}); re-read before multi_edit_file",
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

        // Phase 1: Resolve EVERY edit span against the ORIGINAL snapshot before mutating anything!
        struct PlannedSpan {
            edit_index: usize,
            start: usize,
            end: usize,
            replacement: String,
        }
        let mut spans: Vec<PlannedSpan> = Vec::with_capacity(edits.len());
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
            match find_unique_edit_span(&original, old_text, new_text) {
                Ok((start, end, replacement)) => spans.push(PlannedSpan {
                    edit_index: idx,
                    start,
                    end,
                    replacement,
                }),
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

        // Phase 2: Sort by byte start and reject any overlapping spans!
        spans.sort_by_key(|s| (s.start, s.end));
        for pair in spans.windows(2) {
            let prev = &pair[0];
            let next = &pair[1];
            if prev.end > next.start {
                let prev_line = byte_offset_to_line(&original, prev.start);
                let next_line = byte_offset_to_line(&original, next.start);
                return ToolExecutionResult::err_structured(
                    "overlapping_edits",
                    format!(
                        "edit[{}] (line {prev_line}) overlaps with edit[{}] (line {next_line}); all edits rejected atomically",
                        prev.edit_index, next.edit_index
                    ),
                    Some(json!({
                        "edit_a": prev.edit_index,
                        "edit_b": next.edit_index,
                        "line_a": prev_line,
                        "line_b": next_line
                    })),
                );
            }
        }

        // Phase 3: Apply all validated non-overlapping spans in a single clean pass.
        let mut updated = String::with_capacity(original.len());
        let mut cursor = 0usize;
        for span in &spans {
            updated.push_str(&original[cursor..span.start]);
            updated.push_str(&span.replacement);
            cursor = span.end;
        }
        updated.push_str(&original[cursor..]);

        match FileMutationPipeline::write_bytes(ctx, &path, updated.as_bytes(), ResourceOperation::Edit) {
            Ok(eff) => ToolExecutionResult::ok_structured(
                json!({
                    "path": path.display().to_string(),
                    "applied_edits": edits.len(),
                    "sha256": sha256_hex(updated.as_bytes()),
                    "transition": eff.transition
                }),
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
        ctx.file_state.record_full_access(&dst, sha_hex);
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

// --- 2b. append_file ---

pub struct AppendFileTool;

impl ToolHandler for AppendFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "append_file".into(),
                description: "Atomically append UTF-8 text to a file (creating it if needed) and return its SHA-256 version transition.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" },
                        "ensure_newline": { "type": "boolean", "description": "Insert a leading newline if existing non-empty file does not end with one (default false)" },
                        "create_if_missing": { "type": "boolean", "description": "Create file if it does not yet exist (default true)" }
                    },
                    "required": ["path", "content"]
                }),
            },
            tier: ToolTier::Deferred,
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
        let ensure_newline = args
            .get("ensure_newline")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let create_if_missing = args
            .get("create_if_missing")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let path = resolve_path(&ctx.workspace_root, raw_path);
        if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
            && !path_within_workspace(&ctx.workspace_root, &path)
        {
            return ToolExecutionResult::err_structured(
                "workspace_escape",
                format!("append target '{}' escapes workspace boundary", path.display()),
                None,
            );
        }

        let path_lock = ctx.file_locks.lock_for(&path);
        let _guard = path_lock.lock().unwrap();

        let mut combined = if path.exists() {
            match fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    return ToolExecutionResult::err_structured(
                        "read_error",
                        format!("cannot read {}: {e}", path.display()),
                        None,
                    )
                }
            }
        } else if create_if_missing {
            Vec::new()
        } else {
            return ToolExecutionResult::err_structured(
                "not_found",
                format!("file does not exist and create_if_missing=false: {}", path.display()),
                None,
            );
        };

        if ensure_newline && !combined.is_empty() && !combined.ends_with(b"\n") {
            combined.push(b'\n');
        }
        combined.extend_from_slice(content.as_bytes());
        let new_sha = sha256_hex(&combined);
        let total_bytes = combined.len();

        match FileMutationPipeline::write_bytes(ctx, &path, &combined, ResourceOperation::Edit) {
            Ok(effect) => ToolExecutionResult::ok_structured(
                json!({
                    "path": path.display().to_string(),
                    "appended_bytes": content.len(),
                    "total_bytes": total_bytes,
                    "sha256": new_sha,
                    "transition": effect.transition
                }),
                format!(
                    "Appended {} bytes to {} (total {total_bytes} bytes, sha256: {new_sha})",
                    content.len(),
                    path.display()
                ),
            )
            .with_effects(vec![effect]),
            Err(e) => ToolExecutionResult::err_structured("write_error", e, None),
        }
    }
}

// --- 2c. file_outline ---

pub struct FileOutlineTool;

impl ToolHandler for FileOutlineTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "file_outline".into(),
                description: "Extract a fast structural outline (functions, types, classes, Markdown headings, or config sections) with 1-based line numbers for a single file.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "max_entries": { "type": "integer", "description": "Maximum outline items to return (default 200, max 1000)" }
                    },
                    "required": ["path"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::FilesRead,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(raw_path) = args.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'path'", None);
        };
        let max_entries = args
            .get("max_entries")
            .and_then(|v| v.as_u64())
            .unwrap_or(200)
            .clamp(1, 1_000) as usize;
        let path = resolve_path(&ctx.workspace_root, raw_path);
        if !path.is_file() {
            return ToolExecutionResult::err_structured(
                "not_found",
                format!("file does not exist: {}", path.display()),
                None,
            );
        }
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => return ToolExecutionResult::err_structured("read_error", e.to_string(), None),
        };
        if looks_binary(&bytes) {
            return ToolExecutionResult::err_structured(
                "binary_file",
                format!("cannot build text outline for binary file {}", path.display()),
                None,
            );
        }
        let text = String::from_utf8_lossy(&bytes);
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        static CODE_RE: OnceLock<Regex> = OnceLock::new();
        let code_re = CODE_RE.get_or_init(|| {
            Regex::new(
                r"^(\s*)(?:export\s+)?(?:default\s+)?(?:pub(?:\([^)]*\))?\s+)?(?:public|private|protected|internal|fileprivate|open\s+)?(?:static\s+)?(?:override\s+)?(?:final\s+)?(?:abstract\s+)?(?:unsafe\s+)?(?:async\s+)?(fn|func|function|def|struct|enum|trait|impl|class|interface|protocol|actor|extension|type|mod|module|namespace|object|record|const|static)\s+([A-Za-z_][A-Za-z0-9_<>:]*)",
            )
            .unwrap()
        });
        static MD_RE: OnceLock<Regex> = OnceLock::new();
        let md_re = MD_RE.get_or_init(|| Regex::new(r"^(#{1,6})\s+(.+?)\s*#*$").unwrap());
        static TOML_RE: OnceLock<Regex> = OnceLock::new();
        let toml_re = TOML_RE.get_or_init(|| Regex::new(r"^\s*(\[\[?[^\]]+\]\]?)").unwrap());
        static YAML_RE: OnceLock<Regex> = OnceLock::new();
        let yaml_re = YAML_RE.get_or_init(|| Regex::new(r"^([A-Za-z0-9_.-]+)\s*:").unwrap());

        let mut entries = Vec::new();
        let mut truncated = false;
        let mut in_fenced_code = false;
        let lines: Vec<&str> = text.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            let line_no = i + 1;
            let trimmed = line.trim();
            if matches!(ext.as_str(), "md" | "mdx" | "markdown") {
                if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                    in_fenced_code = !in_fenced_code;
                    continue;
                }
                if !in_fenced_code {
                    if let Some(caps) = md_re.captures(trimmed) {
                        let depth = caps.get(1).map(|m| m.as_str().len()).unwrap_or(1);
                        let title = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                        entries.push(json!({
                            "line": line_no,
                            "kind": format!("h{depth}"),
                            "name": title,
                            "depth": depth,
                            "signature": clip_read_line(trimmed)
                        }));
                    }
                }
            } else if ext == "toml" {
                if let Some(caps) = toml_re.captures(line) {
                    let sec = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    entries.push(json!({
                        "line": line_no,
                        "kind": "section",
                        "name": sec,
                        "depth": if sec.starts_with("[[") { 2 } else { 1 },
                        "signature": clip_read_line(trimmed)
                    }));
                }
            } else if matches!(ext.as_str(), "yaml" | "yml") {
                if let Some(caps) = yaml_re.captures(line) {
                    let key = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    entries.push(json!({
                        "line": line_no,
                        "kind": "key",
                        "name": key,
                        "depth": 1,
                        "signature": clip_read_line(trimmed)
                    }));
                }
            } else if let Some(caps) = code_re.captures(line) {
                let indent = caps.get(1).map(|m| m.as_str().len()).unwrap_or(0);
                let kind = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                let name = caps.get(3).map(|m| m.as_str()).unwrap_or("");
                entries.push(json!({
                    "line": line_no,
                    "kind": kind,
                    "name": name,
                    "depth": (indent / 4) + 1,
                    "signature": clip_read_line(trimmed)
                }));
            }
            if entries.len() >= max_entries {
                truncated = i + 1 < lines.len();
                break;
            }
        }

        let count = entries.len();
        ToolExecutionResult::ok_structured(
            json!({
                "path": path.display().to_string(),
                "language": ext,
                "total_lines": lines.len(),
                "count": count,
                "truncated": truncated,
                "entries": entries
            }),
            format!("Extracted {count} outline item(s) from {}", path.display()),
        )
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
                    let truncated = budget_hit || lines.len() > max_lines;
                    ctx.file_state.record_line_range(
                        &full,
                        sha.clone(),
                        1,
                        rendered.len().max(1),
                        !truncated,
                    );
                    results.push(json!({
                        "path": full.display().to_string(),
                        "sha256": sha,
                        "total_lines": lines.len(),
                        "truncated": truncated,
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
                        ".git" | ".nola" | "target" | "node_modules" | "__pycache__" | ".venv"
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
                description: "Create, list, diff, restore, undo, or redo persistent workspace checkpoints stored under .nola/checkpoints/.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "diff", "restore", "undo", "redo"] },
                        "label": { "type": "string" },
                        "checkpoint_id": { "type": "string" },
                        "paths": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional file paths to scope checkpoint creation"
                        }
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
                let scope: Vec<String> = args
                    .get("paths")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                match ctx.checkpoints.create_scoped(&ctx.workspace_root, label, &scope) {
                    Ok((id, files)) => ToolExecutionResult::ok_structured(
                        json!({"checkpoint_id": id, "files_captured": files}),
                        format!("Created checkpoint {id} ({files} files)"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("checkpoint_error", e, None),
                }
            }
            "list" => {
                let items = ctx.checkpoints.list_in(&ctx.workspace_root);
                let count = items.len();
                let (undo_depth, redo_depth) = ctx.checkpoints.stack_depths(&ctx.workspace_root);
                ToolExecutionResult::ok_structured(
                    json!({
                        "count": count,
                        "undo_depth": undo_depth,
                        "redo_depth": redo_depth,
                        "checkpoints": items
                    }),
                    format!("Listed {count} checkpoint(s)"),
                )
            }
            "diff" => {
                let Some(id) = args.get("checkpoint_id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'checkpoint_id'", None);
                };
                match ctx.checkpoints.diff(&ctx.workspace_root, id) {
                    Ok(diff) => ToolExecutionResult::ok_structured(
                        diff,
                        format!("Computed workspace diff against checkpoint {id}"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("checkpoint_error", e, None),
                }
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
            "undo" => match ctx.checkpoints.undo(ctx) {
                Ok((id, effects)) => {
                    let count = effects.len();
                    ToolExecutionResult::ok_structured(
                        json!({"restored_checkpoint_id": id, "restored_files": count}),
                        format!("Undid workspace changes using {id} ({count} changed file(s))"),
                    )
                    .with_effects(effects)
                }
                Err(e) => ToolExecutionResult::err_structured("checkpoint_error", e, None),
            },
            "redo" => match ctx.checkpoints.redo(ctx) {
                Ok((id, effects)) => {
                    let count = effects.len();
                    ToolExecutionResult::ok_structured(
                        json!({"restored_checkpoint_id": id, "restored_files": count}),
                        format!("Redid workspace changes using {id} ({count} changed file(s))"),
                    )
                    .with_effects(effects)
                }
                Err(e) => ToolExecutionResult::err_structured("checkpoint_error", e, None),
            },
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported checkpoint action '{other}'"),
                None,
            ),
        }
    }
}
