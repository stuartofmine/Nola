fn diff_review(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = arg_string(args, "root").or_else(|| {
        ctx.allowed_dirs
            .first()
            .map(|path| path.display().to_string())
    });
    let Some(root_arg) = root_arg else {
        return error("root is required when no allowed workspace is configured".to_string());
    };
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let git_root = tool_try!(run_git(&root, &["rev-parse", "--show-toplevel"]));
    if !git_root.status.success() {
        return error_with_display(
            format!("Diff review failed:\n{}", command_output_text(&git_root)),
            "Diff review failed".to_string(),
        );
    }
    let paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_project_file(&root, path, &ctx.allowed_dirs));
                if !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    let include_staged = match args.get("include_staged") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_staged must be a boolean".to_string()),
    };
    let max_findings = tool_try!(validate_integer_arg(args, "max_findings", 50, 1, 200)) as usize;
    let (findings, scanned, truncated) =
        diff_review_findings(&root, &paths, include_staged, max_findings);
    ok(
        [
            format!("# Diff review: {}", root.display()),
            format!(
                "git_root: {}",
                String::from_utf8_lossy(&git_root.stdout).trim()
            ),
            format!("scanned_diff_lines: {scanned}"),
            "complete_diff_traversal: true".to_string(),
            "review_scope: deterministic scan of every added and deleted diff line".to_string(),
            String::new(),
            "## Deterministic findings".to_string(),
            if findings.is_empty() {
                "No deterministic hunk-level findings. This does not replace semantic review of the changed code.".to_string()
            } else {
                findings.join("\n")
            },
            if truncated {
                format!("\n(truncated to {max_findings} findings)")
            } else {
                String::new()
            },
        ]
        .join("\n"),
        format!("Diff review ({} findings)", findings.len()),
    )
}

