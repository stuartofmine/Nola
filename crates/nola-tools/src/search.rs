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

pub struct ListDirTool;

impl ToolHandler for ListDirTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "list_dir".into(),
                description: "List files and subdirectories in a workspace directory.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path (default workspace root)" }
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
        let mut entries = Vec::new();
        for item in rd.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let meta = item.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            entries.push(json!({"name": name, "is_dir": is_dir, "bytes": bytes}));
        }
        entries.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });
        let count = entries.len();
        ToolExecutionResult::ok_structured(
            json!({"path": dir.display().to_string(), "count": count, "entries": entries}),
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
                description: "Find files matching a glob pattern (respects .gitignore).".into(),
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

        let mut files = Vec::new();
        let walker = WalkBuilder::new(&base).hidden(false).build();
        for dent in walker.flatten() {
            if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let rel = dent.path().strip_prefix(&base).unwrap_or(dent.path());
            if matcher.is_match(rel) || matcher.is_match(dent.path()) {
                files.push(rel.display().to_string());
                if files.len() >= max_results {
                    break;
                }
            }
        }
        files.sort();
        let count = files.len();
        ToolExecutionResult::ok_structured(
            json!({"pattern": pattern, "count": count, "files": files}),
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
                description: "Search file contents with regex (respects .gitignore) and return full file:line:text matches.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path": { "type": "string" },
                        "case_insensitive": { "type": "boolean" },
                        "max_matches": { "type": "integer", "description": "Default 200" }
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
        let re = match RegexBuilder::new(pattern)
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
        let base = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(&ctx.workspace_root, p))
            .unwrap_or_else(|| ctx.workspace_root.clone());
        let max_matches = args
            .get("max_matches")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, 1000))
            .unwrap_or(200);

        let mut matches = Vec::new();
        let walker = WalkBuilder::new(&base).hidden(false).build();
        'outer: for dent in walker.flatten() {
            if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let Ok(bytes) = fs::read(dent.path()) else {
                continue;
            };
            // Skip binary files quickly
            if bytes.iter().take(4096).any(|&b| b == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let rel = dent
                .path()
                .strip_prefix(&ctx.workspace_root)
                .unwrap_or(dent.path())
                .display()
                .to_string();
            for (idx, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    matches.push(json!({
                        "file": rel,
                        "line": idx + 1,
                        "text": line
                    }));
                    if matches.len() >= max_matches {
                        break 'outer;
                    }
                }
            }
        }
        let count = matches.len();
        ToolExecutionResult::ok_structured(
            json!({
                "pattern": pattern,
                "count": count,
                "truncated": count >= max_matches,
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
