fn apply_exact_edit_with_diagnostics(
    target: &Path,
    source: &str,
    old_text: &str,
    new_text: &str,
    replace_all: bool,
) -> Result<(String, usize), ToolOutcome> {
    let old_text = convert_line_endings_like(source, old_text);
    let new_text = convert_line_endings_like(source, new_text);
    let offsets = exact_match_offsets(source, &old_text);
    if offsets.is_empty() || (!replace_all && offsets.len() != 1) {
        return Err(exact_edit_preflight_failure(
            target,
            source,
            &old_text,
            replace_all,
        ));
    }
    let count = if replace_all { offsets.len() } else { 1 };
    let updated = if replace_all {
        source.replace(&old_text, &new_text)
    } else {
        source.replacen(&old_text, &new_text, 1)
    };
    Ok((updated, count))
}

#[derive(Debug, Clone)]
struct SnapshotReplacement {
    start: usize,
    end: usize,
    replacement: String,
    edit_index: usize,
}

fn apply_snapshot_multi_edits(
    target: &Path,
    source: &str,
    edits_value: &[Value],
) -> Result<(String, Vec<usize>), ToolOutcome> {
    let mut replacements = Vec::new();
    let mut counts = Vec::new();
    for (index, edit_value) in edits_value.iter().enumerate() {
        let Some(edit) = edit_value.as_object() else {
            return Err(error(format!("edit {} must be an object", index + 1)));
        };
        let old_text = match edit.get("old_text").and_then(Value::as_str) {
            Some(value) if !value.is_empty() => convert_line_endings_like(source, value),
            _ => return Err(error(format!("edit {} old_text is required", index + 1))),
        };
        let new_text = match edit.get("new_text").and_then(Value::as_str) {
            Some(value) => convert_line_endings_like(source, value),
            None => return Err(error(format!("edit {} new_text is required", index + 1))),
        };
        if edit.get("replace_all").is_some()
            && !edit.get("replace_all").is_some_and(Value::is_boolean)
        {
            return Err(error(format!(
                "edit {} replace_all must be a boolean",
                index + 1
            )));
        }
        let replace_all = edit
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let offsets = exact_match_offsets(source, &old_text);
        if offsets.is_empty() || (!replace_all && offsets.len() != 1) {
            let mut outcome = exact_edit_preflight_failure(target, source, &old_text, replace_all);
            outcome.text = format!("edit {} failed:\n{}", index + 1, outcome.text);
            return Err(outcome);
        }
        let selected = if replace_all {
            offsets
        } else {
            vec![offsets[0]]
        };
        counts.push(selected.len());
        for start in selected {
            replacements.push(SnapshotReplacement {
                start,
                end: start + old_text.len(),
                replacement: new_text.clone(),
                edit_index: index + 1,
            });
        }
    }

    replacements.sort_by_key(|replacement| replacement.start);
    for pair in replacements.windows(2) {
        if pair[1].start < pair[0].end {
            return Err(error(format!(
                "edit_batch_conflict: edits {} and {} overlap in the original file snapshot near lines {} and {}. Merge the dependent changes into one old_text/new_text edit instead of retrying the same batch.",
                pair[0].edit_index,
                pair[1].edit_index,
                source_line_for_offset(source, pair[0].start),
                source_line_for_offset(source, pair[1].start),
            )));
        }
    }

    let mut updated = source.to_string();
    for replacement in replacements.iter().rev() {
        updated.replace_range(replacement.start..replacement.end, &replacement.replacement);
    }
    if updated.len() > MAX_EDITED_FILE_BYTES {
        return Err(error(format!(
            "updated file exceeds {MAX_EDITED_FILE_BYTES} bytes"
        )));
    }
    Ok((updated, counts))
}

fn guard_existing_files_by_path(
    root: &Path,
    paths: &[PathBuf],
    args: &Map<String, Value>,
) -> Result<(), ToolOutcome> {
    for path in paths {
        if path.is_file() {
            check_expected_sha256_by_path(root, path, args)?;
            guard_existing_file_write(path, &Map::new(), true, None)?;
        }
    }
    Ok(())
}

fn with_file_write_lock<F>(path: &Path, action: F) -> ToolOutcome
where
    F: FnOnce() -> ToolOutcome,
{
    with_file_write_locks(&[path.to_path_buf()], action)
}

fn with_file_write_locks<F>(paths: &[PathBuf], action: F) -> ToolOutcome
where
    F: FnOnce() -> ToolOutcome,
{
    let mut keys = paths.iter().map(|path| file_state_key(path)).collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    let locks = FILE_WRITE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let locks = match locks.lock() {
        Ok(mut guard) => {
            guard.retain(|_, lock| lock.strong_count() > 0);
            keys.into_iter().map(|key| {
                if let Some(lock) = guard.get(&key).and_then(Weak::upgrade) { lock } else {
                    let lock = Arc::new(Mutex::new(()));
                    guard.insert(key, Arc::downgrade(&lock));
                    lock
                }
            }).collect::<Vec<_>>()
        }
        Err(_) => return error("file write lock is poisoned".to_string()),
    };
    let Ok(_guards) = locks.iter().map(|lock| lock.lock()).collect::<Result<Vec<_>, _>>() else {
        return error("file write lock is poisoned".to_string());
    };
    action()
}

fn resolve_failure_path(root: &Path, raw_path: &str) -> Option<PathBuf> {
    let mut value = raw_path.trim().trim_matches(['"', '\'']).to_string();
    if value.starts_with("a/") || value.starts_with("b/") {
        value = value[2..].to_string();
    }
    let candidate = PathBuf::from(&value);
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        root.join(value)
    };
    let resolved = candidate.canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    if resolved.is_file() && resolved.starts_with(root) {
        Some(resolved)
    } else {
        None
    }
}

fn failure_locations(
    root: &Path,
    output: &str,
    max_entries: usize,
) -> Result<Vec<(PathBuf, usize, String)>, ToolOutcome> {
    let path_regex = Regex::new(
        r#"(?P<path>(?:[A-Za-z]:)?(?:\.{1,2}/|/)?[A-Za-z0-9_@+\-./]*[A-Za-z0-9_@+\-]\.(?:py|ts|tsx|js|jsx|rs|go|swift|kt|java|c|cpp|h|hpp|m|mm|rb|php|sh|json|toml|yaml|yml|md|txt)):(?P<line>\d+)(?::\d+)?"#,
    )
    .map_err(|err| error(format!("failure path regex failed: {err}")))?;
    let paren_path_regex = Regex::new(
        r#"(?P<path>(?:[A-Za-z]:)?(?:\.{1,2}/|/)?[A-Za-z0-9_@+\-./]*[A-Za-z0-9_@+\-]\.(?:py|ts|tsx|js|jsx|rs|go|swift|kt|java|c|cpp|h|hpp|m|mm|rb|php|sh|json|toml|yaml|yml|md|txt))\((?P<line>\d+)(?:,\d+)?\)"#,
    )
    .map_err(|err| error(format!("failure paren path regex failed: {err}")))?;
    let rust_panic_regex = Regex::new(r#"\bpanicked at (?P<path>.+?\.rs):(?P<line>\d+)(?::\d+)?"#)
        .map_err(|err| error(format!("Rust panic path regex failed: {err}")))?;
    let traceback_regex = Regex::new(r#"File "([^"]+)", line (\d+)"#)
        .map_err(|err| error(format!("failure traceback regex failed: {err}")))?;
    let grouped_line_regex =
        Regex::new(r#"^\s*(?P<line>\d+):\d+\s+(?:error|warning|fatal|note)\b"#)
            .map_err(|err| error(format!("failure grouped line regex failed: {err}")))?;
    let mut locations = Vec::new();
    let mut seen: Vec<(PathBuf, usize)> = Vec::new();
    let mut current_file: Option<PathBuf> = None;
    for raw_line in output.lines() {
        if locations.len() >= max_entries {
            break;
        }
        let stripped = raw_line.trim();
        if stripped.is_empty() {
            current_file = None;
            continue;
        }
        if let Some(path) = resolve_failure_path(root, stripped) {
            current_file = Some(path);
            continue;
        }
        if let Some(path) = current_file.clone() {
            if let Some(captures) = grouped_line_regex.captures(raw_line) {
                if let Some(line) = captures.name("line") {
                    push_failure_location(
                        &mut locations,
                        &mut seen,
                        path,
                        line.as_str(),
                        raw_line,
                        max_entries,
                    );
                    continue;
                }
            }
        }
        let mut matches = Vec::new();
        for captures in rust_panic_regex.captures_iter(raw_line) {
            if let (Some(path), Some(line)) = (captures.name("path"), captures.name("line")) {
                matches.push((path.as_str().trim().to_string(), line.as_str().to_string()));
            }
        }
        for captures in path_regex.captures_iter(raw_line) {
            if let (Some(path), Some(line)) = (captures.name("path"), captures.name("line")) {
                matches.push((path.as_str().to_string(), line.as_str().to_string()));
            }
        }
        for captures in paren_path_regex.captures_iter(raw_line) {
            if let (Some(path), Some(line)) = (captures.name("path"), captures.name("line")) {
                matches.push((path.as_str().to_string(), line.as_str().to_string()));
            }
        }
        for captures in traceback_regex.captures_iter(raw_line) {
            if let (Some(path), Some(line)) = (captures.get(1), captures.get(2)) {
                matches.push((path.as_str().to_string(), line.as_str().to_string()));
            }
        }
        for (raw_path, line_text) in matches {
            let Some(path) = resolve_failure_path(root, &raw_path) else {
                continue;
            };
            push_failure_location(
                &mut locations,
                &mut seen,
                path.clone(),
                &line_text,
                raw_line,
                max_entries,
            );
            current_file = Some(path);
            if locations.len() >= max_entries {
                break;
            }
        }
    }
    Ok(locations)
}

fn push_failure_location(
    locations: &mut Vec<(PathBuf, usize, String)>,
    seen: &mut Vec<(PathBuf, usize)>,
    path: PathBuf,
    line_text: &str,
    message: &str,
    max_entries: usize,
) {
    if locations.len() >= max_entries {
        return;
    }
    let Ok(line_no) = line_text.parse::<usize>() else {
        return;
    };
    if line_no == 0 || seen.contains(&(path.clone(), line_no)) {
        return;
    }
    seen.push((path.clone(), line_no));
    locations.push((path, line_no, outline_signature(message)));
}

fn failure_category(line: &str) -> &'static str {
    let lowered = line.to_lowercase();
    if [
        "modulenotfounderror",
        "cannot find module",
        "no module named",
        "command not found",
        "no such file or directory",
        "econnrefused",
        "could not resolve host",
        "network is unreachable",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "environment/dependency";
    }
    if [
        "syntaxerror",
        "parse error",
        "unexpected token",
        "unterminated",
        "expected expression",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "syntax/parse";
    }
    if [
        "ts2304",
        "ts2322",
        "type error",
        "mypy",
        "pyright",
        "cannot find name",
        "does not exist on type",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "typecheck";
    }
    if [
        "eslint",
        "ruff",
        "clippy",
        "ktlint",
        "swiftlint",
        " lint",
        " no-explicit-any",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "lint";
    }
    if [
        "assertionerror",
        "assert_eq",
        "expected",
        "actual",
        "failed",
        "failures:",
        "test result: failed",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "test";
    }
    if [
        "error[e",
        "compilation failed",
        "compile",
        "xcodebuild",
        "gradle",
        "javac",
        "kotlinc",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "compile/build";
    }
    "unknown"
}

fn failure_triage_summary(output: &str) -> (HashMap<String, usize>, Vec<String>) {
    let mut counts = HashMap::new();
    let mut signals = Vec::new();
    for raw_line in output.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let category = failure_category(line);
        if category != "unknown" {
            *counts.entry(category.to_string()).or_insert(0) += 1;
            if signals.len() < 20 {
                signals.push(format!("- {category}: {}", outline_signature(line)));
            }
        }
    }
    (counts, signals)
}

fn primary_failure_category(
    counts: &HashMap<String, usize>,
    locations: &[(PathBuf, usize, String)],
) -> String {
    if !counts.is_empty() {
        let mut items = counts.iter().collect::<Vec<_>>();
        items.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        return items[0].0.clone();
    }
    if locations.is_empty() {
        "unknown".to_string()
    } else {
        "located-code-failure".to_string()
    }
}

fn write_file(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let content = match args.get("content") {
        Some(Value::String(value)) => value.clone(),
        _ => return invalid_write_payload("content must contain the literal file text"),
    };
    if content.len() > MAX_WRITE_BYTES {
        return file_payload_capacity_error("write_payload_too_large", content.len());
    }
    let dry_run = tool_try!(bool_arg(args, "dry_run", false));
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, false));
    let before = tool_try!(observe_file_mutation_target(&target));
    tool_try!(validate_mutation_expectation(&before, args));
    if dry_run {
        let old_text = before.as_ref().map(|value| {
            (value.preview.len() as u64 == value.version.bytes)
                .then(|| std::str::from_utf8(&value.preview).ok()).flatten()
        }).unwrap_or(Some(""));
        let diff = old_text.map(|old| format_unified_diff(&target, old, &content, MAX_DIFF_CHARS))
            .unwrap_or_else(|| "Old content is binary or exceeds the bounded preview; no old text was synthesized.".into());
        return ok(
            format!(
                "# Write preview: {}\ndry_run: true\nexists: {}\nbytes: {}\n\n## Diff\n{diff}",
                target.display(),
                before.is_some(),
                content.len()
            ),
            format!("Would write {}", target.display()),
        );
    }
    if before.as_ref().is_some_and(|snapshot| snapshot.version.sha256 != sha256_hex(content.as_bytes()))
        && tool_try!(expected_sha256(args)).is_none() {
        return error_data(json!({
            "code":"existing_file_version_required", "path":target,
            "tool_execution_skipped":true,
            "message":"The target already exists; this is replacement, not file creation.",
            "next_action":"If only new files are authorized, choose a new path. Otherwise inspect the existing file and provide its expected_sha256 for an authorized replacement. A digest is not permission to overwrite."
        }));
    }
    if ctx.cancellation.is_requested() {
        return error("Write cancelled before commit; tool_execution_skipped: true".into());
    }
    let parent_paths = tool_try!(missing_mutation_parents(&target).map_err(error));
    let mut lock_paths = parent_paths.clone();
    lock_paths.push(target.clone());
    with_file_write_locks(&lock_paths, || {
        if ctx.cancellation.is_requested() {
            return error("Write cancelled before commit; tool_execution_skipped: true".into());
        }
        // Full replacement does not consume the old body. Validate the fresh
        // identity/version under the mutation lock without minting read credit.
        let current = tool_try!(revalidate_file_mutation_target(&target, &before, args));
        if let Some(value) = &current {
            if value.version.bytes == content.len() as u64 && value.version.sha256 == sha256_hex(content.as_bytes()) {
                    let version = Some(mutation_version(&current));
                    let mut unchanged = ok_data(
                        json!({
                            "path": target,
                            "changed": false,
                            "version": version,
                            "detail": "The requested content is already present. No mutation or validation occurred."
                        }),
                        format!("Unchanged {}", target.display()),
                    );
                    if let Ok(uri) = url::Url::from_file_path(&target) {
                        unchanged.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
                            local_stop: None,
                            state: nola_host_protocol::tool_result::ExecutionState::Completed,
                            effects: vec![nola_host_protocol::tool_result::ResourceEffect {
                                resource_uri: uri.to_string(),
                                operation: "write".into(),
                                before_version: version.clone(),
                                after_version: version,
                            }],
                            checks: Vec::new(),
                        });
                    }
                    return unchanged;
            }
        }
        let before_version = mutation_version(&current);
        let mut outcome = ok(
            format!("Write target: {}",target.display()),
            format!("Write {}",target.display()));
        if let Err(message) = create_mutation_parents(&target,&ctx.cancellation,&parent_paths,&mut outcome) {
            return partial_file_mutation_failure(outcome,&message);
        }
        if ctx.cancellation.is_requested() {
            return partial_file_mutation_failure(outcome,"Write cancelled after directory preparation");
        }
        if let Err(failure) = revalidate_file_mutation_target(&target,&current,args) {
            return partial_file_mutation_failure(outcome,&format!("File phase refused after directory preparation: {}",failure.text));
        }
        match atomic_write_text_file_mode(&target, &content, current.is_some()) {
            Ok(()) => {
                refresh_file_access_state(&target);
                outcome.text = format!("Wrote {} bytes to {}",content.len(),target.display());
                outcome.display = format!("{} {}",tr(Message::DisplayWrotePrefix),target.display());
                outcome = with_mutation_artifact(outcome,&target,"write",before_version,
                    format!("sha256:{}",sha256_hex(content.as_bytes())));
                let mut format_sections = Vec::new();
                if let Some(format_root) = format_root_for_path(&target, &ctx.allowed_dirs) {
                    match formatting_sections(
                        &format_root,
                        std::slice::from_ref(&target),
                        &ctx.cancellation,
                    ctx.policy.process_sandbox(),
                    ) {
                        Ok(sections) => format_sections = sections,
                        Err(failure) => {
                            refresh_mutation_effect_versions(&mut outcome);
                            return partial_file_mutation_failure(outcome,&failure.text);
                        }
                    }
                    refresh_file_access_state(&target);
                }
                if !refresh_mutation_effect_versions(&mut outcome) {
                    return partial_file_mutation_failure(outcome,"Written resource could not be observed after formatting; its final version is unknown or absent, do not replay the replacement");
                }
                outcome.text.push_str(&format_sections.join(""));
                outcome
            }
            Err(err) => partial_file_mutation_failure(outcome,&format!("Write failed: {err}")),
        }
    })
}