fn change_review(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let max_diagnostics = tool_try!(validate_integer_arg(
        args,
        "max_diagnostics",
        50,
        1,
        MAX_CODE_DIAGNOSTICS
    )) as usize;
    let git_root = tool_try!(run_git(&root, &["rev-parse", "--show-toplevel"]));
    if !git_root.status.success() {
        return error_with_display(
            format!("Change review failed:\n{}", command_output_text(&git_root)),
            "Change review failed".to_string(),
        );
    }
    let status = tool_try!(run_git(&root, &["status", "--short", "--", "."]));
    if !status.status.success() {
        return error_with_display(
            format!("Change review failed:\n{}", command_output_text(&status)),
            "Change review failed".to_string(),
        );
    }
    let unstaged_short = tool_try!(run_git(&root, &["diff", "--shortstat", "--", "."]));
    let staged_short = tool_try!(run_git(
        &root,
        &["diff", "--cached", "--shortstat", "--", "."]
    ));
    let unstaged_stat = tool_try!(run_git(&root, &["diff", "--stat", "--", "."]));
    let staged_stat = tool_try!(run_git(&root, &["diff", "--cached", "--stat", "--", "."]));
    let changed = git_changed_files(&root);
    let (diagnostics, commands, related_tests, target_paths) =
        validation_plan_items(&root, &changed, true);
    let (format_commands, format_notes, _format_targets) = format_plan_items(&root, &target_paths);
    let diagnostic_files = target_paths
        .iter()
        .filter(|path| path.is_file() && is_text_source(path))
        .cloned()
        .collect::<Vec<_>>();
    let (issues, _scanned) = scan_code_diagnostics(&diagnostic_files, max_diagnostics);
    let risks = change_review_risks(&target_paths);
    let dependency_impact = dependency_impact(&root, &target_paths, 20);
    let (changed_symbols, changed_symbol_scanned, changed_symbol_untracked) =
        changed_symbols_for_review(&root, 80);
    let diff_stat = [
        String::from_utf8_lossy(&unstaged_stat.stdout)
            .trim_end()
            .to_string(),
        String::from_utf8_lossy(&staged_stat.stdout)
            .trim_end()
            .to_string(),
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    let suggested = diagnostics
        .iter()
        .chain(commands.iter())
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>();
    ok(
        [
            format!("# Change review: {}", root.display()),
            format!(
                "git_root: {}",
                String::from_utf8_lossy(&git_root.stdout).trim()
            ),
            String::new(),
            "## Changed files".to_string(),
            {
                let text = String::from_utf8_lossy(&status.stdout).trim().to_string();
                if text.is_empty() {
                    "(clean for this root)".to_string()
                } else {
                    text
                }
            },
            String::new(),
            "## Diff summary".to_string(),
            format!("unstaged: {}", {
                let text = String::from_utf8_lossy(&unstaged_short.stdout)
                    .trim()
                    .to_string();
                if text.is_empty() {
                    "(none)".to_string()
                } else {
                    text
                }
            }),
            format!("staged: {}", {
                let text = String::from_utf8_lossy(&staged_short.stdout)
                    .trim()
                    .to_string();
                if text.is_empty() {
                    "(none)".to_string()
                } else {
                    text
                }
            }),
            String::new(),
            "## Diff stat".to_string(),
            if diff_stat.is_empty() {
                "(none)".to_string()
            } else {
                diff_stat
            },
            String::new(),
            "## Diagnostics".to_string(),
            if issues.is_empty() {
                "(none)".to_string()
            } else {
                issues.join("\n")
            },
            String::new(),
            "## Risk hints".to_string(),
            if risks.is_empty() {
                "(none detected)".to_string()
            } else {
                risks
                    .iter()
                    .map(|risk| format!("- {risk}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Changed symbols".to_string(),
            format!("scanned_files: {changed_symbol_scanned}"),
            format!("untracked_files: {changed_symbol_untracked}"),
            if changed_symbols.is_empty() {
                "(none)".to_string()
            } else {
                changed_symbols.join("\n")
            },
            String::new(),
            "## Dependency impact".to_string(),
            if dependency_impact.is_empty() {
                "(none detected)".to_string()
            } else {
                dependency_impact.join("\n")
            },
            String::new(),
            "## Suggested formatting".to_string(),
            if format_commands.is_empty() {
                "(none inferred)".to_string()
            } else {
                format_commands
                    .iter()
                    .map(|command| format!("- {command}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Formatting notes".to_string(),
            if format_notes.is_empty() {
                "(none)".to_string()
            } else {
                format_notes
                    .iter()
                    .map(|note| format!("- {note}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Related tests".to_string(),
            if related_tests.is_empty() {
                "(none)".to_string()
            } else {
                related_tests
                    .iter()
                    .map(|path| format!("- {}", path.display()))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Suggested validation".to_string(),
            if suggested.is_empty() {
                "(none inferred)".to_string()
            } else {
                suggested.join("\n")
            },
        ]
        .join("\n"),
        format!(
            "Change review ({} files, {} risks, {} diagnostics)",
            target_paths.len(),
            risks.len(),
            issues.len()
        ),
    )
}

fn verification_review(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let max_findings = tool_try!(validate_integer_arg(args, "max_findings", 20, 1, 200)) as usize;
    let max_diagnostics = tool_try!(validate_integer_arg(
        args,
        "max_diagnostics",
        50,
        1,
        MAX_CODE_DIAGNOSTICS
    )) as usize;
    let git_root = tool_try!(run_git(&root, &["rev-parse", "--show-toplevel"]));
    if !git_root.status.success() {
        return error_with_display(
            format!(
                "Verification review failed:\n{}",
                command_output_text(&git_root)
            ),
            "Verification review failed".to_string(),
        );
    }
    let status = tool_try!(run_git(&root, &["status", "--short", "--", "."]));
    if !status.status.success() {
        return error_with_display(
            format!(
                "Verification review failed:\n{}",
                command_output_text(&status)
            ),
            "Verification review failed".to_string(),
        );
    }
    let mut target_paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                if path.trim().is_empty() {
                    continue;
                }
                let input = Path::new(path);
                let candidate_arg = if input.is_absolute() {
                    path.to_string()
                } else {
                    root.join(input).display().to_string()
                };
                let resolved = tool_try!(resolve_allowed(&candidate_arg, &ctx.allowed_dirs, true));
                if !resolved.starts_with(&root) {
                    return error(format!(
                        "path is outside verification_review root: {}",
                        resolved.display()
                    ));
                }
                if !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    let changed_paths = git_changed_files(&root);
    let scoped_review = !target_paths.is_empty();
    if !scoped_review {
        target_paths = changed_paths.clone();
    }
    let outside_review = changed_paths.iter().filter(|changed| {
        !target_paths.iter().any(|selected| *changed == selected || changed.starts_with(selected))
    }).map(|path| path.strip_prefix(&root).unwrap_or(path).display().to_string())
        .collect::<Vec<_>>();
    let (diagnostics, commands, related_tests, validation_targets) =
        validation_plan_items(&root, &target_paths, true);
    let (format_commands, _format_notes, _format_targets) =
        format_plan_items(&root, &validation_targets);
    let diagnostic_files = validation_targets
        .iter()
        .filter(|path| path.is_file() && is_text_source(path))
        .cloned()
        .collect::<Vec<_>>();
    let (issues, scanned) = scan_code_diagnostics(&diagnostic_files, max_diagnostics);
    let risks = change_review_risks(&validation_targets);
    let (mut findings, mut truncated) =
        verification_findings(&validation_targets, &issues, &risks, max_findings);
    let (diff_findings, diff_scanned, diff_truncated) =
        diff_review_findings(&root, &validation_targets, true, max_findings);
    for finding in &diff_findings {
        if !findings.contains(finding) {
            findings.push(finding.clone());
        }
    }
    truncated = truncated || diff_truncated || findings.len() > max_findings;
    findings.truncate(max_findings);
    let verdict = verification_verdict(&findings);
    let source_changed = validation_targets.iter().any(|path| is_code_source(path));
    let tests_changed = validation_targets
        .iter()
        .any(|path| is_probable_test_file(path));
    let mut test_gaps = Vec::new();
    if source_changed && !tests_changed {
        test_gaps.push("source files changed without tests changed in this review scope".to_string());
    }
    if related_tests.is_empty() && source_changed {
        test_gaps.push(
            "no related tests were inferred; manually identify coverage for touched behavior"
                .to_string(),
        );
    }
    let suggested_validation = diagnostics
        .iter()
        .chain(format_commands.iter())
        .chain(commands.iter())
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>();
    ok(
        [
            format!("# Verification review: {}", root.display()),
            format!(
                "git_root: {}",
                String::from_utf8_lossy(&git_root.stdout).trim()
            ),
            format!("verdict: {verdict}"),
            format!("review_scope: {}", if scoped_review { "selected_paths" } else { "all_changed_paths" }),
            "This is a scoped static review and validation plan, not an executed build or whole-delivery acceptance. Check the actual delivery file set, including tests/scratch files inside delivery directories; do not delete files automatically.".to_string(),
            "## Reviewed paths".to_string(),
            target_paths.iter().map(|path| format!("- {}", path.strip_prefix(&root).unwrap_or(path).display())).collect::<Vec<_>>().join("\n"),
            "## Changed paths outside review".to_string(),
            if outside_review.is_empty() { "(none detected; ignored/untracked directory contents may require explicit delivery inspection)".to_string() } else { outside_review.iter().map(|path| format!("- {path}")).collect::<Vec<_>>().join("\n") },
            format!("scanned_files: {scanned}"),
            format!("scanned_diff_lines: {diff_scanned}"),
            String::new(),
            "## Findings".to_string(),
            if findings.is_empty() {
                "No deterministic findings.".to_string()
            } else {
                findings.join("\n")
            },
            if truncated {
                format!("\n(truncated to {max_findings} findings)")
            } else {
                String::new()
            },
            "## Test gaps".to_string(),
            if test_gaps.is_empty() {
                "(none detected)".to_string()
            } else {
                test_gaps
                    .iter()
                    .map(|gap| format!("- {gap}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Changed files".to_string(),
            {
                let text = String::from_utf8_lossy(&status.stdout).trim().to_string();
                if text.is_empty() {
                    "(clean for this root)".to_string()
                } else {
                    text
                }
            },
            String::new(),
            "## Suggested validation".to_string(),
            if suggested_validation.is_empty() {
                "(none inferred)".to_string()
            } else {
                suggested_validation.join("\n")
            },
        ]
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n"),
        format!(
            "Verification review ({verdict}, {} findings)",
            findings.len()
        ),
    )
}

fn handoff_context(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let task = match args.get("task") {
        Some(Value::String(value)) => value.trim().to_string(),
        Some(Value::Null) | None => String::new(),
        _ => return error("task must be a string".to_string()),
    };
    let max_plans = tool_try!(validate_integer_arg(args, "max_plans", 5, 0, 20)) as usize;
    let max_diagnostics = tool_try!(validate_integer_arg(
        args,
        "max_diagnostics",
        50,
        1,
        MAX_CODE_DIAGNOSTICS
    )) as usize;
    let max_changed_symbols = tool_try!(validate_integer_arg(
        args,
        "max_changed_symbols",
        80,
        1,
        500
    )) as usize;
    let mut target_paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                if path.trim().is_empty() {
                    continue;
                }
                let input = Path::new(path);
                let candidate_arg = if input.is_absolute() {
                    path.to_string()
                } else {
                    root.join(input).display().to_string()
                };
                let resolved = tool_try!(resolve_allowed(&candidate_arg, &ctx.allowed_dirs, true));
                if !resolved.starts_with(&root) {
                    return error(format!(
                        "path is outside handoff_context root: {}",
                        resolved.display()
                    ));
                }
                if !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    if target_paths.is_empty() {
        target_paths = git_changed_files(&root);
    }
    let (diagnostics, commands, related_tests, validation_targets) =
        validation_plan_items(&root, &target_paths, true);
    let (format_commands, _format_notes, _format_targets) =
        format_plan_items(&root, &validation_targets);
    let diagnostic_files = validation_targets
        .iter()
        .filter(|path| path.is_file() && is_text_source(path))
        .cloned()
        .collect::<Vec<_>>();
    let (issues, _scanned) = scan_code_diagnostics(&diagnostic_files, max_diagnostics);
    let risks = change_review_risks(&validation_targets);
    let (findings, truncated) =
        verification_findings(&validation_targets, &issues, &risks, max_diagnostics);
    let verdict = verification_verdict(&findings);
    let plans = handoff_plan_entries(&root, max_plans);
    let (changed_symbols, changed_symbol_scanned, changed_symbol_untracked) =
        changed_symbols_for_review(&root, max_changed_symbols);

    let mut git_root_text = "(none)".to_string();
    let mut branch_text = "(unknown)".to_string();
    let mut status_text = "(not a git worktree)".to_string();
    let mut recent_commit = "(none)".to_string();
    if let Ok(git_root) = run_git(&root, &["rev-parse", "--show-toplevel"]) {
        if git_root.status.success() {
            git_root_text = String::from_utf8_lossy(&git_root.stdout).trim().to_string();
            if let Ok(branch) = run_git(&root, &["branch", "--show-current"]) {
                let text = String::from_utf8_lossy(&branch.stdout).trim().to_string();
                if branch.status.success() && !text.is_empty() {
                    branch_text = text;
                }
            }
            if let Ok(status) = run_git(&root, &["status", "--short", "--", "."]) {
                let text = String::from_utf8_lossy(&status.stdout).trim().to_string();
                if status.status.success() {
                    status_text = if text.is_empty() {
                        "(clean for this root)".to_string()
                    } else {
                        text
                    };
                }
            }
            if let Ok(log) = run_git(&root, &["log", "-1", "--pretty=format:%h %s"]) {
                let text = String::from_utf8_lossy(&log.stdout).trim().to_string();
                if log.status.success() && !text.is_empty() {
                    recent_commit = text;
                }
            }
        }
    }

    let target_lines = if validation_targets.is_empty() {
        "(none)".to_string()
    } else {
        validation_targets
            .iter()
            .map(|path| format!("- {}", relative_display(path, &root)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let suggested_validation = diagnostics
        .iter()
        .chain(format_commands.iter())
        .chain(commands.iter())
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>();
    ok(
        [
            format!("# Handoff context: {}", root.display()),
            format!(
                "task: {}",
                if task.is_empty() {
                    "(not provided)"
                } else {
                    task.as_str()
                }
            ),
            format!("git_root: {git_root_text}"),
            format!("branch: {branch_text}"),
            format!("recent_commit: {recent_commit}"),
            format!("verification_verdict: {verdict}"),
            String::new(),
            "## Current git state".to_string(),
            status_text,
            String::new(),
            "## Persistent plans".to_string(),
            if plans.is_empty() {
                "(none)".to_string()
            } else {
                plans.join("\n")
            },
            String::new(),
            "## Target files".to_string(),
            target_lines,
            String::new(),
            "## Changed symbols".to_string(),
            format!("scanned_files: {changed_symbol_scanned}"),
            format!("untracked_files: {changed_symbol_untracked}"),
            if changed_symbols.is_empty() {
                "(none)".to_string()
            } else {
                changed_symbols.join("\n")
            },
            String::new(),
            "## Findings to preserve".to_string(),
            if findings.is_empty() {
                "No deterministic findings.".to_string()
            } else {
                findings.join("\n")
            },
            if truncated {
                format!("\n(findings truncated to {max_diagnostics})")
            } else {
                String::new()
            },
            "## Related tests".to_string(),
            if related_tests.is_empty() {
                "(none)".to_string()
            } else {
                related_tests
                    .iter()
                    .map(|path| format!("- {}", relative_display(path, &root)))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Suggested validation".to_string(),
            if suggested_validation.is_empty() {
                "(none inferred)".to_string()
            } else {
                suggested_validation.join("\n")
            },
            String::new(),
            "## Resume checklist".to_string(),
            "- Re-read the persistent plan or target files before editing further.".to_string(),
            "- Review current git state and changed symbols so user changes are preserved."
                .to_string(),
            "- Run or explain the suggested validation before final response.".to_string(),
        ]
        .join("\n"),
        format!(
            "Handoff context ({} files, verdict={verdict})",
            validation_targets.len()
        ),
    )
}

fn changed_symbols(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let paths = match args.get("paths") {
        Some(Value::Array(values)) => {
            let mut out = Vec::new();
            for value in values {
                let Some(path) = value.as_str() else {
                    return error("paths must contain strings".to_string());
                };
                let resolved = tool_try!(resolve_allowed(path, &ctx.allowed_dirs, true));
                if !resolved.starts_with(&root) {
                    return error(format!(
                        "path is outside changed_symbols root: {}",
                        resolved.display()
                    ));
                }
                if !out.contains(&resolved) {
                    out.push(resolved);
                }
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return error("paths must be a list".to_string()),
    };
    let include_untracked = match args.get("include_untracked") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_untracked must be a boolean".to_string()),
    };
    let max_entries = tool_try!(validate_integer_arg(args, "max_entries", 200, 1, 2_000)) as usize;

    let git_root_output = tool_try!(run_git(&root, &["rev-parse", "--show-toplevel"]));
    if !git_root_output.status.success() {
        return error_with_display(
            format!(
                "Changed symbols failed:\n{}",
                command_output_text(&git_root_output)
            ),
            "Changed symbols failed".to_string(),
        );
    }
    let git_root_text = String::from_utf8_lossy(&git_root_output.stdout)
        .trim()
        .to_string();
    let git_root = PathBuf::from(&git_root_text)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&git_root_text));
    let mut pathspecs = git_pathspecs(&git_root, &paths);
    if pathspecs.is_empty() {
        if let Ok(root_scope) = root.strip_prefix(&git_root) {
            let root_scope = path_slash(root_scope);
            if !root_scope.is_empty() {
                pathspecs.push(root_scope);
            }
        }
    }
    let unstaged_args = git_args_with_pathspecs(&["diff", "--unified=0"], &pathspecs);
    let staged_args = git_args_with_pathspecs(&["diff", "--cached", "--unified=0"], &pathspecs);
    let unstaged = tool_try!(run_git_strings(&git_root, &unstaged_args));
    let staged = tool_try!(run_git_strings(&git_root, &staged_args));
    if !unstaged.status.success() || !staged.status.success() {
        return error_with_display(
            format!(
                "Changed symbols failed:\n{}{}",
                command_output_text(&unstaged),
                command_output_text(&staged)
            ),
            "Changed symbols failed".to_string(),
        );
    }

    let mut changed_lines = parse_diff_changed_lines(&String::from_utf8_lossy(&unstaged.stdout));
    merge_changed_lines(
        &mut changed_lines,
        parse_diff_changed_lines(&String::from_utf8_lossy(&staged.stdout)),
    );

    let mut untracked_count = 0;
    if include_untracked {
        for path in git_untracked_files(&git_root, &pathspecs) {
            let Ok(rel) = path.strip_prefix(&git_root).map(path_slash) else {
                continue;
            };
            let line_count = fs::read_to_string(&path)
                .map(|text| text.lines().count().max(1))
                .unwrap_or(1);
            let entry = changed_lines.entry(rel).or_default();
            for line_no in 1..=line_count {
                if !entry.contains(&line_no) {
                    entry.push(line_no);
                }
            }
            untracked_count += 1;
        }
    }

    let (entries, scanned_files) = changed_symbol_entries(&git_root, &changed_lines, max_entries);
    let scope = if paths.is_empty() {
        ".".to_string()
    } else {
        paths
            .iter()
            .filter_map(|path| path.strip_prefix(&git_root).ok())
            .map(path_slash)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut lines = vec![
        format!("# Changed symbols: {}", root.display()),
        format!("git_root: {}", git_root.display()),
        format!("scope: {scope}"),
        format!("scanned_files: {scanned_files}"),
        format!("untracked_files: {untracked_count}"),
        String::new(),
        "## Symbols".to_string(),
        if entries.is_empty() {
            "(none)".to_string()
        } else {
            entries.join("\n")
        },
    ];
    if entries.len() >= max_entries {
        lines.push("...[symbols truncated]".to_string());
    }
    ok(
        lines.join("\n"),
        format!(
            "Changed symbols ({} symbol{})",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" }
        ),
    )
}

fn run_git(root: &Path, args: &[&str]) -> Result<std::process::Output, ToolOutcome> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .spawn()
        .map_err(|err| error(format!("git failed to start: {err}")))?;
    let cancellation = current_execution_control().unwrap_or_default();
    wait_for_child_output(child, &cancellation, Duration::from_secs(60), "git command")
}

#[allow(dead_code)]
fn apply_exact_edit(
    source: &str,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
) -> Result<(String, usize), ToolOutcome> {
    let old_text = convert_line_endings_like(source, old_text);
    let new_text = convert_line_endings_like(source, new_text);
    let count = source.matches(&old_text).count();
    if count == 0 {
        if !replace_all {
            let fuzzy = match find_unique_line_trimmed_match(source, &old_text, &new_text) {
                Ok(value) => value,
                Err(outcome) => return Err(outcome),
            };
            if let Some((matched, replacement)) = fuzzy {
                return Ok((source.replacen(&matched, &replacement, 1), 1));
            }
        }
        return Err(error("old_text was not found".to_string()));
    }
    if count > 1 && !replace_all {
        return Err(error(format!(
            "old_text matched {count} times; set replace_all=true or provide more context"
        )));
    }
    let updated = if replace_all {
        source.replace(&old_text, &new_text)
    } else {
        source.replacen(&old_text, &new_text, 1)
    };
    Ok((updated, count))
}

fn convert_line_endings_like(source: &str, text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if source.contains("\r\n") {
        normalized.replace('\n', "\r\n")
    } else {
        normalized
    }
}

#[allow(dead_code)]
fn line_body(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

#[allow(dead_code)]
fn line_ending(line: &str) -> &'static str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else if line.ends_with('\r') {
        "\r"
    } else {
        ""
    }
}

#[allow(dead_code)]
fn single_line_indent_preserving_replacement(
    matched: &str,
    old_text: &str,
    new_text: &str,
) -> Option<String> {
    if old_text.contains('\n')
        || old_text.contains('\r')
        || new_text.contains('\n')
        || new_text.contains('\r')
        || new_text.is_empty()
        || old_text.trim() != old_text
        || new_text.trim() != new_text
    {
        return None;
    }
    let body = line_body(matched);
    let leading_len = body.len() - body.trim_start().len();
    Some(format!(
        "{}{}{}",
        &body[..leading_len],
        new_text,
        line_ending(matched)
    ))
}

#[allow(dead_code)]
fn find_unique_line_trimmed_match(
    source: &str,
    old_text: &str,
    new_text: &str,
) -> Result<Option<(String, String)>, ToolOutcome> {
    let search_lines = old_text.lines().collect::<Vec<_>>();
    if search_lines.is_empty() {
        return Ok(None);
    }
    let source_lines = source.split_inclusive('\n').collect::<Vec<_>>();
    if search_lines.len() > source_lines.len() {
        return Ok(None);
    }
    let mut candidates: Vec<String> = Vec::new();
    for start in 0..=source_lines.len() - search_lines.len() {
        let window = &source_lines[start..start + search_lines.len()];
        let matched = window
            .iter()
            .zip(search_lines.iter())
            .all(|(line, wanted)| line_body(line).trim() == wanted.trim());
        if matched {
            let candidate = window.join("");
            candidates.push(candidate);
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    if candidates.len() > 1 {
        return Err(error(
            "old_text matched multiple fuzzy candidates; provide more surrounding context"
                .to_string(),
        ));
    }
    let matched = candidates.remove(0);
    let replacement = single_line_indent_preserving_replacement(&matched, old_text, new_text)
        .unwrap_or_else(|| new_text.to_string());
    Ok(Some((matched, replacement)))
}

fn collect_file_tree(
    root: &Path,
    directory: &Path,
    depth: usize,
    max_depth: usize,
    max_entries: usize,
    include_files: bool,
    lines: &mut Vec<String>,
    emitted: &mut usize,
) -> bool {
    if depth >= max_depth {
        return true;
    }
    let Ok(read_dir) = fs::read_dir(directory) else {
        return true;
    };
    let mut entries = read_dir
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        let left_is_file = left.is_file();
        let right_is_file = right.is_file();
        left_is_file
            .cmp(&right_is_file)
            .then_with(|| left.file_name().cmp(&right.file_name()))
    });
    for path in entries {
        if path.is_dir() {
            if path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(should_skip_dir)
            {
                continue;
            }
        } else if !include_files {
            continue;
        }
        *emitted += 1;
        if *emitted > max_entries {
            return false;
        }
        let rel_depth = path
            .strip_prefix(root)
            .map(|rel| rel.components().count())
            .unwrap_or(depth + 1);
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy())
            .unwrap_or_default();
        lines.push(format!(
            "{}- {}{}",
            "  ".repeat(rel_depth),
            name,
            if path.is_dir() { "/" } else { "" }
        ));
        if path.is_dir()
            && !collect_file_tree(
                root,
                &path,
                depth + 1,
                max_depth,
                max_entries,
                include_files,
                lines,
                emitted,
            )
        {
            return false;
        }
    }
    true
}

fn file_tree(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_discovery_root(&root_arg, ctx));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let max_depth = tool_try!(validate_integer_arg(args, "max_depth", 3, 1, 12)) as usize;
    let max_entries = tool_try!(validate_integer_arg(args, "max_entries", 200, 1, 2_000)) as usize;
    let include_files = match args.get("include_files") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_files must be a boolean".to_string()),
    };
    let mut lines = vec![format!(
        "- {}",
        root.file_name()
            .map(|value| value.to_string_lossy())
            .unwrap_or_else(|| root.as_os_str().to_string_lossy())
    )];
    let mut emitted = 0usize;
    let complete = collect_file_tree(
        &root,
        &root,
        0,
        max_depth,
        max_entries,
        include_files,
        &mut lines,
        &mut emitted,
    );
    if !complete {
        lines.push("...[entry limit reached]".to_string());
    }
    ok(
        format!("# File tree: {}\n{}", root.display(), lines.join("\n")),
        format!(
            "File tree {} ({} entries)",
            root.display(),
            emitted.min(max_entries)
        ),
    )
}

fn find_files(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let query = tool_try!(required_string(args, "query"));
    let root = tool_try!(resolve_discovery_root(&root_arg, ctx));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let include_dirs = match args.get("include_dirs") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => false,
        _ => return error("include_dirs must be a boolean".to_string()),
    };
    let max_results = tool_try!(validate_integer_arg(args, "max_results", 100, 1, 2_000)) as usize;
    let max_scanned_entries = tool_try!(validate_integer_arg(
        args,
        "max_scanned_entries",
        20_000,
        1,
        250_000
    )) as usize;
    let max_duration_ms = tool_try!(validate_integer_arg(
        args,
        "max_duration_ms",
        3_000,
        1,
        30_000
    )) as u64;
    let started = Instant::now();
    let query_lower = query.to_ascii_lowercase();
    let mut matches: Vec<(bool, PathBuf)> = Vec::new();
    let mut scanned = 0usize;
    let mut stack = vec![root.clone()];
    let mut result_limit_reached = false;
    let mut scan_limit_reached = false;
    let mut time_limit_reached = false;
    while let Some(dir) = stack.pop() {
        if ctx.cancellation.is_requested() {
            return error_with_display(
                "File search cancelled by user".to_string(),
                "File search cancelled".to_string(),
            );
        }
        if started.elapsed() >= Duration::from_millis(max_duration_ms) {
            time_limit_reached = true;
            break;
        }
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries = Vec::new();
        for entry in read_dir.flatten() {
            if started.elapsed() >= Duration::from_millis(max_duration_ms) {
                time_limit_reached = true;
                break;
            }
            entries.push(entry);
            if entries.len() >= max_scanned_entries.saturating_sub(scanned) {
                break;
            }
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if ctx.cancellation.is_requested() {
                return error_with_display(
                    "File search cancelled by user".to_string(),
                    "File search cancelled".to_string(),
                );
            }
            if scanned >= max_scanned_entries {
                scan_limit_reached = true;
                break;
            }
            if started.elapsed() >= Duration::from_millis(max_duration_ms) {
                time_limit_reached = true;
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if should_skip_dir(&name) {
                    continue;
                }
                scanned += 1;
                if include_dirs && name.to_ascii_lowercase().contains(&query_lower) {
                    matches.push((true, path.clone()));
                    if matches.len() >= max_results {
                        result_limit_reached = true;
                        break;
                    }
                }
                stack.push(path);
            } else if file_type.is_file() {
                scanned += 1;
                if name.to_ascii_lowercase().contains(&query_lower) {
                    matches.push((false, path));
                    if matches.len() >= max_results {
                        result_limit_reached = true;
                        break;
                    }
                }
            }
            if scanned >= max_scanned_entries {
                scan_limit_reached = true;
                break;
            }
        }
        if result_limit_reached || scan_limit_reached || time_limit_reached {
            break;
        }
    }
    matches.sort_by(|left, right| left.1.cmp(&right.1));
    let body = if matches.is_empty() {
        "(no matches)".to_string()
    } else {
        matches
            .iter()
            .map(|(is_dir, path)| {
                let rel = path
                    .strip_prefix(&root)
                    .map(path_slash)
                    .unwrap_or_else(|_| path.display().to_string());
                format!("{} {}", if *is_dir { "dir " } else { "file" }, rel)
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    ok(
        format!(
            "# Find files: {}\nroot: {}\nscanned_entries: {}\n\n{}{}",
            query,
            root.display(),
            scanned,
            body,
            if result_limit_reached {
                "\n...[result limit reached]"
            } else if time_limit_reached {
                "\n...[time limit reached]"
            } else if scan_limit_reached {
                "\n...[scan limit reached]"
            } else {
                ""
            }
        ),
        format!(
            "Find files {query} in {} ({} matches)",
            root.display(),
            matches.len()
        ),
    )
}

fn find_up(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let start_arg = tool_try!(required_string(args, "start"));
    let start = tool_try!(resolve_allowed(&start_arg, &ctx.allowed_dirs, true));
    let patterns_value = match args.get("patterns") {
        Some(Value::Array(values)) if !values.is_empty() => values,
        Some(Value::Array(_)) => return error("patterns must be a non-empty list".to_string()),
        _ => return error("patterns must be a non-empty list".to_string()),
    };
    if patterns_value.len() > 50 {
        return error("patterns must contain at most 50 entries".to_string());
    }
    let mut patterns = Vec::new();
    for value in patterns_value {
        let Some(pattern) = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return error("patterns must contain non-empty strings".to_string());
        };
        if pattern.contains('/') || pattern.contains('\\') {
            return error("patterns must be file names or single-directory globs".to_string());
        }
        let regex = match glob_regex(pattern) {
            Ok(regex) => regex,
            Err(err) => return error(err),
        };
        patterns.push((pattern.to_string(), regex));
    }
    let stop = match args
        .get("stop")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => {
            let stop = tool_try!(resolve_allowed(value, &ctx.allowed_dirs, true));
            if !stop.is_dir() {
                return error(format!("Not a directory: {}", stop.display()));
            }
            Some(stop)
        }
        None => None,
    };
    let max_results = tool_try!(validate_integer_arg(args, "max_results", 50, 1, 500)) as usize;
    let mut directory = if start.is_file() {
        start
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| start.clone())
    } else {
        start.clone()
    };
    let mut matches = Vec::new();
    let mut scanned_dirs = 0usize;
    let mut truncated = false;
    while ctx
        .allowed_dirs
        .iter()
        .any(|base| directory.starts_with(base))
    {
        scanned_dirs += 1;
        let mut entries = fs::read_dir(&directory)
            .map(|read_dir| read_dir.flatten().collect::<Vec<_>>())
            .unwrap_or_default();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if patterns.iter().any(|(_, regex)| regex.is_match(&name)) {
                let path = entry.path();
                if path.exists() && !matches.contains(&path) {
                    matches.push(path);
                    if matches.len() >= max_results {
                        truncated = true;
                        break;
                    }
                }
            }
        }
        if truncated || stop.as_ref().is_some_and(|stop| directory == *stop) {
            break;
        }
        let Some(parent) = directory.parent().map(Path::to_path_buf) else {
            break;
        };
        if parent == directory {
            break;
        }
        directory = parent;
    }
    let body = if matches.is_empty() {
        "(no matches)".to_string()
    } else {
        matches
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    ok(
        format!(
            "# Find up\nstart: {}\npatterns: {}\nscanned_dirs: {}\n\n{}{}",
            start.display(),
            patterns
                .iter()
                .map(|(pattern, _)| pattern.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            scanned_dirs,
            body,
            if truncated {
                "\n...[result limit reached]"
            } else {
                ""
            }
        ),
        format!("Find up ({} matches)", matches.len()),
    )
}

fn list_dir(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    if !target.is_dir() {
        return error(format!("Not a directory: {}", target.display()));
    }
    let mut entries = Vec::new();
    let read_dir = match fs::read_dir(&target) {
        Ok(read_dir) => read_dir,
        Err(err) => return error(format!("List failed: {err}")),
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        let kind = if path.is_dir() { "dir " } else { "file" };
        let size = if path.is_file() {
            path.metadata()
                .map(|m| m.len().to_string())
                .unwrap_or_else(|_| "?".to_string())
        } else {
            "-".to_string()
        };
        entries.push(format!(
            "{kind:4} {size:>10}  {}",
            entry.file_name().to_string_lossy()
        ));
    }
    entries.sort();
    ok(
        format!("# {}\n{}", target.display(), entries.join("\n")),
        format!(
            "{} {} ({} {})",
            tr(Message::DisplayListedPrefix),
            target.display(),
            entries.len(),
            tr(Message::EntriesUnit)
        ),
    )
}

fn glob_files(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = arg_string(args, "root").or_else(|| {
        ctx.allowed_dirs
            .first()
            .map(|path| path.display().to_string())
    });
    let Some(root_arg) = root_arg else {
        return error("root is required when no allowed workspace is configured".to_string());
    };
    let pattern = tool_try!(required_string(args, "pattern"));
    let root = tool_try!(resolve_discovery_root(&root_arg, ctx));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(200)
        .clamp(1, 2000) as usize;
    let regex = match glob_regex(&pattern) {
        Ok(regex) => regex,
        Err(err) => return error(err),
    };
    let mut matches = Vec::new();
    walk_files(&root, usize::MAX, &mut |path| {
        if let Ok(rel) = path.strip_prefix(&root) {
            let text = path_slash(rel);
            if regex.is_match(&text) {
                matches.push(path.display().to_string());
            }
        }
        matches.len() < limit
    });
    ok(
        if matches.is_empty() {
            "(no matches)".to_string()
        } else {
            matches.join("\n")
        },
        format!("glob -> {} {}", matches.len(), tr(Message::MatchesUnit)),
    )
}

fn grep_files(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = arg_string(args, "root")
        .or_else(|| arg_string(args, "path"))
        .or_else(|| {
            ctx.allowed_dirs
                .first()
                .map(|path| path.display().to_string())
        });
    let Some(root_arg) = root_arg else {
        return error("root is required".to_string());
    };
    let pattern = tool_try!(required_string(args, "pattern"));
    let root = tool_try!(resolve_discovery_root(&root_arg, ctx));
    let (search_root, scoped_file) = if root.is_dir() {
        (root.clone(), None)
    } else if root.is_file() {
        let Some(parent) = root.parent() else {
            return error(format!(
                "cannot determine parent directory: {}",
                root.display()
            ));
        };
        (parent.to_path_buf(), Some(root.clone()))
    } else {
        return error(format!("Not a directory or file: {}", root.display()));
    };
    let case_insensitive = match args.get("case_insensitive") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => false,
        _ => return error("case_insensitive must be a boolean".to_string()),
    };
    let regex = match RegexBuilder::new(&pattern)
        .case_insensitive(case_insensitive)
        .build()
    {
        Ok(regex) => regex,
        Err(err) => return error(format!("invalid regex: {err}")),
    };
    let output_mode = match args
        .get("output_mode")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some("content") | None => "content",
        Some("files_with_matches") => "files_with_matches",
        Some("count") => "count",
        Some(_) => {
            return error("output_mode must be content, files_with_matches, or count".to_string())
        }
    };
    let glob = match args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        Some(glob) => match glob_regex(glob) {
            Ok(regex) => Some(regex),
            Err(err) => return error(err),
        },
        None => None,
    };
    let limit = tool_try!(validate_integer_arg(args, "max_matches", 200, 1, 2000)) as usize;
    let context_lines = tool_try!(validate_integer_arg(args, "context_lines", 0, 0, 20)) as usize;
    let offset = tool_try!(validate_integer_arg(args, "offset", 0, 0, 2000)) as usize;
    let head_limit = if args.contains_key("head_limit") {
        Some(tool_try!(validate_integer_arg(
            args,
            "head_limit",
            limit as i64,
            0,
            2000
        )) as usize)
    } else {
        None
    };
    let mut content = Vec::new();
    let mut matching_files = Vec::<PathBuf>::new();
    let mut counts = HashMap::<PathBuf, usize>::new();
    let mut total_matches = 0usize;
    let collection_limit = offset.saturating_add(head_limit.unwrap_or(limit).max(limit)).saturating_add(1);
    let started = std::time::Instant::now();
    let stop = || ctx.cancellation.is_requested() || started.elapsed() >= std::time::Duration::from_secs(30);
    let mut scanned = 0usize;
    let mut oversized = false;

    let mut process_file = |path: &Path| {
        if stop() { return false; }
        scanned += 1;
        if let Some(glob) = &glob {
            if let Ok(rel) = path.strip_prefix(&search_root) {
                if !glob_matches_path(glob, rel) {
                    return (if output_mode == "content" { total_matches } else { matching_files.len() }) < collection_limit;
                }
            }
        }
        let bytes = match fs::File::open(path).and_then(|file| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut std::io::Read::take(file, 4_000_001), &mut bytes)?;
            Ok(bytes)
        }) { Ok(bytes) => bytes, Err(_) => return (if output_mode == "content" { total_matches } else { matching_files.len() }) < collection_limit };
        if bytes.len() > 4_000_000 { oversized = true; return false; }
        if bytes.contains(&0) || bytes.len() > 4_000_000 {
            return (if output_mode == "content" { total_matches } else { matching_files.len() }) < collection_limit;
        }
        let text = String::from_utf8_lossy(&bytes);
        let lines = text.lines().collect::<Vec<_>>();
        let mut emitted_ranges: Vec<(usize, usize)> = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if stop() { return false; }
            if regex.is_match(line) {
                if context_lines == 0 {
                    content.push(format!("{}:{}:{}", path.display(), index + 1, line));
                } else {
                    let match_line = index + 1;
                    let mut start = match_line.saturating_sub(context_lines).max(1);
                    let end = (match_line + context_lines).min(lines.len());
                    if let Some((_, previous_end)) = emitted_ranges.last().copied() {
                        if start <= previous_end + 1 {
                            start = previous_end + 1;
                        }
                    }
                    if start <= end {
                        for line_no in start..=end {
                            let marker = if line_no == match_line { ">" } else { " " };
                            content.push(format!(
                                "{}:{}:{} {}",
                                path.display(),
                                line_no,
                                marker,
                                lines[line_no - 1]
                            ));
                        }
                        emitted_ranges.push((start, end));
                    }
                }
                if !counts.contains_key(path) {
                    matching_files.push(path.to_path_buf());
                }
                *counts.entry(path.to_path_buf()).or_insert(0) += 1;
                total_matches += 1;
                if (if output_mode == "content" { total_matches } else { matching_files.len() }) >= collection_limit {
                    return false;
                }
            }
        }
        (if output_mode == "content" { total_matches } else { matching_files.len() }) < collection_limit
    };
    if let Some(path) = scoped_file.as_ref() {
        let _ = process_file(path);
    } else {
        walk_files_controlled(&search_root, 50_000, &mut process_file, &stop);
    }
    if ctx.cancellation.is_requested() { return error("grep cancelled".into()); }
    if stop() || scanned >= 50_000 { return error("grep scan incomplete: time/file budget exceeded; narrow root or glob".into()); }
    if oversized { return error("grep scan incomplete: file exceeds 4 MB; narrow glob or use bounded read_file".into()); }
    let entries = match output_mode {
        "files_with_matches" => matching_files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
        "count" => matching_files
            .iter()
            .map(|path| {
                format!(
                    "{}:{}",
                    path.display(),
                    counts.get(path).copied().unwrap_or(0)
                )
            })
            .collect::<Vec<_>>(),
        _ => content,
    };
    let effective_limit = head_limit.unwrap_or(limit);
    let window = if effective_limit == 0 {
        entries.iter().skip(offset).cloned().collect::<Vec<_>>()
    } else {
        entries
            .iter()
            .skip(offset)
            .take(effective_limit)
            .cloned()
            .collect::<Vec<_>>()
    };
    let mut text = if window.is_empty() {
        if entries.is_empty() {
            "(no matches)".to_string()
        } else {
            "(no matches in requested window)".to_string()
        }
    } else {
        window.join("\n")
    };
    if offset > 0 || window.len() < entries.len().saturating_sub(offset) {
        text.push_str(&format!(
            "\n...[showing {} of {} entries",
            window.len(),
            entries.len()
        ));
        if offset > 0 {
            text.push_str(&format!(", offset {offset}"));
        }
        text.push(']');
    }
    if (if output_mode == "content" { total_matches } else { matching_files.len() }) >= collection_limit {
        text.push_str("\n...[scan incomplete: requested page boundary reached; more entries may exist]");
    }
    ok(
        text,
        format!(
            "{} {} in {} files",
            total_matches,
            tr(Message::MatchesUnit),
            matching_files.len()
        ),
    )
}

fn glob_matches_path(glob: &Regex, relative_path: &Path) -> bool {
    glob.is_match(&path_slash(relative_path))
        || relative_path
            .file_name()
            .is_some_and(|name| glob.is_match(&name.to_string_lossy()))
}

#[cfg(test)]
mod recursive_glob_contract_tests {
    use super::{glob_matches_path, glob_regex};
    use std::path::Path;

    #[test]
    fn basename_glob_matches_files_in_nested_directories() {
        let glob = glob_regex("*.swift").expect("valid glob");
        assert!(glob_matches_path(
            &glob,
            Path::new("UI/Chat/MacChatWorkspaceView.swift")
        ));
        assert!(!glob_matches_path(
            &glob,
            Path::new("UI/Chat/MacChatWorkspaceView.rs")
        ));
    }
}

/// Trusted catalog data travels separately from model-authored arguments.
pub(crate) struct ToolDiscoveryCatalog {
    pub available: HashSet<String>,
    pub ranked: Vec<String>,
    pub schemas: Vec<Value>,
}

#[cfg(test)]
fn tool_search(args: &Map<String, Value>, skills: &SkillRuntimeContext) -> ToolOutcome {
    let mut catalog = args.get("_nola_available_tool_ids").and_then(Value::as_array).map(|ids| ToolDiscoveryCatalog {
        available: ids.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
        ranked: args.get("_nola_core_tool_search_matches").and_then(Value::as_array)
            .into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect(),
        schemas: args.get("_nola_dynamic_tool_schemas").and_then(Value::as_array).cloned().unwrap_or_default(),
    });
    // Fixtures obey the same complete-snapshot contract as the real Host.
    if let Some(catalog) = &mut catalog {
        for spec in specs_with_skills(skills) {
            if catalog.available.contains(spec.name) && !catalog.schemas.iter().any(|schema|
                schema.get("function").unwrap_or(schema).get("name").and_then(Value::as_str) == Some(spec.name)) {
                let mut schema = json!({"name":spec.name,"description":spec.description,"parameters":spec.parameters});
                schema[TOOL_READ_ONLY_SCHEMA_KEY] = Value::Bool(spec.read_only);
                catalog.schemas.push(schema);
            }
        }
    }
    tool_search_with_catalog(args, skills, catalog.as_ref())
}

fn tool_search_with_catalog(args: &Map<String, Value>, skills: &SkillRuntimeContext, catalog: Option<&ToolDiscoveryCatalog>) -> ToolOutcome {
    let started = std::time::Instant::now();
    let mut outcome = tool_search_from_snapshot(args, skills, catalog);
    let mut data = outcome.data.take().unwrap_or_else(||json!({}));
    let mut metrics = data.get("tool_discovery_metrics").cloned().unwrap_or_else(||json!({}));
    metrics["duration_us"] = json!(started.elapsed().as_micros().min(u64::MAX as u128) as u64);
    metrics["failed"] = json!(outcome.error);
    metrics["output_utf8_bytes"] = json!(outcome.text.len());
    metrics["catalog_source"] = json!(if catalog.is_some() { "host_snapshot" } else { "standalone" });
    if let Some(catalog) = catalog { metrics["available_tool_count"] = json!(catalog.available.len()); }
    data["tool_discovery_metrics"] = metrics;
    outcome.data = Some(data);
    outcome
}

fn tool_search_from_snapshot(args: &Map<String, Value>, skills: &SkillRuntimeContext, catalog: Option<&ToolDiscoveryCatalog>) -> ToolOutcome {
    let Some(query) = arg_string(args, "query") else {
        return error("query is required".to_string());
    };
    let max_results = args.get("max_results").and_then(Value::as_i64).unwrap_or(5);
    if !(1..=MAX_TOOL_SEARCH_RESULTS).contains(&max_results) {
        return error(format!(
            "max_results must be between 1 and {MAX_TOOL_SEARCH_RESULTS}"
        ));
    }
    // The Host supplies the exact catalog separately from model arguments.
    // A missing catalog permits standalone
    // direct-tool behavior; a present value is authoritative and prevents
    // discovery from promising capabilities disabled by the current platform,
    // conversation mode, settings, or permission policy.
    let available_tool_ids = catalog.map(|catalog| &catalog.available);
    let is_available = |name: &str| {
        available_tool_ids
            .as_ref()
            .is_none_or(|available| available.contains(name))
    };
    let core_ranked_tool_ids = catalog.map(|catalog| &catalog.ranked);
    // Resolve every discovery mode against one catalog. Runtime schemas replace
    // static schemas, but never add capabilities outside the frozen Host catalog.
    // Unknown runtime filter metadata remains conservative.
    let injected_dynamic_tools = catalog.map(|catalog| catalog.schemas.clone()).unwrap_or_default();
    // A Host catalog is a snapshot. Never merge mutable thread-local device
    // registrations into it: an old turn must not discover a new schema under
    // the same capability ID after a bridge rebind.
    let dynamic_schemas = if catalog.is_some() {
        injected_dynamic_tools
    } else {
        current_device_bridge_runtime_tools()
            .into_iter()
            .map(|runtime_tool| runtime_tool.schema)
            .collect::<Vec<_>>()
    };
    let static_specs = specs_with_skills(skills);
    let availability = catalog.is_none().then(|| HostCapabilityAvailability::capture(skills));
    let mut specs_vec = static_specs
        .iter()
        .filter(|spec| availability.as_ref().is_some_and(|available| available.enabled(spec.name)))
        .cloned()
        .collect::<Vec<_>>();
    let mut schema_names = HashSet::new();
    for schema in &dynamic_schemas {
        let function = schema.get("function").unwrap_or(schema);
        let Some(name) = function.get("name").and_then(Value::as_str) else {
            return error("tool_discovery_invalid_catalog: missing tool name".into());
        };
        if !is_available(name) {
            continue;
        }
        if name.trim().is_empty() || !schema_names.insert(name) {
            return error("tool_discovery_invalid_catalog: empty or duplicate tool name".into());
        }
        if !function.get("parameters").is_some_and(Value::is_object) {
            return error("tool_discovery_invalid_catalog: missing tool parameter schema".into());
        }
        if availability.as_ref().is_some_and(|available| !available.enabled(name)) { continue; }
        let spec = ToolSpec {
            name,
            description: function.get("description")
                .and_then(Value::as_str).unwrap_or_default(),
            parameters: function.get("parameters")
                .cloned().unwrap_or_else(|| json!({"type": "object"})),
            // Trusted snapshot metadata, never an inference from a familiar
            // tool name. Unknown declarations remain conservative.
            read_only: schema.get(TOOL_READ_ONLY_SCHEMA_KEY)
                .and_then(Value::as_bool).unwrap_or(false),
        };
        if let Some(existing) = specs_vec.iter_mut().find(|entry| entry.name == name) {
            *existing = spec;
        } else {
            specs_vec.push(spec);
        }
    }
    if let Some(catalog) = catalog {
        if catalog.available.iter().any(|name| !schema_names.contains(name.as_str())) {
            return error("tool_discovery_incomplete_catalog: available tool has no frozen schema".into());
        }
    }
    let available_tool_count = available_tool_ids
        .as_ref()
        .map(|available| available.len())
        .unwrap_or(specs_vec.len());

    let (mut matches, mode, filters, missing, suggestion_pool, requested_order) =
        select_tool_specs(&specs_vec, &query, max_results as usize);
    if mode != "select" && filters.is_empty() && !query.trim().is_empty() {
        if let Some(ranked) = &core_ranked_tool_ids {
            matches = ranked
                .iter()
                .filter_map(|capability_id| {
                    specs_vec.iter().find(|spec| spec.name == capability_id)
                })
                .take(max_results as usize)
                .collect();
        }
    }
    let suggestions = if mode == "select" && !missing.is_empty() {
        tool_search_missing_suggestions(&suggestion_pool, &missing, 5)
    } else if matches.is_empty() {
        tool_search_suggest_names(&suggestion_pool, &query, 5)
    } else {
        Vec::new()
    };
    let retry_query = if mode == "select" && !missing.is_empty() {
        tool_search_build_retry_query(
            &filters,
            &suggestions,
            Some(&missing),
            Some(&matches),
            Some(&requested_order),
            &suggestion_pool,
            max_results as usize,
        )
    } else {
        tool_search_build_retry_query(
            &filters,
            &suggestions,
            None,
            None,
            None,
            &suggestion_pool,
            3,
        )
    };
    let reason = tool_search_reason(
        mode,
        &matches,
        &missing,
        &filters,
        &specs_vec,
        &suggestion_pool,
    );
    // An exact identifier miss is not a failed keyword search. Do not turn it
    // into an automatic retry loop or imply that a suggested name is equivalent.
    let retry_query = if reason == "missing_tools" { None } else { retry_query };
    let next_action = tool_search_next_action(&matches, reason, retry_query.as_deref(), &filters);
    let next_action_reason =
        tool_search_next_action_reason(&matches, reason, retry_query.as_deref(), &filters);
    let (filtered_out_names, missing_names) =
        tool_search_reason_details(mode, &missing, &specs_vec);
    let output_meta = json!({
        "query": query,
        "mode": mode,
        "reason": reason,
        "next_action": next_action,
        "next_action_reason": next_action_reason,
        "reason_details": {
            "filtered_out_names": filtered_out_names,
            "missing_names": missing_names,
            "active_filters": filters,
            "requested_select": requested_order,
        },
        "filters": filters,
        "matches": matches.iter().map(|spec| spec.name).collect::<Vec<_>>(),
        "missing": missing,
        "suggestions": suggestions,
        "retry_query": retry_query,
    });
    let output_meta_text =
        serde_json::to_string(&output_meta).unwrap_or_else(|_| output_meta.to_string());
    if matches.is_empty() {
        let missing_line = if mode == "select" && !missing.is_empty() {
            format!("missing: {}\n", missing.join(", "))
        } else {
            String::new()
        };
        let suggestions_line = if suggestions.is_empty() {
            String::new()
        } else {
            format!("suggestions: {}\n", suggestions.join(", "))
        };
        let retry_line = if let Some(query) = &retry_query {
            format!("retry_query: {query}\n")
        } else {
            String::new()
        };
        let guidance = if reason == "missing_tools" {
            "The requested identifiers are absent from the current catalog. Do not repeat this unchanged lookup. Suggestions are alternatives, not equivalent tools. Use an alternative only if it satisfies the request, otherwise report the limitation; absence does not establish why a tool is unavailable."
        } else {
            "Try select:<exact_tool_name> or broader keywords."
        };
        let mut outcome = ok(
            format!(
                "No tools matched query: {query}\n{missing_line}{suggestions_line}{retry_line}tool_search_meta: {output_meta_text}\navailable_tool_count: {}\n{guidance}",
                available_tool_count
            ),
            "0 tool matches".to_string(),
        );
        outcome.data = Some(json!({"tool_discovery_metrics":{
            "available_tool_count":available_tool_count,"matched_tool_count":0,"returned_schema_utf8_bytes":0
        }}));
        return outcome;
    }

    let names = matches
        .iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![
        format!("query: {query}"),
        format!("mode: {mode}"),
        format!(
            "filters: {}",
            if filters.is_empty() {
                "(none)".to_string()
            } else {
                filters.join(", ")
            }
        ),
        format!("available_tool_count: {available_tool_count}"),
        format!("matches: {names}"),
        "Discovery only: call a returned tool directly in the next step with its arguments. Do not repeat this search to execute it.".to_string(),
    ];
    if mode == "select" && !missing.is_empty() {
        lines.push(format!("missing: {}", missing.join(", ")));
    }
    if !suggestions.is_empty() {
        lines.push(format!("suggestions: {}", suggestions.join(", ")));
    }
    if let Some(query) = &retry_query {
        lines.push(format!("retry_query: {query}"));
    }
    lines.push(format!("tool_search_meta: {output_meta_text}"));
    let activated_capability_ids = matches
        .iter()
        .map(|spec| spec.name.to_string())
        .collect::<Vec<_>>();
    lines.push(String::new());
    lines.push("<functions>".to_string());
    let match_count = matches.len();
    let mut schema_bytes = 0usize;
    for spec in matches {
        let function = json!({
            "name": spec.name,
            "description": spec.description,
            "parameters": spec.parameters.clone(),
        });
        let serialized = function.to_string();
        schema_bytes += serialized.len();
        lines.push(format!("<function>{serialized}</function>"));
    }
    lines.push("</functions>".to_string());
    let mut outcome = ok(lines.join("\n"), format!("{match_count} tool match(es)"))
        .with_capability_activation(activated_capability_ids);
    outcome.data = Some(json!({"tool_discovery_metrics":{
        "available_tool_count":available_tool_count,"matched_tool_count":match_count,"returned_schema_utf8_bytes":schema_bytes
    }}));
    outcome
}

#[cfg(test)]
#[test]
fn tool_search_discovers_an_injected_mcp_schema_by_stable_id() {
    let mut args = Map::new();
    args.insert(
        "query".to_string(),
        Value::String("select:codebase_search".to_string()),
    );
    args.insert(
        "_nola_available_tool_ids".to_string(),
        Value::Array(vec![Value::String("codebase_search".to_string())]),
    );
    args.insert(
        "_nola_core_tool_search_matches".to_string(),
        Value::Array(Vec::new()),
    );
    args.insert(
        "_nola_dynamic_tool_schemas".to_string(),
        Value::Array(vec![json!({
            "type": "function",
            "function": {
                "name": "codebase_search",
                "description": "Search the persistent code graph",
                "parameters": {"type": "object", "required": ["project", "query"]}
            }
        })]),
    );

    let outcome = tool_search(&args, &SkillRuntimeContext::default());
    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("codebase_search"));
    assert!(outcome.text.contains("select:codebase_search"));
    let metrics = &outcome.data.as_ref().unwrap()["tool_discovery_metrics"];
    assert_eq!(metrics["matched_tool_count"], 1);
    assert_eq!(metrics["available_tool_count"], 1);
    assert_eq!(metrics["failed"], false);
    assert_eq!(metrics["output_utf8_bytes"].as_u64(), Some(outcome.text.len() as u64));
    assert!(metrics["returned_schema_utf8_bytes"].as_u64().unwrap() > 0);
    assert!(!outcome.text.contains("duration_us"));
}

#[cfg(test)]
#[test]
fn discovery_metrics_distinguish_miss_from_invalid_request() {
    let skills = SkillRuntimeContext::default();
    let invalid = tool_search_with_catalog(&Map::new(), &skills, None);
    assert_eq!(invalid.data.unwrap()["tool_discovery_metrics"]["failed"], true);
    let args = json!({"query":"select:definitely_absent_tool_723"}).as_object().unwrap().clone();
    let absent = tool_search_with_catalog(&args, &skills, None);
    let metrics = &absent.data.as_ref().unwrap()["tool_discovery_metrics"];
    assert_eq!(metrics["failed"], false);
    assert_eq!(metrics["matched_tool_count"], 0);
    assert_eq!(metrics["returned_schema_utf8_bytes"], 0);
}

#[cfg(test)]
#[test]
fn discovery_rejects_incomplete_or_ambiguous_frozen_catalogs() {
    let skills = SkillRuntimeContext::default();
    let args = serde_json::from_value(json!({"query":"select:read_file"})).unwrap();
    let schema = json!({"name":"read_file","description":"Frozen version",
        "parameters":{"type":"object","required":["frozen_path"],"properties":{"frozen_path":{"type":"string"}}}});
    for schemas in [vec![], vec![schema.clone(), schema.clone()],
        vec![json!({"name":"read_file"})]] {
        let catalog = ToolDiscoveryCatalog {
            available: ["read_file".into()].into_iter().collect(), ranked: vec![], schemas,
        };
        let outcome = tool_search_with_catalog(&args, &skills, Some(&catalog));
        assert!(outcome.error, "{}", outcome.text);
        assert!(outcome.activation.is_none());
    }
    let catalog = ToolDiscoveryCatalog {
        available: ["read_file".into()].into_iter().collect(), ranked: vec![], schemas: vec![schema],
    };
    let outcome = tool_search_with_catalog(&args, &skills, Some(&catalog));
    assert!(!outcome.error, "{}", outcome.text);
    assert!(outcome.text.contains("Frozen version"));
    assert!(outcome.text.contains("frozen_path"));
    assert_eq!(outcome.activation.unwrap().capability_ids, vec!["read_file"]);
}

#[cfg(test)]
#[test]
fn discovery_read_only_filter_uses_snapshot_not_builtin_name() {
    let skills = SkillRuntimeContext::default();
    for (name, read_only, expected) in [("read_file", false, false), ("custom_inspect", true, true)] {
        let mut schema = json!({"name":name,"description":"Host declaration",
            "parameters":{"type":"object"}});
        schema[TOOL_READ_ONLY_SCHEMA_KEY] = json!(read_only);
        let catalog = ToolDiscoveryCatalog {
            available: [name.into()].into_iter().collect(), ranked: vec![], schemas: vec![schema],
        };
        let args = serde_json::from_value(json!({"query":format!("read_only:true select:{name}")})).unwrap();
        let outcome = tool_search_with_catalog(&args, &skills, Some(&catalog));
        assert!(!outcome.error, "{}", outcome.text);
        assert_eq!(outcome.activation.is_some(), expected, "{}", outcome.text);
    }
}

fn tool_search_reason(
    mode: &str,
    matches: &[&ToolSpec],
    missing: &[String],
    filters: &[String],
    available: &[ToolSpec],
    suggestion_pool: &[&ToolSpec],
) -> &'static str {
    if mode == "select" && !missing.is_empty() {
        let (filtered_out_names, _missing_names) =
            tool_search_reason_details(mode, missing, available);
        if !filtered_out_names.is_empty() {
            return "filtered_out";
        }
        return "missing_tools";
    }
    if !matches.is_empty() {
        return "ok";
    }
    if !filters.is_empty() && suggestion_pool.is_empty() {
        return "filtered_out";
    }
    "no_keyword_match"
}

#[test]
fn exact_missing_tool_recovery_does_not_loop_like_keyword_search() {
    assert_eq!(tool_search_next_action(&[], "missing_tools", Some("select:other"), &[]),
        "consider_alternatives_or_report_limitation");
    assert_eq!(tool_search_next_action_reason(&[], "missing_tools", None, &[]),
        "exact_identifiers_absent_from_current_catalog");
    assert_eq!(tool_search_next_action(&[], "no_keyword_match", None, &[]), "broaden_keywords");
    assert_eq!(tool_search_next_action(&[], "filtered_out", None, &["read_only:true".into()]), "relax_filters");
    let args = json!({"query":"select:absent_fixture_tool", "_nola_available_tool_ids":[]});
    let outcome = tool_search(args.as_object().unwrap(), &SkillRuntimeContext::from_configured_dirs(Some(vec![])));
    assert!(!outcome.error);
    assert!(outcome.text.contains("exact_identifiers_absent_from_current_catalog"));
    assert!(outcome.text.contains("\"retry_query\":null"));
    assert!(!outcome.text.contains("Try select:"));
}

fn tool_search_next_action(
    matches: &[&ToolSpec],
    reason: &str,
    retry_query: Option<&str>,
    filters: &[String],
) -> &'static str {
    if !matches.is_empty() {
        return "proceed_with_matches";
    }
    if reason == "missing_tools" {
        return "consider_alternatives_or_report_limitation";
    }
    if retry_query.is_some() {
        return "retry_with_select";
    }
    if reason == "filtered_out" && !filters.is_empty() {
        return "relax_filters";
    }
    "broaden_keywords"
}

fn tool_search_next_action_reason(
    matches: &[&ToolSpec],
    reason: &str,
    retry_query: Option<&str>,
    filters: &[String],
) -> &'static str {
    if !matches.is_empty() {
        return "matches_available";
    }
    if reason == "missing_tools" {
        return "exact_identifiers_absent_from_current_catalog";
    }
    if retry_query.is_some() {
        return "retry_query_available";
    }
    if reason == "filtered_out" && !filters.is_empty() {
        return "active_filters_excluded_all_candidates";
    }
    "no_retry_query_available"
}

fn tool_search_reason_details(
    mode: &str,
    missing: &[String],
    available: &[ToolSpec],
) -> (Vec<String>, Vec<String>) {
    let mut filtered_out_names = Vec::new();
    let mut missing_names = Vec::new();
    if mode == "select" {
        let available_names = available
            .iter()
            .map(|spec| spec.name.to_ascii_lowercase())
            .collect::<Vec<_>>();
        for name in missing {
            let lowered = name.to_ascii_lowercase();
            if available_names
                .iter()
                .any(|candidate| candidate == &lowered)
            {
                filtered_out_names.push(name.clone());
            } else {
                missing_names.push(name.clone());
            }
        }
    }
    (filtered_out_names, missing_names)
}

fn tool_search_terms(value: &str) -> Vec<String> {
    value
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect()
}

fn tool_search_name_score(query: &str, candidate: &str) -> i64 {
    let q = query.trim().to_ascii_lowercase();
    let c = candidate.trim().to_ascii_lowercase();
    if q.is_empty() || c.is_empty() {
        return 0;
    }
    if q == c {
        return 10_000;
    }
    let mut score: i64 = 0;
    if c.starts_with(&q) {
        score += 350;
    }
    if q.starts_with(&c) {
        score += 300;
    }
    if c.contains(&q) {
        score += 220;
    }
    if q.contains(&c) {
        score += 180;
    }
    let prefix = q.chars().zip(c.chars()).take_while(|(a, b)| a == b).count() as i64;
    score += prefix * 12;

    let q_terms = tool_search_terms(&q);
    let c_terms = tool_search_terms(&c);
    let overlap = q_terms
        .iter()
        .filter(|term| c_terms.iter().any(|candidate_term| candidate_term == *term))
        .count() as i64;
    score += overlap * 60;
    score -= (q.len() as i64 - c.len() as i64).abs();
    score
}

fn tool_search_suggest_names(specs_vec: &[&ToolSpec], query: &str, limit: usize) -> Vec<String> {
    let mut scored = specs_vec
        .iter()
        .map(|spec| (tool_search_name_score(query, spec.name), spec.name))
        .filter(|(score, _)| *score > 0)
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_score, name)| name.to_string())
        .collect()
}

fn tool_search_missing_suggestions(
    specs_vec: &[&ToolSpec],
    missing: &[String],
    limit: usize,
) -> Vec<String> {
    let mut ordered = Vec::new();
    for miss in missing {
        for suggestion in tool_search_suggest_names(specs_vec, miss, 3) {
            if !ordered
                .iter()
                .any(|existing: &String| existing == &suggestion)
            {
                ordered.push(suggestion);
                if ordered.len() >= limit {
                    return ordered;
                }
            }
        }
    }
    ordered
}

fn tool_search_filter_tokens(filters: &[String]) -> Vec<String> {
    filters
        .iter()
        .filter_map(|item| item.split_once('='))
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
        .map(|(key, value)| format!("{key}:{value}"))
        .collect()
}

fn tool_search_build_retry_query(
    filters: &[String],
    suggestions: &[String],
    missing: Option<&[String]>,
    matched: Option<&[&ToolSpec]>,
    requested_order: Option<&[String]>,
    suggestion_pool: &[&ToolSpec],
    limit: usize,
) -> Option<String> {
    let mut picked = Vec::new();
    if let (Some(missing), Some(requested_order)) = (missing, requested_order) {
        let mut replacement_by_missing: HashMap<String, String> = HashMap::new();
        for name in missing {
            let candidates = tool_search_suggest_names(suggestion_pool, name, 1);
            if let Some(first) = candidates.first() {
                if !first.is_empty() {
                    replacement_by_missing.insert(name.to_ascii_lowercase(), first.clone());
                }
            }
        }
        let mut matched_by_lower: HashMap<String, String> = HashMap::new();
        if let Some(matched) = matched {
            for spec in matched {
                matched_by_lower.insert(spec.name.to_ascii_lowercase(), spec.name.to_string());
            }
        }
        for requested in requested_order {
            if requested.is_empty() {
                continue;
            }
            let lowered = requested.to_ascii_lowercase();
            if let Some(matched_name) = matched_by_lower.get(&lowered) {
                picked.push(matched_name.clone());
            } else if let Some(replacement) = replacement_by_missing.get(&lowered) {
                picked.push(replacement.clone());
            } else {
                picked.push(requested.clone());
            }
        }
    } else if let Some(missing) = missing {
        if let Some(matched) = matched {
            picked.extend(matched.iter().map(|spec| spec.name.to_string()));
        }
        for name in missing {
            let candidates = tool_search_suggest_names(suggestion_pool, name, 1);
            if let Some(first) = candidates.first() {
                if !first.is_empty() && !picked.iter().any(|existing: &String| existing == first) {
                    picked.push(first.clone());
                    if picked.len() >= limit {
                        break;
                    }
                }
            }
        }
    }
    if picked.is_empty() {
        picked = suggestions
            .iter()
            .filter(|value| !value.is_empty())
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
    }
    if picked.is_empty() {
        return None;
    }
    let prefix = tool_search_filter_tokens(filters).join(" ");
    let select_part = format!("select:{}", picked.join(","));
    Some(if prefix.is_empty() {
        select_part
    } else {
        format!("{prefix} {select_part}")
    })
}

fn select_tool_specs<'a>(
    specs_vec: &'a [ToolSpec<'a>],
    query: &str,
    max_results: usize,
) -> (
    Vec<&'a ToolSpec<'a>>,
    &'static str,
    Vec<String>,
    Vec<String>,
    Vec<&'a ToolSpec<'a>>,
    Vec<String>,
) {
    let stripped = query.trim();
    let (raw_terms, requested, category_filter, read_only_filter, has_params, excludes, filters) =
        tool_search_query_parts(stripped);
    let filtered_specs = specs_vec
        .iter()
        .filter(|spec| {
            category_filter
                .as_deref()
                .is_none_or(|expected| tool_spec_category(spec.name) == expected)
                && read_only_filter.is_none_or(|expected| spec.read_only == expected)
                && has_params
                    .iter()
                    .all(|param| tool_spec_has_param(spec, param))
                && !tool_spec_is_excluded(spec.name, &excludes)
        })
        .collect::<Vec<_>>();
    if let Some(requested) = requested {
        let requested_order = requested.clone();
        let mut matches = Vec::new();
        let mut missing = Vec::new();
        for name in requested {
            if let Some(spec) = filtered_specs
                .iter()
                .find(|spec| spec.name.eq_ignore_ascii_case(&name))
            {
                matches.push(*spec);
            } else {
                missing.push(name);
            }
        }
        return (
            matches,
            "select",
            filters,
            missing,
            filtered_specs,
            requested_order,
        );
    }

    if !stripped.is_empty()
        && !stripped.contains(char::is_whitespace)
        && !stripped.starts_with('+')
        && !stripped.contains(':')
        && !stripped.contains('=')
    {
        if let Some(spec) = filtered_specs
            .iter()
            .find(|spec| spec.name.eq_ignore_ascii_case(stripped))
        {
            return (
                vec![*spec],
                "exact",
                filters,
                Vec::new(),
                filtered_specs,
                Vec::new(),
            );
        }

        let lowered = stripped.to_ascii_lowercase();
        if lowered.len() >= 4 {
            let mut prefix_matches = filtered_specs
                .iter()
                .copied()
                .filter(|spec| spec.name.to_ascii_lowercase().starts_with(&lowered))
                .collect::<Vec<_>>();
            if !prefix_matches.is_empty() {
                prefix_matches.sort_by(|a, b| a.name.cmp(b.name));
                prefix_matches.truncate(max_results);
                return (
                    prefix_matches,
                    "prefix",
                    filters,
                    Vec::new(),
                    filtered_specs,
                    Vec::new(),
                );
            }
        }
    }

    let raw_terms = raw_terms;
    let required_terms = raw_terms
        .iter()
        .filter_map(|(required, term)| required.then_some(term.as_str()))
        .collect::<Vec<_>>();
    let query_terms = raw_terms
        .iter()
        .map(|(_required, term)| term.as_str())
        .collect::<Vec<_>>();
    if query_terms.is_empty() {
        let mut matches = filtered_specs;
        matches.sort_by(|a, b| a.name.cmp(b.name));
        matches.truncate(max_results);
        let suggestion_pool = matches.clone();
        return (
            matches,
            "filter",
            filters,
            Vec::new(),
            suggestion_pool,
            Vec::new(),
        );
    }
    let mut term_patterns: HashMap<String, Regex> = HashMap::new();
    for term in query_terms.iter().chain(required_terms.iter()) {
        if term_patterns.contains_key(*term) {
            continue;
        }
        let pattern = format!(r"\b{}\b", regex::escape(term));
        if let Ok(compiled) = Regex::new(&pattern) {
            term_patterns.insert((*term).to_string(), compiled);
        }
    }
    // Rank metadata terms by their selectivity in this actual catalog. Common
    // words (including substrings in tool names) must not bury a rare declared
    // format/capability. No language stop-list or intent-to-tool routing is used.
    let term_scores = query_terms.iter().map(|term| {
        filtered_specs.iter().map(|spec|
            tool_spec_score(spec, &[*term], &[], &term_patterns)
        ).collect::<Vec<_>>()
    }).collect::<Vec<_>>();
    let term_weights = term_scores.iter().map(|scores| {
        let frequency = scores.iter().filter(|score| **score > 0).count();
        ((filtered_specs.len() + 1) / (frequency + 1)).max(1).ilog2() as i64 + 1
    }).collect::<Vec<_>>();
    let mut scored = filtered_specs
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, spec)| {
            if !required_terms.is_empty()
                && tool_spec_score(spec, &query_terms, &required_terms, &term_patterns) == 0 {
                return None;
            }
            let score = term_scores.iter().zip(&term_weights)
                .map(|(scores, weight)| scores[index] * weight).sum::<i64>();
            (score > 0).then_some((score, spec.name, spec))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    let matches = scored
        .into_iter()
        .take(max_results)
        .map(|(_score, _name, spec)| spec)
        .collect::<Vec<_>>();
    let suggestion_pool = filtered_specs;
    (
        matches,
        "keyword",
        filters,
        Vec::new(),
        suggestion_pool,
        Vec::new(),
    )
}

