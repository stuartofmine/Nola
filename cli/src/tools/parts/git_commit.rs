// SPDX-License-Identifier: Apache-2.0
// A bounded Host-owned Git transaction, with postconditions checked against
// Git objects rather than model-declared verification effects. Not a sandbox:
// hooks and external concurrent Git writers remain independent actors.

/// Paths are repository-relative facts, never a task-selection decision.
fn git_change_inventory(repository_root: &Path, observed_root: &Path) -> Result<Value, String> {
    let output = git_command_output(repository_root, &[
        "status", "--porcelain=v1", "-z", "--untracked-files=all", "--no-renames", "--",
        observed_root.to_str().ok_or("Non-UTF8 observed root")?,
    ])?;
    git_inventory_from_status(&output, repository_root, observed_root)
}

fn git_inventory_from_status(output: &[u8], repository_root: &Path, observed_root: &Path) -> Result<Value, String> {
    let mut entries = Vec::new();
    let mut total = 0usize;
    let mut untracked = 0usize;
    let mut bytes = 0usize;
    let mut truncated = false;
    for record in output.split(|byte| *byte == 0).filter(|record| !record.is_empty()) {
        if record.len() < 4 || record[2] != b' ' {
            return Err("Invalid Git porcelain status record".into());
        }
        let path = std::str::from_utf8(&record[3..]).map_err(|_| "Non-UTF8 Git path")?;
        total += 1;
        if &record[..2] == b"??" { untracked += 1; }
        if truncated || entries.len() >= 512 || bytes + path.len() > 128 * 1024 {
            truncated = true;
            continue;
        }
        bytes += path.len();
        entries.push(json!({"path":path,
            "index_status":char::from(record[0]).to_string(),
            "worktree_status":char::from(record[1]).to_string(),
            "tracking":if &record[..2] == b"??" { "untracked" } else { "tracked" }}));
    }
    Ok(json!({"repository_root":repository_root,"observed_root":observed_root,
        "path_base":"repository_root", "repository_scope_complete":repository_root == observed_root,
        "total_entries":total,"tracked_entries":total-untracked,"untracked_entries":untracked,
        "returned_entries":entries.len(),"entries":entries,"truncated":truncated,
        "tracking_semantics":"Tracking describes index membership only. Untracked files are pending additions, not evidence of when or by whom they were created. Determine task-relative existence from earlier observations, not tracking status.",
        "creation_time_attested":false,
        "content_audited":false,"task_completion_attested":false}))
}

fn git_commit(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let resolved = required_string(args, "root").and_then(|root| {
        if !Path::new(&root).is_absolute() {
            return Err(error("root must be absolute".into()));
        }
        resolve_allowed(&root, &ctx.allowed_dirs, true)
    });
    let root = match resolved {
        Ok(root) => root,
        Err(outcome) => {
            return error_data(
                json!({"error":"git_commit_preflight_failed", "tool_execution_skipped":true,
            "message":outcome.text}),
            )
        }
    };
    let mut effects_started = false;
    let result =
        git_commit_verified_with_cancellation(args, &root, &mut effects_started, &ctx.cancellation);
    match result {
        Ok(receipt) => ToolOutcome {
            full_output_ref: None,
            data: Some(receipt.clone()),
            execution: Some(git_execution_receipt(&root, &receipt)),
            display: format!(
                "Committed {} files ({})",
                receipt["paths"].as_array().unwrap().len(),
                receipt["commit"].as_str().unwrap()
            ),
            text: receipt.to_string(),
            error: false,
            artifacts: Vec::new(),
            activation: None,
        },
        Err(message) => error_data(json!({
            "error":if effects_started { "git_commit_execution_unsettled" } else { "git_commit_precondition_failed" }, "message":message,
            "tool_execution_skipped":!effects_started,
            "effects_may_have_occurred":effects_started,
            "observed_head":git_commit_text(&root, &["rev-parse", "HEAD"]).ok(),
            "recovery":if effects_started {
                "Inspect HEAD and index before retrying. No rollback or history rewrite was performed."
            } else {
                "Correct the concrete selection or refresh conflicting state. This operation does not require covering other changed files or adopting a workflow."
            }
        })),
    }
}

