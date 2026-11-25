// SPDX-License-Identifier: Apache-2.0
//! Native Git tool (`status`, `diff`, `log`, `commit`).

use std::process::Command;

use nola_types::{
    CapabilityDomain, ContentVersion, ResourceEffect, ResourceOperation, ToolExecutionResult,
    ToolSchema, ToolSpec, ToolTier, VersionTransition,
};
use serde_json::{json, Value};

use crate::file_pipeline::{path_within_workspace, resolve_path};
use crate::{ToolContext, ToolHandler};

fn reject_flag_injection(val: &str, name: &str) -> Result<(), ToolExecutionResult> {
    if val.starts_with('-') {
        return Err(ToolExecutionResult::err_structured(
            "invalid_param",
            format!("'{name}' cannot start with '-'"),
            None,
        ));
    }
    Ok(())
}

pub struct GitTool;

impl ToolHandler for GitTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "git".into(),
                description: "Inspect repository status/diff/log/show/blame or create commits/worktrees.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["status", "diff", "log", "show", "blame", "commit", "worktree_list", "worktree_create", "worktree_remove"]
                        },
                        "message": { "type": "string", "description": "Commit message (for action=commit)" },
                        "author": { "type": "string", "description": "Optional commit author 'Name <email>' (for action=commit)" },
                        "target": { "type": "string", "description": "Commit ref (show), file path (blame), or worktree path (worktree_create/remove)" },
                        "path": { "type": "string", "description": "Optional file/directory filter for diff or log" },
                        "staged": { "type": "boolean", "description": "Diff staged changes (--cached) for action=diff" },
                        "stat_only": { "type": "boolean", "description": "Return compact --stat summary for action=diff" },
                        "branch": { "type": "string", "description": "Optional branch name for worktree_create" },
                        "limit": { "type": "integer", "description": "Log entry count (default 10)" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::Process,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(action) = args.get("action").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'action'", None);
        };
        match action {
            "status" => run_git(ctx, &["status", "--short", "--branch"], "git status"),
            "diff" => {
                let staged = args.get("staged").and_then(|v| v.as_bool()).unwrap_or(false);
                let stat_only = args.get("stat_only").and_then(|v| v.as_bool()).unwrap_or(false);
                let mut cmd_args: Vec<&str> = vec!["diff"];
                if staged {
                    cmd_args.push("--cached");
                }
                if stat_only {
                    cmd_args.push("--stat");
                }
                if let Some(p) = args.get("path").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                    if let Err(e) = reject_flag_injection(p, "path") {
                        return e;
                    }
                    cmd_args.push("--");
                    cmd_args.push(p);
                }
                run_git(ctx, &cmd_args, "git diff")
            }
            "log" => {
                let n = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10)
                    .clamp(1, 100);
                let n_str = n.to_string();
                let mut cmd_args: Vec<&str> = vec!["log", "--oneline", "-n", &n_str];
                if let Some(p) = args.get("path").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                    if let Err(e) = reject_flag_injection(p, "path") {
                        return e;
                    }
                    cmd_args.push("--");
                    cmd_args.push(p);
                }
                run_git(ctx, &cmd_args, "git log")
            }
            "show" => {
                let rev = args.get("target").and_then(|v| v.as_str()).unwrap_or("HEAD");
                if let Err(e) = reject_flag_injection(rev, "target") {
                    return e;
                }
                run_git(ctx, &["show", "--stat", rev, "--"], "git show")
            }
            "blame" => {
                let Some(path) = args.get("target").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "blame requires 'target' file path", None);
                };
                if let Err(e) = reject_flag_injection(path, "target") {
                    return e;
                }
                run_git(ctx, &["blame", "-L", "1,120", "--", path], "git blame")
            }
            "worktree_list" => run_git(ctx, &["worktree", "list", "--porcelain"], "git worktree list"),
            "worktree_create" => {
                let Some(path) = args.get("target").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "worktree_create requires 'target' path", None);
                };
                if let Err(e) = reject_flag_injection(path, "target") {
                    return e;
                }
                let resolved = resolve_path(&ctx.workspace_root, path);
                if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
                    && !path_within_workspace(&ctx.workspace_root, &resolved)
                {
                    return ToolExecutionResult::err_structured(
                        "workspace_escape",
                        format!("worktree target '{}' escapes workspace boundary", resolved.display()),
                        None,
                    );
                }
                let path_s = resolved.to_string_lossy().into_owned();
                if let Some(branch) = args.get("branch").and_then(|v| v.as_str()) {
                    if let Err(e) = reject_flag_injection(branch, "branch") {
                        return e;
                    }
                    run_git(ctx, &["worktree", "add", "-b", branch, "--", &path_s], "git worktree add")
                } else {
                    run_git(ctx, &["worktree", "add", "--detach", "--", &path_s], "git worktree add")
                }
            }
            "worktree_remove" => {
                let Some(path) = args.get("target").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "worktree_remove requires 'target' path", None);
                };
                if let Err(e) = reject_flag_injection(path, "target") {
                    return e;
                }
                let resolved = resolve_path(&ctx.workspace_root, path);
                if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
                    && !path_within_workspace(&ctx.workspace_root, &resolved)
                {
                    return ToolExecutionResult::err_structured(
                        "workspace_escape",
                        format!("worktree target '{}' escapes workspace boundary", resolved.display()),
                        None,
                    );
                }
                let path_s = resolved.to_string_lossy().into_owned();
                run_git(ctx, &["worktree", "remove", "--force", "--", &path_s], "git worktree remove")
            }
            "commit" => {
                let Some(msg) = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                else {
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "commit requires non-empty 'message'",
                        None,
                    );
                };
                let author = args
                    .get("author")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                if let Some(a) = author {
                    if let Err(e) = reject_flag_injection(a, "author") {
                        return e;
                    }
                }
                if let Err(err_res) = verify_git_identity(ctx) {
                    return err_res;
                }
                let before_head = git_head(ctx);
                let mut cmd = git_command(ctx);
                cmd.args(["commit", "-m", msg]);
                if let Some(a) = author {
                    cmd.arg(format!("--author={a}"));
                }
                let out = cmd.output();
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
                    Ok(o) => {
                        let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                        let msg = if stderr.is_empty() {
                            String::from_utf8_lossy(&o.stdout).trim().to_string()
                        } else {
                            stderr
                        };
                        ToolExecutionResult::err_structured("git_commit_failed", msg, None)
                    }
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