fn file_payload_capacity_error(category: &str, actual_bytes: usize) -> ToolOutcome {
    error_data_with_display(json!({
        "error": category,
        "deterministic_failure_category": category,
        "tool_execution_skipped": true,
        "max_utf8_bytes": MAX_WRITE_BYTES,
        "actual_utf8_bytes": actual_bytes,
        "message": "Final UTF-8 payload exceeds the file mutation capacity; no file was changed.",
        "next_action": "Choose a bounded operation that preserves the intended create, replace, or append semantics. Do not convert replacement into append or blindly replay an uncertain append."
    }), "File payload exceeds capacity".into())
}

fn invalid_write_payload(message: &str) -> ToolOutcome {
    error(format!(
        "{message}\n\
tool_execution_skipped: true\n\
deterministic_failure_category: invalid_tool_arguments\n\
next_action: retry_write_file_with_literal_content"
    ))
}

fn append_file(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let content = match args.get("content").and_then(Value::as_str) {
        Some(value) if !value.is_empty() => value,
        _ => return error("content is required".to_string()),
    };
    if content.len() > MAX_WRITE_BYTES {
        return file_payload_capacity_error("append_chunk_too_large", content.len());
    }
    let create = tool_try!(bool_arg(args, "create", false));
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, false));
    let snapshot = tool_try!(observe_file_mutation_target(&target));
    tool_try!(validate_mutation_expectation(&snapshot,args));
    let target_existed = snapshot.is_some();
    // Appending preserves the existing bytes without interpreting them or
    // locating an old-text range. Fresh mutation snapshots, explicit expected
    // versions and locked revalidation remain mandatory; a model full-read
    // ledger is not an append precondition (and is not durable across hosts).
    if !target_existed {
        if !create {
            return error(
                "target does not exist; retry the first chunk with create=true".to_string(),
            );
        }
    }

    let parent_paths = tool_try!(missing_mutation_parents(&target).map_err(error));
    let mut paths = parent_paths.clone();
    paths.push(target.clone());
    with_file_write_locks(&paths, || {
        let current = tool_try!(revalidate_file_mutation_target(&target,&snapshot,args));
        let before = current.as_ref().map_or(0,|snapshot|snapshot.version.bytes);
        let before_version = mutation_version(&current);
        let mut outcome = ok(format!("Append target: {}",target.display()),format!("Append {}",target.display()));
        if let Err(message) = create_mutation_parents(&target,&ctx.cancellation,&parent_paths,&mut outcome) {
            return partial_file_mutation_failure(outcome,&message);
        }
        if ctx.cancellation.is_requested() {
            return partial_file_mutation_failure(outcome,"Append cancelled after directory preparation");
        }
        if let Err(failure) = revalidate_file_mutation_target(&target,&current,args) {
            return partial_file_mutation_failure(outcome,&format!("File phase refused after directory preparation: {}",failure.text));
        }
        let mut options = fs::OpenOptions::new();
        options.append(true);
        // Creating an absent target must not append into a late competing
        // file whose preimage was never admitted or observed.
        if current.is_none() { options.create_new(true); }
        let mut file = match options.open(&target) {
            Ok(file) => file,
            Err(err) => return partial_file_mutation_failure(outcome,&format!("Open for append failed: {err}")),
        };
        let mut written = 0;
        let mut failure = None;
        while written < content.len() {
            match file.write(&content.as_bytes()[written..]) {
                Ok(0) => { failure = Some("Append made no write progress".to_string()); break; }
                Ok(count) => written += count,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => { failure = Some(format!("Append failed: {err}")); break; }
            }
        }
        if failure.is_none() {
            if let Err(err) = file.flush() { failure = Some(format!("Append flush failed: {err}")); }
        }
        if current.is_none() || written > 0 {
            outcome = outcome.with_file_artifact(&target,"append");
            let after = observe_file_mutation_target(&target).ok().map(|snapshot|mutation_version(&snapshot));
            if let Some(effect) = outcome.execution.as_mut().and_then(|receipt|receipt.effects.last_mut()) {
                effect.before_version = Some(before_version);
                effect.after_version = after;
            }
            refresh_file_access_state(&target);
        }
        if let Some(message) = failure {
            return partial_file_mutation_failure(outcome,&message);
        }
        outcome.text = format!(
                "Appended {} bytes to {} (file bytes: {} -> {}; mode=append; existing content was preserved and never truncated{}).",
                content.len(),
                target.display(),
                before,
                before.saturating_add(content.len() as u64),
                if target_existed && create {
                    "; create=true only permits creation when missing"
                } else {
                    ""
                }
            );
        outcome.display = format!("{} {}",tr(Message::DisplayWrotePrefix),target.display());
        outcome
    })
}

fn notebook_edit(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let dry_run = tool_try!(bool_arg(args, "dry_run", false));
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    if !target.is_file() {
        return error(format!("Not a file: {}", target.display()));
    }
    tool_try!(ensure_notebook_path(&target));
    tool_try!(guard_existing_file_write(&target, args, true, None));
    let cell_index = tool_try!(validate_integer_arg(
        args,
        "cell_index",
        0,
        0,
        1_000_000_000
    )) as usize;
    let edit_mode = match args.get("edit_mode").and_then(Value::as_str) {
        Some(value) if ["replace", "insert", "delete"].contains(&value) => value,
        Some(_) => return error("edit_mode must be replace, insert, or delete".to_string()),
        None => "replace",
    };
    let cell_type = match args.get("cell_type").and_then(Value::as_str) {
        Some(value) if ["code", "markdown", "raw"].contains(&value) => value,
        Some(_) => return error("cell_type must be code, markdown, or raw".to_string()),
        None => "code",
    };
    let source = match args.get("source").and_then(Value::as_str) {
        Some(value) => value,
        None if edit_mode == "delete" => "",
        None => return error("source is required for insert/replace".to_string()),
    };
    if source.len() > MAX_NOTEBOOK_SOURCE_CHARS {
        return error(format!(
            "source exceeds {MAX_NOTEBOOK_SOURCE_CHARS} characters"
        ));
    }
    let content = match fs::read_to_string(&target) {
        Ok(value) => value,
        Err(err) => return error(format!("Notebook edit failed: {err}")),
    };
    let mut notebook: Value = match serde_json::from_str(&content) {
        Ok(value) => value,
        Err(err) => return error(format!("Notebook edit failed: {err}")),
    };
    let nbformat = notebook
        .get("nbformat")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let nbformat_minor = notebook
        .get("nbformat_minor")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let Some(cells) = notebook.get_mut("cells").and_then(Value::as_array_mut) else {
        return error("Notebook edit failed: notebook JSON must contain a cells array".to_string());
    };
    match edit_mode {
        "insert" => {
            if cell_index > cells.len() {
                return error(format!(
                    "cell_index {cell_index} is out of range for insert; notebook has {} cells.",
                    cells.len()
                ));
            }
            let mut cell = json!({
                "cell_type": cell_type,
                "metadata": {},
                "source": notebook_source_from_text(source)
            });
            if cell_type == "code" {
                cell["execution_count"] = Value::Null;
                cell["outputs"] = Value::Array(Vec::new());
            }
            if nbformat > 4 || (nbformat == 4 && nbformat_minor >= 5) {
                let millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_millis())
                    .unwrap_or(0);
                cell["id"] = Value::String(format!("nola-{millis}-{cell_index}"));
            }
            cells.insert(cell_index, cell);
        }
        "delete" => {
            if cell_index >= cells.len() {
                return error(format!(
                    "cell_index {cell_index} does not exist; notebook has {} cells.",
                    cells.len()
                ));
            }
            cells.remove(cell_index);
        }
        _ => {
            if cell_index >= cells.len() {
                return error(format!(
                    "cell_index {cell_index} does not exist; notebook has {} cells.",
                    cells.len()
                ));
            }
            let Some(cell) = cells.get_mut(cell_index).and_then(Value::as_object_mut) else {
                return error(format!(
                    "cell_index {cell_index} is not a notebook cell object"
                ));
            };
            cell.insert(
                "cell_type".to_string(),
                Value::String(cell_type.to_string()),
            );
            cell.insert("source".to_string(), notebook_source_from_text(source));
            if cell_type == "code" {
                cell.entry("metadata".to_string())
                    .or_insert_with(|| json!({}));
                cell.insert("execution_count".to_string(), Value::Null);
                cell.insert("outputs".to_string(), Value::Array(Vec::new()));
            } else {
                cell.remove("execution_count");
                cell.remove("outputs");
            }
        }
    }
    let updated = match serde_json::to_string_pretty(&notebook) {
        Ok(value) => value + "\n",
        Err(err) => return error(format!("Notebook serialization failed: {err}")),
    };
    if dry_run {
        let diff = format_unified_diff(&target, &content, &updated, MAX_DIFF_CHARS);
        return ok(
            format!(
                "# Notebook edit preview: {}\ndry_run: true\nedit_mode: {edit_mode}\ncell_index: {cell_index}\n\n## Diff\n{diff}",
                target.display()
            ),
            format!(
                "Would notebook {edit_mode} {} cell {cell_index}",
                target.display()
            ),
        );
    }
    with_file_write_lock(&target, || match fs::write(&target, updated) {
        Ok(()) => {
            refresh_file_access_state(&target);
            ok(
                format!(
                    "Notebook {edit_mode} complete: {} cell_index={cell_index}",
                    target.display()
                ),
                format!(
                    "Notebook {edit_mode} {} cell {cell_index}",
                    target.display()
                ),
            )
            .with_file_artifact(&target, "edit")
        }
        Err(err) => error(format!("Notebook write failed: {err}")),
    })
}

fn move_file(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let source_arg = tool_try!(required_string(args, "source"));
    let destination_arg = tool_try!(required_string(args, "destination"));
    let overwrite = tool_try!(bool_arg(args, "overwrite", false));
    let dry_run = tool_try!(bool_arg(args, "dry_run", false));
    let update_imports = tool_try!(bool_arg(args, "update_imports", false));
    let _max_import_updates = tool_try!(validate_integer_arg(
        args,
        "max_import_updates",
        100,
        0,
        MAX_DEPENDENCY_IMPORTS
    )) as usize;
    let source = tool_try!(resolve_allowed(&source_arg, &ctx.allowed_dirs, true));
    let destination = tool_try!(resolve_allowed(&destination_arg, &ctx.allowed_dirs, false));
    if !source.is_file() {
        return error(format!("Source is not a file: {}", source.display()));
    }
    if source == destination {
        return error("source and destination are the same file".to_string());
    }
    let source_before = tool_try!(observe_file_mutation_target(&source));
    if source_before.is_none() { return error("Move source disappeared before planning".into()); }
    tool_try!(validate_mutation_expectation(&source_before, args));
    let destination_before = tool_try!(observe_file_mutation_target(&destination));
    if source_before.as_ref().zip(destination_before.as_ref()).is_some_and(|(source,destination)| same_mutation_resource(source,destination)) {
        return error("Source and destination identify the same physical file".into());
    }
    if destination_before.is_some() {
        if !overwrite {
            return error(format!(
                "Destination already exists: {}",
                destination.display()
            ));
        }
        if !destination.is_file() {
            return error(format!(
                "Destination is not a file: {}",
                destination.display()
            ));
        }
    }
    let input = Value::Object(args.clone());
    let plan = match move_import_plan(ctx, &input) { Ok(plan) => plan, Err(message) => return error(message) };
    let import_updates = &plan.updates;
    let scanned_importers = plan.scanned;
    if dry_run {
        let mut lines = vec![
            "dry_run: true".to_string(),
            format!(
                "Would move {} -> {}",
                source.display(),
                destination.display()
            ),
            format!("would_overwrite: {}", destination.exists() && overwrite),
        ];
        if update_imports {
            lines.extend([
                String::new(),
                "## Import updates".to_string(),
                format!("scanned_files: {scanned_importers}"),
                "updated_files: 0".to_string(),
            ]);
            let changed_lines = import_updates
                .iter()
                .flat_map(|update| update.lines.iter().cloned())
                .collect::<Vec<_>>();
            if changed_lines.is_empty() {
                lines.push("(none)".to_string());
            } else {
                lines.extend(changed_lines);
            }
        }
        return ok(
            lines.join("\n"),
            format!(
                "Would move {} -> {}",
                source.display(),
                destination.display()
            ),
        );
    }
    if ctx.cancellation.is_requested() { return error("Move cancelled before commit; tool_execution_skipped: true".into()); }
    if let Err(message) = permission_resolution::validate_move_import_plan(ctx, &input, &plan) {
        return permission_resource_execution_failure(message);
    }
    let parent_paths = tool_try!(missing_mutation_parents(&destination).map_err(error));
    let mut paths = parent_paths.clone();
    paths.extend([source.clone(), destination.clone()]);
    paths.extend(import_updates.iter().map(|update| update.path.clone()));
    with_file_write_locks(&paths, || {
        tool_try!(revalidate_file_mutation_target(&source, &source_before, args));
        tool_try!(revalidate_file_mutation_target(&destination, &destination_before, &Map::new()));
        let current = match move_import_plan(ctx, &input) { Ok(plan) => plan, Err(message) => return error(message) };
        if let Err(message) = permission_resolution::validate_move_import_plan(ctx, &input, &current) {
            return permission_resource_execution_failure(message);
        }
        if current.updates.iter().map(|update| &update.path).collect::<Vec<_>>()
            != plan.updates.iter().map(|update| &update.path).collect::<Vec<_>>() {
            return permission_resource_execution_failure("Import lock set changed before mutation".into());
        }
        if ctx.cancellation.is_requested() { return error("Move cancelled before commit; tool_execution_skipped: true".into()); }
        let mut outcome = commit_move_entry_with_parents(&source,&destination,&source_before,&destination_before,
            args,&ctx.cancellation,&parent_paths,|source,destination|atomic_move_file(source,destination,overwrite));
        if outcome.error { return outcome; }
        for update in &current.updates {
            if ctx.cancellation.is_requested() { return partial_file_mutation_failure(outcome, "Import updates cancelled after the move; existing effects were not rolled back"); }
            let observed = match observe_file_mutation_target(&update.path) {
                Ok(Some(value)) if value.version == update.version => value,
                _ => return partial_file_mutation_failure(outcome, "Importer changed before its edit; existing effects were not rolled back"),
            };
            if let Err(err) = atomic_write_text_file(&update.path, &update.updated) {
                return partial_file_mutation_failure(outcome, &format!("Import edit failed: {err}; existing effects were not rolled back"));
            }
            refresh_file_access_state(&update.path);
            outcome = with_mutation_artifact(outcome, &update.path, "edit", format!("sha256:{}", observed.version.sha256),
                format!("sha256:{}", sha256_hex(update.updated.as_bytes())));
        }
        if update_imports {
            outcome.text.push_str(&format!("\n\n## Import updates\nscanned_files: {}\nupdated_files: {}", current.scanned, current.updates.len()));
            for update in &current.updates { outcome.text.push_str(&format!("\n{}", update.lines.join("\n"))); }
        }
        outcome
    })
}

