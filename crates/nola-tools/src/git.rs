// SPDX-License-Identifier: Apache-2.0
//! Native Git tool (`status`, `diff`, `log`, `show`, `blame`, `commit`,
//! `worktree_list`, `worktree_create`, `worktree_remove`, `worktree_integrate`).

use std::path::Path;
use std::process::Command;

use nola_types::{
    CapabilityDomain, ContentVersion, ResourceEffect, ResourceOperation, ToolExecutionResult,
    ToolSchema, ToolSpec, ToolTier, VersionTransition,
};
use serde_json::{json, Value};

use crate::file_pipeline::{path_within_workspace, resolve_path};
use crate::{ToolContext, ToolHandler};

const MAX_BLAME_LINES: usize = 500;

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
                description: "Inspect repository status/diff/log/show/blame or create commits/worktrees and integrate worktrees safely.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": [
                                "status",
                                "diff",
                                "log",
                                "show",
                                "blame",
                                "commit",
                                "worktree_list",
                                "worktree_create",
                                "worktree_remove",
                                "worktree_integrate"
                            ]
                        },
                        "message": { "type": "string", "description": "Commit message (for action=commit)" },
                        "author": { "type": "string", "description": "Optional commit author 'Name <email>' (for action=commit)" },
                        "target": { "type": "string", "description": "Commit ref (show), file path (blame), or worktree path (worktree_create/remove/integrate)" },
                        "path": { "type": "string", "description": "Optional file/directory filter for diff or log" },
                        "staged": { "type": "boolean", "description": "Diff staged changes (--cached) for action=diff" },
                        "stat_only": { "type": "boolean", "description": "Return compact --stat summary for action=diff" },
                        "branch": { "type": "string", "description": "Optional branch name for worktree_create or worktree_integrate" },
                        "force": { "type": "boolean", "description": "Force removal of dirty worktree for action=worktree_remove (default false)" },
                        "strategy": {
                            "type": "string",
                            "enum": ["merge", "cherry_pick"],
                            "description": "Integration strategy for worktree_integrate (default 'merge')"
                        },
                        "start_line": { "type": "integer", "description": "Optional 1-based start line for action=blame (default 1)" },
                        "end_line": { "type": "integer", "description": "Optional 1-based inclusive end line for action=blame (default start_line + 119, max 500 lines)" },
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
                let mut cmd_args: Vec<&str> = vec!["diff", "--no-ext-diff", "--no-textconv"];
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
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "blame requires 'target' file path",
                        None,
                    );
                };
                if let Err(e) = reject_flag_injection(path, "target") {
                    return e;
                }
                let start_line = args
                    .get("start_line")
                    .and_then(|v| v.as_u64())
                    .map(|n| (n as usize).max(1))
                    .unwrap_or(1);
                let raw_end = args
                    .get("end_line")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or_else(|| start_line.saturating_add(119));
                if raw_end < start_line {
                    return ToolExecutionResult::err_structured(
                        "invalid_param",
                        format!("blame end_line ({raw_end}) must be >= start_line ({start_line})"),
                        None,
                    );
                }
                let end_line = raw_end.min(start_line.saturating_add(MAX_BLAME_LINES - 1));
                let range_arg = format!("{start_line},{end_line}");
                run_git(ctx, &["blame", "-L", &range_arg, "--", path], "git blame")
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
                let force = args
                    .get("force")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let path_s = resolved.to_string_lossy().into_owned();
                if force {
                    run_git(
                        ctx,
                        &["worktree", "remove", "--force", "--", &path_s],
                        "git worktree remove",
                    )
                } else {
                    run_git(
                        ctx,
                        &["worktree", "remove", "--", &path_s],
                        "git worktree remove",
                    )
                }
            }
            "worktree_integrate" => worktree_integrate(ctx, &args),
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

