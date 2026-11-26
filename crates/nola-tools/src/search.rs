// SPDX-License-Identifier: Apache-2.0
//! Workspace navigation and search tools (`list_dir`, `glob`, `grep`, `workspace_status`).
//! Guarantees FULL matched content in `ToolModelPayload::Structured::model_data`.

use std::fs;
use std::process::Command;

use globset::Glob;
use ignore::WalkBuilder;
use nola_types::{CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
use regex::RegexBuilder;
use serde_json::{json, Value};

use crate::file_pipeline::resolve_path;
use crate::{ToolContext, ToolHandler};

const MAX_GREP_LINE_CHARS: usize = 300;

fn is_noise_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "target" | "node_modules" | "__pycache__" | ".venv" | ".mypy_cache" | ".pytest_cache"
    )
}

fn clip_grep_line(line: &str) -> String {
    let count = line.chars().count();
    if count <= MAX_GREP_LINE_CHARS {
        line.to_string()
    } else {
        let prefix: String = line.chars().take(MAX_GREP_LINE_CHARS).collect();
        format!("{prefix}... [{count} chars]")
    }
}

pub struct ListDirTool;

impl ToolHandler for ListDirTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "list_dir".into(),
                description: "List files and subdirectories in a workspace directory (supports optional max_depth 1..4).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path (default workspace root)" },
                        "max_depth": { "type": "integer", "description": "Traversal depth 1..4 (default 1)" },
                        "max_entries": { "type": "integer", "description": "Max entries returned (default 200, max 500)" }
                    }
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::FilesRead,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let raw = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let dir = resolve_path(&ctx.workspace_root, raw);
        let max_depth = args
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .clamp(1, 4) as usize;
        let max_entries = args
            .get("max_entries")
            .and_then(|v| v.as_u64())
            .unwrap_or(200)
            .clamp(1, 500) as usize;

        if !dir.exists() {
            return ToolExecutionResult::err_structured(
                "not_found",
                format!("path does not exist: {}", dir.display()),
                None,
            );
        }
        if !dir.is_dir() {
            return ToolExecutionResult::err_structured(
                "not_a_directory",
                format!("path is not a directory: {}", dir.display()),
                None,
            );
        }

        let mut entries: Vec<(bool, String, u64)> = Vec::new();
        let mut truncated = false;
        if max_depth == 1 {
            let rd = match fs::read_dir(&dir) {
                Ok(r) => r,
                Err(e) => {
                    return ToolExecutionResult::err_structured(
                        "read_dir_error",
                        format!("cannot list {}: {e}", dir.display()),
                        None,
                    )
                }
            };
            for item in rd.flatten() {
                let name = item.file_name().to_string_lossy().into_owned();
                let meta = item.metadata().ok();
                let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                entries.push((is_dir, name, bytes));
            }
        } else {
            let walker = WalkBuilder::new(&dir)
                .hidden(false)
                .max_depth(Some(max_depth))
                .filter_entry(|e| e.depth() == 0 || !is_noise_dir(&e.file_name().to_string_lossy()))
                .build();
            for dent in walker.flatten() {
                if dent.depth() == 0 {
                    continue;
                }
                let rel = dent
                    .path()
                    .strip_prefix(&dir)
                    .unwrap_or(dent.path())
                    .display()
                    .to_string();
                let is_dir = dent.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let bytes = dent.metadata().map(|m| m.len()).unwrap_or(0);
                entries.push((is_dir, rel, bytes));
                if entries.len() > max_entries {
                    truncated = true;
                    break;
                }
            }
        }
        // Directories first, then alphabetical
        entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        if entries.len() > max_entries {
            entries.truncate(max_entries);
            truncated = true;
        }
        let json_entries: Vec<Value> = entries
            .into_iter()
            .map(|(is_dir, name, bytes)| json!({"name": name, "is_dir": is_dir, "bytes": bytes}))
            .collect();
        let count = json_entries.len();
        ToolExecutionResult::ok_structured(
            json!({
                "path": dir.display().to_string(),
                "count": count,
                "truncated": truncated,
                "entries": json_entries
            }),
            format!("Listed {count} entries in {}", dir.display()),
        )
    }
}