fn tool_search_query_parts(
    query: &str,
) -> (
    Vec<(bool, String)>,
    Option<Vec<String>>,
    Option<String>,
    Option<bool>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
) {
    let mut raw_terms = Vec::new();
    let mut requested = None;
    let mut category_filter = None;
    let mut read_only_filter = None;
    let mut has_params = Vec::new();
    let mut excludes = Vec::new();
    let mut filters = Vec::new();
    for raw in query.split_whitespace() {
        let lowered = raw.to_ascii_lowercase();
        if lowered.starts_with("select:") {
            let names = raw
                .split_once(':')
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .split([',', '\n'])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if !names.is_empty() {
                requested.get_or_insert_with(Vec::new).extend(names);
            }
            continue;
        }
        if lowered.starts_with("category:") || lowered.starts_with("category=") {
            let value = raw
                .split_once(':')
                .or_else(|| raw.split_once('='))
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if value == "web_safe" || value == "local_dev" {
                category_filter = Some(value.clone());
                filters.push(format!("category={value}"));
            }
            continue;
        }
        if lowered.starts_with("read_only:") || lowered.starts_with("read_only=") {
            let value = raw
                .split_once(':')
                .or_else(|| raw.split_once('='))
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            let parsed = match value.as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            };
            if let Some(parsed) = parsed {
                read_only_filter = Some(parsed);
                filters.push(format!("read_only={parsed}"));
            }
            continue;
        }
        if lowered.starts_with("has_param:") || lowered.starts_with("has_param=") {
            let value = raw
                .split_once(':')
                .or_else(|| raw.split_once('='))
                .map(|(_, tail)| tail)
                .unwrap_or_default()
                .trim()
                .to_string();
            if !value.is_empty() {
                has_params.push(value.clone());
                filters.push(format!("has_param={value}"));
            }
            continue;
        }
        if lowered.starts_with("exclude:") || lowered.starts_with("exclude=") {
            let value = raw
                .split_once(':')
                .or_else(|| raw.split_once('='))
                .map(|(_, tail)| tail)
                .unwrap_or_default();
            let parsed = tool_search_exclude_patterns(value);
            if !parsed.is_empty() {
                for item in parsed {
                    filters.push(format!("exclude={item}"));
                    excludes.push(item);
                }
            }
            continue;
        }
        let required = raw.starts_with('+');
        let cleaned = raw
            .trim_start_matches('+')
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    ch.to_ascii_lowercase()
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_");
        if !cleaned.is_empty() {
            raw_terms.push((required, cleaned));
        }
    }
    (
        raw_terms,
        requested,
        category_filter,
        read_only_filter,
        has_params,
        excludes,
        filters,
    )
}

