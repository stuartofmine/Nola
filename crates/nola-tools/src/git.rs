// SPDX-License-Identifier: Apache-2.0
//! Native Git tool (`status`, `diff`, `log`, `commit`).

use std::process::Command;

use nola_types::{
    CapabilityDomain, ContentVersion, ResourceEffect, ResourceOperation, ToolExecutionResult,
    ToolSchema, ToolSpec, ToolTier, VersionTransition,
};
use serde_json::{json, Value};

use crate::{ToolContext, ToolHandler};

pub struct GitTool;

impl ToolHandler for GitTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "git".into(),
                description: "Inspect repository status/diff/log or create a commit.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["status", "diff", "log", "commit"] },
                        "message": { "type": "string", "description": "Commit message (for action=commit)" },
                        "limit": { "type": "integer", "description": "Log entry count (default 10)" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::FilesWrite,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(action) = args.get("action").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'action'", None);
        };
        match action {
            "status" => run_git(ctx, &["status", "--short", "--branch"], "git status"),
            "diff" => run_git(ctx, &["diff"], "git diff"),
            "log" => {
                let n = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10)
                    .clamp(1, 100);
                run_git(
                    ctx,
                    &["log", "--oneline", "-n", &n.to_string()],
                    "git log",
                )
            }
            "commit" => {
                let Some(msg) = args.get("message").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) else {
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "commit requires non-empty 'message'",
                        None,
                    );
                };
                let before_head = git_head(ctx);
                let out = Command::new("git")
                    .args(["commit", "-m", msg])
                    .current_dir(&ctx.workspace_root)
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        let after_head = git_head(ctx).unwrap_or_else(|| "HEAD".into());
                        let transition = match before_head {
                            Some(b) => VersionTransition::Modified {
                                before: ContentVersion::GitCommit(b),
                                after: ContentVersion::GitCommit(after_head.clone()),
                            },
                            None => VersionTransition::Created {
                                after: ContentVersion::GitCommit(after_head.clone()),
                            },
                        };
                        let effect = ResourceEffect {
                            uri: ctx.workspace_root.display().to_string(),
                            operation: ResourceOperation::Commit,
                            transition,
                        };
                        ToolExecutionResult::ok_structured(
                            json!({
                                "commit": after_head,
                                "stdout": String::from_utf8_lossy(&o.stdout).trim()
                            }),
                            format!("Created git commit {after_head}"),
                        )
                        .with_effects(vec![effect])
                    }
                    Ok(o) => ToolExecutionResult::err_structured(
                        "git_commit_failed",
                        String::from_utf8_lossy(&o.stderr).trim(),
                        None,
                    ),
                    Err(e) => ToolExecutionResult::err_structured("git_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported git action '{other}'"),
                None,
            ),
        }
    }
}

fn git_head(ctx: &ToolContext) -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&ctx.workspace_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn run_git(ctx: &ToolContext, args: &[&str], label: &str) -> ToolExecutionResult {
    match Command::new("git")
        .args(args)
        .current_dir(&ctx.workspace_root)
        .output()
    {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
            ToolExecutionResult::ok_structured(
                json!({"command": label, "output": stdout}),
                format!("Ran {label}"),
            )
        }
        Ok(o) => ToolExecutionResult::err_structured(
            "git_failed",
            String::from_utf8_lossy(&o.stderr).trim(),
            None,
        ),
        Err(e) => ToolExecutionResult::err_structured("git_error", e.to_string(), None),
    }
}