pub struct GlobTool;

impl ToolHandler for GlobTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "glob".into(),
                description: "Find files matching a glob pattern (respects .gitignore and skips build/cache noise dirs).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path": { "type": "string", "description": "Optional base directory" },
                        "max_results": { "type": "integer", "description": "Maximum matches (default 200)" }
                    },
                    "required": ["pattern"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::FilesRead,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'pattern'", None);
        };
        let matcher = match Glob::new(pattern).map(|g| g.compile_matcher()) {
            Ok(m) => m,
            Err(e) => {
                return ToolExecutionResult::err_structured(
                    "invalid_glob",
                    format!("invalid glob '{pattern}': {e}"),
                    None,
                )
            }
        };
        let base = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(&ctx.workspace_root, p))
            .unwrap_or_else(|| ctx.workspace_root.clone());
        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, 2000))
            .unwrap_or(200);

        if !base.exists() {
            return ToolExecutionResult::err_structured(
                "not_found",
                format!("path does not exist: {}", base.display()),
                None,
            );
        }
        let mut matches: Vec<(u128, String)> = Vec::new();
        let mut truncated = false;
        let walker = WalkBuilder::new(&base)
            .hidden(false)
            .filter_entry(|e| e.depth() == 0 || !is_noise_dir(&e.file_name().to_string_lossy()))
            .build();
        for dent in walker.flatten() {
            if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let rel = dent.path().strip_prefix(&base).unwrap_or(dent.path());
            if matcher.is_match(rel) || matcher.is_match(dent.path()) {
                let mtime = dent
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                matches.push((mtime, rel.display().to_string()));
                if matches.len() >= max_results * 4 {
                    truncated = true;
                    break;
                }
            }
        }
        if matches.len() > max_results {
            // Keep the most recently modified matches when truncated
            matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            matches.truncate(max_results);
            truncated = true;
        }
        let mut files: Vec<String> = matches.into_iter().map(|(_, p)| p).collect();
        files.sort();
        let count = files.len();
        ToolExecutionResult::ok_structured(
            json!({"pattern": pattern, "count": count, "truncated": truncated, "files": files}),
            format!("Found {count} file(s) matching '{pattern}'"),
        )
    }
}

pub struct GrepTool;