fn git_execution_receipt(
    root: &Path,
    receipt: &Value,
) -> nola_host_protocol::tool_result::ExecutionReceipt {
    use nola_host_protocol::tool_result::*;
    let resource_uri = url::Url::from_file_path(root.join(".git"))
        .expect("absolute Git path")
        .to_string();
    let version = receipt["commit"]
        .as_str()
        .expect("verified commit")
        .to_string();
    ExecutionReceipt {
        local_stop: None,
        state: ExecutionState::Completed,
        effects: vec![ResourceEffect {
            resource_uri: resource_uri.clone(),
            operation: "git.commit".into(),
            before_version: receipt["before_commit"].as_str().map(str::to_string),
            after_version: Some(version.clone()),
        }],
        checks: [
            "parent_matches",
            "tree_matches",
            "selected_paths_exact",
            "unselected_index_preserved",
            "selected_worktree_matches",
        ]
        .into_iter()
        .map(|predicate| PostconditionCheck {
            kind: nola_host_protocol::tool_result::ExecutionCheckKind::Postcondition,
            resource_uri: resource_uri.clone(),
            version: version.clone(),
            predicate: predicate.into(),
            passed: true,
        })
        .chain(receipt["paths"].as_array().expect("verified paths").iter().map(|path| PostconditionCheck {
            kind: ExecutionCheckKind::Postcondition,
            resource_uri: url::Url::from_file_path(root.join(path.as_str().expect("verified path")))
                .expect("absolute selected path").to_string(),
            version: version.clone(),
            predicate: "git.committed_path".into(),
            passed: true,
        }))
        .collect(),
    }
}

fn git_commit_text(root: &Path, arguments: &[&str]) -> Result<String, String> {
    String::from_utf8(git_command_output(root, arguments)?)
        .map(|text| text.trim().to_string())
        .map_err(|_| "Git returned non-UTF8 data".into())
}

fn git_commit_paths(
    root: &Path,
    arguments: &[&str],
) -> Result<std::collections::BTreeSet<String>, String> {
    git_command_output(root, arguments)?
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map_err(|_| "Non-UTF8 paths require a separate operation".into())
        })
        .collect()
}

fn git_commit_selected(
    args: &Map<String, Value>,
    key: &str,
    root: &Path,
) -> Result<std::collections::BTreeSet<String>, String> {
    let entries = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} must be an array"))?;
    let mut paths = std::collections::BTreeSet::new();
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    for entry in entries {
        let supplied = entry.as_str().ok_or("Every path must be a string")?;
        let supplied_path = Path::new(supplied);
        let relative = if supplied_path.is_absolute() {
            if let Ok(path) = supplied_path
                .strip_prefix(root)
                .or_else(|_| Path::new(supplied).strip_prefix(&canonical_root))
            {
                path.to_path_buf()
            } else {
                // Normalize directory aliases without following a selected leaf
                // symlink: Git commits the link object, not its target.
                let parent = supplied_path
                    .parent()
                    .ok_or("Missing selected path parent")?
                    .canonicalize()
                    .map_err(|error| error.to_string())?;
                parent
                    .join(
                        supplied_path
                            .file_name()
                            .ok_or("Missing selected file name")?,
                    )
                    .strip_prefix(&canonical_root)
                    .map_err(|_| format!("Selected path is outside the repository: {supplied}"))?
                    .to_path_buf()
            }
        } else {
            supplied_path.to_path_buf()
        };
        let path = relative.to_str().ok_or("Selected path is not UTF-8")?;
        if path.is_empty()
            || path.contains('\0')
            || Path::new(path)
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
            || path.split('/').any(|part| part == ".git")
        {
            return Err(format!(
                "Expected an individual repository-relative path: {path}"
            ));
        }
        if !paths.insert(path.to_string()) {
            return Err(format!("Duplicate path: {path}"));
        }
    }
    Ok(paths)
}