fn git_command(ctx: &ToolContext) -> Command {
    let mut cmd = Command::new("git");
    cmd.args([
        "-c",
        "core.fsmonitor=false",
        "-c",
        "diff.external=",
        "-c",
        "core.pager=cat",
    ])
    .env("GIT_TERMINAL_PROMPT", "0")
    .env("GIT_PAGER", "cat")
    .current_dir(&ctx.workspace_root);
    cmd
}

fn git_config_get(ctx: &ToolContext, key: &str) -> Option<String> {
    git_command(ctx)
        .args(["config", "--get", key])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn verify_git_identity(ctx: &ToolContext) -> Result<(), ToolExecutionResult> {
    let has_name = std::env::var("GIT_COMMITTER_NAME")
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
        || git_config_get(ctx, "user.name").is_some();
    let has_email = std::env::var("GIT_COMMITTER_EMAIL")
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
        || git_config_get(ctx, "user.email").is_some();
    if !has_name || !has_email {
        let mut missing = Vec::new();
        if !has_name {
            missing.push("user.name");
        }
        if !has_email {
            missing.push("user.email");
        }
        return Err(ToolExecutionResult::err_structured(
            "git_identity_missing",
            format!(
                "git commit requires configured identity ({}); set via `git config user.name` and `git config user.email`",
                missing.join(", ")
            ),
            Some(json!({"missing_config": missing})),
        ));
    }
    Ok(())
}

fn git_head(ctx: &ToolContext) -> Option<String> {
    git_command(ctx)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn run_git(ctx: &ToolContext, args: &[&str], label: &str) -> ToolExecutionResult {
    match git_command(ctx).args(args).output() {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout).into_owned();
            ToolExecutionResult::ok_structured(
                json!({"command": label, "output": stdout}),
                format!("Ran {label}"),
            )
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            let msg = if stderr.is_empty() {
                String::from_utf8_lossy(&o.stdout).trim().to_string()
            } else {
                stderr
            };
            ToolExecutionResult::err_structured("git_failed", msg, None)
        }
        Err(e) => ToolExecutionResult::err_structured("git_error", e.to_string(), None),
    }
}