fn delete_file(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let dry_run = tool_try!(bool_arg(args, "dry_run", false));
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    if !target.is_file() {
        return error(format!("Not a file: {}", target.display()));
    }
    let before = tool_try!(observe_file_mutation_target(&target));
    let Some(observed) = &before else { return error("Delete target disappeared".into()); };
    tool_try!(validate_mutation_expectation(&before, args));
    let size = observed.version.bytes;
    if dry_run {
        return ok(
            format!(
                "dry_run: true\nWould delete {} ({size} bytes)",
                target.display()
            ),
            format!("Would delete {}", target.display()),
        );
    }
    with_file_write_lock(&target, || {
        tool_try!(revalidate_file_mutation_target(&target, &before, args));
        if ctx.cancellation.is_requested() { return error("Delete cancelled before commit; tool_execution_skipped: true".into()); }
        match fs::remove_file(&target) {
        Ok(()) => {
            forget_file_access_state(&target);
            with_mutation_artifact(ok(
                format!("Deleted {} ({size} bytes)", target.display()),
                format!("Deleted {}", target.display()),
            ), &target, "delete", mutation_version(&before), "absent:v1".into())
        }
        Err(err) => error(format!("Delete failed: {err}")),
        }
    })
}

fn replace_symbol_body(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let symbol = tool_try!(required_string(args, "symbol"));
    let new_text = match args.get("new_text").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value,
        _ => return error("new_text is required".to_string()),
    };
    if new_text.len() > MAX_WRITE_BYTES {
        return error(format!("new_text exceeds {MAX_WRITE_BYTES} bytes"));
    }
    let occurrence = tool_try!(validate_integer_arg(args, "occurrence", 1, 1, 100)) as usize;
    let dry_run = match args.get("dry_run") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("dry_run must be a boolean".to_string()),
    };
    let show_line_numbers = match args.get("show_line_numbers") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("show_line_numbers must be a boolean".to_string()),
    };
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    if !target.is_file() {
        return error(format!("Not a file: {}", target.display()));
    }
    tool_try!(guard_existing_file_write(&target, args, false, None));
    let source = match fs::read_to_string(&target) {
        Ok(text) => text,
        Err(err) => return error(format!("Read failed: {err}")),
    };
    let lines = source.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let Some((kind, line_no, start_index, end_index, signature)) =
        tool_try!(find_symbol_body_range(&lines, &symbol, occurrence))
    else {
        return error(format!(
            "symbol not found: {symbol} occurrence {occurrence}"
        ));
    };
    let start_line = start_index + 1;
    let line_count = end_index - start_index;
    let updated = tool_try!(replace_line_range(
        &source, start_line, line_count, new_text
    ));
    if updated.len() > MAX_EDITED_FILE_BYTES {
        return error(format!(
            "updated file exceeds {MAX_EDITED_FILE_BYTES} bytes"
        ));
    }
    let old_body = lines[start_index..end_index].join("\n");
    let replacement = convert_line_endings_like(&source, new_text)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut sections = vec![
        format!("# Replace symbol body: {}", target.display()),
        format!("symbol: {kind} {symbol}"),
        format!("definition_line: {line_no}"),
        format!("replace_lines: {start_line}-{end_index}"),
        format!("dry_run: {dry_run}"),
        format!("signature: {signature}"),
        String::new(),
        "## Current".to_string(),
        format_numbered_lines(&old_body, start_line, show_line_numbers),
        String::new(),
        "## Replacement".to_string(),
        format_numbered_lines(&replacement, start_line, show_line_numbers),
    ];
    if dry_run {
        return ok(
            sections.join("\n"),
            format!("Replace symbol preview ({kind} {symbol})"),
        );
    }
    with_file_write_lock(&target, || match fs::write(&target, updated) {
        Ok(()) => {
            refresh_file_access_state(&target);
            sections.extend([String::new(), "Applied.".to_string()]);
            if let Some(format_root) = format_root_for_path(&target, &ctx.allowed_dirs) {
                match formatting_sections(
                    &format_root,
                    std::slice::from_ref(&target),
                    &ctx.cancellation,
                    ctx.policy.process_sandbox(),
                ) {
                    Ok(format_sections) => sections.extend(format_sections),
                    Err(outcome) => return outcome,
                }
                refresh_file_access_state(&target);
            }
            ok(
                sections.join("\n"),
                format!("Replaced {kind} {symbol} in {}", target.display()),
            )
            .with_file_artifact(&target, "edit")
        }
        Err(err) => error(format!("Write failed: {err}")),
    })
}

fn edit_file(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let old_text = match args.get("old_text").and_then(Value::as_str) {
        Some(value) if !value.is_empty() => value,
        _ => return error("old_text is required".to_string()),
    };
    let new_text = match args.get("new_text").and_then(Value::as_str) {
        Some(value) => value,
        None => return error("new_text is required".to_string()),
    };
    let replace_all = args
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let expected_replacements = match args.get("expected_replacements") {
        Some(Value::Number(value)) => match value.as_u64().filter(|count| *count > 0) {
            Some(count) => Some(count as usize),
            None => return error("expected_replacements must be a positive integer".to_string()),
        },
        Some(Value::Null) | None => None,
        Some(_) => return error("expected_replacements must be a positive integer".to_string()),
    };
    if replace_all && expected_replacements.is_none() {
        return error(
            "replace_all=true requires expected_replacements so a broad edit cannot silently modify an unexpected number of locations"
                .to_string(),
        );
    }
    if !replace_all && expected_replacements.is_some() {
        return error("expected_replacements is only valid with replace_all=true".to_string());
    }
    if args.get("dry_run").is_some() && !args.get("dry_run").is_some_and(Value::is_boolean) {
        return error("dry_run must be a boolean".to_string());
    }
    let dry_run = args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    let source = match fs::read_to_string(&target) {
        Ok(text) => text,
        Err(err) => return error(format!("Read failed: {err}")),
    };
    if let Some(expected) = expected_replacements {
        let normalized_old_text = convert_line_endings_like(&source, old_text);
        let actual = exact_match_offsets(&source, &normalized_old_text).len();
        if actual != expected {
            return error(format!(
                "Bulk edit preflight failed: expected_replacements_mismatch\nexpected_replacements: {expected}\nedit_match_count: {actual}\nnext_action: inspect the reported match count and use a unique old_text, or retry only after deliberately updating expected_replacements."
            ));
        }
    }
    tool_try!(guard_exact_edit_existing_file_write(
        &target, args, &source, old_text
    ));
    let (updated, count) = tool_try!(apply_exact_edit_with_diagnostics(
        &target,
        &source,
        old_text,
        new_text,
        replace_all,
    ));
    if updated == source {
        return no_effect_edit_outcome(&target);
    }
    if updated.len() > MAX_EDITED_FILE_BYTES {
        return error(format!(
            "updated file exceeds {MAX_EDITED_FILE_BYTES} bytes"
        ));
    }
    let diff = format_unified_diff(&target, &source, &updated, MAX_DIFF_CHARS);
    if dry_run {
        return ok(
            format!(
                "# Edit preview: {}\ndry_run: true\nreplacements: {count}\n\n## Diff\n{diff}",
                target.display()
            ),
            format!("Edit preview {}", target.display()),
        );
    }
    with_file_write_lock(&target, || {
        let (format_sections, committed) = match commit_formatted_file_edit(
            &target,
            &source,
            &updated,
            &ctx.allowed_dirs,
            false,
            &ctx.cancellation,
                    ctx.policy.process_sandbox(),
        ) {
            Ok(committed) => committed,
            Err(outcome) => return outcome,
        };
        let diff = format_unified_diff(&target, &source, &committed, MAX_DIFF_CHARS);
        ok(
            format!(
                "Edited {} ({count} replacement{})\n\n## Diff\n{diff}{}",
                target.display(),
                if count == 1 { "" } else { "s" },
                format_sections.join("")
            ),
            format!("{} {}", tr(Message::DisplayEditedPrefix), target.display()),
        )
        .with_file_artifact_version(&target, "edit", Some(format!("sha256:{}", sha256_hex(source.as_bytes()))))
    })
}

fn split_lines_keepends(source: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (index, ch) in source.char_indices() {
        if ch == '\n' {
            lines.push(source[start..index + 1].to_string());
            start = index + 1;
        }
    }
    if start < source.len() {
        lines.push(source[start..].to_string());
    }
    lines
}

fn replace_line_range(
    source: &str,
    start_line: usize,
    line_count: usize,
    new_text: &str,
) -> Result<String, ToolOutcome> {
    let lines = split_lines_keepends(source);
    if start_line == 0 {
        return Err(error("start_line must be >= 1".to_string()));
    }
    if start_line > lines.len() + 1 {
        return Err(error(format!(
            "start_line exceeds file length ({} lines)",
            lines.len()
        )));
    }
    let start_index = start_line - 1;
    let end_index = start_index + line_count;
    if end_index > lines.len() {
        return Err(error(format!(
            "line range exceeds file length ({} lines)",
            lines.len()
        )));
    }
    let mut replacement = convert_line_endings_like(source, new_text);
    if !replacement.is_empty()
        && !replacement.ends_with('\n')
        && !replacement.ends_with('\r')
        && end_index < lines.len()
    {
        replacement.push_str(if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        });
    }
    let mut updated = String::new();
    updated.push_str(&lines[..start_index].join(""));
    updated.push_str(&replacement);
    updated.push_str(&lines[end_index..].join(""));
    Ok(updated)
}

fn edit_file_lines(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let start_line = tool_try!(validate_integer_arg(
        args,
        "start_line",
        1,
        1,
        1_000_000_000
    )) as usize;
    let line_count = tool_try!(validate_integer_arg(
        args,
        "line_count",
        1,
        0,
        MAX_LINE_COUNT
    )) as usize;
    let new_text = match args.get("new_text").and_then(Value::as_str) {
        Some(value) => value,
        None => return error("new_text is required".to_string()),
    };
    if new_text.len() > MAX_WRITE_BYTES {
        return error(format!("new_text exceeds {MAX_WRITE_BYTES} bytes"));
    }
    if args.get("dry_run").is_some() && !args.get("dry_run").is_some_and(Value::is_boolean) {
        return error("dry_run must be a boolean".to_string());
    }
    let dry_run = args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    if !target.is_file() {
        return error(format!("Not a file: {}", target.display()));
    }
    tool_try!(guard_existing_file_write(
        &target,
        args,
        false,
        Some((
            start_line,
            start_line.saturating_add(line_count).saturating_sub(1)
        ))
    ));
    let source = match fs::read_to_string(&target) {
        Ok(text) => text,
        Err(err) => return error(format!("Read failed: {err}")),
    };
    let updated = tool_try!(replace_line_range(
        &source, start_line, line_count, new_text
    ));
    if updated == source {
        return no_effect_edit_outcome(&target);
    }
    if updated.len() > MAX_EDITED_FILE_BYTES {
        return error(format!(
            "updated file exceeds {MAX_EDITED_FILE_BYTES} bytes"
        ));
    }
    let diff = format_unified_diff(&target, &source, &updated, MAX_DIFF_CHARS);
    let action = if line_count == 0 {
        "Inserted"
    } else {
        "Replaced"
    };
    if dry_run {
        return ok(
            format!(
                "# Line edit preview: {}\ndry_run: true\naction: {}\nstart_line: {start_line}\nline_count: {line_count}\n\n## Diff\n{diff}",
                target.display(),
                action.to_lowercase()
            ),
            format!("Line edit preview {}", target.display()),
        );
    }
    with_file_write_lock(&target, || {
        let (format_sections, committed) = match commit_formatted_file_edit(
            &target,
            &source,
            &updated,
            &ctx.allowed_dirs,
            false,
            &ctx.cancellation,
                    ctx.policy.process_sandbox(),
        ) {
            Ok(committed) => committed,
            Err(outcome) => return outcome,
        };
        let diff = format_unified_diff(&target, &source, &committed, MAX_DIFF_CHARS);
        ok(
            format!(
                "{action} {line_count} line{} at {}:{start_line}\n\n## Diff\n{diff}{}",
                if line_count == 1 { "" } else { "s" },
                target.display(),
                format_sections.join("")
            ),
            format!("{} {}", tr(Message::DisplayEditedPrefix), target.display()),
        )
        .with_file_artifact_version(&target, "edit", Some(format!("sha256:{}", sha256_hex(source.as_bytes()))))
    })
}