pub(crate) fn tool_spec_category(name: &str) -> &'static str {
    if name.starts_with("browser_")
        || name.starts_with("chrome_native_")
        || name.starts_with("computer_")
    {
        return "web_safe";
    }
    match name {
        "system_time" | "web_fetch" | "web_search" | "tool_search" | "tool_output_list"
        | "tool_output_read" | "tool_output_search" | "tool_output_find" | "set_thinking_mode"
        | "goal_complete" | "agent_task_status" | "update_agent_todos" | "propose_agent_plan" => {
            "web_safe"
        }
        _ => "local_dev",
    }
}

fn tool_spec_has_param(spec: &ToolSpec, param: &str) -> bool {
    spec.parameters
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key(param))
}

fn tool_search_exclude_patterns(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect()
}

fn tool_spec_is_excluded(name: &str, excludes: &[String]) -> bool {
    let lowered = name.to_ascii_lowercase();
    excludes.iter().any(|pattern| {
        if let Some(prefix) = pattern.strip_suffix('*') {
            !prefix.is_empty() && lowered.starts_with(prefix)
        } else {
            lowered == *pattern
        }
    })
}

fn tool_spec_search_hint(name: &str) -> &'static str {
    match name {
        "read_symbol" => "codeintel symbol definition references",
        "workspace_symbols" => "codeintel workspace symbol index",
        "implementation_search" => "codeintel implementation callers overrides",
        "code_navigation" => "codeintel goto definition declaration",
        _ => "",
    }
}