fn git_commit_verified_with_cancellation(
    args: &Map<String, Value>,
    root: &Path,
    effects_started: &mut bool,
    cancellation: &RuntimeExecutionControl,
) -> Result<Value, String> {
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    let git_root = PathBuf::from(git_commit_text(root, &["rev-parse", "--show-toplevel"])?);
    if git_root.canonicalize().map_err(|e| e.to_string())? != canonical {
        return Err("root must be the Git repository root, not a subdirectory".into());
    }
    let selected = git_commit_selected(args, "paths", root)?;
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or("A nonempty commit message is required")?;
    if selected.is_empty() {
        return Err("Select at least one individual file".into());
    }
    let uri_bytes: usize = selected.iter().map(|path| {
        url::Url::from_file_path(canonical.join(path)).expect("absolute selected path").as_str().len()
    }).sum();
    if selected.len() > 1000 || uri_bytes > 128 * 1024 {
        return Err("Selected paths exceed the bounded execution receipt (1000 files / 128 KiB resource URIs); this request cannot be attested within the path-proof budget. No changes were made.".into());
    }
    for path in &selected {
        if fs::symlink_metadata(root.join(path)).is_ok_and(|metadata| metadata.is_dir()) {
            return Err(format!(
                "Selected path is a directory, not an individual file: {path}. List its changed files and pass their exact paths. No changes were made; additional permission does not correct this selection."
            ));
        }
    }
    let before = git_commit_text(root, &["rev-parse", "HEAD"])?;
    let mut changed = git_commit_paths(
        root,
        &["diff", "--name-only", "--no-renames", "-z", "HEAD", "--"],
    )?;
    changed.extend(git_commit_paths(
        root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?);
    if !selected.is_subset(&changed) {
        return Err(
            "Selected paths must have current changes; refresh those paths before retrying".into(),
        );
    }
    let index_before = git_index_entries(root)?;
    // The operation is scoped to selected paths, not a repository-wide task.
    // Unselected source, documents and staged work need no exclusion reason.
    for path in &selected {
        let full = root.join(path);
        if bash_token_is_sensitive_path(path) {
            return Err(format!(
                "Selected path is protected by resource policy: {path}"
            ));
        }
        // Git records a symlink itself, not its target. Resolve the parent to
        // prevent an intermediate symlink from changing the selected root.
        resolve_allowed(
            &full
                .parent()
                .ok_or("Missing path parent")?
                .to_string_lossy(),
            std::slice::from_ref(&canonical),
            false,
        )
        .map_err(|outcome| outcome.text)?;
    }
    // Compute the expected tree in a private index. Never stage unrelated
    // files or rewrite the user's existing index just to prepare this call.
    let preparation = tempfile::tempdir().map_err(|error| error.to_string())?;
    let index = preparation.path().join("index");
    git_private_index_output(root, &index, &["read-tree", &before])?;
    let mut stage_args = vec!["add", "--"];
    stage_args.extend(selected.iter().map(String::as_str));
    // Git clean filters can execute user-configured programs even with a
    // private index. From here failures cannot truthfully claim no execution.
    *effects_started = true;
    git_private_index_output(root, &index, &stage_args)?;
    let expected_tree = String::from_utf8(git_private_index_output(root, &index, &["write-tree"])?)
        .map_err(|error| error.to_string())?
        .trim()
        .to_string();
    if git_commit_text(root, &["rev-parse", "HEAD"])? != before {
        return Err("HEAD changed concurrently before commit".into());
    }
    if git_index_entries(root)? != index_before {
        return Err("Index changed concurrently before commit; refresh state".into());
    }
    // Native commit --only needs new files to be known to the real index.
    // Register intent-to-add only for selected new paths; never stage or clear
    // unrelated entries. On failure this selected-path effect is reported,
    // not silently rolled back over possible concurrent user changes.
    let new_paths: Vec<&str> = selected
        .iter()
        .filter(|path| {
            !index_before.contains_key(*path) && fs::symlink_metadata(root.join(path)).is_ok()
        })
        .map(String::as_str)
        .collect();
    if !new_paths.is_empty() {
        let mut args = vec!["add", "--intent-to-add", "--"];
        args.extend(new_paths);
        git_command_output(root, &args)?;
    }
    git_commit_process(root, message, &selected, cancellation)?;
    let commit = git_commit_text(root, &["rev-parse", "HEAD"])?;
    let parent = git_commit_text(root, &["rev-parse", "HEAD^"])?;
    let tree = git_commit_text(root, &["rev-parse", "HEAD^{tree}"])?;
    let actual = git_commit_paths(
        root,
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "--no-renames",
            "-r",
            "-z",
            "HEAD",
        ],
    )?;
    if parent != before || tree != expected_tree || actual != selected {
        return Err(format!(
            "Commit {commit} exists but parent/tree/path postconditions failed"
        ));
    }
    let mut diff_args = vec!["diff", "--quiet", "HEAD", "--"];
    diff_args.extend(selected.iter().map(String::as_str));
    git_command_output(root, &diff_args)?;
    let outside_selection = |entries: BTreeMap<String, String>| {
        entries
            .into_iter()
            .filter(|(path, _)| !selected.contains(path))
            .collect::<BTreeMap<_, _>>()
    };
    if outside_selection(index_before) != outside_selection(git_index_entries(root)?) {
        return Err(format!(
            "Commit {commit} exists but unrelated index entries changed; inspect before retrying"
        ));
    }
    let remaining = git_change_inventory(&canonical, &canonical)?;
    Ok(
        json!({"type":"git_commit_receipt", "root":canonical, "before_commit":before,
        "commit":commit, "parent":parent, "expected_tree":expected_tree, "actual_tree":tree,
        "paths":actual, "unselected_index_preserved":true, "postconditions_verified":true,
        "scope_source":"explicit_selection_checked_against_git_objects", "task_completion_attested":false,
        "workspace_clean_at_observation":remaining["total_entries"] == 0,
        "remaining_changed_file_count":remaining["total_entries"],"remaining_changes":remaining}),
    )
}