fn multi_edit_file(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let path = tool_try!(required_string(args, "path"));
    let edits_value = match args.get("edits").and_then(Value::as_array) {
        Some(value) if !value.is_empty() => value,
        _ => return error("edits must be a non-empty array".to_string()),
    };
    if edits_value.len() > MAX_MULTI_EDITS {
        return error(format!(
            "edits must contain at most {MAX_MULTI_EDITS} items"
        ));
    }
    if args.get("dry_run").is_some() && !args.get("dry_run").is_some_and(Value::is_boolean) {
        return error("dry_run must be a boolean".to_string());
    }
    let dry_run = args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let target = tool_try!(resolve_allowed(&path, &ctx.allowed_dirs, true));
    tool_try!(check_expected_sha256(&target, args));
    let source = match fs::read_to_string(&target) {
        Ok(text) => text,
        Err(err) => return error(format!("Read failed: {err}")),
    };

    let (updated, counts) = tool_try!(apply_snapshot_multi_edits(&target, &source, edits_value));

    let diff = format_unified_diff(&target, &source, &updated, MAX_DIFF_CHARS);
    let total: usize = counts.iter().sum();
    if dry_run {
        return ok(
            format!(
                "# Multi-edit preview: {}\ndry_run: true\nedits: {}\nreplacements: {total}\n\n## Diff\n{diff}",
                target.display(),
                counts.len()
            ),
            format!("Multi-edit preview {}", target.display()),
        );
    }
    with_file_write_lock(&target, || {
        let (format_sections, committed) = match commit_formatted_file_edit(
            &target,
            &source,
            &updated,
            &ctx.allowed_dirs,
            false,
            &ctx.cancellation,
                    ctx.policy.process_sandbox(),
        ) {
            Ok(committed) => committed,
            Err(outcome) => return outcome,
        };
        let diff = format_unified_diff(&target, &source, &committed, MAX_DIFF_CHARS);
        ok(
            format!(
                "Edited {} ({} edits, {total} total replacement{})\nedit_mode: snapshot_atomic\n\n## Diff\n{diff}{}",
                target.display(),
                counts.len(),
                if total == 1 { "" } else { "s" },
                format_sections.join("")
            ),
            format!("{} {}", tr(Message::DisplayEditedPrefix), target.display()),
        )
        .with_file_artifact(&target, "edit")
    })
}

#[cfg(test)]
mod snapshot_multi_edit_tests {
    use super::*;

    #[test]
    fn snapshot_multi_edit_applies_non_overlapping_changes_atomically() {
        let edits = json!([
            {"old_text":"one\n","new_text":"first\nsecond\n"},
            {"old_text":"three\n","new_text":"last\n"}
        ]);
        let (updated, counts) = apply_snapshot_multi_edits(
            Path::new("/tmp/snapshot-test.txt"),
            "one\ntwo\nthree\n",
            edits.as_array().unwrap(),
        )
        .unwrap();

        assert_eq!(updated, "first\nsecond\ntwo\nlast\n");
        assert_eq!(counts, vec![1, 1]);
    }

    #[test]
    fn snapshot_multi_edit_rejects_overlapping_changes() {
        let edits = json!([
            {"old_text":"alpha beta","new_text":"left"},
            {"old_text":"beta gamma","new_text":"right"}
        ]);
        let outcome = apply_snapshot_multi_edits(
            Path::new("/tmp/snapshot-test.txt"),
            "alpha beta gamma",
            edits.as_array().unwrap(),
        )
        .unwrap_err();

        assert!(outcome.text.contains("edit_batch_conflict"));
    }
}

fn replace_in_files(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let old_text = match args.get("old_text").and_then(Value::as_str) {
        Some(value) if !value.is_empty() => value,
        _ => return error("old_text is required".to_string()),
    };
    let new_text = match args.get("new_text").and_then(Value::as_str) {
        Some(value) => value,
        None => return error("new_text is required".to_string()),
    };
    if new_text.len() > MAX_WRITE_BYTES {
        return error(format!("new_text exceeds {MAX_WRITE_BYTES} bytes"));
    }
    let glob_text = args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("**/*");
    let glob = match glob_regex(glob_text) {
        Ok(regex) => regex,
        Err(err) => return error(err),
    };
    let replace_all = match args.get("replace_all") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("replace_all must be a boolean".to_string()),
    };
    let dry_run = match args.get("dry_run") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("dry_run must be a boolean".to_string()),
    };
    let max_files = tool_try!(validate_integer_arg(
        args,
        "max_files",
        100,
        1,
        MAX_REPLACE_FILES
    )) as usize;
    tool_try!(expected_sha256_by_path(args));
    let mut scanned = 0usize;
    let mut truncated = false;
    let mut failure: Option<String> = None;
    let mut updates: Vec<(PathBuf, String, usize)> = Vec::new();
    let mut versions = HashMap::new();
    walk_files(&root, usize::MAX, &mut |path| {
        scanned += 1;
        if updates.len() >= max_files {
            truncated = true;
            return false;
        }
        if !is_text_source(path) {
            return true;
        }
        let Ok(rel) = path.strip_prefix(&root) else {
            return true;
        };
        if !matches!(glob_text, "**/*" | "**") && !glob.is_match(&path_slash(rel)) {
            return true;
        }
        let Ok(source) = fs::read_to_string(path) else {
            return true;
        };
        let file_old = convert_line_endings_like(&source, old_text);
        let file_new = convert_line_endings_like(&source, new_text);
        let count = source.matches(&file_old).count();
        if count == 0 {
            return true;
        }
        let updated = if replace_all {
            source.replace(&file_old, &file_new)
        } else {
            source.replacen(&file_old, &file_new, 1)
        };
        if updated.len() > MAX_EDITED_FILE_BYTES {
            failure = Some(format!(
                "updated file exceeds {MAX_EDITED_FILE_BYTES} bytes: {}",
                path.display()
            ));
            return false;
        }
        let version = match capture_exact_mutation_preimage(path, &source) {
            Ok(version) => version, Err(message) => { failure = Some(message); return false; }
        };
        versions.insert(path.to_path_buf(), version);
        updates.push((
            path.to_path_buf(),
            updated,
            if replace_all { count } else { 1 },
        ));
        true
    });
    if let Some(message) = failure {
        return error(message);
    }
    let (mutation, format_sections) = if dry_run { (None, Vec::new()) } else {
        let (outcome, sections) = commit_exact_file_batch(&root, &updates, &versions, args, ctx, true);
        if outcome.error { return outcome; }
        (Some(outcome), sections)
    };
    let total: usize = updates.iter().map(|(_path, _updated, count)| count).sum();
    let file_lines = if updates.is_empty() {
        "(none)".to_string()
    } else {
        updates
            .iter()
            .map(|(path, _updated, count)| {
                let digest = file_sha256_hex(path).unwrap_or_else(|_| "unavailable".to_string());
                format!("- {}: {count} sha256={digest}", path.display())
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut outcome = ok(
        format!(
            "# Replace in files: {}\nglob: {}\ndry_run: {}\nscanned_files: {}\nmatched_files: {}\nreplacements: {}\n\n## Files\n{}{}{}",
            root.display(),
            glob_text,
            dry_run,
            scanned,
            updates.len(),
            total,
            file_lines,
            if truncated {
                "\n...[file match limit reached]"
            } else {
                ""
            },
            format_sections.join("")
        ),
        format!(
            "{} {total} occurrence{} in {} file{}",
            if dry_run { "Previewed" } else { "Replaced" },
            if total == 1 { "" } else { "s" },
            updates.len(),
            if updates.len() == 1 { "" } else { "s" }
        ),
    );
    if let Some(mutation) = mutation { outcome.artifacts = mutation.artifacts; outcome.execution = mutation.execution; }
    outcome
}

fn rename_symbol(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let old_symbol = tool_try!(required_string(args, "old_symbol"));
    let new_symbol = tool_try!(required_string(args, "new_symbol"));
    let identifier_regex =
        Regex::new(r"^[A-Za-z_$][A-Za-z0-9_$]*$").expect("valid identifier regex");
    if !identifier_regex.is_match(&old_symbol) {
        return error("old_symbol must be an identifier".to_string());
    }
    if !identifier_regex.is_match(&new_symbol) {
        return error("new_symbol must be an identifier".to_string());
    }
    if old_symbol == new_symbol {
        return error("old_symbol and new_symbol must differ".to_string());
    }
    let glob_text = args
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("**/*");
    let glob = match glob_regex(glob_text) {
        Ok(regex) => regex,
        Err(err) => return error(err),
    };
    let dry_run = match args.get("dry_run") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("dry_run must be a boolean".to_string()),
    };
    let max_files = tool_try!(validate_integer_arg(
        args,
        "max_files",
        100,
        1,
        MAX_REPLACE_FILES
    )) as usize;
    tool_try!(expected_sha256_by_path(args));
    let reference_regex = tool_try!(symbol_reference_regex(&old_symbol));
    let mut scanned = 0usize;
    let mut truncated = false;
    let mut failure: Option<String> = None;
    let mut updates: Vec<(PathBuf, String, usize)> = Vec::new();
    let mut versions = HashMap::new();
    let mut previews: Vec<String> = Vec::new();
    walk_files(&root, usize::MAX, &mut |path| {
        scanned += 1;
        if updates.len() >= max_files {
            truncated = true;
            return false;
        }
        if !is_code_source(path) {
            return true;
        }
        let Ok(rel) = path.strip_prefix(&root) else {
            return true;
        };
        if !matches!(glob_text, "**/*" | "**") && !glob.is_match(&path_slash(rel)) {
            return true;
        }
        let Ok(source) = fs::read_to_string(path) else {
            return true;
        };
        let count = reference_regex.captures_iter(&source).count();
        if count == 0 {
            return true;
        }
        let updated = reference_regex
            .replace_all(&source, |captures: &regex::Captures<'_>| {
                format!(
                    "{}{}{}",
                    captures
                        .name("prefix")
                        .map(|value| value.as_str())
                        .unwrap_or(""),
                    new_symbol,
                    captures
                        .name("suffix")
                        .map(|value| value.as_str())
                        .unwrap_or("")
                )
            })
            .to_string();
        if updated.len() > MAX_EDITED_FILE_BYTES {
            failure = Some(format!(
                "updated file exceeds {MAX_EDITED_FILE_BYTES} bytes: {}",
                path.display()
            ));
            return false;
        }
        for (line_no, line) in source.lines().enumerate() {
            if previews.len() >= 40 {
                break;
            }
            if reference_regex.is_match(line) {
                previews.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    line_no + 1,
                    outline_signature(line)
                ));
            }
        }
        let version = match capture_exact_mutation_preimage(path, &source) {
            Ok(version) => version, Err(message) => { failure = Some(message); return false; }
        };
        versions.insert(path.to_path_buf(), version);
        updates.push((path.to_path_buf(), updated, count));
        true
    });
    if let Some(message) = failure {
        return error(message);
    }
    let mutation = if dry_run { None } else {
        let (outcome, _) = commit_exact_file_batch(&root, &updates, &versions, args, ctx, false);
        if outcome.error { return outcome; }
        Some(outcome)
    };
    let total: usize = updates.iter().map(|(_path, _updated, count)| count).sum();
    let file_lines = if updates.is_empty() {
        "(none)".to_string()
    } else {
        updates
            .iter()
            .map(|(path, _updated, count)| {
                let digest = file_sha256_hex(path).unwrap_or_else(|_| "unavailable".to_string());
                format!("- {}: {count} sha256={digest}", path.display())
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let preview_lines = if previews.is_empty() {
        "(none)".to_string()
    } else {
        let mut lines = previews.join("\n");
        if previews.len() >= 40 {
            lines.push_str("\n...[match preview truncated]");
        }
        lines
    };
    let mut outcome = ok(
        format!(
            "# Rename symbol: {} -> {}\nroot: {}\nglob: {}\ndry_run: {}\nscanned_files: {}\nmatched_files: {}\nreplacements: {}\n\n## Files\n{}{}\n\n## Match preview\n{}",
            old_symbol,
            new_symbol,
            root.display(),
            glob_text,
            dry_run,
            scanned,
            updates.len(),
            total,
            file_lines,
            if truncated {
                "\n...[file match limit reached]"
            } else {
                ""
            },
            preview_lines
        ),
        format!(
            "{} {total} occurrence{} in {} file{}",
            if dry_run { "Previewed" } else { "Renamed" },
            if total == 1 { "" } else { "s" },
            updates.len(),
            if updates.len() == 1 { "" } else { "s" }
        ),
    );
    if let Some(mutation) = mutation { outcome.artifacts = mutation.artifacts; outcome.execution = mutation.execution; }
    outcome
}

fn apply_patch_file(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let patch = match args.get("patch").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value,
        _ => return error("patch is required".to_string()),
    };
    let patch = normalize_unified_patch(patch);
    if patch.len() > MAX_PATCH_BYTES {
        return error(format!("patch exceeds {MAX_PATCH_BYTES} bytes"));
    }
    let dry_run = tool_try!(bool_arg(args, "dry_run", true));
    tool_try!(expected_sha256_by_path(args));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let paths = tool_try!(patch_paths(&patch));
    let touched_paths = paths.iter().map(|path| root.join(path)).collect::<Vec<_>>();
    let before = tool_try!(observe_patch_targets(&touched_paths));
    if !dry_run {
        tool_try!(guard_existing_files_by_path(&root, &touched_paths, args));
    }
    let file_lines = paths.iter().zip(&before).map(|(path, snapshot)| {
        let digest = snapshot.as_ref().map(|snapshot| snapshot.version.sha256.as_str()).unwrap_or("(new file)");
        format!("- {path} sha256={digest}")
    }).collect::<Vec<_>>();
    let mut missing_parents = Vec::new();
    for path in &touched_paths {
        for parent in tool_try!(missing_mutation_parents(path).map_err(error)) {
            if !missing_parents.contains(&parent) { missing_parents.push(parent); }
        }
    }
    let (check, recount) = tool_try!(check_git_apply(&root, &patch, &ctx.cancellation));
    if !check.status.success() {
        return error_with_display(
            format!("Patch check failed:\n{}", command_output_text(&check)),
            "Patch check failed".to_string(),
        );
    }
    if dry_run {
        return ok(
            format!(
                "# Patch preview: {}\ndry_run: true\nfiles:\n{}\n\n## Patch\n{}",
                root.display(),
                file_lines.join("\n"),
                patch
            ),
            format!(
                "Patch preview ({} file{})",
                paths.len(),
                if paths.len() == 1 { "" } else { "s" }
            ),
        );
    }
    let mut lock_paths = touched_paths.clone();
    lock_paths.extend(missing_parents.iter().cloned());
    with_file_write_locks(&lock_paths, || {
        tool_try!(guard_existing_files_by_path(&root, &touched_paths, args));
        let current = tool_try!(observe_patch_targets(&touched_paths));
        if before.iter().zip(&current).any(|(before, current)| {
            before.as_ref().map(|value| &value.version) != current.as_ref().map(|value| &value.version)
        }) {
            return error("Patch preimage changed before commit; no mutation was started".into());
        }
        // Check again under the same lock as execution. Only read-only checks
        // may retry; once execution starts its outcome must be observed.
        let check = tool_try!(run_git_apply_once(&root, &patch, true, recount, &ctx.cancellation));
        if !check.status.success() {
            return error(format!("Patch check failed: {}", command_output_text(&check)));
        }
        let current = tool_try!(observe_patch_targets(&touched_paths));
        if before.iter().zip(&current).any(|(before, current)| {
            before.as_ref().map(|value| &value.version) != current.as_ref().map(|value| &value.version)
        }) {
            return error("Patch preimage changed during check; no mutation was started".into());
        }
        let applied = run_git_apply_once(&root, &patch, false, recount, &ctx.cancellation);
        let outcome = match applied {
            Ok(output) if output.status.success() => ok(
                format!("Applied patch in {}\nfiles:\n{}", root.display(), file_lines.join("\n")),
                format!("Applied patch ({} file{})", paths.len(), if paths.len() == 1 { "" } else { "s" }),
            ),
            Ok(output) => error(format!("Patch apply failed: {}", command_output_text(&output))),
            Err(outcome) => outcome,
        };
        let outcome = observe_patch_effects(outcome, &touched_paths, &before);
        observe_patch_parent_changes(outcome, &missing_parents)
    })
}

fn observe_patch_parent_changes(mut outcome: ToolOutcome, missing_parents: &[PathBuf]) -> ToolOutcome {
    for parent in missing_parents {
        let observed = created_directory_version(parent);
        if matches!(observed, Ok(None)) { continue; }
        let unknown = observed.is_err();
        let after_version = observed.ok().flatten();
        let receipt = outcome.execution.get_or_insert_with(Default::default);
        if let Ok(uri) = url::Url::from_file_path(parent) {
            // git apply owns its mkdir internally. An external writer does not
            // acquire our locks: this is an observed difference, not attribution.
            receipt.effects.push(nola_host_protocol::tool_result::ResourceEffect {
                resource_uri: uri.into(), operation: "observed_directory_change".into(),
                before_version: Some("absent:v1".into()), after_version,
            });
        }
        if unknown {
            receipt.state = nola_host_protocol::tool_result::ExecutionState::Unknown;
            outcome.error = true;
            outcome.text.push_str(&format!("\nPatch parent cannot be observed: {}; effects remain unknown. Do not automatically replay.", parent.display()));
        } else {
            outcome.text.push_str(&format!("\nObserved directory appeared: {} (creation ownership is not established).", parent.display()));
        }
    }
    outcome
}

fn observe_patch_targets(paths: &[PathBuf]) -> Result<Vec<Option<FileMutationSnapshot>>, ToolOutcome> {
    for path in paths {
        // O_NOFOLLOW protects the final entry; reject symlink ancestors too.
        for ancestor in path.ancestors() {
            if fs::symlink_metadata(ancestor).is_ok_and(|value| value.file_type().is_symlink()) {
                return Err(error(format!("Patch target contains a symlink: {}", ancestor.display())));
            }
        }
    }
    paths.iter().map(|path| observe_file_mutation_target(path)).collect()
}

fn observe_patch_effects(mut outcome: ToolOutcome, paths: &[PathBuf], before: &[Option<FileMutationSnapshot>]) -> ToolOutcome {
    let failed = outcome.error;
    // Artifact construction skips error outcomes. Collect actual effects first,
    // then restore the execution failure instead of losing partial evidence.
    outcome.error = false;
    let mut unknown = false;
    for (path, before) in paths.iter().zip(before) {
        match observe_file_mutation_target(path) {
            Ok(after) => {
                if after.is_some() { refresh_file_access_state(path); } else { forget_file_access_state(path); }
                if before.as_ref().map(|value| &value.version) == after.as_ref().map(|value| &value.version) { continue; }
                let operation = match (before.is_some(), after.is_some()) {
                    (false, true) => "create", (true, false) => "delete", _ => "edit",
                };
                outcome = with_mutation_artifact(outcome, path, operation, mutation_version(before), mutation_version(&after));
            }
            Err(_) => {
                unknown = true;
                forget_file_access_state(path);
                if let Ok(uri) = url::Url::from_file_path(path) {
                    outcome.execution.get_or_insert_with(Default::default).effects.push(
                        nola_host_protocol::tool_result::ResourceEffect {
                            resource_uri: uri.into(), operation: "edit".into(),
                            before_version: Some(mutation_version(before)), after_version: None,
                        },
                    );
                }
            }
        }
    }
    let receipt = outcome.execution.get_or_insert_with(|| nola_host_protocol::tool_result::ExecutionReceipt {
        local_stop: None, state: nola_host_protocol::tool_result::ExecutionState::Unknown,
        effects: Vec::new(), checks: Vec::new(),
    });
    receipt.state = if unknown || (failed && receipt.effects.is_empty()) {
        nola_host_protocol::tool_result::ExecutionState::Unknown
    } else if failed {
        nola_host_protocol::tool_result::ExecutionState::Partial
    } else { nola_host_protocol::tool_result::ExecutionState::Completed };
    if failed || unknown {
        outcome.error = true;
        outcome.text.push_str("\nDo not replay this mutation automatically; inspect recorded effects and current resources before recovery.");
    }
    outcome
}

fn normalize_unified_patch(patch: &str) -> String {
    let mut lines = patch.lines().collect::<Vec<_>>();
    if lines.first().is_some_and(|line| line.trim() == "*** Begin Patch") {
        lines.remove(0);
    }
    if lines.last().is_some_and(|line| line.trim() == "*** End Patch") {
        lines.pop();
    }
    let mut normalized = lines.join("\n");
    normalized.push('\n');
    normalized
}

fn patch_paths(patch: &str) -> Result<Vec<String>, ToolOutcome> {
    let mut paths = Vec::new();
    let mut declared = Vec::new();
    for line in patch.lines() {
        // The executable format must not contain effects absent from unified
        // headers (mode-only, binary, copy or rename-only Git records).
        if line.starts_with("GIT binary patch") || line.starts_with("Binary files ")
            || line.starts_with("old mode ") || line.starts_with("new mode ")
            || line.starts_with("copy from ") || line.starts_with("copy to ") {
            return Err(error("Patch contains an unsupported binary, mode-only or copy operation; use explicit file operations".into()));
        }
        if let Some(mode) = line.strip_prefix("new file mode ").or_else(|| line.strip_prefix("deleted file mode ")) {
            if !matches!(mode, "100644" | "100755") {
                return Err(error("Patch requires regular files; symlink and submodule patches are unsupported".into()));
            }
        }
        if let Some(index) = line.strip_prefix("index ") {
            if let Some(mode) = index.split_whitespace().nth(1) {
                if !matches!(mode, "100644" | "100755") {
                    return Err(error("Patch index requires a regular file mode".into()));
                }
            }
        }
        if let Some(raw) = line.strip_prefix("diff --git ") {
            let pair = raw.split_whitespace().collect::<Vec<_>>();
            if pair.len() != 2 { return Err(error("Ambiguous Git patch paths are unsupported".into())); }
            for path in pair {
                if let Some(path) = normalize_patch_path(path)? { declared.push(path); }
            }
        }
        if let Some(raw) = line.strip_prefix("rename from ").or_else(|| line.strip_prefix("rename to ")) {
            // These headers have no a/ or b/ prefix. Do not strip a real first
            // directory named a or b.
            if let Some(path) = normalize_patch_path(&format!("a/{raw}"))? { declared.push(path); }
        }
        if let Some(raw_path) = line
            .strip_prefix("--- ")
            .or_else(|| line.strip_prefix("+++ "))
        {
            if let Some(path) = normalize_patch_path(raw_path)? {
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
    }
    if paths.is_empty() {
        return Err(error(
            "patch must contain unified diff file headers".to_string(),
        ));
    }
    if declared.iter().any(|path| !paths.contains(path)) {
        return Err(error("Every Git patch resource must appear in unified file headers; metadata-only operations are unsupported".into()));
    }
    paths.sort();
    Ok(paths)
}

fn normalize_patch_path(raw_path: &str) -> Result<Option<String>, ToolOutcome> {
    let mut path = raw_path.trim();
    if path.is_empty() || path == "/dev/null" {
        return Ok(None);
    }
    path = path.split_once('\t').map(|(head, _)| head).unwrap_or(path);
    if path.contains('"') || path.contains('\\') || path.contains(' ') || path.chars().any(char::is_control) {
        return Err(error(format!("unsafe patch path: {raw_path}; ambiguous or quoted paths are unsupported")));
    }
    if let Some(stripped) = path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")) {
        path = stripped;
    } else {
        return Err(error(format!("unsafe patch path: {raw_path}; unified headers require a/ or b/ prefixes")));
    }
    if path.is_empty()
        || path.starts_with('/')
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(error(format!("unsafe patch path: {raw_path}")));
    }
    Ok(Some(path.to_string()))
}

fn check_git_apply(
    root: &Path,
    patch: &str,
    cancellation: &RuntimeExecutionControl,
) -> Result<(std::process::Output, bool), ToolOutcome> {
    let first = run_git_apply_once(root, patch, true, false, cancellation)?;
    if first.status.success() {
        return Ok((first, false));
    }
    run_git_apply_once(root, patch, true, true, cancellation).map(|output| (output, true))
}

fn run_git_apply_once(
    root: &Path,
    patch: &str,
    check: bool,
    recount: bool,
    cancellation: &RuntimeExecutionControl,
) -> Result<std::process::Output, ToolOutcome> {
    if cancellation.is_requested() {
        let mut outcome = error("git apply cancelled before process start; tool_execution_skipped: true".into());
        outcome.execution = Some(nola_host_protocol::tool_result::ExecutionReceipt {
            state: nola_host_protocol::tool_result::ExecutionState::NotStarted,
            ..Default::default()
        });
        return Err(outcome);
    }
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .arg("apply")
        .arg("--whitespace=nowarn");
    if recount {
        command.arg("--recount");
    }
    if check {
        command.arg("--check");
    }
    command
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => return Err(error(format!("git apply failed to start: {err}"))),
    };
    if let Some(stdin) = child.stdin.as_mut() {
        if let Err(err) = stdin.write_all(patch.as_bytes()) {
            let _ = terminate_process_tree(&mut child, Duration::from_secs(2));
            return Err(error(format!("git apply stdin failed: {err}")));
        }
    }
    wait_for_child_output(child, cancellation, Duration::from_secs(60), "git apply")
}