fn worktree_integrate(ctx: &ToolContext, args: &Value) -> ToolExecutionResult {
    let target_opt = args
        .get("target")
        .or_else(|| args.get("worktree_path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let branch_opt = args
        .get("branch")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let strategy = args
        .get("strategy")
        .and_then(|v| v.as_str())
        .unwrap_or("merge");
    if !matches!(strategy, "merge" | "cherry_pick") {
        return ToolExecutionResult::err_structured(
            "invalid_param",
            format!("unsupported strategy '{strategy}'; expected 'merge' or 'cherry_pick'"),
            None,
        );
    }

    if let Some(t) = target_opt {
        if let Err(e) = reject_flag_injection(t, "target") {
            return e;
        }
    }
    if let Some(b) = branch_opt {
        if let Err(e) = reject_flag_injection(b, "branch") {
            return e;
        }
    }
    if target_opt.is_none() && branch_opt.is_none() {
        return ToolExecutionResult::err_structured(
            "missing_param",
            "worktree_integrate requires 'target' worktree path or 'branch'",
            None,
        );
    }

    // 1. Verify target (current workspace) working tree is completely clean
    //    (ignoring registered child worktree directories that live inside workspace_root).
    let registered_wt_rel: Vec<String> = git_command(ctx)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let root_canon = std::fs::canonicalize(&ctx.workspace_root)
                .unwrap_or_else(|_| ctx.workspace_root.clone());
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.strip_prefix("worktree "))
                .filter_map(|p| {
                    let wp = std::fs::canonicalize(p).unwrap_or_else(|_| Path::new(p).to_path_buf());
                    if wp == root_canon {
                        return None;
                    }
                    wp.strip_prefix(&root_canon)
                        .ok()
                        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                })
                .collect()
        })
        .unwrap_or_default();

    let status_out = match git_command(ctx).args(["status", "--porcelain"]).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|line| {
                if let Some(untracked) = line.strip_prefix("?? ") {
                    let trimmed = untracked.trim().trim_end_matches('/');
                    !registered_wt_rel
                        .iter()
                        .any(|wt| trimmed == wt || trimmed.starts_with(&format!("{wt}/")))
                } else {
                    !line.trim().is_empty()
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Ok(o) => {
            return ToolExecutionResult::err_structured(
                "git_failed",
                String::from_utf8_lossy(&o.stderr).trim().to_string(),
                None,
            )
        }
        Err(e) => return ToolExecutionResult::err_structured("git_error", e.to_string(), None),
    };
    if !status_out.is_empty() {
        return ToolExecutionResult::err_structured(
            "dirty_working_tree",
            "target working tree has uncommitted changes; commit or stash before worktree_integrate",
            Some(json!({"dirty_status": status_out})),
        );
    }

    // 2. Resolve source commit & verify worktree cleanliness when target path is given
    let (source_ref, source_commit) = if let Some(raw_wt) = target_opt {
        let wt_path = resolve_path(&ctx.workspace_root, raw_wt);
        if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
            && !path_within_workspace(&ctx.workspace_root, &wt_path)
        {
            return ToolExecutionResult::err_structured(
                "workspace_escape",
                format!("worktree target '{}' escapes workspace boundary", wt_path.display()),
                None,
            );
        }
        if !wt_path.exists() {
            return ToolExecutionResult::err_structured(
                "not_found",
                format!("worktree path does not exist: {}", wt_path.display()),
                None,
            );
        }
        let wt_status = match git_command_in(&wt_path).args(["status", "--porcelain"]).output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Ok(o) => {
                return ToolExecutionResult::err_structured(
                    "git_failed",
                    String::from_utf8_lossy(&o.stderr).trim().to_string(),
                    None,
                )
            }
            Err(e) => return ToolExecutionResult::err_structured("git_error", e.to_string(), None),
        };
        if !wt_status.is_empty() {
            return ToolExecutionResult::err_structured(
                "dirty_worktree",
                format!(
                    "source worktree '{}' has uncommitted changes; commit them before integrating",
                    wt_path.display()
                ),
                Some(json!({"worktree": wt_path.display().to_string(), "dirty_status": wt_status})),
            );
        }
        let wt_head = match git_command_in(&wt_path).args(["rev-parse", "HEAD"]).output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Ok(o) => {
                return ToolExecutionResult::err_structured(
                    "git_failed",
                    String::from_utf8_lossy(&o.stderr).trim().to_string(),
                    None,
                )
            }
            Err(e) => return ToolExecutionResult::err_structured("git_error", e.to_string(), None),
        };
        let label = branch_opt
            .map(ToString::to_string)
            .or_else(|| {
                git_command_in(&wt_path)
                    .args(["symbolic-ref", "--short", "HEAD"])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| wt_head.clone());
        (label, wt_head)
    } else {
        let br = branch_opt.unwrap();
        let sha = match git_command(ctx).args(["rev-parse", "--verify", br]).output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Ok(o) => {
                return ToolExecutionResult::err_structured(
                    "invalid_ref",
                    format!(
                        "cannot resolve branch '{br}': {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    ),
                    None,
                )
            }
            Err(e) => return ToolExecutionResult::err_structured("git_error", e.to_string(), None),
        };
        (br.to_string(), sha)
    };

    // 3. Baseline commit & merge-base check
    let Some(before_head) = git_head(ctx) else {
        return ToolExecutionResult::err_structured(
            "git_failed",
            "cannot resolve current HEAD commit in workspace",
            None,
        );
    };
    let merge_base = git_command(ctx)
        .args(["merge-base", &before_head, &source_commit])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    if before_head == source_commit || merge_base.as_deref() == Some(source_commit.as_str()) {
        return ToolExecutionResult::ok_structured(
            json!({
                "integrated": false,
                "already_up_to_date": true,
                "strategy": strategy,
                "baseline_commit": before_head,
                "source_commit": source_commit,
                "merge_base": merge_base
            }),
            format!("Branch/worktree '{source_ref}' is already integrated into HEAD"),
        );
    }

    if let Err(err_res) = verify_git_identity(ctx) {
        return err_res;
    }

    // 4. Execute merge or cherry-pick with explicit conflict refusal & clean rollback
    let (ok, output_text) = if strategy == "cherry_pick" {
        let Some(base) = merge_base.as_deref() else {
            return ToolExecutionResult::err_structured(
                "unrelated_histories",
                "cannot cherry-pick: no common merge-base between HEAD and source",
                Some(json!({"baseline_commit": before_head, "source_commit": source_commit})),
            );
        };
        let range = format!("{base}..{source_commit}");
        let rev_out = match git_command(ctx)
            .args(["rev-list", "--reverse", &range])
            .output()
        {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
            Ok(o) => {
                return ToolExecutionResult::err_structured(
                    "git_failed",
                    String::from_utf8_lossy(&o.stderr).trim().to_string(),
                    None,
                )
            }
            Err(e) => return ToolExecutionResult::err_structured("git_error", e.to_string(), None),
        };
        let commits: Vec<&str> = rev_out.lines().map(str::trim).filter(|s| !s.is_empty()).collect();
        if commits.is_empty() {
            return ToolExecutionResult::ok_structured(
                json!({
                    "integrated": false,
                    "already_up_to_date": true,
                    "strategy": strategy,
                    "baseline_commit": before_head,
                    "source_commit": source_commit
                }),
                "No commits to cherry-pick",
            );
        }
        let mut cmd = git_command(ctx);
        cmd.arg("cherry-pick").args(&commits);
        match cmd.output() {
            Ok(o) if o.status.success() => (true, String::from_utf8_lossy(&o.stdout).trim().to_string()),
            Ok(o) => {
                let detail = format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&o.stdout).trim(),
                    String::from_utf8_lossy(&o.stderr).trim()
                )
                .trim()
                .to_string();
                let _ = git_command(ctx).args(["cherry-pick", "--abort"]).output();
                // Guarantee HEAD is cleanly restored to baseline even after multi-commit partial pick
                if git_head(ctx).as_deref() != Some(before_head.as_str()) {
                    let _ = git_command(ctx)
                        .args(["reset", "--hard", &before_head])
                        .output();
                }
                (false, detail)
            }
            Err(e) => return ToolExecutionResult::err_structured("git_error", e.to_string(), None),
        }
    } else {
        let mut cmd = git_command(ctx);
        cmd.args(["merge", "--no-edit", &source_commit]);
        match cmd.output() {
            Ok(o) if o.status.success() => (true, String::from_utf8_lossy(&o.stdout).trim().to_string()),
            Ok(o) => {
                let detail = format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&o.stdout).trim(),
                    String::from_utf8_lossy(&o.stderr).trim()
                )
                .trim()
                .to_string();
                let _ = git_command(ctx).args(["merge", "--abort"]).output();
                if git_head(ctx).as_deref() != Some(before_head.as_str()) {
                    let _ = git_command(ctx)
                        .args(["reset", "--hard", &before_head])
                        .output();
                }
                (false, detail)
            }
            Err(e) => return ToolExecutionResult::err_structured("git_error", e.to_string(), None),
        }
    };

    if !ok {
        return ToolExecutionResult::err_structured(
            "integration_conflict",
            format!(
                "worktree_integrate ({strategy}) refused due to conflict and cleanly rolled back to {before_head}"
            ),
            Some(json!({
                "rolled_back": true,
                "strategy": strategy,
                "baseline_commit": before_head,
                "source_ref": source_ref,
                "source_commit": source_commit,
                "merge_base": merge_base,
                "detail": output_text
            })),
        );
    }

    let after_head = git_head(ctx).unwrap_or_else(|| before_head.clone());
    let effect = ResourceEffect {
        uri: ctx.workspace_root.display().to_string(),
        operation: ResourceOperation::Commit,
        transition: VersionTransition::Modified {
            before: ContentVersion::GitCommit(before_head.clone()),
            after: ContentVersion::GitCommit(after_head.clone()),
        },
    };
    ToolExecutionResult::ok_structured(
        json!({
            "integrated": true,
            "strategy": strategy,
            "baseline_commit": before_head,
            "source_ref": source_ref,
            "source_commit": source_commit,
            "merge_base": merge_base,
            "commit": after_head,
            "stdout": output_text
        }),
        format!("Integrated '{source_ref}' via {strategy} -> {after_head}"),
    )
    .with_effects(vec![effect])
}