impl ToolHandler for GrepTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "grep".into(),
                description: "Search file contents with regex or fixed_strings (respects .gitignore; supports include glob filter, context_lines, and files_only).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path": { "type": "string", "description": "Optional file or directory to search" },
                        "include": { "type": "string", "description": "Optional file glob filter, e.g. '*.rs' or '**/*.py'" },
                        "case_insensitive": { "type": "boolean" },
                        "fixed_strings": { "type": "boolean", "description": "Treat pattern as literal text instead of regex" },
                        "context_lines": { "type": "integer", "description": "Lines of surrounding context (0..5, default 0)" },
                        "files_only": { "type": "boolean", "description": "Return only matching file paths with counts" },
                        "max_matches": { "type": "integer", "description": "Default 150" }
                    },
                    "required": ["pattern"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::FilesRead,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(pattern) = args.get("pattern").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'pattern'", None);
        };
        let case_insensitive = args
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let fixed_strings = args
            .get("fixed_strings")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let effective_pattern = if fixed_strings {
            regex::escape(pattern)
        } else {
            pattern.to_string()
        };
        let re = match RegexBuilder::new(&effective_pattern)
            .case_insensitive(case_insensitive)
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                return ToolExecutionResult::err_structured(
                    "invalid_regex",
                    format!("invalid regex '{pattern}': {e}"),
                    None,
                )
            }
        };
        let include_matcher = match args.get("include").and_then(|v| v.as_str()) {
            Some(pat) if !pat.trim().is_empty() => match Glob::new(pat).map(|g| g.compile_matcher())
            {
                Ok(m) => Some(m),
                Err(e) => {
                    return ToolExecutionResult::err_structured(
                        "invalid_include_glob",
                        format!("invalid include glob '{pat}': {e}"),
                        None,
                    );
                }
            },
            _ => None,
        };
        let context_lines = args
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .min(5) as usize;
        let files_only = args
            .get("files_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let base = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(&ctx.workspace_root, p))
            .unwrap_or_else(|| ctx.workspace_root.clone());
        let max_matches = args
            .get("max_matches")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, 1000))
            .unwrap_or(150);

        if !base.exists() {
            return ToolExecutionResult::err_structured(
                "not_found",
                format!("path does not exist: {}", base.display()),
                None,
            );
        }
        let mut matches = Vec::new();
        let mut file_hits = Vec::new();
        let mut truncated = false;
        let walker = WalkBuilder::new(&base)
            .hidden(false)
            .filter_entry(|e| e.depth() == 0 || !is_noise_dir(&e.file_name().to_string_lossy()))
            .build();
        'outer: for dent in walker.flatten() {
            if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let rel_path = dent.path().strip_prefix(&ctx.workspace_root).unwrap_or(dent.path());
            if let Some(ref inc) = include_matcher {
                let file_name = dent.path().file_name().unwrap_or_default();
                if !inc.is_match(rel_path) && !inc.is_match(file_name) {
                    continue;
                }
            }
            if dent.metadata().map(|m| m.len() > 10 * 1024 * 1024).unwrap_or(true) {
                continue;
            }
            let Ok(bytes) = fs::read(dent.path()) else {
                continue;
            };
            if bytes.iter().take(4096).any(|&b| b == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let rel = rel_path.display().to_string();
            let lines: Vec<&str> = text.lines().collect();
            let mut count_in_file = 0usize;
            for (idx, line) in lines.iter().enumerate() {
                if re.is_match(line) {
                    count_in_file += 1;
                    if !files_only {
                        let mut item = json!({
                            "file": rel,
                            "line": idx + 1,
                            "text": clip_grep_line(line)
                        });
                        if context_lines > 0 {
                            let b_start = idx.saturating_sub(context_lines);
                            let before: Vec<String> = lines[b_start..idx]
                                .iter()
                                .map(|l| clip_grep_line(l))
                                .collect();
                            let a_end = (idx + 1 + context_lines).min(lines.len());
                            let after: Vec<String> = lines[idx + 1..a_end]
                                .iter()
                                .map(|l| clip_grep_line(l))
                                .collect();
                            item["before"] = json!(before);
                            item["after"] = json!(after);
                        }
                        matches.push(item);
                        if matches.len() >= max_matches {
                            truncated = true;
                            break 'outer;
                        }
                    }
                }
            }
            if files_only && count_in_file > 0 {
                file_hits.push(json!({"file": rel, "matches": count_in_file}));
                if file_hits.len() >= max_matches {
                    truncated = true;
                    break 'outer;
                }
            }
        }
        if files_only {
            let count = file_hits.len();
            return ToolExecutionResult::ok_structured(
                json!({
                    "pattern": pattern,
                    "files_only": true,
                    "count": count,
                    "truncated": truncated,
                    "files": file_hits
                }),
                format!("Found {count} file(s) matching '{pattern}'"),
            );
        }
        let count = matches.len();
        ToolExecutionResult::ok_structured(
            json!({
                "pattern": pattern,
                "count": count,
                "truncated": truncated,
                "matches": matches
            }),
            format!("Found {count} grep match(es) for '{pattern}'"),
        )
    }
}

pub struct WorkspaceStatusTool;

impl ToolHandler for WorkspaceStatusTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "workspace_status".into(),
                description: "Inspect workspace root, git status summary, and running background jobs.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::FilesRead,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, _args: Value) -> ToolExecutionResult {
        let git_status = Command::new("git")
            .args(["status", "--short", "--branch"])
            .current_dir(&ctx.workspace_root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "(not a git repository)".into());
        let jobs = ctx.shell.list_jobs();
        ToolExecutionResult::ok_structured(
            json!({
                "workspace_root": ctx.workspace_root.display().to_string(),
                "git_status": git_status,
                "background_jobs": jobs
            }),
            format!("Workspace status ({} background job(s))", jobs.len()),
        )
    }
}