fn command_output_text(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        "git apply failed".to_string()
    } else {
        stdout
    }
}

fn git_pathspecs(root: &Path, paths: &[PathBuf]) -> Vec<String> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut specs = Vec::new();
    for path in paths {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let Ok(rel) = path.strip_prefix(&root) else {
            continue;
        };
        let rel = path_slash(rel);
        if !rel.is_empty() && !specs.contains(&rel) {
            specs.push(rel);
        }
    }
    specs
}

fn git_args_with_pathspecs(base: &[&str], pathspecs: &[String]) -> Vec<String> {
    let mut args = base
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    args.push("--".to_string());
    if pathspecs.is_empty() {
        args.push(".".to_string());
    } else {
        args.extend(pathspecs.iter().cloned());
    }
    args
}

fn run_git_strings(root: &Path, args: &[String]) -> Result<std::process::Output, ToolOutcome> {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_git(root, &refs)
}

fn git_untracked_files(root: &Path, pathspecs: &[String]) -> Vec<PathBuf> {
    let args = git_args_with_pathspecs(&["ls-files", "--others", "--exclude-standard"], pathspecs);
    let Ok(output) = run_git_strings(root, &args) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut files = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let rel = line.trim();
        if rel.is_empty() {
            continue;
        }
        let candidate = root.join(rel);
        let candidate = candidate.canonicalize().unwrap_or(candidate);
        if candidate.is_file() && candidate.starts_with(&root) && !files.contains(&candidate) {
            files.push(candidate);
        }
    }
    files
}

fn parse_diff_changed_lines(text: &str) -> HashMap<String, Vec<usize>> {
    let Ok(hunk_regex) = Regex::new(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@") else {
        return HashMap::new();
    };
    let mut changed: HashMap<String, Vec<usize>> = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(marker) = line.strip_prefix("+++ ") {
            let marker = marker.trim();
            current = if marker == "/dev/null" {
                None
            } else if let Some(path) = marker.strip_prefix("b/") {
                Some(path.to_string())
            } else {
                Some(marker.to_string())
            };
            if let Some(path) = &current {
                changed.entry(path.clone()).or_default();
            }
            continue;
        }
        let Some(captures) = hunk_regex.captures(line) else {
            continue;
        };
        let Some(path) = &current else {
            continue;
        };
        let start = captures
            .get(1)
            .and_then(|value| value.as_str().parse::<usize>().ok())
            .unwrap_or(1);
        let count = captures
            .get(2)
            .and_then(|value| value.as_str().parse::<usize>().ok())
            .unwrap_or(1);
        if count == 0 {
            continue;
        }
        let lines = changed.entry(path.clone()).or_default();
        for line_no in start..(start + count) {
            if !lines.contains(&line_no) {
                lines.push(line_no);
            }
        }
    }
    changed
}

fn merge_changed_lines(
    target: &mut HashMap<String, Vec<usize>>,
    incoming: HashMap<String, Vec<usize>>,
) {
    for (path, lines) in incoming {
        let entry = target.entry(path).or_default();
        for line_no in lines {
            if !entry.contains(&line_no) {
                entry.push(line_no);
            }
        }
    }
}

fn format_line_ranges(line_numbers: &[usize]) -> String {
    let mut sorted = line_numbers.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let Some(first) = sorted.first().copied() else {
        return String::new();
    };
    let mut ranges = Vec::new();
    let mut start = first;
    let mut previous = first;
    for line_no in sorted.into_iter().skip(1) {
        if line_no == previous + 1 {
            previous = line_no;
            continue;
        }
        if start == previous {
            ranges.push(start.to_string());
        } else {
            ranges.push(format!("{start}-{previous}"));
        }
        start = line_no;
        previous = line_no;
    }
    if start == previous {
        ranges.push(start.to_string());
    } else {
        ranges.push(format!("{start}-{previous}"));
    }
    ranges.join(", ")
}

fn symbol_for_changed_line(
    lines: &[String],
    line_no: usize,
) -> (usize, &'static str, String, String) {
    let mut nearest = (1, "module", "module".to_string(), "module".to_string());
    for (index, line) in lines.iter().take(line_no).enumerate() {
        if let Ok(Some((kind, name))) = outline_symbol(line) {
            nearest = (index + 1, kind, name, outline_signature(line));
        }
    }
    nearest
}