fn git_command(ctx: &ToolContext) -> Command {
    git_command_in(&ctx.workspace_root)
}

fn git_command_in(dir: &Path) -> Command {
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
    .current_dir(dir);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_remove_respects_force_flag() {
        let dir = tempfile::tempdir().unwrap();
        let init = Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(init.success());
        Command::new("git")
            .args(["config", "user.name", "Nola Test"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@nola.local"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::fs::write(dir.path().join("README.md"), "hello\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-q", "-m", "init"])
            .current_dir(dir.path())
            .status()
            .unwrap();

        let ctx = ToolContext::new(dir.path());
        let tool = GitTool;

        // 1. Clean worktree removes cleanly with force: false
        let c1 = tool.execute(
            &ctx,
            json!({"action": "worktree_create", "target": "wt-clean", "branch": "br-clean"}),
        );
        assert!(!c1.is_error, "{:?}", c1.payload);
        let r1 = tool.execute(
            &ctx,
            json!({"action": "worktree_remove", "target": "wt-clean", "force": false}),
        );
        assert!(!r1.is_error, "clean worktree remove failed: {:?}", r1.payload);
        assert!(!dir.path().join("wt-clean").exists());

        // 2. Dirty worktree is refused when force is false or omitted, and removed when force: true
        let c2 = tool.execute(
            &ctx,
            json!({"action": "worktree_create", "target": "wt-dirty", "branch": "br-dirty"}),
        );
        assert!(!c2.is_error, "{:?}", c2.payload);
        std::fs::write(dir.path().join("wt-dirty/README.md"), "uncommitted\n").unwrap();

        let r2_default = tool.execute(
            &ctx,
            json!({"action": "worktree_remove", "target": "wt-dirty"}),
        );
        assert!(r2_default.is_error, "dirty worktree must not be force-removed by default");
        assert!(dir.path().join("wt-dirty").exists());

        let r2_false = tool.execute(
            &ctx,
            json!({"action": "worktree_remove", "target": "wt-dirty", "force": false}),
        );
        assert!(r2_false.is_error, "dirty worktree must not be removed with force: false");
        assert!(dir.path().join("wt-dirty").exists());

        let r2_force = tool.execute(
            &ctx,
            json!({"action": "worktree_remove", "target": "wt-dirty", "force": true}),
        );
        assert!(!r2_force.is_error, "force: true should remove dirty worktree: {:?}", r2_force.payload);
        assert!(!dir.path().join("wt-dirty").exists());
    }
}