fn tool_spec_score(
    spec: &ToolSpec,
    query_terms: &[&str],
    required_terms: &[&str],
    term_patterns: &HashMap<String, Regex>,
) -> i64 {
    let name = spec.name.to_ascii_lowercase();
    let description = spec.description.to_ascii_lowercase();
    let schema = spec.parameters.get("description").and_then(Value::as_str)
        .unwrap_or_default().to_ascii_lowercase();
    let hint = tool_spec_search_hint(spec.name).to_ascii_lowercase();

    let name_parts = name
        .replace('_', " ")
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    for term in required_terms {
        let in_name = name_parts.iter().any(|part| part == term)
            || name_parts.iter().any(|part| part.contains(term));
        let in_text = term_patterns.get(*term).is_some_and(|pattern| {
            pattern.is_match(&description) || pattern.is_match(&schema) || pattern.is_match(&hint)
        });
        if !in_name && !in_text {
            return 0;
        }
    }

    let mut score = 0;
    let mut matched_terms = 0_i64;
    for term in query_terms {
        let pattern = term_patterns.get(*term);
        let exact_name = *term == name;
        let exact_name_part = name_parts.iter().any(|part| part == term);
        let partial_name_part = name_parts.iter().any(|part| part.contains(term));
        let description_match = pattern.is_some_and(|re| re.is_match(&description));
        let schema_match = pattern.is_some_and(|re| re.is_match(&schema));
        let hint_match = pattern.is_some_and(|re| re.is_match(&hint));
        if exact_name
            || exact_name_part
            || partial_name_part
            || description_match
            || schema_match
            || hint_match
        {
            matched_terms += 1;
        }
        if exact_name {
            score += 120;
        }
        if exact_name_part {
            score += 40;
        } else if partial_name_part {
            score += 25;
        }
        if description_match {
            score += 10;
        }
        if schema_match {
            score += 5;
        }
        if hint_match {
            score += 8;
        }
    }
    // Prefer capabilities whose declared metadata explains more of the user's
    // request. A single name match should not outrank a tool whose description
    // covers several requested operations or data formats.
    score += matched_terms * matched_terms * 20;
    score
}