fn changed_symbol_entries(
    root: &Path,
    changed_lines_by_rel: &HashMap<String, Vec<usize>>,
    max_entries: usize,
) -> (Vec<String>, usize) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut grouped: HashMap<(String, usize, String, String, String), Vec<usize>> = HashMap::new();
    let mut rels = changed_lines_by_rel.keys().cloned().collect::<Vec<_>>();
    rels.sort();
    let mut scanned_files = 0;
    for rel in rels {
        if grouped.len() >= max_entries {
            break;
        }
        let path = root.join(&rel);
        let path = path.canonicalize().unwrap_or(path);
        if !path.starts_with(&root) || !path.is_file() || !is_text_source(&path) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        scanned_files += 1;
        let lines = text.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
        let mut changed_lines = changed_lines_by_rel.get(&rel).cloned().unwrap_or_default();
        changed_lines.sort_unstable();
        changed_lines.dedup();
        for line_no in changed_lines {
            if grouped.len() >= max_entries {
                break;
            }
            if line_no == 0 {
                continue;
            }
            let capped_line = line_no.min(lines.len());
            let (symbol_line, kind, name, signature) = symbol_for_changed_line(&lines, capped_line);
            let key = (rel.clone(), symbol_line, kind.to_string(), name, signature);
            let entry = grouped.entry(key).or_default();
            if !entry.contains(&line_no) {
                entry.push(line_no);
            }
        }
    }
    let mut entries = grouped.into_iter().collect::<Vec<_>>();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    (
        entries
            .into_iter()
            .map(|((rel, symbol_line, kind, name, signature), lines)| {
                format!(
                    "- {rel}:{symbol_line}: {kind} {name} | {signature}\n  changed_lines: {}",
                    format_line_ranges(&lines)
                )
            })
            .collect(),
        scanned_files,
    )
}

fn format_new_file_diff(root: &Path, path: &Path, max_chars: usize) -> String {
    let rel = path
        .strip_prefix(root)
        .map(path_slash)
        .unwrap_or_else(|_| path.display().to_string());
    let Ok(bytes) = fs::read(path) else {
        return format!("diff --git a/{rel} b/{rel}\n# unable to read untracked file\n");
    };
    let capped = &bytes[..bytes.len().min(MAX_READ_BYTES as usize)];
    let text = String::from_utf8_lossy(capped);
    let lines = text.lines().collect::<Vec<_>>();
    let mut diff = format!(
        "diff --git a/{rel} b/{rel}\nnew file mode 100644\nindex 0000000..0000000\n--- /dev/null\n+++ b/{rel}\n@@ -0,0 +1,{} @@\n",
        lines.len()
    );
    for line in lines {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    if diff.len() > max_chars {
        format!(
            "{}\n...[untracked diff truncated]\n",
            diff.chars().take(max_chars).collect::<String>().trim_end()
        )
    } else {
        diff
    }
}

fn parse_git_blame_porcelain(text: &str, max_lines: usize) -> Vec<String> {
    let commit_re = Regex::new(r"^\^?[0-9a-f]{7,40}$").expect("valid blame commit regex");
    let mut entries = Vec::new();
    let mut commit = String::new();
    let mut line_no = String::new();
    let mut author = String::from("unknown");
    let mut summary = String::from("(no summary)");
    for line in text.lines() {
        if entries.len() >= max_lines {
            break;
        }
        if let Some(content) = line.strip_prefix('\t') {
            let short_commit = if commit.len() > 10 {
                &commit[..10]
            } else {
                commit.as_str()
            };
            entries.push(format!(
                "{}: {} {} | {} | {}",
                if line_no.is_empty() { "?" } else { &line_no },
                if short_commit.is_empty() {
                    "unknown"
                } else {
                    short_commit
                },
                author,
                summary,
                content
            ));
            commit.clear();
            line_no.clear();
            author = String::from("unknown");
            summary = String::from("(no summary)");
            continue;
        }
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() >= 3 && commit_re.is_match(parts[0]) {
            commit = parts[0].trim_start_matches('^').to_string();
            line_no = parts[2].to_string();
        } else if let Some(value) = line.strip_prefix("author ") {
            author = value.to_string();
        } else if let Some(value) = line.strip_prefix("summary ") {
            summary = value.to_string();
        }
    }
    entries
}

fn is_safe_git_revision(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && !value.starts_with('-')
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '/' | '~' | '^' | ':' | '-')
        })
}

fn git_history(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let scoped_path = match args.get("path") {
        Some(Value::String(value)) if !value.trim().is_empty() => {
            let path = tool_try!(resolve_allowed(value, &ctx.allowed_dirs, true));
            if !path.starts_with(&root) {
                return error(format!("path is outside history root: {}", path.display()));
            }
            Some(path)
        }
        Some(Value::Null) | None => None,
        Some(Value::String(_)) => None,
        _ => return error("path must be a string".to_string()),
    };
    let max_commits = tool_try!(validate_integer_arg(args, "max_commits", 10, 1, 50)) as usize;
    let max_chars = tool_try!(validate_integer_arg(
        args,
        "max_chars",
        40_000,
        1_000,
        MAX_DIFF_CHARS as i64
    )) as usize;
    let git_root = tool_try!(run_git(&root, &["rev-parse", "--show-toplevel"]));
    if !git_root.status.success() {
        return error_with_display(
            format!("Git history failed:\n{}", command_output_text(&git_root)),
            "Git history failed".to_string(),
        );
    }
    let pathspecs = git_pathspecs(
        &root,
        &scoped_path
            .as_ref()
            .map(|path| vec![path.clone()])
            .unwrap_or_default(),
    );
    let mut log_args = vec![
        "log".to_string(),
        format!("-{max_commits}"),
        "--date=short".to_string(),
        "--pretty=format:commit %h%nDate: %ad%nAuthor: %an%nSubject: %s".to_string(),
        "--name-status".to_string(),
        "--".to_string(),
    ];
    if pathspecs.is_empty() {
        log_args.push(".".to_string());
    } else {
        log_args.extend(pathspecs.iter().cloned());
    }
    let history = tool_try!(run_git_strings(&root, &log_args));
    if !history.status.success() {
        return error_with_display(
            format!("Git history failed:\n{}", command_output_text(&history)),
            "Git history failed".to_string(),
        );
    }
    let raw_history = String::from_utf8_lossy(&history.stdout).trim().to_string();
    let truncated = raw_history.chars().count() > max_chars;
    let history_text = if raw_history.is_empty() {
        "(no commits for this scope)".to_string()
    } else if truncated {
        format!(
            "{}\n...[history truncated]",
            raw_history
                .chars()
                .take(max_chars)
                .collect::<String>()
                .trim_end()
        )
    } else {
        raw_history.clone()
    };
    let commit_count = raw_history.matches("commit ").count();
    ok(
        format!(
            "# Git history: {}\ngit_root: {}\nscope: {}\n\n{}",
            root.display(),
            String::from_utf8_lossy(&git_root.stdout).trim(),
            if pathspecs.is_empty() {
                ".".to_string()
            } else {
                pathspecs.join(", ")
            },
            history_text
        ),
        format!(
            "Git history ({commit_count} commit{})",
            if commit_count == 1 { "" } else { "s" }
        ),
    )
}

fn git_show(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let revision = tool_try!(required_string(args, "revision"));
    if !is_safe_git_revision(&revision) {
        return error("revision contains unsupported characters".to_string());
    }
    let scoped_path = match args.get("path") {
        Some(Value::String(value)) if !value.trim().is_empty() => {
            let path = tool_try!(resolve_allowed(value, &ctx.allowed_dirs, true));
            if !path.starts_with(&root) {
                return error(format!("path is outside show root: {}", path.display()));
            }
            Some(path)
        }
        Some(Value::Null) | None => None,
        Some(Value::String(_)) => None,
        _ => return error("path must be a string".to_string()),
    };
    let include_patch = match args.get("include_patch") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => true,
        _ => return error("include_patch must be a boolean".to_string()),
    };
    let max_chars = tool_try!(validate_integer_arg(
        args,
        "max_chars",
        40_000,
        1_000,
        MAX_DIFF_CHARS as i64
    )) as usize;
    let git_root = tool_try!(run_git(&root, &["rev-parse", "--show-toplevel"]));
    if !git_root.status.success() {
        return error_with_display(
            format!("Git show failed:\n{}", command_output_text(&git_root)),
            "Git show failed".to_string(),
        );
    }
    let pathspecs = git_pathspecs(
        &root,
        &scoped_path
            .as_ref()
            .map(|path| vec![path.clone()])
            .unwrap_or_default(),
    );
    let mut show_args = vec![
        "show".to_string(),
        "--date=short".to_string(),
        "--find-renames".to_string(),
        "--stat".to_string(),
        "--pretty=format:commit %h%nDate: %ad%nAuthor: %an%nSubject: %s".to_string(),
        if include_patch {
            "--patch".to_string()
        } else {
            "--no-patch".to_string()
        },
        revision.clone(),
        "--".to_string(),
    ];
    show_args.extend(pathspecs.iter().cloned());
    let shown = tool_try!(run_git_strings(&root, &show_args));
    if !shown.status.success() {
        return error_with_display(
            format!("Git show failed:\n{}", command_output_text(&shown)),
            "Git show failed".to_string(),
        );
    }
    let raw_body = String::from_utf8_lossy(&shown.stdout).trim().to_string();
    let truncated = raw_body.chars().count() > max_chars;
    let body = if raw_body.is_empty() {
        "(no output for this revision)".to_string()
    } else if truncated {
        format!(
            "{}\n...[show truncated]",
            raw_body
                .chars()
                .take(max_chars)
                .collect::<String>()
                .trim_end()
        )
    } else {
        raw_body
    };
    ok(
        format!(
            "# Git show: {}\ngit_root: {}\nscope: {}\n\n{}",
            revision,
            String::from_utf8_lossy(&git_root.stdout).trim(),
            if pathspecs.is_empty() {
                ".".to_string()
            } else {
                pathspecs.join(", ")
            },
            body
        ),
        format!("Git show {revision}"),
    )
}

fn git_blame(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let path_arg = tool_try!(required_string(args, "path"));
    let path = tool_try!(resolve_allowed(&path_arg, &ctx.allowed_dirs, true));
    if !path.is_file() {
        return error(format!("Not a file: {}", path.display()));
    }
    if !path.starts_with(&root) {
        return error(format!("path is outside blame root: {}", path.display()));
    }
    let pathspecs = git_pathspecs(&root, &[path.clone()]);
    let Some(pathspec) = pathspecs.first() else {
        return error("path is outside blame root".to_string());
    };
    let start_line = tool_try!(validate_integer_arg(
        args,
        "start_line",
        1,
        1,
        1_000_000_000
    )) as usize;
    let line_count = tool_try!(validate_integer_arg(args, "line_count", 80, 1, 500)) as usize;
    let end_line = start_line + line_count - 1;
    let git_root = tool_try!(run_git(&root, &["rev-parse", "--show-toplevel"]));
    if !git_root.status.success() {
        return error_with_display(
            format!("Git blame failed:\n{}", command_output_text(&git_root)),
            "Git blame failed".to_string(),
        );
    }
    let blame_args = vec![
        "blame".to_string(),
        "--line-porcelain".to_string(),
        "-L".to_string(),
        format!("{start_line},{end_line}"),
        "--".to_string(),
        pathspec.clone(),
    ];
    let blame = tool_try!(run_git_strings(&root, &blame_args));
    if !blame.status.success() {
        return error_with_display(
            format!("Git blame failed:\n{}", command_output_text(&blame)),
            "Git blame failed".to_string(),
        );
    }
    let raw_blame = String::from_utf8_lossy(&blame.stdout);
    let entries = parse_git_blame_porcelain(&raw_blame, line_count);
    ok(
        format!(
            "# Git blame: {}\ngit_root: {}\nrange: {}-{}\n\n{}",
            path.display(),
            String::from_utf8_lossy(&git_root.stdout).trim(),
            start_line,
            end_line,
            if entries.is_empty() {
                "(no blame entries for this range)".to_string()
            } else {
                entries.join("\n")
            }
        ),
        format!(
            "Git blame ({} line{})",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" }
        ),
    )
}