fn git_commit_process(
    root: &Path,
    message: &str,
    selected: &std::collections::BTreeSet<String>,
    cancellation: &RuntimeExecutionControl,
) -> Result<(), String> {
    let mut arguments = vec!["commit", "--only", "-m", message, "--"];
    arguments.extend(selected.iter().map(String::as_str));
    git_mutation_process(root, &arguments, cancellation, "Git commit")
}

fn git_mutation_process(
    root: &Path,
    arguments: &[&str],
    cancellation: &RuntimeExecutionControl,
    operation: &str,
) -> Result<(), String> {
    let stdout = tempfile::tempfile().map_err(|e| e.to_string())?;
    let stderr = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut command = git_operation_command(root);
    command
        .current_dir(root)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        if cancellation.is_requested() || started.elapsed() >= Duration::from_secs(120) {
            let _ = terminate_process_tree(&mut child, Duration::from_secs(2));
            return Err(format!("{operation} interrupted or timed out; inspect Git metadata and working directories before retrying"));
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => return Err(format!(
                "{operation} exited with {status}; inspect Git metadata and working directories; no automatic rollback was performed."
            )),
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

#[cfg(test)]
mod git_commit_tests {
    use super::*;

    fn git_commit_verified(
        args: &Map<String, Value>,
        root: &Path,
        started: &mut bool,
    ) -> Result<Value, String> {
        git_commit_verified_with_cancellation(
            args,
            root,
            started,
            &RuntimeExecutionControl::default(),
        )
    }

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["config", "user.name", "Fixture"],
            vec!["config", "user.email", "fixture@localhost"],
            vec!["config", "commit.gpgsign", "false"],
        ] {
            git_command_output(root.path(), &args).unwrap();
        }
        fs::write(root.path().join("README.md"), "before\n").unwrap();
        git_command_output(root.path(), &["add", "README.md"]).unwrap();
        git_command_output(root.path(), &["commit", "-m", "baseline"]).unwrap();
        fs::write(root.path().join("README.md"), "after\n").unwrap();
        fs::write(root.path().join("native_host_output.rs"), "fn main() {}\n").unwrap();
        fs::write(root.path().join(".env"), "FAKE_SECRET=fixture\n").unwrap();
        fs::write(root.path().join("nola"), b"\xcf\xfa\xed\xfeFAKE").unwrap();
        root
    }

    fn input() -> Value {
        json!({"paths":["README.md", "native_host_output.rs"], "message":"fixture change"})
    }

    #[test]
    fn inventory_preserves_exact_paths_tracking_and_scope_without_task_claims() {
        let fixture = fixture();
        let root = fixture.path().canonicalize().unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/a\nb.txt"), "new").unwrap();
        let inventory = git_change_inventory(&root, &root.join("nested")).unwrap();
        assert_eq!(inventory["total_entries"], 1);
        assert_eq!(inventory["entries"][0]["path"], "nested/a\nb.txt");
        assert_eq!(inventory["entries"][0]["tracking"], "untracked");
        assert_eq!(inventory["repository_scope_complete"], false);
        assert_eq!(inventory["content_audited"], false);
        assert_eq!(inventory["task_completion_attested"], false);
        assert_eq!(inventory["creation_time_attested"], false);
        assert!(inventory["tracking_semantics"].as_str().unwrap().contains("not evidence of when or by whom"));
    }

    #[test]
    fn commit_exposes_remaining_files_and_typed_selected_path_checks() {
        let fixture = fixture();
        let root = fixture.path().canonicalize().unwrap();
        let receipt = git_commit_verified(input().as_object().unwrap(), &root, &mut false).unwrap();
        let remaining = receipt["remaining_changes"]["entries"].as_array().unwrap();
        assert!(remaining.iter().any(|entry| entry["path"] == "nola" && entry["tracking"] == "untracked"));
        assert_eq!(receipt["remaining_changed_file_count"], remaining.len());
        let execution = git_execution_receipt(&root, &receipt);
        execution.validate().unwrap();
        let paths: Vec<_> = execution.checks.iter().filter(|check| check.predicate == "git.committed_path").collect();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|check| check.version == receipt["commit"].as_str().unwrap()));
        assert_eq!(execution.effects.len(), 1);
    }

    #[test]
    fn inventory_truncation_is_explicit_and_commit_budget_precedes_effects() {
        let fixture = fixture();
        let root = fixture.path().canonicalize().unwrap();
        let paths: Vec<_> = (0..1001).map(|index| format!("item-{index}")).collect();
        for path in &paths { fs::write(root.join(path), "new").unwrap(); }
        let inventory = git_change_inventory(&root, &root).unwrap();
        assert_eq!(inventory["returned_entries"], 512);
        assert_eq!(inventory["truncated"], true);
        assert!(inventory["total_entries"].as_u64().unwrap() > 512);
        let mut started = false;
        let args = json!({"paths":paths,"message":"too many"});
        assert!(git_commit_verified(args.as_object().unwrap(), &root, &mut started).unwrap_err().contains("bounded execution receipt"));
        assert!(!started);
    }

    #[test]
    fn directory_selection_reports_the_contract_error_before_change_membership() {
        let root = fixture();
        fs::create_dir(root.path().join("Sources")).unwrap();
        fs::write(root.path().join("Sources/new.swift"), "fixture").unwrap();
        let before = git_commit_text(root.path(), &["rev-parse", "HEAD"]).unwrap();
        let args = json!({"paths":["Sources/"], "message":"fixture"});
        let mut started = false;
        let error = git_commit_verified(args.as_object().unwrap(), root.path(), &mut started).unwrap_err();
        assert!(error.contains("directory, not an individual file"), "{error}");
        assert!(!started);
        assert_eq!(git_commit_text(root.path(), &["rev-parse", "HEAD"]).unwrap(), before);
    }

    #[test]
    fn arbitrary_selections_compose_without_repository_wide_obligations() {
        let root = fixture();
        let mut previous = git_commit_text(root.path(), &["rev-parse", "HEAD"]).unwrap();
        let mut args = input();
        for path in ["README.md", "native_host_output.rs", "nola"] {
            args["paths"] = json!([path]);
            let mut started = false;
            let receipt =
                git_commit_verified(args.as_object().unwrap(), root.path(), &mut started).unwrap();
            assert!(started);
            assert_eq!(receipt["paths"], json!([path]));
            assert_eq!(receipt["parent"], previous);
            assert_eq!(receipt["task_completion_attested"], false);
            previous = receipt["commit"].as_str().unwrap().to_owned();
            assert_eq!(
                fs::read_to_string(root.path().join(".env")).unwrap(),
                "FAKE_SECRET=fixture\n"
            );
        }
    }

    #[test]
    fn actual_git_objects_prove_only_the_selected_operation() {
        let root = fixture();
        let mut started = false;
        let receipt =
            git_commit_verified(input().as_object().unwrap(), root.path(), &mut started).unwrap();
        assert!(started);
        assert_eq!(receipt["expected_tree"], receipt["actual_tree"]);
        assert_eq!(receipt["parent"], receipt["before_commit"]);
        assert_eq!(receipt["paths"], input()["paths"]);
        assert_eq!(receipt["unselected_index_preserved"], true);
        assert_eq!(receipt["task_completion_attested"], false);
        assert_eq!(receipt["workspace_clean_at_observation"], false);
        assert!(receipt["remaining_changed_file_count"].as_u64().unwrap() > 0);
        assert!(!receipt.to_string().contains("FAKE_SECRET"));
        assert_eq!(
            git_commit_paths(
                root.path(),
                &["ls-files", "--others", "--exclude-standard", "-z"]
            )
            .unwrap(),
            [".env".into(), "nola".into()].into()
        );
        let mut started = false;
        assert!(
            git_commit_verified(input().as_object().unwrap(), root.path(), &mut started).is_err()
        );
        assert!(!started, "a repeat cannot create a second commit");
    }

    #[test]
    fn invalid_or_protected_selection_is_rejected_before_commit() {
        for selected in [
            json!([]),
            json!(["../outside"]),
            json!([".git/config"]),
            json!(["missing"]),
            json!(["README.md", ".env"]),
            json!(["README.md", "README.md"]),
        ] {
            let root = fixture();
            let mut args = input();
            args["paths"] = selected;
            let mut started = false;
            assert!(
                git_commit_verified(args.as_object().unwrap(), root.path(), &mut started).is_err()
            );
            assert!(!started);
            assert!(
                git_commit_paths(root.path(), &["diff", "--cached", "--name-only", "-z"])
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[test]
    fn existing_unrelated_index_is_preserved() {
        let root = fixture();
        git_command_output(root.path(), &["add", "native_host_output.rs"]).unwrap();
        let staged = git_index_entries(root.path()).unwrap()["native_host_output.rs"].clone();
        fs::write(
            root.path().join("native_host_output.rs"),
            "// unstaged revision\n",
        )
        .unwrap();
        let args = json!({"paths":["README.md"],"message":"fixture"});
        let mut started = false;
        git_commit_verified(args.as_object().unwrap(), root.path(), &mut started).unwrap();
        assert!(started);
        assert_eq!(
            git_index_entries(root.path()).unwrap()["native_host_output.rs"],
            staged
        );
        assert_eq!(
            fs::read_to_string(root.path().join("native_host_output.rs")).unwrap(),
            "// unstaged revision\n"
        );
        assert_eq!(
            git_commit_paths(root.path(), &["diff", "--cached", "--name-only", "-z"]).unwrap(),
            ["native_host_output.rs".into()].into()
        );
    }

    #[test]
    #[cfg(unix)]
    fn hook_failure_does_not_rollback_or_claim_success() {
        let root = fixture();
        git_command_output(root.path(), &["add", "native_host_output.rs"]).unwrap();
        let index = git_index_entries(root.path()).unwrap();
        let mut started = false;
        use std::os::unix::fs::PermissionsExt;
        let hook = root.path().join(".git/hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        let before = git_commit_text(root.path(), &["rev-parse", "HEAD"]).unwrap();
        assert!(
            git_commit_verified(input().as_object().unwrap(), root.path(), &mut started).is_err()
        );
        assert!(started);
        assert_eq!(
            git_commit_text(root.path(), &["rev-parse", "HEAD"]).unwrap(),
            before
        );
        assert_eq!(git_index_entries(root.path()).unwrap(), index);
    }

    #[test]
    #[cfg(unix)]
    fn deletion_and_dangling_symlink_are_selected_git_objects() {
        let root = fixture();
        fs::remove_file(root.path().join("README.md")).unwrap();
        std::os::unix::fs::symlink("missing-target", root.path().join("link")).unwrap();
        let args = json!({"paths":["README.md", "link"], "message":"selected objects"});
        let result =
            git_commit_verified(args.as_object().unwrap(), root.path(), &mut false).unwrap();
        assert_eq!(result["paths"], json!(["README.md", "link"]));
        assert_eq!(
            git_commit_text(root.path(), &["show", "HEAD:link"]).unwrap(),
            "missing-target"
        );
        assert!(git_commit_text(root.path(), &["ls-tree", "HEAD", "link"])
            .unwrap()
            .starts_with("120000 blob"));
    }

    #[test]
    fn literal_pathspecs_and_whole_worktree_versions_are_explicit() {
        let root = fixture();
        git_command_output(root.path(), &["add", "README.md"]).unwrap();
        fs::write(root.path().join("README.md"), "whole worktree version\n").unwrap();
        for name in ["-file", "a[1]:file.rs", "a1:file.rs"] {
            fs::write(root.path().join(name), "literal\n").unwrap();
        }
        let args =
            json!({"paths":["README.md", "-file", "a[1]:file.rs"], "message":"literal selection"});
        let receipt =
            git_commit_verified(args.as_object().unwrap(), root.path(), &mut false).unwrap();
        assert_eq!(
            receipt["paths"],
            json!(["-file", "README.md", "a[1]:file.rs"])
        );
        assert_eq!(
            git_commit_text(root.path(), &["show", "HEAD:README.md"]).unwrap(),
            "whole worktree version"
        );
        assert!(git_commit_paths(
            root.path(),
            &["ls-files", "--others", "--exclude-standard", "-z"]
        )
        .unwrap()
        .contains("a1:file.rs"));
    }
}