fn tool_output_list_in_store(args: &Map<String, Value>, store: &ToolOutputStore) -> ToolOutcome {
    let limit = tool_try!(validate_integer_arg(
        args,
        "limit",
        20,
        1,
        MAX_TOOL_OUTPUT_LIST
    )) as usize;
    let root = store.root();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return ok(
                "(no saved tool outputs)".to_string(),
                "0 saved tool outputs".to_string(),
            )
        }
        Err(error) => return tool_output_store_error(error.into(), "tool_output_list"),
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if store.resolve(&path).is_err() { return None; }
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then_some((path, metadata))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| {
        b.1.modified()
            .ok()
            .cmp(&a.1.modified().ok())
            .then_with(|| a.0.cmp(&b.0))
    });
    if files.is_empty() {
        return ok(
            "(no saved tool outputs)".to_string(),
            "0 saved tool outputs".to_string(),
        );
    }
    let mut lines = vec!["# Saved tool outputs".to_string()];
    for (path, metadata) in files.into_iter().take(limit) {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        lines.push(format!(
            "- {} | size_bytes={} | mtime={modified}",
            path.display(),
            metadata.len()
        ));
    }
    ok(
        lines.join("\n"),
        format!("{} saved tool output(s)", lines.len().saturating_sub(1)),
    )
}

fn tool_output_store_error(error: output_store::ReadError, capability: &str) -> ToolOutcome {
    error_data_with_display(json!({
            "error": error.code(),
            "message": error.to_string(),
            "tool_execution_skipped": true,
            "recovery": {
                "capability": capability,
                "instruction": "Use an exact saved output reference from this session; tool_output_list can recover a missing reference. For range reads use preview_end_byte_offset or next_offset from the previous result. Never request workspace access to the result store."
            }
        }),
        "Saved tool output could not be accessed".to_string(),
    )
}

fn tool_output_read_in_store(args: &Map<String, Value>, store: &ToolOutputStore) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let offset = tool_try!(validate_integer_arg(args, "offset", 0, 0, i64::MAX)) as u64;
    let requested = tool_try!(validate_integer_arg(
        args, "max_bytes", MAX_TOOL_OUTPUT_SLICE_BYTES, 1, MAX_TOOL_OUTPUT_READ_REQUEST_BYTES
    ));
    let pointer = match args.get("json_pointer") {
        Some(Value::String(pointer)) => Some(pointer.as_str()), None | Some(Value::Null) => None,
        _ => return error("json_pointer must be a JSON Pointer string".into()),
    };
    let result = match pointer {
        Some(pointer) => store.read_selection(Path::new(&path), Some(pointer), offset,
            requested.min(MAX_TOOL_OUTPUT_SLICE_BYTES) as usize, MAX_TOOL_RESULT_CHARS.saturating_sub(512)),
        None => store.read(Path::new(&path), offset,
            requested.min(MAX_TOOL_OUTPUT_SLICE_BYTES) as usize, MAX_TOOL_RESULT_CHARS.saturating_sub(512)),
    };
    match result {
        Ok(page) => {
            let display = format!("Tool output slice {}/{}", page.offset, page.size_bytes);
            ok_data(serde_json::to_value(&page).expect("slice serializes"), display)
        }
        Err(error) => tool_output_store_error(error, "tool_output_read"),
    }
}

fn bounded_output_line<'a>(line: &'a str, regex: &regex::Regex) -> (usize, &'a str, bool) {
    if line.len() <= 1_024 { return (0, line, false); }
    let anchor = regex.find(line).map_or(0, |found| found.start());
    let mut start = anchor.saturating_sub(256);
    while !line.is_char_boundary(start) { start -= 1; }
    let mut end = (start + 1_024).min(line.len());
    while !line.is_char_boundary(end) { end -= 1; }
    (start, &line[start..end], true)
}

fn bounded_tool_output_report(
    mut lines: Vec<String>, mut truncated: bool, header: impl Fn(bool) -> String,
) -> String {
    loop {
        let body = format!("{}\n\n{}", header(truncated), if lines.is_empty() {
            if truncated { "(report limited; narrow the search or read a specific region)".into() }
            else { "(no matches)".into() }
        } else { lines.join("\n") });
        if serde_json::to_string(&body).expect("report serializes").chars().count()
            <= MAX_TOOL_RESULT_CHARS.saturating_sub(256) {
            return body;
        }
        truncated = true;
        if lines.pop().is_none() {
            return "Tool output report metadata exceeds the response budget; narrow the query. Original evidence remains saved.".into();
        }
    }
}

fn tool_output_search_in_store(args: &Map<String, Value>, store: &ToolOutputStore) -> ToolOutcome {
    let path_arg = tool_try!(required_string(args, "path"));
    let pattern = tool_try!(required_string(args, "pattern"));
    let path = match store.resolve(Path::new(&path_arg)) {
        Ok(path) => path,
        Err(error) => return tool_output_store_error(error, "tool_output_search"),
    };
    let regex_mode = tool_try!(bool_arg(args, "regex", false));
    let case_sensitive = tool_try!(bool_arg(args, "case_sensitive", false));
    let context_lines = tool_try!(validate_integer_arg(args, "context_lines", 0, 0, MAX_TOOL_OUTPUT_CONTEXT_LINES)) as usize;
    let max_matches = tool_try!(validate_integer_arg(args, "max_matches", 50, 1, MAX_TOOL_OUTPUT_MATCHES)) as usize;
    let content = match store.model_text(&path) {
        Ok(content) => content,
        Err(error) => return tool_output_store_error(error.into(), "tool_output_search"),
    };
    let regex = if regex_mode {
        match RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
        {
            Ok(regex) => regex,
            Err(err) => return error(format!("invalid regex: {err}")),
        }
    } else {
        match RegexBuilder::new(&regex::escape(&pattern))
            .case_insensitive(!case_sensitive)
            .build()
        {
            Ok(regex) => regex,
            Err(err) => return error(format!("invalid search pattern: {err}")),
        }
    };

    let lines = content.lines().collect::<Vec<_>>();
    let offsets = line_byte_offsets(&content);
    let mut selected_lines = std::collections::BTreeSet::new();
    let mut matched_lines = std::collections::BTreeSet::new();
    let mut matches = 0usize;
    let mut truncated = false;
    for (index, line) in lines.iter().enumerate() {
        if !regex.is_match(line) {
            continue;
        }
        matches += 1;
        if matches > max_matches {
            truncated = true;
            continue;
        }
        matched_lines.insert(index);
        let start = index.saturating_sub(context_lines);
        let end = (index + context_lines).min(lines.len().saturating_sub(1));
        selected_lines.extend(start..=end);
    }
    // Overlapping context windows share each line once. The path belongs in
    // the header, not on every line: navigation must not bury tail evidence
    // in duplicate path prefixes and recursively truncate its own result.
    let mut output = Vec::new();
    let mut encoded_chars = 0usize;
    for current in selected_lines {
            let marker = if matched_lines.contains(&current) { ">" } else { " " };
            let (start, snippet, line_truncated) = bounded_output_line(lines[current], &regex);
            truncated |= line_truncated;
            let rendered = format!(
                "{}:{}:{} {}",
                current + 1,
                offsets.get(current).copied().unwrap_or(0) + start,
                marker,
                snippet
            );
            encoded_chars = encoded_chars.saturating_add(
                serde_json::to_string(&rendered).expect("search snippet serializes").chars().count());
            if encoded_chars > MAX_TOOL_RESULT_CHARS.saturating_sub(1_024) {
                truncated = true;
                break;
            }
            output.push(rendered);
    }
    let body = bounded_tool_output_report(output, truncated, |truncated| format!(
        "# Tool output search\npath: {}\npattern: {pattern}\nmatches: {matches}\ntruncated: {truncated}\noffset_space: model_evidence_utf8\nformat: line:byte_offset:match_marker content",
        path.display()));
    ok(body, format!("{matches} tool output match(es)"))
}