fn git_diff(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
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
                    return error(format!("path is outside diff root: {}", resolved.display()));
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
    let git_root = tool_try!(run_git(&root, &["rev-parse", "--show-toplevel"]));
    if !git_root.status.success() {
        return error_with_display(
            format!("Git diff failed:\n{}", command_output_text(&git_root)),
            "Git diff failed".to_string(),
        );
    }
    let pathspecs = git_pathspecs(&root, &paths);
    let unstaged_args = git_args_with_pathspecs(&["diff", "--no-ext-diff", "--no-textconv"], &pathspecs);
    let staged_args = git_args_with_pathspecs(
        &["diff", "--no-ext-diff", "--no-textconv", "--cached"],
        &pathspecs,
    );
    let status_args = git_args_with_pathspecs(
        &[
            "status",
            "--porcelain=v1",
            if include_untracked { "--untracked-files=all" } else { "--untracked-files=no" },
        ],
        &pathspecs,
    );
    let status = tool_try!(run_git_strings(&root, &status_args));
    if !status.status.success() {
        return error(format!(
            "Unable to inspect Git status:\n{}", command_output_text(&status)
        ));
    }
    let status_text = String::from_utf8_lossy(&status.stdout);
    let unstaged = tool_try!(run_git_strings(&root, &unstaged_args));
    if !unstaged.status.success() {
        return error_with_display(
            format!("Git diff failed:\n{}", command_output_text(&unstaged)),
            "Git diff failed".to_string(),
        );
    }
    let staged = tool_try!(run_git_strings(&root, &staged_args));
    if !staged.status.success() {
        return error_with_display(
            format!("Git diff failed:\n{}", command_output_text(&staged)),
            "Git diff failed".to_string(),
        );
    }

    let unstaged_text_raw = String::from_utf8_lossy(&unstaged.stdout).to_string();
    let staged_text_raw = String::from_utf8_lossy(&staged.stdout).to_string();
    let untracked = if include_untracked {
        let listing_args = git_args_with_pathspecs(
            &["ls-files", "--others", "--exclude-standard", "-z"], &pathspecs,
        );
        let listing = tool_try!(run_git_strings(&root, &listing_args));
        if !listing.status.success() {
            return error(format!(
                "Unable to enumerate untracked files:\n{}", command_output_text(&listing)
            ));
        }
        // NUL-delimited names preserve whitespace, Unicode and embedded newlines.
        // Do not canonicalize symlinks into their targets: the link is the change.
        let listing = match std::str::from_utf8(&listing.stdout) {
            Ok(listing) => listing,
            Err(_) => return error(
                "Untracked file names are not valid UTF-8; cannot provide a complete diff".into()
            ),
        };
        listing.split('\0')
            .filter(|name| !name.is_empty())
            .map(|name| root.join(name))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut untracked_diffs = Vec::new();
    for path in &untracked {
        // Git supplies binary/symlink semantics and a complete textual patch.
        // Only the common output layer may shorten the model-facing preview.
        let diff = tool_try!(run_git_strings(
            &root,
            &[
                "diff".into(), "--no-ext-diff".into(), "--no-textconv".into(),
                "--no-index".into(), "--".into(), "/dev/null".into(),
                crate::platform_paths::shell_path(path),
            ],
        ));
        if !matches!(diff.status.code(), Some(0 | 1)) {
            return error(format!(
                "Unable to inspect untracked file {}:\n{}",
                path.display(), command_output_text(&diff)
            ));
        }
        untracked_diffs.push(String::from_utf8_lossy(&diff.stdout).into_owned());
    }
    let changed_files = status_text.lines().count();
    let mut sections = vec![
        format!("# Git diff: {}", root.display()),
        format!(
            "git_root: {}",
            String::from_utf8_lossy(&git_root.stdout).trim()
        ),
        format!(
            "scope: {}",
            if pathspecs.is_empty() {
                ".".to_string()
            } else {
                pathspecs.join(", ")
            }
        ),
        String::new(),
        "## Changed file inventory (Git porcelain: index / worktree)".to_string(),
        if status_text.is_empty() {
            "(none)".to_string()
        } else {
            status_text.trim_end().to_string()
        },
        String::new(),
        "## Unstaged diff".to_string(),
        if unstaged_text_raw.is_empty() {
            "(none)".to_string()
        } else {
            unstaged_text_raw.trim_end().to_string()
        },
        String::new(),
        "## Staged diff".to_string(),
        if staged_text_raw.is_empty() {
            "(none)".to_string()
        } else {
            staged_text_raw.trim_end().to_string()
        },
        String::new(),
        "## Untracked files".to_string(),
        if untracked.is_empty() {
            "(none)".to_string()
        } else {
            untracked
                .iter()
                .map(|path| format!("- {}", path.display()))
                .collect::<Vec<_>>()
                .join("\n")
        },
    ];
    if !untracked_diffs.is_empty() {
        sections.extend([
            String::new(),
            "## Untracked diff".to_string(),
            untracked_diffs.join("\n").trim_end().to_string(),
        ]);
    }
    ok(
        sections.join("\n"),
        format!(
            "Git diff ({changed_files} changed file{})",
            if changed_files == 1 { "" } else { "s" }
        ),
    )
}

fn workspace_checkpoint_store() -> &'static Mutex<HashMap<String, WorkspaceCheckpoint>> {
    WORKSPACE_CHECKPOINTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn workspace_checkpoint_from_record(record: DbWorkspaceCheckpoint) -> WorkspaceCheckpoint {
    WorkspaceCheckpoint {
        id: record.id,
        label: record.label,
        root: PathBuf::from(record.root),
        git_root: record.git_root,
        revision: record.revision,
        source: record.source,
        created_at: record.created_at,
        status: record.status,
        untracked: record.untracked,
    }
}

fn workspace_checkpoint_record(checkpoint: &WorkspaceCheckpoint) -> DbWorkspaceCheckpoint {
    DbWorkspaceCheckpoint {
        id: checkpoint.id.clone(),
        root: checkpoint.root.to_string_lossy().to_string(),
        git_root: checkpoint.git_root.clone(),
        revision: checkpoint.revision.clone(),
        source: checkpoint.source.clone(),
        label: checkpoint.label.clone(),
        created_at: checkpoint.created_at,
        status: checkpoint.status.clone(),
        untracked: checkpoint.untracked.clone(),
    }
}

fn sync_workspace_checkpoint_store(
    store: &mut HashMap<String, WorkspaceCheckpoint>,
    database: &NolaStore,
    root: &Path,
) -> Result<(), String> {
    let records = database.list_workspace_checkpoints(&root.to_string_lossy())?;
    let persisted_ids = records
        .iter()
        .map(|record| record.id.clone())
        .collect::<HashSet<_>>();
    store.retain(|id, checkpoint| checkpoint.root != root || persisted_ids.contains(id));
    for record in records {
        store.insert(record.id.clone(), workspace_checkpoint_from_record(record));
    }
    Ok(())
}

fn workspace_checkpoint_ref(checkpoint_id: &str) -> String {
    format!("refs/nola/checkpoints/{checkpoint_id}")
}

fn protect_workspace_checkpoint_revision(
    root: &Path,
    checkpoint_id: &str,
    revision: &str,
) -> Result<(), ToolOutcome> {
    let reference = workspace_checkpoint_ref(checkpoint_id);
    let result = run_git_strings(
        root,
        &[
            "update-ref".to_string(),
            reference.clone(),
            revision.to_string(),
        ],
    )?;
    if result.status.success() {
        Ok(())
    } else {
        Err(error_with_display(
            format!(
                "Workspace checkpoint ref creation failed for {reference}:\n{}",
                command_output_text(&result)
            ),
            "Workspace checkpoint could not be protected".to_string(),
        ))
    }
}

fn delete_workspace_checkpoint_ref(root: &Path, checkpoint_id: &str) -> Result<(), ToolOutcome> {
    let reference = workspace_checkpoint_ref(checkpoint_id);
    let result = run_git_strings(
        root,
        &["update-ref".to_string(), "-d".to_string(), reference],
    )?;
    if result.status.success() {
        Ok(())
    } else {
        Err(error_with_display(
            format!(
                "Workspace checkpoint ref cleanup failed:\n{}",
                command_output_text(&result)
            ),
            "Workspace checkpoint cleanup failed".to_string(),
        ))
    }
}

fn workspace_checkpoint_id(store: &HashMap<String, WorkspaceCheckpoint>) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let base = format!("cp-{nanos}-{}", std::process::id());
    let mut candidate = base.clone();
    let mut suffix = 2;
    while store.contains_key(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    candidate
}

fn checkpoint_private_index_output(root: &Path, index: &Path, arguments: &[&str]) -> Result<Vec<u8>, ToolOutcome> {
    let mut command = git_operation_command(root);
    command.env("GIT_INDEX_FILE", index).args(arguments).stdin(Stdio::null())
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().map_err(|e|error(e.to_string()))?;
    let cancellation = current_execution_control().unwrap_or_default();
    let output = wait_for_child_output(child, &cancellation, Duration::from_secs(60), "Git recovery snapshot")?;
    if !output.status.success() {
        return Err(error_with_display(format!("Workspace checkpoint failed: {}", command_output_text(&output)),
            "Workspace checkpoint failed".into()));
    }
    Ok(output.stdout)
}

fn tracked_snapshot_revision(root: &Path) -> Result<(String, String), ToolOutcome> {
    let head = run_git(root, &["rev-parse", "--verify", "HEAD"])?;
    if !head.status.success() {
        return Err(error_with_display(
            format!(
                "Workspace checkpoint failed:\n{}",
                command_output_text(&head)
            ),
            "Workspace checkpoint failed".to_string(),
        ));
    }
    // Snapshot with a private index. Intent-to-add in the live index must not
    // prevent recovery, and a checkpoint must never refresh/stage user state.
    let snapshot = tempfile::tempdir().map_err(|e|error(e.to_string()))?;
    let private_index = snapshot.path().join("index");
    let index = git_commit_text(root, &["rev-parse", "--path-format=absolute", "--git-path", "index"])
        .map_err(error)?;
    fs::copy(index, &private_index).map_err(|e|error(e.to_string()))?;
    let added = |visibility| -> Result<std::collections::BTreeSet<String>, ToolOutcome> {
        git_commit_paths(root, &["diff", "--cached", "--no-ext-diff", "--no-textconv", visibility,
            "--name-only", "-z", "--diff-filter=A", "--"]).map_err(error)
    };
    let visible = added("--ita-visible-in-index")?;
    let staged = added("--ita-invisible-in-index")?;
    let intents = visible.difference(&staged).map(String::as_str).collect::<Vec<_>>();
    if !intents.is_empty() {
        // Do not execute repository conversion/filter programs while preparing
        // a recovery snapshot. The live index remains byte-for-byte untouched.
        require_nonexecuting_worktree_preparation(root).map_err(error)?;
        for paths in intents.chunks(100) {
            let mut arguments = vec!["add", "-A", "--"];
            arguments.extend_from_slice(paths);
            checkpoint_private_index_output(root, &private_index, &arguments)?;
        }
    }
    let stash = checkpoint_private_index_output(root, &private_index, &["-c", "user.name=Nola Checkpoint", "-c", "user.email=nola-checkpoint@localhost", "stash", "create", "nola workspace checkpoint"])?;
    let stash_text = String::from_utf8_lossy(&stash).trim().to_string();
    if stash_text.is_empty() {
        Ok((
            String::from_utf8_lossy(&head.stdout).trim().to_string(),
            "HEAD".to_string(),
        ))
    } else {
        Ok((stash_text, "working tree snapshot".to_string()))
    }
}

fn workspace_checkpoint(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("create");
    if !matches!(action, "create" | "list" | "diff" | "restore") {
        return error("action must be create, list, diff, or restore".to_string());
    }
    let max_diff_chars = tool_try!(validate_integer_arg(
        args,
        "max_diff_chars",
        40_000,
        1_000,
        MAX_DIFF_CHARS as i64
    )) as usize;
    let root_canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
    // File preimages use the same durable checkpoint contract but do not have
    // a Git repository. Resolve persisted records before probing Git so list,
    // preview and recovery also work after a cold restart in regular folders.
    {
        let checkpoint_database = match NolaStore::open_default() {
            Ok(database) => database,
            Err(store_error) => {
                return error(format!(
                    "Open workspace checkpoint store failed: {store_error}"
                ))
            }
        };
        let records = match checkpoint_database
            .list_workspace_checkpoints(&root_canonical.to_string_lossy())
        {
            Ok(records) => records,
            Err(store_error) => {
                return error(format!("Load workspace checkpoints failed: {store_error}"))
            }
        };
        if action == "list" {
            let mut lines = vec![
                "# Workspace checkpoints".to_string(),
                format!("root: {}", root.display()),
                String::new(),
            ];
            if records.is_empty() {
                lines.push("(none)".to_string());
            } else {
                for checkpoint in records.iter().rev().take(MAX_CHECKPOINTS).rev() {
                    let label = if checkpoint.label.is_empty() {
                        String::new()
                    } else {
                        format!(" label={}", checkpoint.label)
                    };
                    lines.push(format!(
                        "- {} source={}{}",
                        checkpoint.id, checkpoint.source, label
                    ));
                }
            }
            return ok(
                lines.join("\n"),
                format!("Workspace checkpoints ({})", records.len()),
            );
        }
        if matches!(action, "diff" | "restore") {
            let checkpoint_id = tool_try!(required_string(args, "checkpoint_id"));
            let Some(record) = records
                .into_iter()
                .find(|checkpoint| checkpoint.id == checkpoint_id)
            else {
                return error(format!(
                    "Workspace checkpoint not found for this root: {checkpoint_id}"
                ));
            };
            if record.source == "file_preimage" {
                let checkpoint = workspace_checkpoint_from_record(record);
                let dry_run = tool_try!(bool_arg(args, "dry_run", true));
                return file_preimage_checkpoint_action(
                    &checkpoint,
                    &root_canonical,
                    ctx,
                    action,
                    max_diff_chars,
                    dry_run,
                );
            }
        }
    }
    let git_root = tool_try!(run_git(&root, &["rev-parse", "--show-toplevel"]));
    if !git_root.status.success() {
        return error_with_display(
            format!(
                "Workspace checkpoint failed:\n{}",
                command_output_text(&git_root)
            ),
            "Workspace checkpoint failed".to_string(),
        );
    }
    let git_root_text = String::from_utf8_lossy(&git_root.stdout).trim().to_string();
    let root_canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
    let store_mutex = workspace_checkpoint_store();
    let checkpoint_database = match NolaStore::open_default() {
        Ok(database) => database,
        Err(err) => return error(format!("Open workspace checkpoint store failed: {err}")),
    };
    {
        let mut store = match store_mutex.lock() {
            Ok(value) => value,
            Err(_) => return error("workspace checkpoint store is unavailable".to_string()),
        };
        if let Err(err) =
            sync_workspace_checkpoint_store(&mut store, &checkpoint_database, &root_canonical)
        {
            return error(format!("Load workspace checkpoints failed: {err}"));
        }
    }

    if action == "list" {
        let store = match store_mutex.lock() {
            Ok(value) => value,
            Err(_) => return error("workspace checkpoint store is unavailable".to_string()),
        };
        let mut matching = store
            .values()
            .filter(|checkpoint| checkpoint.root == root_canonical)
            .cloned()
            .collect::<Vec<_>>();
        matching.sort_by_key(|checkpoint| checkpoint.created_at);
        let mut lines = vec![
            "# Workspace checkpoints".to_string(),
            format!("root: {}", root.display()),
            format!("git_root: {git_root_text}"),
            String::new(),
        ];
        if matching.is_empty() {
            lines.push("(none)".to_string());
        } else {
            for checkpoint in matching.iter().rev().take(MAX_CHECKPOINTS).rev() {
                let label = if checkpoint.label.is_empty() {
                    String::new()
                } else {
                    format!(" label={}", checkpoint.label)
                };
                lines.push(format!(
                    "- {} revision={} source={}{}",
                    checkpoint.id, checkpoint.revision, checkpoint.source, label
                ));
            }
        }
        return ok(
            lines.join("\n"),
            format!("Workspace checkpoints ({})", matching.len()),
        );
    }

    if action == "create" {
        let checkpoint_id = {
            let store = match store_mutex.lock() {
                Ok(value) => value,
                Err(_) => return error("workspace checkpoint store is unavailable".to_string()),
            };
            workspace_checkpoint_id(&store)
        };
        let (revision, source) = tool_try!(tracked_snapshot_revision(&root));
        tool_try!(protect_workspace_checkpoint_revision(
            &root,
            &checkpoint_id,
            &revision
        ));
        let status = tool_try!(run_git(&root, &["status", "--short", "--", "."]));
        let untracked = git_untracked_files(&root, &[])
            .iter()
            .filter_map(|path| path.strip_prefix(&root_canonical).ok().map(path_slash))
            .collect::<Vec<_>>();
        let status_text = if status.status.success() {
            String::from_utf8_lossy(&status.stdout).trim().to_string()
        } else {
            String::new()
        };
        let checkpoint = WorkspaceCheckpoint {
            id: checkpoint_id.clone(),
            label: args
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            root: root_canonical.clone(),
            git_root: git_root_text.clone(),
            revision: revision.clone(),
            source: source.clone(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0),
            status: status_text,
            untracked: untracked.clone(),
        };
        if let Err(err) =
            checkpoint_database.save_workspace_checkpoint(&workspace_checkpoint_record(&checkpoint))
        {
            let _ = delete_workspace_checkpoint_ref(&root, &checkpoint_id);
            return error(err);
        }
        let mut store = match store_mutex.lock() {
            Ok(value) => value,
            Err(_) => return error("workspace checkpoint store is unavailable".to_string()),
        };
        store.insert(checkpoint_id.clone(), checkpoint);
        while store
            .values()
            .filter(|checkpoint| checkpoint.root == root_canonical)
            .count()
            > MAX_CHECKPOINTS
        {
            let oldest = store
                .values()
                .filter(|checkpoint| checkpoint.root == root_canonical)
                .min_by_key(|checkpoint| checkpoint.created_at)
                .map(|checkpoint| checkpoint.id.clone());
            if let Some(id) = oldest {
                if let Err(err) = checkpoint_database.delete_workspace_checkpoint(&id) {
                    return error(err);
                }
                store.remove(&id);
                let _ = delete_workspace_checkpoint_ref(&root, &id);
            } else {
                break;
            }
        }
        let lines = vec![
            "# Workspace checkpoint created".to_string(),
            format!("checkpoint_id: {checkpoint_id}"),
            format!("root: {}", root.display()),
            format!("git_root: {git_root_text}"),
            format!("revision: {revision}"),
            format!("source: {source}"),
            String::new(),
            "## Baseline status".to_string(),
            {
                let text = String::from_utf8_lossy(&status.stdout).trim().to_string();
                if text.is_empty() {
                    "(clean for this root)".to_string()
                } else {
                    text
                }
            },
            String::new(),
            "## Baseline untracked files".to_string(),
            if untracked.is_empty() {
                "(none)".to_string()
            } else {
                untracked
                    .iter()
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        ];
        return ok(
            lines.join("\n"),
            format!("Workspace checkpoint {checkpoint_id}"),
        );
    }

    let checkpoint_id = tool_try!(required_string(args, "checkpoint_id"));
    let checkpoint = {
        let store = match store_mutex.lock() {
            Ok(value) => value,
            Err(_) => return error("workspace checkpoint store is unavailable".to_string()),
        };
        store.get(&checkpoint_id).cloned()
    };
    let Some(checkpoint) = checkpoint else {
        return error(format!(
            "Workspace checkpoint not found for this root: {checkpoint_id}"
        ));
    };
    if checkpoint.root != root_canonical {
        return error(format!(
            "Workspace checkpoint not found for this root: {checkpoint_id}"
        ));
    }
    let diff_args = vec![
        "diff".to_string(),
        checkpoint.revision.clone(),
        "--".to_string(),
        ".".to_string(),
    ];
    let diff = tool_try!(run_git_strings(&root, &diff_args));
    if !diff.status.success() {
        return error_with_display(
            format!(
                "Workspace checkpoint diff failed:\n{}",
                command_output_text(&diff)
            ),
            "Workspace checkpoint diff failed".to_string(),
        );
    }
    let shortstat_args = vec![
        "diff".to_string(),
        "--shortstat".to_string(),
        checkpoint.revision.clone(),
        "--".to_string(),
        ".".to_string(),
    ];
    let shortstat = tool_try!(run_git_strings(&root, &shortstat_args));
    let status = tool_try!(run_git(&root, &["status", "--short", "--", "."]));
    let diff_raw = String::from_utf8_lossy(&diff.stdout).to_string();
    let diff_text = diff_raw.chars().take(max_diff_chars).collect::<String>();
    let truncated = diff_text.chars().count() < diff_raw.chars().count();
    let baseline_untracked = checkpoint.untracked.iter().cloned().collect::<Vec<_>>();
    let current_untracked = git_untracked_files(&root, &[])
        .iter()
        .filter_map(|path| path.strip_prefix(&root_canonical).ok().map(path_slash))
        .collect::<Vec<_>>();
    let mut new_untracked = current_untracked
        .iter()
        .filter(|item| !baseline_untracked.contains(item))
        .cloned()
        .collect::<Vec<_>>();
    new_untracked.sort();
    let mut removed_untracked = baseline_untracked
        .iter()
        .filter(|item| !current_untracked.contains(item))
        .cloned()
        .collect::<Vec<_>>();
    removed_untracked.sort();
    let mut untracked_lines = Vec::new();
    untracked_lines.extend(new_untracked.iter().map(|item| format!("+ {item}")));
    untracked_lines.extend(removed_untracked.iter().map(|item| format!("- {item}")));
    let mut lines = vec![
        "# Workspace checkpoint diff".to_string(),
        format!("checkpoint_id: {checkpoint_id}"),
        format!("root: {}", root.display()),
        format!("git_root: {}", checkpoint.git_root),
        format!("revision: {}", checkpoint.revision),
        format!("source: {}", checkpoint.source),
        String::new(),
        "## Tracked diff since checkpoint".to_string(),
        if diff_text.is_empty() {
            "(none)".to_string()
        } else {
            diff_text.trim_end().to_string()
        },
        String::new(),
        "## Shortstat".to_string(),
        {
            let text = String::from_utf8_lossy(&shortstat.stdout)
                .trim()
                .to_string();
            if shortstat.status.success() && !text.is_empty() {
                text
            } else {
                "(no tracked diff)".to_string()
            }
        },
        String::new(),
        "## Current status".to_string(),
        {
            let text = String::from_utf8_lossy(&status.stdout).trim().to_string();
            if status.status.success() && !text.is_empty() {
                text
            } else {
                "(clean for this root)".to_string()
            }
        },
        String::new(),
        "## Untracked changes since checkpoint".to_string(),
        if untracked_lines.is_empty() {
            "(none)".to_string()
        } else {
            untracked_lines.join("\n")
        },
    ];
    if !checkpoint.status.is_empty() {
        lines.extend([
            String::new(),
            "## Baseline status".to_string(),
            checkpoint.status,
        ]);
    }
    if truncated {
        lines.extend([String::new(), "...[diff truncated]".to_string()]);
    }
    if action == "restore" {
        let dry_run = tool_try!(bool_arg(args, "dry_run", true));
        let remove_new_untracked = tool_try!(bool_arg(args, "remove_new_untracked", false));
        lines[0] = if dry_run {
            "# Workspace checkpoint restore preview".to_string()
        } else {
            "# Workspace checkpoint restored".to_string()
        };
        lines.extend([
            String::new(),
            "## Restore settings".to_string(),
            format!("dry_run: {dry_run}"),
            "tracked_files: restore worktree files to checkpoint revision".to_string(),
            format!("remove_new_untracked: {remove_new_untracked}"),
        ]);
        if dry_run {
            lines.extend([
                String::new(),
                "No files changed. Call again with dry_run=false to restore tracked worktree files."
                    .to_string(),
            ]);
            return ok(
                lines.join("\n"),
                format!("Workspace checkpoint restore preview {checkpoint_id}"),
            );
        }

        let changed_names_args = vec![
            "diff".to_string(),
            "--name-only".to_string(),
            "-z".to_string(),
            checkpoint.revision.clone(),
            "--".to_string(),
            ".".to_string(),
        ];
        let changed_names = tool_try!(run_git_strings(&root, &changed_names_args));
        if !changed_names.status.success() {
            return error_with_display(
                format!(
                    "Workspace checkpoint resource discovery failed:\n{}",
                    command_output_text(&changed_names)
                ),
                "Workspace checkpoint restore was not started".to_string(),
            );
        }
        let mut artifact_paths = Vec::new();
        for raw in changed_names.stdout.split(|byte| *byte == 0) {
            if raw.is_empty() {
                continue;
            }
            let relative = match std::str::from_utf8(raw) {
                Ok(value) => value,
                Err(_) => {
                    return error_with_display(
                        "Workspace checkpoint contains a non-UTF-8 resource path".to_string(),
                        "Workspace checkpoint restore was not started".to_string(),
                    )
                }
            };
            let relative = match file_preimage_relative_path(relative) {
                Ok(path) => path,
                Err(outcome) => return outcome,
            };
            artifact_paths.push(root_canonical.join(relative));
        }
        if remove_new_untracked {
            for relative in &new_untracked {
                let relative = match file_preimage_relative_path(relative) {
                    Ok(path) => path,
                    Err(outcome) => return outcome,
                };
                artifact_paths.push(root_canonical.join(relative));
            }
        }
        artifact_paths.sort();
        artifact_paths.dedup();
        let before = tool_try!(capture_file_artifact_states(&artifact_paths));

        let restore_args = vec![
            "restore".to_string(),
            "--source".to_string(),
            checkpoint.revision.clone(),
            "--worktree".to_string(),
            "--".to_string(),
            ".".to_string(),
        ];
        let restore = tool_try!(run_git_strings(&root, &restore_args));
        if !restore.status.success() {
            return error_with_display(
                format!(
                    "Workspace checkpoint restore failed:\n{}",
                    command_output_text(&restore)
                ),
                "Workspace checkpoint restore failed".to_string(),
            );
        }
        let mut removed_files = Vec::new();
        if remove_new_untracked {
            for rel in &new_untracked {
                let path = root_canonical.join(rel);
                let resolved = path.canonicalize().unwrap_or(path);
                if resolved.starts_with(&root_canonical) && resolved.is_file() {
                    if fs::remove_file(&resolved).is_ok() {
                        removed_files.push(rel.clone());
                    }
                }
            }
        }
        let post_status = tool_try!(run_git(&root, &["status", "--short", "--", "."]));
        let post_status_text = String::from_utf8_lossy(&post_status.stdout)
            .trim()
            .to_string();
        lines.extend([
            String::new(),
            "## Removed new untracked files".to_string(),
            if removed_files.is_empty() {
                "(none)".to_string()
            } else {
                removed_files
                    .iter()
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            String::new(),
            "## Status after restore".to_string(),
            if post_status.status.success() && !post_status_text.is_empty() {
                post_status_text
            } else {
                "(clean for this root)".to_string()
            },
        ]);
        let after = tool_try!(capture_file_artifact_states(&artifact_paths));
        return ok(
            lines.join("\n"),
            format!("Workspace checkpoint restored {checkpoint_id}"),
        )
        .with_file_artifact_changes(&before, &after);
    }
    ok(
        lines.join("\n"),
        format!("Workspace checkpoint diff {checkpoint_id}"),
    )
}

fn workspace_status(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    let root_arg = tool_try!(required_string(args, "root"));
    let root = tool_try!(resolve_allowed(&root_arg, &ctx.allowed_dirs, true));
    if !root.is_dir() {
        return error(format!("Not a directory: {}", root.display()));
    }
    let include_diff = match args.get("include_diff") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Null) | None => false,
        _ => return error("include_diff must be a boolean".to_string()),
    };
    let git_root = tool_try!(run_git(&root, &["rev-parse", "--show-toplevel"]));
    if !git_root.status.success() {
        let entry_count = fs::read_dir(&root)
            .ok()
            .map(|entries| entries.filter_map(Result::ok).count())
            .unwrap_or_default();
        return ok(
            format!(
                "# Workspace status\nroot: {}\nversion_control: none\nentries: {}\n\n\
This workspace is a regular directory, not a Git repository. File and document tools remain available.",
                root.display(),
                entry_count
            ),
            format!("Workspace ready · {entry_count} entries"),
        );
    }
    let branch = tool_try!(run_git(&root, &["branch", "--show-current"]));
    let status = tool_try!(run_git(&root, &["status", "--short", "--", "."]));
    if !status.status.success() {
        return error_with_display(
            format!("Git status failed:\n{}", command_output_text(&status)),
            "Git status failed".to_string(),
        );
    }
    let shortstat = tool_try!(run_git(&root, &["diff", "--shortstat", "--", "."]));
    let diff_stat = tool_try!(run_git(&root, &["diff", "--stat", "--", "."]));
    let status_text = String::from_utf8_lossy(&status.stdout).trim_end().to_string();
    let repository_root = String::from_utf8_lossy(&git_root.stdout).trim().to_string();
    let repository_scope_complete = fs::canonicalize(&repository_root).ok().as_ref() == Some(&root);
    let inventory_root = tool_try!(fs::canonicalize(&repository_root).map_err(|err| error(err.to_string())));
    let change_inventory = tool_try!(git_change_inventory(&inventory_root, &root).map_err(error));
    let changed_count = change_inventory["total_entries"].as_u64().expect("inventory count");
    let untracked_count = change_inventory["untracked_entries"].as_u64().expect("inventory count");
    let mut lines = vec![
        "# Workspace status".to_string(),
        format!("changed_entries: {changed_count} (individual Git status paths; includes untracked files)"),
        format!("tracked_changed_entries: {}", changed_count - untracked_count),
        format!("untracked_entries: {untracked_count} (included in changed_entries, not additional)"),
        format!("repository_scope_complete: {repository_scope_complete}"),
        if repository_scope_complete {
            "scope: Git repository root".to_string()
        } else {
            format!("scope: subdirectory only; not a repository-wide result. Git root is {repository_root}. For a repository-wide task, request access if needed and inspect that explicit root; do not conclude the repository is clean from this result.")
        },
        format!("root: {}", root.display()),
        format!(
            "git_root: {}",
            String::from_utf8_lossy(&git_root.stdout).trim()
        ),
        format!("branch: {}", {
            let value = String::from_utf8_lossy(&branch.stdout).trim().to_string();
            if value.is_empty() {
                "(detached or unknown)".to_string()
            } else {
                value
            }
        }),
        String::new(),
        "## Changed files".to_string(),
        if status_text.is_empty() {
            "(clean for this root)".to_string()
        } else {
            status_text.clone()
        },
        String::new(),
        "## Diff summary".to_string(),
        {
            let value = String::from_utf8_lossy(&shortstat.stdout)
                .trim()
                .to_string();
            if value.is_empty() {
                "(no unstaged diff for this root)".to_string()
            } else {
                value
            }
        },
    ];
    let stat_text = String::from_utf8_lossy(&diff_stat.stdout)
        .trim_end()
        .to_string();
    if !stat_text.is_empty() {
        lines.extend([String::new(), "## Diff stat".to_string(), stat_text]);
    }
    let mut diff_outputs = Map::new();
    if include_diff {
        // Keep source bodies separate from status metadata: headings in source
        // are not section boundaries, and a large diff must not bury its index.
        let mut outputs = Map::new();
        for (kind, command) in [
            ("unstaged", vec!["diff", "--", "."]),
            ("staged", vec!["diff", "--cached", "--", "."]),
        ] {
            let diff = tool_try!(run_git(&root, &command));
            if !diff.status.success() {
                return error_with_display(
                    format!("Git {kind} diff failed:\n{}", command_output_text(&diff)),
                    "Git diff failed".to_string(),
                );
            }
            let text = String::from_utf8_lossy(&diff.stdout);
            let path = tool_try!(ctx.output_store.persist(&text).map_err(|err| error(err.to_string())));
            outputs.insert(kind.to_string(), json!({
                "path": path.display().to_string(),
                "size_bytes": text.len(),
            }));
        }
        lines.insert(1, format!("diff_outputs: {}", Value::Object(outputs.clone())));
        diff_outputs = outputs;
        lines.insert(2, "Read/search these complete staged and unstaged outputs with tool_output_read/tool_output_search. Untracked file contents are not in Git diffs. These references establish availability, not semantic audit coverage.".to_string());
    }
    let mut outcome = ok(
        lines.join("\n"),
        format!(
            "Workspace status ({changed_count} changed file{})",
            if changed_count == 1 { "" } else { "s" }
        ),
    );
    outcome.data = Some(json!({"change_inventory":change_inventory,"diff_outputs":diff_outputs}));
    if !diff_outputs.is_empty() {
        // The producer of saved references also exposes their readers. This
        // is typed Host activation, not routing based on presentation text.
        outcome.activation = Some(nola_core::agent::host::HostCapabilityActivation {
            source: "full_output_ref".into(),
            capability_ids: vec!["tool_output_read".into(), "tool_output_search".into()],
            capability_descriptors: Vec::new(),
        });
    }
    outcome
}