fn tool_output_find_in_store(args: &Map<String, Value>, store: &ToolOutputStore) -> ToolOutcome {
    let pattern = tool_try!(required_string(args, "pattern"));
    let regex_mode = tool_try!(bool_arg(args, "regex", false));
    let case_sensitive = tool_try!(bool_arg(args, "case_sensitive", false));
    let context_lines = tool_try!(validate_integer_arg(args, "context_lines", 0, 0, MAX_TOOL_OUTPUT_CONTEXT_LINES)) as usize;
    let max_matches = tool_try!(validate_integer_arg(args, "max_matches", 50, 1, MAX_TOOL_OUTPUT_MATCHES)) as usize;
    let max_outputs = tool_try!(validate_integer_arg(args, "max_outputs", 20, 1, MAX_TOOL_OUTPUT_LIST)) as usize;

    let root = store.root();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return ok(
                "# Tool output find\nscanned_outputs: 0\nmatches: 0\ntruncated: false\n\n(no saved tool outputs)"
                    .to_string(),
                "0 tool output match(es)".to_string(),
            )
        }
        Err(error) => return tool_output_store_error(error.into(), "tool_output_find"),
    };

    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if store.resolve(&path).is_err() { return None; }
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then_some((path, metadata))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| {
        b.1.modified()
            .ok()
            .cmp(&a.1.modified().ok())
            .then_with(|| a.0.cmp(&b.0))
    });

    let regex = if regex_mode {
        match RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
        {
            Ok(regex) => regex,
            Err(err) => return error(format!("invalid regex: {err}")),
        }
    } else {
        match RegexBuilder::new(&regex::escape(&pattern))
            .case_insensitive(!case_sensitive)
            .build()
        {
            Ok(regex) => regex,
            Err(err) => return error(format!("invalid search pattern: {err}")),
        }
    };

    let mut output = Vec::new();
    let mut scanned = 0usize;
    let mut matches = 0usize;
    let mut truncated = false;
    let mut encoded_chars = 0usize;
    for (path, metadata) in files.into_iter().take(max_outputs) {
        scanned += 1;
        let content = match store.model_text(&path) {
            Ok(content) => content,
            Err(error) => return tool_output_store_error(error.into(), "tool_output_find"),
        };
        let lines = content.lines().collect::<Vec<_>>();
        let offsets = line_byte_offsets(&content);
        let path_text = path.display().to_string();
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let path_match = regex.is_match(&path_text) || regex.is_match(file_name);
        let mut file_matches = 0usize;
        let mut file_output = Vec::new();
        let mut selected_lines = std::collections::BTreeSet::new();
        let mut matched_lines = std::collections::BTreeSet::new();
        for (index, line) in lines.iter().enumerate() {
            if !regex.is_match(line) {
                continue;
            }
            file_matches += 1;
            matches += 1;
            if matches > max_matches {
                truncated = true;
                break;
            }
            matched_lines.insert(index);
            let start = index.saturating_sub(context_lines);
            let end = (index + context_lines).min(lines.len().saturating_sub(1));
            selected_lines.extend(start..=end);
        }
        for current in selected_lines {
                let marker = if matched_lines.contains(&current) { ">" } else { " " };
                let (start, snippet, line_truncated) = bounded_output_line(lines[current], &regex);
                truncated |= line_truncated;
                let rendered = format!(
                    "{}:{}:{} {}",
                    current + 1,
                    offsets.get(current).copied().unwrap_or(0) + start,
                    marker,
                    snippet
                );
                let chars = serde_json::to_string(&rendered).expect("snippet serializes").chars().count();
                if encoded_chars.saturating_add(chars) > MAX_TOOL_RESULT_CHARS.saturating_sub(2_048) {
                    truncated = true;
                    break;
                }
                encoded_chars += chars;
                file_output.push(rendered);
        }
        if file_matches == 0 && !path_match {
            continue;
        }
        let stat = metadata;
        output.push(format!("### {}", path.display()));
        output.push(format!("source_size_bytes: {}\nsize_bytes: {}", stat.len(), content.len()));
        output.push(format!(
            "mtime: {}",
            stat.modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or_default()
        ));
        if path_match {
            output.push(format!(
                "path_match: {}",
                path.file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("")
            ));
        }
        output.extend(file_output);
        if file_matches == 0 && path_match {
            output.push("(matched saved output filename)".to_string());
        }
        output.push(String::new());
        if matches > max_matches || encoded_chars >= MAX_TOOL_RESULT_CHARS.saturating_sub(3_072) {
            truncated = true;
            break;
        }
    }
    if output.last().is_some_and(|line| line.is_empty()) {
        output.pop();
    }
    let body = bounded_tool_output_report(output, truncated, |truncated| format!(
        "# Tool output find\npattern: {pattern}\nscanned_outputs: {scanned}\nmatches: {matches}\ntruncated: {truncated}\noffset_space: model_evidence_utf8\nformat: line:byte_offset:match_marker content"));
    ok(body, format!("{matches} tool output match(es)"))
}

fn bool_arg(args: &Map<String, Value>, key: &str, default: bool) -> Result<bool, ToolOutcome> {
    match args.get(key) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(Value::Null) | None => Ok(default),
        _ => Err(error(format!("{key} must be a boolean"))),
    }
}

fn line_byte_offsets(text: &str) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut cursor = 0usize;
    for line in text.split_inclusive('\n') {
        offsets.push(cursor);
        cursor += line.len();
    }
    if offsets.is_empty() {
        offsets.push(0);
    }
    offsets
}

#[cfg(test)]
fn tool_output_root() -> PathBuf {
    env::temp_dir().join("nola-cli-tool-outputs")
}


fn set_thinking_mode(args: &Map<String, Value>) -> ToolOutcome {
    let Some(enabled) = args.get("enabled").and_then(Value::as_bool) else {
        return error("enabled must be a boolean".to_string());
    };
    let reason = arg_string(args, "reason").unwrap_or_default();
    if reason.trim().is_empty() {
        return error("reason is required".to_string());
    }
    let mode = if enabled { "enabled" } else { "disabled" };
    ok(
        format!(
            "Thinking mode changed for subsequent Agent model calls.\n\
agent_thinking_enabled: {enabled}\n\
agent_thinking_mode: {mode}\n\
reason: {}",
            reason.trim()
        ),
        format!("thinking {mode}"),
    )
}

fn memory_save(args: &Map<String, Value>, binding: &nola_domain::memory_owner::MemoryExecutionBinding) -> ToolOutcome {
    // Providers frequently emit an ISO local date-time when the source
    // transcript has no timezone. The memory contract explicitly permits a
    // date-only value in that situation, so normalize this losslessly to its
    // calendar date before schema validation instead of spending the turn on
    // an avoidable correction loop.
    let normalized = normalize_memory_temporal_arguments(args);
    memory_save_impl(&normalized, binding)
}

fn normalize_memory_temporal_arguments(args: &Map<String, Value>) -> Map<String, Value> {
    let mut normalized = args.clone();
    for field in ["mentioned_at", "event_time", "range_start", "range_end"] {
        let Some(Value::String(value)) = normalized.get(field).cloned() else {
            continue;
        };
        let value = value.trim();
        if chrono::DateTime::parse_from_rfc3339(value).is_ok()
            || chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
        {
            continue;
        }
        let local_date = ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"]
            .into_iter()
            .find_map(|format| chrono::NaiveDateTime::parse_from_str(value, format).ok())
            .map(|date_time| date_time.date().format("%Y-%m-%d").to_string());
        if let Some(local_date) = local_date {
            normalized.insert(field.to_string(), Value::String(local_date));
        }
    }
    normalized
}

fn memory_save_impl(args: &Map<String, Value>, binding: &nola_domain::memory_owner::MemoryExecutionBinding) -> ToolOutcome {
    let scope = args
        .get("scope")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("global");
    let Some(kind) = args
        .get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            matches!(
                *value,
                "user"
                    | "identity"
                    | "feedback"
                    | "decision"
                    | "project"
                    | "reference"
                    | "preference"
                    | "other"
            )
        })
    else {
        return memory_save_invalid_arguments(
            "kind must be one of: user, identity, feedback, decision, project, reference, preference, other".to_string(),
            "Choose one of the enum values declared by the memory_save schema.",
        );
    };
    let Some(content) = args
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return memory_save_invalid_arguments(
            "content is required".to_string(),
            "Provide a non-empty content string.",
        );
    };
    if scope.chars().count() > 512 || scope.contains(['\n', '\r']) {
        return memory_save_invalid_arguments(
            "scope must be a single line no longer than 512 characters".to_string(),
            "Provide a shorter single-line scope.",
        );
    }
    if content.chars().count() > 8_000 {
        return memory_save_invalid_arguments(
            "content must not exceed 8000 characters".to_string(),
            "Reduce content to the smallest durable facts and retry memory_save.",
        );
    }
    let summary = args
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let source = args
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let source_role = args
        .get("source_role")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| matches!(*value, "user" | "assistant" | "tool" | "system" | "other"));
    let temporal_value = |name: &str| {
        args.get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let mentioned_at = temporal_value("mentioned_at");
    let event_time = temporal_value("event_time");
    let range_start = temporal_value("range_start");
    let range_end = temporal_value("range_end");
    let tags = args
        .get("tags")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if summary.is_some_and(|value| value.chars().count() > 1_000) {
        return memory_save_invalid_arguments(
            "summary must not exceed 1000 characters".to_string(),
            "Shorten the optional summary.",
        );
    }
    if source.is_some_and(|value| value.chars().count() > 512) {
        return memory_save_invalid_arguments(
            "source must not exceed 512 characters".to_string(),
            "Shorten the optional source reference.",
        );
    }
    if tags.len() > 32 || tags.iter().any(|tag| tag.chars().count() > 64) {
        return memory_save_invalid_arguments(
            "tags must contain at most 32 values of 64 characters each".to_string(),
            "Reduce the tag count or shorten individual tags.",
        );
    }
    let valid_temporal = |value: &str| {
        chrono::DateTime::parse_from_rfc3339(value).is_ok()
            || chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
    };
    if [mentioned_at, event_time, range_start, range_end]
        .into_iter()
        .flatten()
        .any(|value| !valid_temporal(value))
    {
        return memory_save_invalid_arguments(
            "memory timestamps must be RFC3339 timestamps or ISO dates".to_string(),
            "Retry memory_save with an RFC3339 timestamp including its UTC offset; when the source timezone is unknown, use the ISO date only.",
        );
    }
    if range_start.is_some() != range_end.is_some() {
        return memory_save_invalid_arguments(
            "memory interval requires both range_start and range_end".to_string(),
            "Provide both interval bounds or omit both.",
        );
    }
    if let (Some(start), Some(end)) = (range_start, range_end) {
        let instant = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|date| date.timestamp_millis())
                .or_else(|_| {
                    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map(|date| {
                        date.and_hms_opt(0, 0, 0)
                            .expect("midnight is a valid time")
                            .and_utc()
                            .timestamp_millis()
                    })
                })
                .ok()
        };
        if instant(start) > instant(end) {
            return memory_save_invalid_arguments(
                "memory range_start must not follow range_end".to_string(),
                "Correct the interval ordering and retry memory_save.",
            );
        }
    }
    let confidence = args
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.9);
    let importance = args
        .get("importance")
        .and_then(Value::as_f64)
        .unwrap_or(0.5);
    let sensitivity = args
        .get("sensitivity")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let expires_at = args
        .get("expires_at")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let supersedes = args
        .get("supersedes")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !(0.0..=1.0).contains(&confidence) || !(0.0..=1.0).contains(&importance) {
        return memory_save_invalid_arguments(
            "confidence and importance must be between 0 and 1".to_string(),
            "Use numeric confidence and importance values in the inclusive range 0 through 1.",
        );
    }
    let observed_at = mentioned_at
        .map(str::to_string)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let provenance = json!({
        "source": source.unwrap_or("agent_explicit_save"),
        "observed_at": observed_at,
        "source_role": source_role,
        "temporal": {
            "mentioned_at": mentioned_at,
            "event_time": event_time,
            "range_start": range_start,
            "range_end": range_end,
        },
    });
    match NolaStore::open_default().and_then(|store| {
        store.save_structured_memory(
            binding,
            scope,
            kind,
            content,
            summary,
            &tags,
            source,
            confidence,
            importance,
            sensitivity,
            expires_at,
            &supersedes,
            &provenance,
        )
    }) {
        Ok(observed) => memory_save_observed_outcome(&observed.record, observed.readback_error.as_deref()),
        Err(err) => error(format!("memory_save failed: {err}")),
    }
}

fn memory_save_invalid_arguments(message: String, next_action: &str) -> ToolOutcome {
    error(
        json!({
            "error": "invalid_tool_arguments",
            "message": message,
            "tool_execution_skipped": true,
            "next_action": next_action,
        })
        .to_string(),
    )
}

fn memory_save_observed_outcome(stored: &nola_application::MemoryRecord, readback_error: Option<&str>) -> ToolOutcome {
    use nola_host_protocol::tool_result::{ExecutionReceipt, ExecutionState, ResourceEffect, PostconditionCheck};
    use sha2::{Digest, Sha256};
    let bytes = match serde_json::to_vec(stored) {
        Ok(bytes) => bytes,
        Err(err) => return error(format!("Memory persisted but version encoding failed: {err}")),
    };
    let version = format!("memory-record:v1:sha256:{:x}", Sha256::digest(&bytes));
    let id = &stored.record.id;
    let artifact_uri = format!("nola://memory/{id}");
    let mut outcome = ok(
        format!("memory_saved: {id}\nartifact_uri: {artifact_uri}\ncontent: {}\nnext_action: do not pass the nola URI to a filesystem reader; search only when the user requested recall, reconciliation, deletion, or scope isolation", stored.record.content),
        "memory saved".into(),
    );
    outcome.execution = Some(ExecutionReceipt {
        local_stop: None,
        state: ExecutionState::Completed,
        effects: vec![ResourceEffect {
            resource_uri: artifact_uri.clone(), operation: "write".into(),
            before_version: None, after_version: Some(version.clone()),
        }],
        checks: vec![PostconditionCheck {
            kind: nola_host_protocol::tool_result::ExecutionCheckKind::Postcondition,
            resource_uri: artifact_uri.clone(), version,
            predicate: "canonical_memory_record_read_back_matches_commit".into(), passed: readback_error.is_none(),
        }],
    });
    outcome.artifacts.push(nola_core::agent::host::HostArtifact {
        uri: artifact_uri,
        name: format!("memory record {id}"),
        mime_type: Some("application/json".to_string()),
        operation: Some("write".to_string()),
    });
    if let Some(message) = readback_error {
        outcome.error = true;
        outcome.text.push_str(&format!("\nVerification failed after commit: {message}"));
    }
    outcome
}

#[cfg(test)]
#[test]
fn memory_save_success_exposes_a_canonical_persisted_record_artifact() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("memory.db")).unwrap();
    let observed = store.save_structured_memory(&nola_domain::memory_owner::MemoryExecutionBinding::local_profile(), "global", "reference", "durable fact", None,
        &[], None, 1.0, 0.0, None, None, &[], &json!({})).unwrap();
    assert!(observed.readback_error.is_none());
    let stored = observed.record;
    let outcome = memory_save_observed_outcome(&stored, None);
    let expected_uri = format!("nola://memory/{}", stored.record.id);

    assert!(!outcome.error);
    assert_eq!(outcome.artifacts.len(), 1);
    let artifact = &outcome.artifacts[0];
    assert_eq!(artifact.uri, expected_uri);
    assert_eq!(artifact.mime_type.as_deref(), Some("application/json"));
    assert_eq!(artifact.operation.as_deref(), Some("write"));
    artifact.validate().expect("canonical Host artifact");
    assert!(outcome.text.contains(&format!("artifact_uri: {expected_uri}")));
    let receipt = outcome.execution.unwrap();
    receipt.validate().unwrap();
    assert_eq!(receipt.effects[0].resource_uri, expected_uri);
    assert_eq!(receipt.effects[0].after_version.as_ref(), Some(&receipt.checks[0].version));
    assert!(receipt.checks[0].passed);
    assert_eq!(receipt.checks[0].predicate, "canonical_memory_record_read_back_matches_commit");
    assert!(receipt.checks[0].version.starts_with("memory-record:v1:sha256:"));
    drop(store);
    let reopened = NolaStore::open(&root.path().join("memory.db")).unwrap();
    assert_eq!(reopened.memory_record(&stored.record.id).unwrap().unwrap()["content"], "durable fact");
    let mut changed = stored.clone();
    changed.record.content = "different persisted contents".into();
    assert_ne!(memory_save_observed_outcome(&changed, None).execution.unwrap().checks[0].version, receipt.checks[0].version);
    let failed = memory_save_observed_outcome(&stored, Some("read-back changed"));
    assert!(failed.error);
    let failed_receipt = failed.execution.unwrap();
    assert_eq!(failed_receipt.effects, receipt.effects);
    assert!(!failed_receipt.checks[0].passed);
}

#[cfg(test)]
#[test]
fn memory_save_normalizes_timezone_unknown_local_datetime_to_iso_date() {
    let normalized = normalize_memory_temporal_arguments(
        json!({
            "kind": "reference",
            "content": "dated fact",
            "event_time": "2026-09-12T08:30:00"
        })
        .as_object()
        .expect("arguments"),
    );
    assert_eq!(normalized["event_time"], "2026-09-12");
}

fn memory_search(args: &Map<String, Value>, binding: &nola_domain::memory_owner::MemoryExecutionBinding) -> ToolOutcome {
    let scope = args
        .get("scope")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("global");
    let kind = args
        .get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let limit = args
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(10)
        .clamp(1, 50) as usize;
    match NolaStore::open_default()
        .and_then(|store| search_memories_with_soft_kind(&store, binding, scope, query, kind, limit))
    {
        Ok((memories, kind_filter_relaxed)) => {
            let text = serde_json::to_string_pretty(&json!({
                "scope": scope,
                "requested_kind": kind,
                "kind_filter_relaxed": kind_filter_relaxed,
                "count": memories.len(),
                "memories": memories,
            }))
            .unwrap_or_else(|_| "{\"memories\":[]}".to_string());
            ok(text, format!("{} memories", memories.len()))
        }
        Err(err) => error(format!("memory_search failed: {err}")),
    }
}

fn apply_core_memory_selection(memories: Vec<Value>, limit: usize) -> Vec<Value> {
    use nola_core::agent::{
        select_memories, MemoryKind, MemoryNamespace, MemoryPolicy, MemoryProvenance,
        MemoryProvenanceKind, MemoryRecord, MemoryRetention, MemoryScope, MemorySelectionInput,
        MemorySourceRole, MemoryTemporalMetadata,
    };

    let records = memories
        .iter()
        .filter_map(|value| {
            let id = value.get("id")?.as_str()?.trim();
            let content = value.get("content")?.as_str()?.trim();
            if id.is_empty() || content.is_empty() {
                return None;
            }
            let stored_scope = value
                .get("scope")?
                .as_str()?
                .trim();
            let (scope, namespace) = match stored_scope {
                "global" => (MemoryScope::Global, MemoryNamespace::Personal),
                "workspace" => (MemoryScope::Global, MemoryNamespace::Workspace),
                "conversation" => (MemoryScope::Conversation, MemoryNamespace::Conversation),
                "turn" => (MemoryScope::Turn, MemoryNamespace::Conversation),
                custom if custom.starts_with("custom:") && !custom[7..].trim().is_empty() => (
                    MemoryScope::Global, MemoryNamespace::Custom(custom[7..].to_string()),
                ),
                _ => return None,
            };
            let kind = match value.get("kind").and_then(Value::as_str) {
                Some("preference") => MemoryKind::Preference,
                Some("user" | "identity") => MemoryKind::Identity,
                Some("decision") => MemoryKind::Decision,
                Some("project") => MemoryKind::Project,
                Some("feedback") => MemoryKind::Feedback,
                Some("reference") => MemoryKind::Reference,
                _ => MemoryKind::Other,
            };
            let tags = value
                .get("tags")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            let supersedes = value
                .get("supersedes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            let provenance_value = value.get("provenance").and_then(Value::as_object);
            let provenance_source = provenance_value
                .and_then(|object| object.get("source"))
                .and_then(Value::as_str)
                .or_else(|| value.get("source").and_then(Value::as_str))
                .filter(|source| !source.trim().is_empty())
                .unwrap_or("cli_memory_store")
                .to_string();
            let provenance_kind = match provenance_value
                .and_then(|object| object.get("kind"))
                .and_then(Value::as_str)
            {
                Some("explicit_user") => MemoryProvenanceKind::ExplicitUser,
                Some("tool_result") => MemoryProvenanceKind::ToolResult,
                Some("imported") => MemoryProvenanceKind::Imported,
                Some("synced") => MemoryProvenanceKind::Synced,
                Some("agent_inference") => MemoryProvenanceKind::AgentInference,
                _ if provenance_source == "agent_explicit_save" => {
                    MemoryProvenanceKind::AgentInference
                }
                _ => MemoryProvenanceKind::Other,
            };
            let observed_at = provenance_value
                .and_then(|object| object.get("observed_at"))
                .and_then(Value::as_str)
                .or_else(|| value.get("updated_at").and_then(Value::as_str))
                .map(str::to_string);
            let source_role = provenance_value
                .and_then(|object| object.get("source_role"))
                .cloned()
                .and_then(|value| serde_json::from_value::<MemorySourceRole>(value).ok());
            let source_roles = provenance_value
                .and_then(|object| object.get("source_roles"))
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
                .unwrap_or_default();
            let temporal = provenance_value
                .and_then(|object| object.get("temporal"))
                .cloned()
                .and_then(|value| serde_json::from_value::<MemoryTemporalMetadata>(value).ok())
                .unwrap_or_default();
            let retention = value
                .get("expires_at")
                .and_then(Value::as_str)
                .filter(|expires_at| !expires_at.trim().is_empty())
                .map(|expires_at| MemoryRetention::ExpiresAt {
                    expires_at: expires_at.to_string(),
                })
                .unwrap_or_else(|| {
                    if supersedes.is_empty() {
                        MemoryRetention::Permanent
                    } else {
                        MemoryRetention::UntilSuperseded
                    }
                });
            Some(MemoryRecord {
                id: id.to_string(),
                owner: serde_json::from_value(value.get("owner")?.clone()).ok()?,
                namespace,
                scope,
                kind,
                content: content.to_string(),
                summary: value
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                tags,
                confidence: value
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32,
                importance: value
                    .get("importance")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32,
                sensitivity: value
                    .get("sensitivity")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                retention,
                supersedes,
                provenance: MemoryProvenance {
                    kind: provenance_kind,
                    source: provenance_source,
                    session_id: provenance_value
                        .and_then(|object| object.get("session_id"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    turn_id: provenance_value
                        .and_then(|object| object.get("turn_id"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    item_ids: provenance_value
                        .and_then(|object| object.get("item_ids"))
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                    observed_at,
                    source_role,
                    source_roles,
                    temporal,
                },
            })
        })
        .collect::<Vec<_>>();
    let plan = select_memories(MemorySelectionInput {
        policy: MemoryPolicy {
            search_limit: limit,
            // Application Recall admits ordinary records at its authorization
            // boundary. Preserve the same policy while applying Core's final
            // ranking/supersession pass; otherwise this second pass silently
            // drops every record explicitly labeled `normal`.
            allowed_sensitivities: vec!["normal".to_string()],
            ..MemoryPolicy::default()
        },
        records,
        mode: nola_core::agent::MemoryRecallMode::OnDemand,
        allowed_namespaces: Vec::new(),
        include_session_retention: true,
        now: Some(chrono::Utc::now().to_rfc3339()),
        maximum_input_tokens: None,
        token_accounting: nola_core::agent::MemoryTokenAccounting::HostExact,
        token_counts: std::collections::BTreeMap::new(),
    });
    let selected = plan
        .selected
        .into_iter()
        .map(|entry| (entry.record.id, entry.requires_current_state_verification))
        .collect::<std::collections::BTreeMap<_, _>>();
    memories
        .into_iter()
        .filter_map(|mut value| {
            let id = value.get("id")?.as_str()?;
            let requires_verification = *selected.get(id)?;
            if let Value::Object(object) = &mut value {
                object.insert(
                    "requires_current_state_verification".to_string(),
                    Value::Bool(requires_verification),
                );
            }
            Some(value)
        })
        .collect()
}

#[cfg(test)]
mod core_memory_policy_tests {
    use super::*;

    fn memory(id: &str, kind: &str, confidence: f64, supersedes: &[&str]) -> Value {
        json!({
            "id": id,
            "owner": {"kind":"profile","profile_id":"local"},
            "scope": "global",
            "kind": kind,
            "content": format!("memory {id}"),
            "summary": null,
            "tags": [],
            "source": "agent_explicit_save",
            "confidence": confidence,
            "importance": 0.8,
            "sensitivity": null,
            "expires_at": null,
            "supersedes": supersedes,
            "provenance": {
                "kind": if kind == "preference" { "explicit_user" } else { "tool_result" },
                "source": "fixture",
                "observed_at": "2026-08-01T00:00:00Z",
                "source_role": "user",
                "temporal": {
                    "mentioned_at": "2026-08-01T00:00:00Z",
                    "event_time": "2026-07-31"
                }
            },
            "updated_at": "2026-08-01T00:00:00Z"
        })
    }

    #[test]
    fn current_memory_projection_requires_its_declared_scope_and_owner() {
        let mut unbound = memory("missing-scope", "preference", 0.95, &[]);
        unbound.as_object_mut().unwrap().remove("scope");
        let mut unknown = memory("unknown-scope", "preference", 0.95, &[]);
        unknown["scope"] = json!("conversaton");
        assert!(apply_core_memory_selection(vec![unbound, unknown], 10).is_empty());
    }

    #[test]
    fn core_policy_filters_low_confidence_and_superseded_rows() {
        let selected = apply_core_memory_selection(
            vec![
                memory("old", "preference", 0.95, &[]),
                memory("low", "preference", 0.2, &[]),
                memory("current", "preference", 0.95, &["old"]),
            ],
            10,
        );
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0]["id"], "current");
        assert_eq!(selected[0]["requires_current_state_verification"], false);
        assert_eq!(selected[0]["provenance"]["source_role"], "user");
        assert_eq!(
            selected[0]["provenance"]["temporal"]["event_time"],
            "2026-07-31"
        );
    }

    #[test]
    fn core_policy_marks_mutable_tool_snapshots_for_verification() {
        let selected =
            apply_core_memory_selection(vec![memory("project", "project", 0.95, &[])], 10);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0]["requires_current_state_verification"], true);
    }
}

fn search_memories_with_soft_kind(
    store: &NolaStore,
    binding: &nola_domain::memory_owner::MemoryExecutionBinding,
    scope: &str,
    query: Option<&str>,
    kind: Option<&str>,
    limit: usize,
) -> Result<(Vec<Value>, bool), String> {
    let candidate_limit = limit.saturating_mul(4).clamp(limit, 50);
    let filtered = apply_core_memory_selection(
        store.search_memories(binding, scope, query, kind, candidate_limit)?,
        limit,
    );
    let Some(query) = query else {
        return Ok((filtered, false));
    };

    if kind.is_some() {
        let broad = apply_core_memory_selection(
            store.search_memories(binding, scope, Some(query), None, candidate_limit)?,
            limit,
        );
        let filtered_relevance = filtered
            .iter()
            .map(|candidate| memory_candidate_relevance(candidate, query))
            .max()
            .unwrap_or_default();
        let broad_relevance = broad
            .iter()
            .map(|candidate| memory_candidate_relevance(candidate, query))
            .max()
            .unwrap_or_default();

        // A category supplied by the model is only a preference. Keep it when
        // it finds an equally relevant fact, but do not let a partial lexical
        // match hide a better match saved under another category.
        if !broad.is_empty() && (filtered.is_empty() || broad_relevance > filtered_relevance) {
            let broad = if filtered.is_empty() {
                broad
            } else {
                broad
                    .into_iter()
                    .filter(|candidate| {
                        memory_candidate_relevance(candidate, query) > filtered_relevance
                    })
                    .collect()
            };
            return Ok((broad, true));
        }
    }

    if !filtered.is_empty() {
        return Ok((filtered, false));
    }

    // Model-selected memory kinds are classification hints, not authoritative
    // facts. A later model may classify the same durable fact differently from
    // the model that saved it. Never let that hide a textually relevant memory.
    if kind.is_some() {
        let broad_exact = apply_core_memory_selection(
            store.search_memories(binding, scope, Some(query), None, candidate_limit)?,
            limit,
        );
        if !broad_exact.is_empty() {
            return Ok((broad_exact, true));
        }
    }

    let candidates = store.search_memories(binding, scope, None, None, 50)?;
    Ok((
        apply_core_memory_selection(
            rank_memory_candidates(candidates, query, candidate_limit),
            limit,
        ),
        kind.is_some(),
    ))
}

fn memory_candidate_relevance(candidate: &Value, query: &str) -> (bool, usize) {
    let searchable = ["content", "summary", "source"]
        .into_iter()
        .filter_map(|key| candidate.get(key).and_then(Value::as_str))
        .chain(
            candidate
                .get("tags")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        )
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let normalized_query = query.trim().to_lowercase();
    let terms = memory_query_terms(query);
    (
        !normalized_query.is_empty() && searchable.contains(&normalized_query),
        terms
            .iter()
            .filter(|term| searchable.contains(*term))
            .count(),
    )
}

fn rank_memory_candidates(mut candidates: Vec<Value>, query: &str, limit: usize) -> Vec<Value> {
    let terms = memory_query_terms(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let minimum_matches = if terms.len() == 1 {
        1
    } else {
        (terms.len() * 3).div_ceil(5).max(2)
    };
    let mut scored = candidates
        .drain(..)
        .filter_map(|candidate| {
            let matched = memory_candidate_relevance(&candidate, query).1;
            (matched >= minimum_matches).then_some((matched, candidate))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.0.cmp(&left.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, candidate)| candidate)
        .collect()
}

fn memory_query_terms(query: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "and", "are", "for", "from", "in", "is", "of", "on", "or", "the", "to", "with",
    ];
    let mut terms = query
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 2 && !STOP_WORDS.contains(term))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms.truncate(16);
    terms
}

fn document_search(args: &Map<String, Value>) -> ToolOutcome {
    let scope = args
        .get("scope")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("web:local");
    let kind = args
        .get("kind")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let limit = args
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(10)
        .clamp(1, 100) as usize;
    match NolaStore::open_default()
        .and_then(|store| store.search_documents(scope, query, kind, limit))
    {
        Ok(documents) => {
            let text = serde_json::to_string_pretty(&json!({
                "scope": scope,
                "kind": kind,
                "count": documents.len(),
                "documents": documents,
            }))
            .unwrap_or_else(|_| "{\"documents\":[]}".to_string());
            ok(text, format!("{} documents", documents.len()))
        }
        Err(err) => error(format!("document_search failed: {err}")),
    }
}

fn summarize_todo_changes(old_todos: &[AgentTodo], new_todos: &[AgentTodo]) -> Vec<String> {
    let mut changes = Vec::new();
    for (index, todo) in new_todos.iter().enumerate() {
        match old_todos.get(index) {
            Some(previous)
                if previous.content == todo.content && previous.status == todo.status => {}
            Some(previous) if previous.content == todo.content => changes.push(format!(
                "{}: {} -> {} ({})",
                index + 1,
                previous.status,
                todo.status,
                compact_todo_content(&todo.content)
            )),
            Some(previous) => changes.push(format!(
                "{}: replaced ({}) -> ({})",
                index + 1,
                compact_todo_content(&previous.content),
                compact_todo_content(&todo.content)
            )),
            None => changes.push(format!(
                "{}: added {} ({})",
                index + 1,
                todo.status,
                compact_todo_content(&todo.content)
            )),
        }
    }
    for (index, todo) in old_todos.iter().enumerate().skip(new_todos.len()) {
        changes.push(format!(
            "{}: removed ({})",
            index + 1,
            compact_todo_content(&todo.content)
        ));
    }
    changes
}

fn compact_todo_content(content: &str) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= 48 {
        return compact;
    }
    let mut shortened = compact.chars().take(47).collect::<String>();
    shortened.push('…');
    shortened
}
