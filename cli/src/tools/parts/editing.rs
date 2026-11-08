fn suggest_project_commands(root: &Path, manifests: &[PathBuf]) -> Vec<String> {
    let mut commands = Vec::new();
    for package_json in manifests
        .iter()
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("package.json"))
    {
        if let Ok(text) = fs::read_to_string(package_json) {
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                if let Some(scripts) = value.get("scripts").and_then(Value::as_object) {
                    for script in ["test", "typecheck", "lint", "build"] {
                        if scripts.contains_key(script) {
                            let dir = package_json.parent().unwrap_or(root);
                            let rel = if dir == root {
                                ".".to_string()
                            } else {
                                relative_display(dir, root)
                            };
                            commands.push(format!("cd {rel} && npm run {script}"));
                        }
                    }
                }
            }
        }
    }
    let names = manifests
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect::<Vec<_>>();
    if names.contains(&"Cargo.toml") {
        commands.push("cargo test".to_string());
    }
    if names.iter().any(|name| {
        matches!(
            *name,
            "pyproject.toml" | "pytest.ini" | "tox.ini" | "requirements.txt"
        )
    }) {
        commands.push(format!("{} -m pytest", preferred_python_command()));
    }
    if names.contains(&"go.mod") {
        commands.push("go test ./...".to_string());
    }
    if names.iter().any(|name| {
        matches!(
            *name,
            "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts"
        )
    }) {
        commands.push("./gradlew test".to_string());
    }
    if names.contains(&"Package.swift") {
        commands.push("swift test".to_string());
    }
    commands.extend(task_file_commands(root));
    let mut deduped = Vec::new();
    for command in commands {
        if !deduped.contains(&command) {
            deduped.push(command);
        }
    }
    deduped
}

fn test_command_for_file(root: &Path, path: &Path) -> Option<String> {
    let rel = path
        .strip_prefix(root)
        .map(path_slash)
        .unwrap_or_else(|_| path.display().to_string());
    match path.extension().and_then(|value| value.to_str()) {
        Some("py") => Some(format!("{} -m pytest {rel}", preferred_python_command())),
        Some("ts" | "tsx" | "js" | "jsx") => Some(format!("npm test -- {rel}")),
        Some("rs") => Some("cargo test".to_string()),
        Some("kt" | "java") => Some("./gradlew test".to_string()),
        Some("swift") => Some("xcodebuild test".to_string()),
        _ => None,
    }
}

fn validation_plan_items(
    root: &Path,
    paths: &[PathBuf],
    include_project_commands: bool,
) -> (Vec<String>, Vec<String>, Vec<PathBuf>, Vec<PathBuf>) {
    let (manifests, _languages, _docs, _scanned, _truncated) = scan_project(root, 5000);
    let target_paths = if paths.is_empty() {
        git_changed_files(root)
    } else {
        paths.to_vec()
    };
    let mut commands = Vec::new();
    let mut related_tests = Vec::new();
    for path in &target_paths {
        if is_probable_test_file(path) {
            if let Some(command) = test_command_for_file(root, path) {
                if !commands.contains(&command) {
                    commands.push(command);
                }
            }
            continue;
        }
        if path.is_file() {
            let (tests, _same_name, _references, _scanned) = scan_related_files(path, root, 50);
            for test in tests {
                if !related_tests.contains(&test) {
                    related_tests.push(test.clone());
                }
                if let Some(command) = test_command_for_file(root, &test) {
                    if !commands.contains(&command) {
                        commands.push(command);
                    }
                }
            }
        }
    }
    if include_project_commands {
        for command in suggest_project_commands(root, &manifests) {
            if !commands.contains(&command) {
                commands.push(command);
            }
        }
    }
    let diagnostics = vec![format!(
        "code_diagnostics root={}{}",
        root.display(),
        if target_paths.is_empty() {
            String::new()
        } else {
            format!(
                " paths={}",
                target_paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    )];
    (diagnostics, commands, related_tests, target_paths)
}

fn resolve_project_file(
    root: &Path,
    raw: &str,
    allowed_dirs: &[PathBuf],
) -> Result<PathBuf, ToolOutcome> {
    let candidate = {
        let path = Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    };
    let resolved = resolve_allowed(&candidate.display().to_string(), allowed_dirs, true)?;
    if !resolved.is_file() {
        return Err(error(format!("Not a file: {}", resolved.display())));
    }
    if !resolved.starts_with(root) {
        return Err(error(format!(
            "path is outside test_context root: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn resolve_project_seed_path(
    root: &Path,
    raw: &str,
    allowed_dirs: &[PathBuf],
) -> Result<PathBuf, ToolOutcome> {
    let candidate = {
        let path = Path::new(raw);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    };
    let resolved = resolve_allowed(&candidate.display().to_string(), allowed_dirs, true)?;
    if !resolved.is_file() && !resolved.is_dir() {
        return Err(error(format!(
            "Not a file or directory: {}",
            resolved.display()
        )));
    }
    if !resolved.starts_with(root) {
        return Err(error(format!(
            "path is outside project root: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn test_framework_signals(root: &Path, manifests: &[PathBuf]) -> Vec<String> {
    let names = manifests
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect::<Vec<_>>();
    let mut signals = Vec::new();
    if names
        .iter()
        .any(|name| matches!(*name, "pyproject.toml" | "pytest.ini" | "tox.ini"))
    {
        signals.push("pytest via project config".to_string());
    } else if manifests.iter().any(|path| {
        path.file_name().and_then(|name| name.to_str()) == Some("requirements.txt")
            && fs::read_to_string(path)
                .map(|text| text.to_lowercase().contains("pytest"))
                .unwrap_or(false)
    }) {
        signals.push("pytest via requirements.txt".to_string());
    }
    for package_json in manifests
        .iter()
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("package.json"))
    {
        let scripts = package_scripts(package_json);
        let dir = package_json.parent().unwrap_or(root);
        let rel = if dir == root {
            ".".to_string()
        } else {
            relative_display(dir, root)
        };
        for script in scripts.into_iter().filter(|script| {
            matches!(
                script.as_str(),
                "test" | "vitest" | "jest" | "mocha" | "playwright"
            )
        }) {
            signals.push(format!("npm script `{script}` in {rel}/package.json"));
        }
    }
    if names.contains(&"Cargo.toml") {
        signals.push("Rust cargo test via Cargo.toml".to_string());
    }
    if names.contains(&"go.mod") {
        signals.push("Go tests via go.mod".to_string());
    }
    if names.iter().any(|name| {
        matches!(
            *name,
            "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts"
        )
    }) {
        signals.push("Gradle tests via build.gradle/settings.gradle".to_string());
    }
    if names.contains(&"Package.swift") {
        signals.push("Swift Package tests via Package.swift".to_string());
    }
    signals
}

fn test_import_lines(text: &str, max_imports: usize) -> Vec<String> {
    let mut imports = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let is_import = trimmed.starts_with("import ")
            || trimmed.starts_with("from ")
            || trimmed.starts_with("use ")
            || trimmed.starts_with("mod ")
            || trimmed.starts_with("const ") && trimmed.contains("require(")
            || trimmed.starts_with("@Test")
            || trimmed.starts_with("@ParameterizedTest");
        if is_import {
            imports.push(outline_signature(trimmed));
        }
        if imports.len() >= max_imports {
            break;
        }
    }
    imports
}

fn test_sample_sections(
    root: &Path,
    tests: &[PathBuf],
    max_tests: usize,
    max_symbols: usize,
) -> Result<Vec<String>, ToolOutcome> {
    let mut sections = Vec::new();
    for test in tests.iter().take(max_tests) {
        let Ok(text) = fs::read_to_string(test) else {
            continue;
        };
        let mut lines = vec![format!("### {}", relative_display(test, root))];
        if let Some(command) = test_command_for_file(root, test) {
            lines.push(format!("command: {command}"));
        }
        let symbols = extract_outline(&text, max_symbols)?;
        if !symbols.is_empty() {
            lines.push("symbols:".to_string());
            lines.extend(symbols.into_iter().map(|symbol| format!("- {symbol}")));
        }
        let imports = test_import_lines(&text, 8);
        if !imports.is_empty() {
            lines.push("imports:".to_string());
            lines.extend(imports.into_iter().map(|line| format!("- {line}")));
        }
        sections.push(lines.join("\n"));
    }
    Ok(sections)
}

fn test_context_items(
    root: &Path,
    paths: &[PathBuf],
    max_tests: usize,
) -> (Vec<String>, Vec<PathBuf>, Vec<PathBuf>, Vec<String>, usize) {
    let (manifests, _languages, _docs, _scanned, _truncated) = scan_project(root, 5000);
    let target_paths = if paths.is_empty() {
        git_changed_files(root)
            .into_iter()
            .filter(|path| path.is_file())
            .collect::<Vec<_>>()
    } else {
        paths.to_vec()
    };
    let mut related_tests = Vec::new();
    let mut scanned_total = 0usize;
    for path in &target_paths {
        if is_probable_test_file(path) {
            if !related_tests.contains(path) {
                related_tests.push(path.clone());
            }
            continue;
        }
        let (tests, _same_name, _references, scanned) =
            scan_related_files(path, root, 50.max(max_tests * 4));
        scanned_total += scanned;
        for test in tests {
            if !related_tests.contains(&test) {
                related_tests.push(test);
            }
        }
    }
    let mut commands = Vec::new();
    for test in &related_tests {
        if let Some(command) = test_command_for_file(root, test) {
            if !commands.contains(&command) {
                commands.push(command);
            }
        }
    }
    for command in suggest_project_commands(root, &manifests) {
        if !commands.contains(&command) {
            commands.push(command);
        }
    }
    (
        test_framework_signals(root, &manifests),
        target_paths,
        related_tests,
        commands,
        scanned_total,
    )
}

fn package_scripts(package_json: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(package_json) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    value
        .get("scripts")
        .and_then(Value::as_object)
        .map(|scripts| scripts.keys().cloned().collect())
        .unwrap_or_default()
}

fn has_file_name(paths: &[PathBuf], expected: &str) -> bool {
    paths.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == expected)
    })
}

fn has_prettier_config(paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".prettier") || name == "prettier.config.js")
    })
}

fn pyproject_contains(manifests: &[PathBuf], needle: &str) -> bool {
    manifests.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "pyproject.toml")
            && fs::read_to_string(path)
                .map(|text| text.to_lowercase().contains(needle))
                .unwrap_or(false)
    })
}

fn join_targets_for_ext(root: &Path, targets: &[PathBuf], extensions: &[&str]) -> String {
    targets
        .iter()
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| extensions.contains(&ext))
        })
        .map(|path| relative_display(path, root))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_plan_items(root: &Path, paths: &[PathBuf]) -> (Vec<String>, Vec<String>, Vec<PathBuf>) {
    let (manifests, _languages, _docs, _scanned, _truncated) = scan_project(root, 5000);
    let (configs, _config_scanned) = scan_convention_config_files(root, 40);
    let mut targets = paths
        .iter()
        .filter(|path| path.is_file() && is_code_source(path))
        .cloned()
        .collect::<Vec<_>>();
    if targets.is_empty() {
        targets = git_changed_files(root)
            .into_iter()
            .filter(|path| path.is_file() && is_code_source(path))
            .collect();
    }

    let mut commands = Vec::new();
    let mut notes = Vec::new();

    for package_json in manifests
        .iter()
        .filter(|path| path.file_name().and_then(|name| name.to_str()) == Some("package.json"))
    {
        let scripts = package_scripts(package_json);
        let dir = package_json.parent().unwrap_or(root);
        let rel = if dir == root {
            ".".to_string()
        } else {
            relative_display(dir, root)
        };
        let prefix = if rel == "." {
            String::new()
        } else {
            format!("cd {rel} && ")
        };
        if scripts.iter().any(|script| script == "format") {
            commands.push(format!("{prefix}npm run format"));
        }
        if scripts.iter().any(|script| script == "lint") {
            commands.push(format!("{prefix}npm run lint"));
        }
    }

    let web_join = join_targets_for_ext(root, &targets, &["js", "jsx", "ts", "tsx", "css", "html"]);
    if !web_join.is_empty() && has_prettier_config(&configs) {
        commands.push(format!("npx prettier --write {web_join}"));
    }

    let py_join = join_targets_for_ext(root, &targets, &["py"]);
    if !py_join.is_empty() {
        let python = preferred_python_command();
        if has_file_name(&configs, "ruff.toml") || pyproject_contains(&manifests, "ruff") {
            commands.push(format!("{python} -m ruff format {py_join}"));
            commands.push(format!("{python} -m ruff check {py_join}"));
        } else if pyproject_contains(&manifests, "black") {
            commands.push(format!("{python} -m black {py_join}"));
        } else {
            notes.push(
                "No configured Python formatter detected; skipping automatic formatting."
                    .to_string(),
            );
        }
    }

    if !join_targets_for_ext(root, &targets, &["rs"]).is_empty() {
        if has_file_name(&manifests, "Cargo.toml") {
            commands.push("cargo fmt".to_string());
        } else {
            notes.push(
                "No Cargo.toml found for Rust targets; skipping project formatting."
                    .to_string(),
            );
        }
    }
    let go_join = join_targets_for_ext(root, &targets, &["go"]);
    if !go_join.is_empty() {
        commands.push(format!("gofmt -w {go_join}"));
    }
    if !join_targets_for_ext(root, &targets, &["swift"]).is_empty() {
        if has_file_name(&configs, ".swiftformat") {
            commands.push("swiftformat .".to_string());
        }
        if has_file_name(&configs, ".swiftlint.yml") {
            commands.push("swiftlint".to_string());
        }
    }
    if !join_targets_for_ext(root, &targets, &["kt", "java"]).is_empty() {
        if has_file_name(&configs, ".ktlint.yml") {
            commands.push("ktlint -F".to_string());
        }
        if has_file_name(&manifests, "build.gradle")
            || has_file_name(&manifests, "build.gradle.kts")
        {
            commands.push("./gradlew ktlintFormat".to_string());
        }
    }

    if commands.is_empty() {
        notes.push("No formatter command inferred from targets and project config.".to_string());
    }

    let mut deduped = Vec::new();
    for command in commands {
        if !deduped.contains(&command) {
            deduped.push(command);
        }
    }
    (deduped, notes, targets)
}

fn format_command_is_formatter(command: &str) -> bool {
    let lowered = command.to_lowercase();
    if lowered.contains("lint")
        || lowered.contains("check")
        || lowered.contains("test")
        || lowered.contains("pytest")
    {
        return false;
    }
    [
        "cargo fmt",
        "gofmt -w",
        "swiftformat",
        "ktlint -f",
        "ktlintformat",
        "npx prettier --write",
        "-m ruff format",
        "-m black",
        "npm run format",
    ]
    .iter()
    .any(|token| lowered.contains(token))
}

fn format_root_for_path(path: &Path, allowed_dirs: &[PathBuf]) -> Option<PathBuf> {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut best: Option<PathBuf> = None;
    for allowed in allowed_dirs {
        let base = allowed.canonicalize().unwrap_or_else(|_| allowed.clone());
        if !resolved.starts_with(&base) {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|current| base.components().count() > current.components().count())
        {
            best = Some(base);
        }
    }
    best
}

fn formatting_sections(
    root: &Path,
    paths: &[PathBuf],
    cancellation: &RuntimeExecutionControl,
    process_sandbox: Option<&crate::process_sandbox::FrozenProcessSandbox>,
) -> Result<Vec<String>, ToolOutcome> {
    let code_paths = paths
        .iter()
        .filter(|path| path.is_file() && is_code_source(path))
        .cloned()
        .collect::<Vec<_>>();
    if code_paths.is_empty() {
        return Ok(Vec::new());
    }
    let (commands, _notes, _targets) = format_plan_items(root, &code_paths);
    let formatter_commands = commands
        .into_iter()
        .filter(|command| format_command_is_formatter(command))
        .collect::<Vec<_>>();
    if formatter_commands.is_empty() {
        return Ok(Vec::new());
    }
    let mut sections = vec![String::new(), "## Formatting".to_string()];
    for command in formatter_commands {
        let output = run_shell_command(root, &command, cancellation,process_sandbox)?;
        if !output.status.success() {
            let detail = command_output_text(&output);
            return Err(error(format!("{command}\n{detail}")));
        }
        let mut snippet = successful_command_output_text(&output.stdout, &output.stderr);
        if snippet.len() > 2000 {
            snippet.truncate(2000);
            snippet = snippet.trim_end().to_string() + "\n...[output truncated]";
        }
        if snippet.trim().is_empty() {
            sections.push(format!("- {command}: ok"));
        } else {
            sections.push(format!("- {command}: ok\n{}", snippet.trim()));
        }
    }
    Ok(sections)
}

fn successful_command_output_text(stdout: &[u8], stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    String::from_utf8_lossy(stdout).trim().to_string()
}

fn commit_formatted_file_edit(
    target: &Path,
    original: &str,
    updated: &str,
    allowed_dirs: &[PathBuf],
    format_after_edit: bool,
    cancellation: &RuntimeExecutionControl,
    process_sandbox: Option<&crate::process_sandbox::FrozenProcessSandbox>,
) -> Result<(Vec<String>, String), ToolOutcome> {
    let current = fs::read_to_string(target)
        .map_err(|err| error(format!("Read before commit failed: {err}")))?;
    if current != original {
        return Err(error(format!(
            "File changed during edit preflight. Re-read the exact target and retry: {}",
            target.display()
        )));
    }
    atomic_write_text_file(target, updated)
        .map_err(|err| error(format!("Atomic write failed: {err}")))?;
    refresh_file_access_state(target);

    let format_sections = if format_after_edit {
        let Some(format_root) = format_root_for_path(target, allowed_dirs) else {
            return Ok((Vec::new(), updated.to_string()));
        };
        match formatting_sections(
            &format_root,
            std::slice::from_ref(&target.to_path_buf()),
            cancellation,
            process_sandbox,
        ) {
            Ok(sections) => sections,
            Err(mut outcome) => {
                match atomic_write_text_file(target, original) {
                    Ok(()) => {
                        refresh_file_access_state(target);
                        outcome.text.push_str(
                            "\nedit_rolled_back: true\nfile_state: restored_to_pre_edit_content",
                        );
                    }
                    Err(err) => {
                        outcome
                            .text
                            .push_str(&format!("\nedit_rolled_back: false\nrollback_error: {err}"));
                    }
                }
                return Err(outcome);
            }
        }
    } else {
        vec![
            String::new(),
            "## Formatting".to_string(),
            "- deferred until the final edit batch or explicit format/validation step".to_string(),
        ]
    };
    let committed = match fs::read_to_string(target) {
        Ok(content) => content,
        Err(err) => {
            let rollback = atomic_write_text_file(target, original);
            refresh_file_access_state(target);
            return Err(error(match rollback {
                Ok(()) => format!(
                    "Read after formatting failed: {err}\nedit_rolled_back: true\nfile_state: restored_to_pre_edit_content"
                ),
                Err(rollback_err) => format!(
                    "Read after formatting failed: {err}\nedit_rolled_back: false\nrollback_error: {rollback_err}"
                ),
            }));
        }
    };
    refresh_file_access_state(target);
    Ok((format_sections, committed))
}

fn atomic_write_text_file(target: &Path, content: &str) -> std::io::Result<()> {
    atomic_write_text_file_mode(target, content, true)
}

fn atomic_write_text_file_mode(target: &Path, content: &str, overwrite: bool) -> std::io::Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("nola-edit");
    let permissions = fs::metadata(target)
        .ok()
        .map(|metadata| metadata.permissions());
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut last_error = None;

    for attempt in 0..8_u8 {
        let temporary = parent.join(format!(
            ".{file_name}.nola-tmp-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                last_error = Some(err);
                continue;
            }
            Err(err) => return Err(err),
        };
        if let Err(err) =
            std::io::Write::write_all(&mut file, content.as_bytes()).and_then(|_| file.sync_all())
        {
            let _ = fs::remove_file(&temporary);
            return Err(err);
        }
        drop(file);
        if let Some(permissions) = permissions.clone() {
            if let Err(err) = fs::set_permissions(&temporary, permissions) {
                let _ = fs::remove_file(&temporary);
                return Err(err);
            }
        }
        if let Err(err) = atomic_move_file(&temporary, target, overwrite) {
            let _ = fs::remove_file(&temporary);
            return Err(err);
        }
        return Ok(());
    }

    Err(last_error.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not reserve an atomic edit file",
        )
    }))
}

#[cfg(test)]
mod edit_transaction_output_tests {
    use super::*;

    #[test]
    fn repeated_write_is_an_observation_not_a_new_mutation() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(), allowed_dirs: vec![root.to_path_buf()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".into(), Vec::new()),
            permission_scope: Default::default(), cancellation: Default::default(),
            plan_state: Default::default(), output_store: ToolOutputStore::new(root.join("outputs")),
            browser: BrowserRuntime::new(root.join("browser")),
        };
        let target = root.join("artifact.txt");
        let args = json!({"path": target, "content": "same"});
        let created = write_file(args.as_object().unwrap(), &ctx);
        assert!(!created.error, "{}", created.text);
        let effect = &created.execution.as_ref().unwrap().effects[0];
        assert_eq!(effect.before_version.as_deref(), Some("absent:v1"));
        let version = effect.after_version.clone();
        let modified = fs::metadata(&target).unwrap().modified().unwrap();
        let repeated = write_file(args.as_object().unwrap(), &ctx);
        assert!(!repeated.error, "{}", repeated.text);
        assert_eq!(repeated.data.as_ref().unwrap()["changed"], false);
        let unchanged = &repeated.execution.as_ref().unwrap().effects[0];
        assert_eq!(unchanged.before_version, unchanged.after_version);
        assert!(repeated.artifacts.is_empty());
        assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), modified);
        let changed = write_file(json!({"path": target, "content": "different", "expected_sha256":sha256_hex(b"same")}).as_object().unwrap(), &ctx);
        assert!(!changed.error, "{}", changed.text);
        let effect = &changed.execution.as_ref().unwrap().effects[0];
        assert_eq!(effect.before_version, version);
        assert_ne!(effect.before_version, effect.after_version);
    }

    #[test]
    fn append_preserves_unread_binary_preimage_and_rejects_wrong_expected_version() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let target = root.join("unread.bin");
        let original = [0xff, 0x00, b'\n', 0xfe];
        fs::write(&target, original).unwrap();
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(),
            allowed_dirs: vec![root.to_path_buf()],
            policy: crate::tools::ToolExecutionPolicy::new(
                Vec::new(), true, "allow".into(), Vec::new(),
            ),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: ToolOutputStore::new(root.join("outputs")),
            browser: BrowserRuntime::new(root.join("browser")),
        };
        let digest = file_sha256_hex(&target).unwrap();
        let appended = append_file(json!({
            "path": target, "content": "appended\n",
        }).as_object().unwrap(), &ctx);
        assert!(!appended.error, "{}", appended.text);
        let expected = [original.as_slice(), b"appended\n"].concat();
        assert_eq!(fs::read(&target).unwrap(), expected);
        let receipt = appended.execution.as_ref().expect("actual append receipt");
        assert_eq!(receipt.state, nola_host_protocol::tool_result::ExecutionState::Completed);
        assert_eq!(receipt.effects.len(), 1);
        assert_ne!(receipt.effects[0].before_version, receipt.effects[0].after_version);

        // Neither the successful append nor the current content is a model
        // read. An explicit stale version still refuses without another byte.
        let refused = append_file(json!({
            "path": target, "content": "must-not-appear\n", "expected_sha256": digest,
        }).as_object().unwrap(), &ctx);
        assert!(refused.error, "{}", refused.text);
        assert_eq!(fs::read(&target).unwrap(), expected);
        assert!(refused.execution.as_ref().is_none_or(|receipt| receipt.effects.is_empty()));
    }

    #[test]
    fn mutation_capacity_uses_final_utf8_bytes_for_text_json_and_append() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(), allowed_dirs: vec![root.to_path_buf()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".into(), Vec::new()),
            permission_scope: Default::default(), cancellation: Default::default(),
            plan_state: Default::default(), output_store: ToolOutputStore::new(root.join("outputs")),
            browser: BrowserRuntime::new(root.join("browser")),
        };
        for (index, unit) in ["x", "中", "🙂"].iter().enumerate() {
            let exact = format!("{}{}", unit.repeat(MAX_WRITE_BYTES / unit.len()), "x".repeat(MAX_WRITE_BYTES % unit.len()));
            assert_eq!(exact.len(), MAX_WRITE_BYTES);
            let target = root.join(format!("text-{index}.txt"));
            let arguments = json!({"path":target,"content":exact});
            let accepted = write_file(arguments.as_object().unwrap(), &ctx);
            assert!(!accepted.error, "{}", accepted.text);
            assert_eq!(fs::read(&target).unwrap(), exact.as_bytes());
            let oversized = json!({"path":target,"content":format!("{exact}x")});
            let rejected = write_file(oversized.as_object().unwrap(), &ctx);
            assert_eq!(rejected.data.as_ref().unwrap()["actual_utf8_bytes"], MAX_WRITE_BYTES + 1);
            assert_eq!(rejected.data.as_ref().unwrap()["tool_execution_skipped"], true);
            assert_eq!(fs::read(&target).unwrap(), exact.as_bytes());
            let missing = root.join(format!("missing-{index}/file.txt"));
            let rejected = write_file(json!({"path":missing,"content":format!("{exact}x")}).as_object().unwrap(), &ctx);
            assert!(rejected.error);
            assert!(!missing.parent().unwrap().exists());

            let appended = root.join(format!("append-{index}.txt"));
            let result = append_file(json!({"path":appended,"content":exact,"create":true}).as_object().unwrap(), &ctx);
            assert!(!result.error, "{}", result.text);
            let result = append_file(json!({"path":appended,"content":format!("{exact}x")}).as_object().unwrap(), &ctx);
            assert_eq!(result.data.as_ref().unwrap()["deterministic_failure_category"], "append_chunk_too_large");
            assert_eq!(fs::read(&appended).unwrap(), exact.as_bytes());

            let overhead = serde_json::to_string_pretty(&json!({"payload":""})).unwrap().len() + 1;
            let available = MAX_WRITE_BYTES - overhead;
            let value = format!("{}{}", unit.repeat(available / unit.len()), "x".repeat(available % unit.len()));
            let object = json!({"payload":value});
            let serialized = format!("{}\n", serde_json::to_string_pretty(&object).unwrap());
            assert_eq!(serialized.len(), MAX_WRITE_BYTES);
            let json_target = root.join(format!("json-{index}.txt"));
            let result = write_file(json!({"path":json_target,"content":serialized}).as_object().unwrap(), &ctx);
            assert!(!result.error, "{}", result.text);
            assert_eq!(fs::read(&json_target).unwrap(), serialized.as_bytes());
            let result = write_file(json!({"path":json_target,"content":format!("{serialized}x")}).as_object().unwrap(), &ctx);
            assert_eq!(result.data.as_ref().unwrap()["actual_utf8_bytes"], MAX_WRITE_BYTES + 1);
            assert_eq!(fs::read(&json_target).unwrap(), serialized.as_bytes());
        }
        // Escape expansion and pretty-format overhead are part of the same
        // capacity, not just the lengths of the object's string values.
        let escaped = json!({"path":root.join("escaped/file.txt"),"content":serde_json::to_string_pretty(&json!({"payload":"\"".repeat(MAX_WRITE_BYTES / 2)})).unwrap()});
        let rejected = write_file(escaped.as_object().unwrap(), &ctx);
        assert!(rejected.error);
        assert!(!root.join("escaped").exists());
    }

    #[cfg(unix)]
    #[test]
    fn complete_file_replacement_preserves_mode_and_rejects_outside_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let fixture = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = fixture.path().join("replace.txt");
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).unwrap();
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(), allowed_dirs: vec![fixture.path().to_path_buf()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".into(), Vec::new()),
            permission_scope: Default::default(), cancellation: Default::default(),
            plan_state: Default::default(), output_store: ToolOutputStore::new(fixture.path().join("outputs")),
            browser: BrowserRuntime::new(fixture.path().join("browser")),
        };
        assert!(!read_file(json!({"path":target}).as_object().unwrap(), &ctx).error);
        let replacement = "new".repeat(20_000);
        let result = write_file(json!({"path":target,"content":replacement,"expected_sha256":sha256_hex(b"old")}).as_object().unwrap(), &ctx);
        assert!(!result.error, "{}", result.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), replacement);
        assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o750);
        let private = outside.path().join("private.txt");
        fs::write(&private, "private").unwrap();
        let link = fixture.path().join("escape.txt");
        symlink(&private, &link).unwrap();
        assert!(write_file(json!({"path":link,"content":"forbidden"}).as_object().unwrap(), &ctx).error);
        assert_eq!(fs::read_to_string(private).unwrap(), "private");
        ctx.cancellation.request();
        let cancelled_target = fixture.path().join("cancelled/new.txt");
        assert!(write_file(json!({"path":cancelled_target,"content":"cancelled"}).as_object().unwrap(), &ctx).error);
        assert!(!cancelled_target.parent().unwrap().exists());
    }

    #[test]
    fn successful_empty_formatter_output_stays_empty() {
        assert_eq!(successful_command_output_text(b"", b""), "");
    }

    #[test]
    fn successful_formatter_output_prefers_stderr() {
        assert_eq!(
            successful_command_output_text(b"stdout", b"warning on stderr"),
            "warning on stderr"
        );
    }

    #[test]
    fn append_file_builds_a_large_new_file_in_ordered_chunks() {
        let directory = std::env::temp_dir().join(format!(
            "nola-append-file-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("generated.txt");
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(),
            allowed_dirs: vec![directory.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(directory.join("browser-runtime")),
        };

        let mut first = Map::new();
        first.insert(
            "path".to_string(),
            Value::String(target.display().to_string()),
        );
        first.insert("content".to_string(), Value::String("first\n".to_string()));
        first.insert("create".to_string(), Value::Bool(true));
        let first_outcome = append_file(&first, &ctx);
        assert!(!first_outcome.error, "{}", first_outcome.text);

        let mut second = Map::new();
        second.insert(
            "path".to_string(),
            Value::String(target.display().to_string()),
        );
        second.insert("content".to_string(), Value::String("second\n".to_string()));
        let second_outcome = append_file(&second, &ctx);
        assert!(!second_outcome.error, "{}", second_outcome.text);
        assert_eq!(fs::read_to_string(&target).unwrap(), "first\nsecond\n");

        let mut existing_with_create = Map::new();
        existing_with_create.insert(
            "path".to_string(),
            Value::String(target.display().to_string()),
        );
        existing_with_create.insert("content".to_string(), Value::String("third\n".to_string()));
        existing_with_create.insert("create".to_string(), Value::Bool(true));
        let existing_outcome = append_file(&existing_with_create, &ctx);
        assert!(!existing_outcome.error, "{}", existing_outcome.text);
        assert!(existing_outcome.text.contains("never truncated"));
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "first\nsecond\nthird\n"
        );

        let ordinary_source_target = directory.join("ordinary-source.txt");
        let mut ordinary_source = Map::new();
        ordinary_source.insert(
            "path".to_string(),
            Value::String(ordinary_source_target.display().to_string()),
        );
        ordinary_source.insert("content".to_string(), Value::String("x".repeat(8_000)));
        let ordinary_source_outcome = write_file(&ordinary_source, &ctx);
        assert!(!ordinary_source_outcome.error, "{}", ordinary_source_outcome.text);
        assert_eq!(fs::metadata(&ordinary_source_target).unwrap().len(), 8_000);

        let oversized_write_target = directory.join("oversized-write.txt");
        let mut oversized_write = Map::new();
        oversized_write.insert(
            "path".to_string(),
            Value::String(oversized_write_target.display().to_string()),
        );
        oversized_write.insert(
            "content".to_string(),
            Value::String("x".repeat(MAX_WRITE_BYTES + 1)),
        );
        let oversized_write_outcome = write_file(&oversized_write, &ctx);
        assert!(oversized_write_outcome.error);
        assert_eq!(oversized_write_outcome.data.as_ref().unwrap()["deterministic_failure_category"], "write_payload_too_large");
        assert!(!oversized_write_outcome.text.contains("call_append_file"));
        assert!(!oversized_write_target.exists());

        let mut oversized = second;
        oversized.insert(
            "content".to_string(),
            Value::String("x".repeat(MAX_WRITE_BYTES + 1)),
        );
        let oversized_outcome = append_file(&oversized, &ctx);
        assert!(oversized_outcome.error);
        assert!(oversized_outcome
            .text
            .contains(&MAX_WRITE_BYTES.to_string()));
        assert_eq!(oversized_outcome.data.as_ref().unwrap()["tool_execution_skipped"], true);
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "first\nsecond\nthird\n"
        );

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn atomic_edit_supports_existing_files_larger_than_write_payload_limit() {
        let directory = std::env::temp_dir().join(format!(
            "nola-large-edit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("large.txt");
        let original = format!("{}old\n", "x".repeat(MAX_WRITE_BYTES + 32));
        let updated = original.replacen("old", "new", 1);
        fs::write(&target, &original).unwrap();

        let (_, committed) = commit_formatted_file_edit(
            &target,
            &original,
            &updated,
            std::slice::from_ref(&directory),
            false,
            &RuntimeExecutionControl::default(),
            None,
        )
        .unwrap();

        assert!(committed.len() > MAX_WRITE_BYTES);
        assert!(committed.len() < MAX_EDITED_FILE_BYTES);
        assert_eq!(fs::read_to_string(&target).unwrap(), updated);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn edit_file_lines_plans_and_edits_a_large_existing_file_directly() {
        let directory = std::env::temp_dir().join(format!(
            "nola-large-line-edit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("large.txt");
        fs::write(&target, format!("old\n{}", "x\n".repeat(260_000))).unwrap();
        let ctx = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(),
            allowed_dirs: vec![directory.clone()],
            policy: crate::tools::ToolExecutionPolicy::new(Vec::new(), true, "allow".to_string(), Vec::new()),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: BrowserRuntime::new(directory.join("browser-runtime")),
        };
        let mut read_args = Map::new();
        read_args.insert(
            "path".to_string(),
            Value::String(target.display().to_string()),
        );
        read_args.insert("start_line".to_string(), Value::from(1));
        read_args.insert("line_count".to_string(), Value::from(1));
        let read = read_file(&read_args, &ctx);
        assert!(!read.error, "{}", read.text);
        assert!(read.text.contains("large_file=true"));
        assert!(read.text.contains("edit_strategy:"));

        let mut edit_args = Map::new();
        edit_args.insert(
            "path".to_string(),
            Value::String(target.display().to_string()),
        );
        edit_args.insert("start_line".to_string(), Value::from(1));
        edit_args.insert("line_count".to_string(), Value::from(1));
        edit_args.insert("new_text".to_string(), Value::String("new\n".to_string()));
        let edited = edit_file_lines(&edit_args, &ctx);

        assert!(!edited.error, "{}", edited.text);
        let committed = fs::read_to_string(&target).unwrap();
        assert!(committed.len() > MAX_WRITE_BYTES);
        assert!(committed.starts_with("new\n"));
        let _ = fs::remove_dir_all(directory);
    }
}

fn run_shell_command(
    root: &Path,
    command: &str,
    cancellation: &RuntimeExecutionControl,
    process_sandbox: Option<&crate::process_sandbox::FrozenProcessSandbox>,
) -> Result<std::process::Output, ToolOutcome> {
    if process_sandbox.is_some() {
        return Err(error_data(json!({"error":"native_shell_sandbox_requires_supervision","command_not_started":true,
            "message":"This direct formatter entry has no supervised sandbox lifecycle. Run the command through the normal bash tool; the required policy was not dropped."})));
    }
    let mut cmd = Command::new("sh");
    cmd.arg("-lc")
        .arg(command)
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    cmd.process_group(0);
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return Err(error(format!("formatter command failed to start: {err}"))),
    };
    wait_for_child_output(
        child,
        cancellation,
        Duration::from_secs(120),
        "formatter command",
    )
}

fn wait_for_child_output(
    mut child: Child,
    cancellation: &RuntimeExecutionControl,
    timeout: Duration,
    label: &str,
) -> Result<std::process::Output, ToolOutcome> {
    drop(child.stdin.take());
    let stdout_reader = child.stdout.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = std::io::Read::read_to_end(&mut pipe, &mut output);
            output
        })
    });
    let stderr_reader = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut output = Vec::new();
            let _ = std::io::Read::read_to_end(&mut pipe, &mut output);
            output
        })
    });
    let started = Instant::now();
    let status = loop {
        if cancellation.is_requested() {
            let _ = terminate_process_tree(&mut child, Duration::from_secs(2));
            return Err(error_with_display(
                format!("{label} cancelled by user"),
                format!("{label} cancelled"),
            ));
        }
        if started.elapsed() >= timeout {
            let _ = terminate_process_tree(&mut child, Duration::from_secs(2));
            return Err(error_with_display(
                format!("{label} timed out after {}s", timeout.as_secs()),
                format!("{label} timed out"),
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(err) => {
                let _ = terminate_process_tree(&mut child, Duration::from_secs(2));
                return Err(error(format!("{label} wait failed: {err}")));
            }
        }
    };
    let stdout = stdout_reader
        .map(|reader| reader.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr = stderr_reader
        .map(|reader| reader.join().unwrap_or_default())
        .unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn instruction_search_dirs(root: &Path, target: &Path) -> Vec<PathBuf> {
    let current = if target.is_dir() {
        target.to_path_buf()
    } else {
        target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf())
    };
    if !current.starts_with(root) {
        return Vec::new();
    }
    let mut dirs = Vec::new();
    let mut cursor = current;
    loop {
        dirs.push(cursor.clone());
        if cursor == root {
            break;
        }
        let Some(parent) = cursor.parent() else {
            break;
        };
        if parent == cursor {
            break;
        }
        cursor = parent.to_path_buf();
    }
    dirs.reverse();
    dirs
}

fn read_instruction_file(path: &Path, max_bytes: usize) -> std::io::Result<(String, u64, bool)> {
    let mut file = File::open(path)?;
    let byte_count = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut data = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(max_bytes as u64)
        .read_to_end(&mut data)?;
    let truncated = byte_count > max_bytes as u64;
    Ok((
        strip_markdown_frontmatter(&String::from_utf8_lossy(&data)),
        byte_count,
        truncated,
    ))
}

fn read_skill_file(path: &Path, max_bytes: usize) -> std::io::Result<(String, u64, bool)> {
    let mut file = File::open(path)?;
    let byte_count = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut data = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(max_bytes as u64)
        .read_to_end(&mut data)?;
    Ok((
        String::from_utf8_lossy(&data).into_owned(),
        byte_count,
        byte_count > max_bytes as u64,
    ))
}

fn read_text_prefix(path: &Path, max_bytes: usize) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut data = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(max_bytes as u64)
        .read_to_end(&mut data)?;
    Ok(String::from_utf8_lossy(&data).to_string())
}

fn split_frontmatter_paths(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .filter_map(|item| {
            let mut cleaned = item.trim().trim_matches(['\'', '"']).to_string();
            if let Some(rest) = cleaned.strip_prefix('-') {
                cleaned = rest.trim().trim_matches(['\'', '"']).to_string();
            }
            if cleaned.ends_with("/**") {
                cleaned.truncate(cleaned.len().saturating_sub(3));
            }
            if cleaned.is_empty() {
                None
            } else {
                Some(cleaned)
            }
        })
        .collect()
}

fn markdown_frontmatter_body(text: &str) -> Option<(&str, &str)> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let marker = rest.find("\n---\n").or_else(|| rest.find("\r\n---\r\n"))?;
    let body = &rest[..marker];
    let content_start = marker
        + if rest[marker..].starts_with("\r\n---\r\n") {
            7
        } else {
            5
        };
    Some((body, &rest[content_start..]))
}

fn instruction_frontmatter_paths(text: &str) -> Option<Vec<String>> {
    let (body, _) = markdown_frontmatter_body(text)?;
    let lines = body.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        let stripped = line.trim();
        let raw = stripped
            .strip_prefix("paths:")
            .or_else(|| stripped.strip_prefix("globs:"));
        if let Some(raw) = raw {
            let mut value = raw.trim().to_string();
            if value.starts_with('[') && value.ends_with(']') && value.len() >= 2 {
                value = value[1..value.len() - 1].to_string();
            }
            let mut patterns = split_frontmatter_paths(&value);
            for nested in lines.iter().skip(index + 1) {
                let nested_stripped = nested.trim();
                if nested_stripped.starts_with('-') {
                    patterns.extend(split_frontmatter_paths(nested_stripped));
                    continue;
                }
                if (nested.starts_with(' ') || nested.starts_with('\t'))
                    && nested_stripped.is_empty()
                {
                    continue;
                }
                break;
            }
            return Some(patterns);
        }
    }
    None
}

fn strip_markdown_frontmatter(text: &str) -> String {
    markdown_frontmatter_body(text)
        .map(|(_, content)| content.to_string())
        .unwrap_or_else(|| text.to_string())
}

fn markdown_frontmatter_fields(text: &str) -> HashMap<String, String> {
    let Some((body, _)) = markdown_frontmatter_body(text) else {
        return HashMap::new();
    };
    let lines = body.lines().collect::<Vec<_>>();
    let mut fields = HashMap::new();
    let mut index = 0usize;
    while index < lines.len() {
        let stripped = lines[index].trim();
        if stripped.is_empty() || stripped.starts_with('#') || !stripped.contains(':') {
            index += 1;
            continue;
        }
        let Some((key, value)) = stripped.split_once(':') else {
            index += 1;
            continue;
        };
        let key = key.trim().to_string();
        let mut value = value.trim().trim_matches(['\'', '"']).to_string();
        if value.is_empty() {
            let mut nested = Vec::new();
            while index + 1 < lines.len() {
                let nested_line = lines[index + 1];
                let nested_stripped = nested_line.trim();
                if let Some(item) = nested_stripped.strip_prefix('-') {
                    nested.push(item.trim().trim_matches(['\'', '"']).to_string());
                    index += 1;
                    continue;
                }
                if (nested_line.starts_with(' ') || nested_line.starts_with('\t'))
                    && !nested_stripped.is_empty()
                {
                    nested.push(nested_stripped.trim_matches(['\'', '"']).to_string());
                    index += 1;
                    continue;
                }
                break;
            }
            value = nested
                .into_iter()
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
        }
        if !key.is_empty() {
            fields.insert(key, value);
        }
        index += 1;
    }
    fields
}

fn instruction_pattern_matches(base: &Path, target: &Path, patterns: &[String]) -> bool {
    if patterns.is_empty() || patterns.iter().all(|pattern| pattern == "**") {
        return true;
    }
    let Ok(resolved_target) = target.canonicalize() else {
        return false;
    };
    let Ok(resolved_base) = base.canonicalize() else {
        return false;
    };
    let Ok(relative) = resolved_target.strip_prefix(&resolved_base) else {
        return false;
    };
    let relative = path_slash(relative);
    let candidates = if resolved_target.is_dir() {
        vec![relative.clone(), format!("{relative}/")]
    } else {
        vec![relative]
    };
    patterns.iter().any(|pattern| {
        let normalized = pattern.strip_prefix("./").unwrap_or(pattern);
        glob_regex(normalized)
            .map(|regex| candidates.iter().any(|candidate| regex.is_match(candidate)))
            .unwrap_or(false)
    })
}

fn instruction_rule_base(path: &Path) -> PathBuf {
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir.file_name().and_then(|name| name.to_str()) == Some("rules") {
            if let Some(parent) = dir.parent() {
                let marker = parent.file_name().and_then(|name| name.to_str());
                if marker == Some(".nola") {
                    return parent.parent().unwrap_or(parent).to_path_buf();
                }
            }
        }
        current = dir.parent();
    }
    path.parent().unwrap_or(path).to_path_buf()
}

fn instruction_file_applies(path: &Path, root: &Path, targets: &[PathBuf]) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return true;
    };
    let Some(patterns) = instruction_frontmatter_paths(&text) else {
        return true;
    };
    let base = instruction_rule_base(path);
    let fallback = vec![root.to_path_buf()];
    let effective_targets = if targets.is_empty() {
        fallback.as_slice()
    } else {
        targets
    };
    effective_targets
        .iter()
        .any(|target| instruction_pattern_matches(&base, target, &patterns))
}

fn collect_instruction_rule_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for rel_dir in INSTRUCTION_RULE_DIRS {
        let rules_dir = directory.join(rel_dir);
        if !rules_dir.is_dir() {
            continue;
        }
        walk_files(&rules_dir, usize::MAX, &mut |path| {
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("mdc")
                })
            {
                files.push(path.to_path_buf());
            }
            true
        });
    }
    files.sort();
    files
}

fn collect_instruction_files(root: &Path, targets: &[PathBuf], names: &[String]) -> Vec<PathBuf> {
    let mut seen = Vec::<PathBuf>::new();
    let mut files = Vec::<PathBuf>::new();
    let fallback = vec![root.to_path_buf()];
    let target_slice = if targets.is_empty() {
        fallback.as_slice()
    } else {
        targets
    };
    for target in target_slice {
        for directory in instruction_search_dirs(root, target) {
            for name in names {
                let candidate = directory.join(name);
                if candidate.is_file()
                    && !seen.contains(&candidate)
                    && instruction_file_applies(&candidate, root, target_slice)
                {
                    seen.push(candidate.clone());
                    files.push(candidate);
                }
            }
            for rel_path in INSTRUCTION_REL_PATHS {
                let candidate = directory.join(rel_path);
                if candidate.is_file()
                    && !seen.contains(&candidate)
                    && instruction_file_applies(&candidate, root, target_slice)
                {
                    seen.push(candidate.clone());
                    files.push(candidate);
                }
            }
            for candidate in collect_instruction_rule_files(&directory) {
                if candidate.is_file()
                    && !seen.contains(&candidate)
                    && instruction_file_applies(&candidate, root, target_slice)
                {
                    seen.push(candidate.clone());
                    files.push(candidate);
                }
            }
        }
    }
    files
}

fn discover_skill_files(root: &Path, max_skills: usize) -> (Vec<PathBuf>, usize) {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut scanned = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !should_skip_dir(&name) {
                    stack.push(path);
                }
            } else if path.is_file() {
                scanned += 1;
                if name == "SKILL.md" {
                    files.push(path);
                    if files.len() >= max_skills {
                        files.sort();
                        return (files, scanned);
                    }
                }
            }
        }
    }
    files.sort();
    (files, scanned)
}

fn discover_agent_files(root: &Path, max_agents: usize) -> (Vec<PathBuf>, usize) {
    let mut seen = Vec::<PathBuf>::new();
    let mut files = Vec::<PathBuf>::new();
    let mut scanned = 0usize;
    for rel_dir in AGENT_DIRS {
        let directory = root.join(rel_dir);
        if !directory.is_dir() {
            continue;
        }
        walk_files(&directory, usize::MAX, &mut |path| {
            scanned += 1;
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
                if !seen.contains(&resolved) {
                    seen.push(resolved);
                    files.push(path.to_path_buf());
                }
            }
            files.len() < max_agents
        });
        if files.len() >= max_agents {
            break;
        }
    }
    files.sort();
    (files, scanned)
}

fn discover_command_files(root: &Path, max_commands: usize) -> (Vec<PathBuf>, usize) {
    let mut seen = Vec::<PathBuf>::new();
    let mut files = Vec::<PathBuf>::new();
    let mut scanned = 0usize;
    for rel_dir in COMMAND_DIRS {
        let directory = root.join(rel_dir);
        if !directory.is_dir() {
            continue;
        }
        walk_files(&directory, usize::MAX, &mut |path| {
            scanned += 1;
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
                if !seen.contains(&resolved) {
                    seen.push(resolved);
                    files.push(path.to_path_buf());
                }
            }
            files.len() < max_commands
        });
        if files.len() >= max_commands {
            break;
        }
    }
    files.sort();
    (files, scanned)
}

fn discover_agent_config_files(root: &Path, max_files: usize) -> (Vec<PathBuf>, usize) {
    // Agent configuration lives in a handful of shallow, well-known directories.
    // Do not let a plugin directory with generated content (or a symlink cycle)
    // turn this lightweight context lookup into an unbounded foreground task.
    let scan_budget = max_files.saturating_mul(200).clamp(1_000, 20_000);
    let mut seen = Vec::<PathBuf>::new();
    let mut files = Vec::<PathBuf>::new();
    let mut scanned = 0usize;
    let add =
        |path: PathBuf, files: &mut Vec<PathBuf>, seen: &mut Vec<PathBuf>, scanned: &mut usize| {
            *scanned += 1;
            if !path.is_file() {
                return true;
            }
            let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.contains(&resolved) {
                seen.push(resolved);
                files.push(path);
            }
            files.len() < max_files
        };

    for rel_path in AGENT_CONFIG_REL_PATHS {
        if !add(root.join(rel_path), &mut files, &mut seen, &mut scanned) {
            files.sort();
            return (files, scanned);
        }
    }

    for rel_dir in AGENT_EXTENSION_DIRS {
        let directory = root.join(rel_dir);
        if !directory.is_dir() {
            continue;
        }
        walk_files(
            &directory,
            scan_budget.saturating_sub(scanned).max(1),
            &mut |path| {
                let is_extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|ext| {
                        AGENT_EXTENSION_EXTS
                            .iter()
                            .any(|known| ext.eq_ignore_ascii_case(known))
                    });
                if !is_extension {
                    scanned += 1;
                    return true;
                }
                add(path.to_path_buf(), &mut files, &mut seen, &mut scanned)
            },
        );
        if files.len() >= max_files || scanned >= scan_budget {
            break;
        }
    }

    files.sort();
    (files, scanned)
}

fn discover_ci_files(root: &Path, max_files: usize) -> (Vec<PathBuf>, usize) {
    let mut seen = Vec::<PathBuf>::new();
    let mut files = Vec::<PathBuf>::new();
    let mut scanned = 0usize;
    let add =
        |path: PathBuf, files: &mut Vec<PathBuf>, seen: &mut Vec<PathBuf>, scanned: &mut usize| {
            *scanned += 1;
            if !path.is_file() {
                return true;
            }
            let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.contains(&resolved) {
                seen.push(resolved);
                files.push(path);
            }
            files.len() < max_files
        };
    for rel_path in CI_DIRECT_REL_PATHS {
        if !add(root.join(rel_path), &mut files, &mut seen, &mut scanned) {
            files.sort();
            return (files, scanned);
        }
    }
    for rel_dir in CI_WORKFLOW_DIRS {
        let directory = root.join(rel_dir);
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        let mut workflow_files = entries
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        workflow_files.sort();
        for path in workflow_files {
            let is_yaml = path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("yml") || ext.eq_ignore_ascii_case("yaml")
                });
            if !is_yaml {
                scanned += 1;
                continue;
            }
            if !add(path, &mut files, &mut seen, &mut scanned) {
                files.sort();
                return (files, scanned);
            }
        }
    }
    files.sort();
    (files, scanned)
}

fn skill_title(path: &Path, content: &str) -> String {
    for line in content.lines().take(40) {
        let stripped = line.trim();
        // Built-in and uploaded Skills use YAML front matter as their stable
        // capability identity. Prefer that machine-readable id over the first
        // prose heading so exact `skill_context(name:)` routing is reliable.
        if let Some(value) = stripped.strip_prefix("name:") {
            let value = value.trim().trim_matches(['"', '\'']);
            if !value.is_empty() {
                return value.to_string();
            }
        }
        if stripped.to_lowercase().starts_with("# skill:") {
            let value = stripped
                .split_once(':')
                .map(|(_, value)| value.trim())
                .unwrap_or("");
            if !value.is_empty() {
                return value.to_string();
            }
        }
        if let Some(value) = stripped.strip_prefix("# ") {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
        if stripped.to_lowercase().starts_with("name:") {
            let value = stripped
                .split_once(':')
                .map(|(_, value)| value.trim().trim_matches(['"', '\'']))
                .unwrap_or("");
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("Skill")
        .to_string()
}

fn agent_name(path: &Path, fields: &HashMap<String, String>) -> String {
    fields.get("name").cloned().unwrap_or_else(|| {
        path.file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("agent")
            .to_string()
    })
}

fn agent_description(content: &str, fields: &HashMap<String, String>) -> String {
    if let Some(description) = fields.get("description") {
        if !description.is_empty() {
            return description.clone();
        }
    }
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(outline_signature)
        .unwrap_or_default()
}

fn command_name(path: &Path, root: &Path, fields: &HashMap<String, String>) -> String {
    if let Some(name) = fields.get("name").filter(|name| !name.is_empty()) {
        return name.clone();
    }
    let rel = path
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
        .with_extension("");
    let rel = path_slash(&rel);
    for prefix in COMMAND_DIRS {
        let prefix = prefix.trim_matches('/');
        if rel == prefix {
            return path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("command")
                .to_string();
        }
        if let Some(value) = rel.strip_prefix(&format!("{prefix}/")) {
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("command")
        .to_string()
}

fn command_description(content: &str, fields: &HashMap<String, String>) -> String {
    if let Some(description) = fields.get("description") {
        if !description.is_empty() {
            return description.clone();
        }
    }
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.chars().take(240).collect())
        .unwrap_or_default()
}

fn sensitive_agent_config_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    [
        "api_key",
        "apikey",
        "api-key",
        "auth_token",
        "access_token",
        "refresh_token",
        "token",
        "secret",
        "password",
        "authorization",
        "cookie",
        "credential",
        "private_key",
        "private-key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn redact_agent_config_text(text: &str) -> String {
    let json_re = Regex::new(r#""(?P<key>[^"]+)"\s*:\s*"(?P<value>[^"]*)""#).unwrap();
    let mut redacted = json_re
        .replace_all(text, |captures: &regex::Captures| {
            let key = captures.name("key").map(|m| m.as_str()).unwrap_or("");
            if sensitive_agent_config_key(key) {
                format!("\"{key}\": \"<redacted>\"")
            } else {
                captures
                    .get(0)
                    .map(|m| m.as_str())
                    .unwrap_or("")
                    .to_string()
            }
        })
        .to_string();
    let line_re =
        Regex::new(r#"(?im)^(?P<prefix>\s*(?P<key>[A-Za-z0-9_.-]+)\s*[:=]\s*)(?P<value>.+)$"#)
            .unwrap();
    redacted = line_re
        .replace_all(&redacted, |captures: &regex::Captures| {
            let key = captures.name("key").map(|m| m.as_str()).unwrap_or("");
            if sensitive_agent_config_key(key) {
                format!(
                    "{}<redacted>",
                    captures.name("prefix").map(|m| m.as_str()).unwrap_or("")
                )
            } else {
                captures
                    .get(0)
                    .map(|m| m.as_str())
                    .unwrap_or("")
                    .to_string()
            }
        })
        .to_string();
    redacted
}

fn agent_config_kind(path: &Path, root: &Path) -> String {
    let rel = relative_display(path, root);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_lowercase();
    if rel.starts_with(".claude/settings") {
        "Claude settings".to_string()
    } else if rel == ".mcp.json" || name.contains("mcp") {
        "MCP config".to_string()
    } else if rel.starts_with(".opencode/tool") || rel.starts_with(".opencode/tools") {
        "OpenCode custom tool".to_string()
    } else if rel.starts_with(".opencode/plugin")
        || rel.starts_with(".opencode/plugins")
        || rel.starts_with(".claude/plugins")
    {
        "agent plugin".to_string()
    } else if name.contains("opencode") || rel.starts_with(".opencode/") {
        "OpenCode config".to_string()
    } else {
        "agent config".to_string()
    }
}

fn agent_config_signals(raw: &str) -> Vec<String> {
    let mut signals = Vec::new();
    let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
        let key_re = Regex::new(r#""([A-Za-z0-9_.-]+)"\s*:"#).unwrap();
        let mut keys = Vec::new();
        for captures in key_re.captures_iter(raw) {
            let Some(key) = captures.get(1).map(|m| m.as_str().to_string()) else {
                continue;
            };
            if !keys.contains(&key) {
                keys.push(key);
            }
            if keys.len() >= 10 {
                break;
            }
        }
        if !keys.is_empty() {
            signals.push(format!("keys: {}", keys.join(", ")));
        }
        return signals;
    };
    let Some(object) = parsed.as_object() else {
        return signals;
    };
    let top_keys = object.keys().take(12).cloned().collect::<Vec<_>>();
    if !top_keys.is_empty() {
        signals.push(format!("keys: {}", top_keys.join(", ")));
    }
    if let Some(permissions) = object
        .get("permissions")
        .or_else(|| object.get("permission"))
    {
        if let Some(permission_obj) = permissions.as_object() {
            if let Some(mode) = permission_obj
                .get("defaultMode")
                .or_else(|| permission_obj.get("mode"))
                .and_then(value_to_scalar_string)
            {
                signals.push(format!("permissions.defaultMode: {mode}"));
            }
            for key in ["allow", "deny", "ask", "allowedTools", "deniedTools"] {
                if let Some(count) = permission_obj
                    .get(key)
                    .and_then(|value| value.as_array())
                    .map(Vec::len)
                {
                    signals.push(format!("permissions.{key}: {count} entries"));
                }
            }
        } else if let Some(count) = permissions.as_array().map(Vec::len) {
            signals.push(format!("permissions: {count} rules"));
        }
    }
    if let Some(hooks) = object.get("hooks").and_then(Value::as_object) {
        signals.push(format!(
            "hooks: {}",
            hooks.keys().take(8).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if let Some(mcp) = object
        .get("mcpServers")
        .or_else(|| object.get("mcp"))
        .and_then(Value::as_object)
    {
        signals.push(format!(
            "mcpServers: {}",
            mcp.keys().take(8).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if let Some(count) = object
        .get("plugin")
        .or_else(|| object.get("plugins"))
        .and_then(Value::as_array)
        .map(Vec::len)
    {
        signals.push(format!("plugins: {count} entries"));
    }
    if let Some(agents) = object
        .get("agent")
        .or_else(|| object.get("agents"))
        .and_then(Value::as_object)
    {
        signals.push(format!(
            "agents: {}",
            agents
                .keys()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(provider) = object.get("provider").and_then(Value::as_object) {
        signals.push(format!(
            "providers: {}",
            provider
                .keys()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(model) = object.get("model").and_then(value_to_scalar_string) {
        signals.push(format!("model: {model}"));
    }
    signals
}

fn value_to_scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn ci_file_kind(path: &Path, root: &Path) -> String {
    let rel = relative_display(path, root);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_lowercase();
    if rel.starts_with(".github/workflows/") {
        "GitHub Actions".to_string()
    } else if name.starts_with(".gitlab-ci") {
        "GitLab CI".to_string()
    } else if name.starts_with("azure-pipelines") {
        "Azure Pipelines".to_string()
    } else if name.starts_with("bitbucket-pipelines") {
        "Bitbucket Pipelines".to_string()
    } else if rel.starts_with(".circleci/") {
        "CircleCI".to_string()
    } else if name == "makefile" || name == "gnumakefile" {
        "Makefile".to_string()
    } else if name == "justfile" {
        "justfile".to_string()
    } else if name.starts_with("taskfile") {
        "Taskfile".to_string()
    } else {
        "CI/task config".to_string()
    }
}

fn ci_yaml_jobs(text: &str) -> Vec<String> {
    let mut jobs = Vec::new();
    let mut in_jobs = false;
    let job_re = Regex::new(r"^\s{2}([A-Za-z0-9_.-]+):\s*(?:#.*)?$").unwrap();
    for line in text.lines() {
        if line.trim() == "jobs:" && !line.starts_with(char::is_whitespace) {
            in_jobs = true;
            continue;
        }
        if in_jobs && !line.is_empty() && !line.starts_with(char::is_whitespace) {
            break;
        }
        if in_jobs {
            if let Some(job) = job_re
                .captures(line)
                .and_then(|captures| captures.get(1))
                .map(|matched| matched.as_str().to_string())
            {
                if !jobs.contains(&job) {
                    jobs.push(job);
                }
            }
        }
        if jobs.len() >= 12 {
            break;
        }
    }
    jobs
}

fn ci_yaml_commands(text: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let run_re = Regex::new(r"^-?\s*run:\s*(.+)$").unwrap();
    for line in text.lines() {
        let stripped = line.trim();
        let Some(command) = run_re
            .captures(stripped)
            .and_then(|captures| captures.get(1))
            .map(|matched| {
                matched
                    .as_str()
                    .trim()
                    .trim_matches(['"', '\''])
                    .to_string()
            })
        else {
            continue;
        };
        if command.is_empty() || command == "|" || command == ">" {
            continue;
        }
        let command = command.chars().take(180).collect::<String>();
        if !commands.contains(&command) {
            commands.push(command);
        }
        if commands.len() >= 12 {
            break;
        }
    }
    commands
}

fn make_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let target_re = Regex::new(r"^([A-Za-z0-9_.-]+)\s*:").unwrap();
    for line in text.lines() {
        if line.starts_with(['\t', ' ', '.']) {
            continue;
        }
        if line.contains(":=") {
            continue;
        }
        if let Some(target) = target_re
            .captures(line)
            .and_then(|captures| captures.get(1))
            .map(|matched| matched.as_str().to_string())
        {
            if !target.contains('%') && !targets.contains(&target) {
                targets.push(target);
            }
        }
        if targets.len() >= 20 {
            break;
        }
    }
    targets
}

fn just_recipes(text: &str) -> Vec<String> {
    let mut recipes = Vec::new();
    let recipe_re = Regex::new(r"^([A-Za-z0-9_-]+)(?:\([^)]*\))?\s*(?:[:=]|$)").unwrap();
    for line in text.lines() {
        if line.starts_with([' ', '\t', '#', '@']) {
            continue;
        }
        if let Some(recipe) = recipe_re
            .captures(line)
            .and_then(|captures| captures.get(1))
            .map(|matched| matched.as_str().to_string())
        {
            if !recipes.contains(&recipe) {
                recipes.push(recipe);
            }
        }
        if recipes.len() >= 20 {
            break;
        }
    }
    recipes
}

fn taskfile_tasks(text: &str) -> Vec<String> {
    let mut tasks = Vec::new();
    let mut in_tasks = false;
    let task_re = Regex::new(r"^\s{2}([A-Za-z0-9_.-]+):").unwrap();
    for line in text.lines() {
        if line.trim() == "tasks:" && !line.starts_with(char::is_whitespace) {
            in_tasks = true;
            continue;
        }
        if in_tasks && !line.is_empty() && !line.starts_with(char::is_whitespace) {
            break;
        }
        if in_tasks {
            if let Some(task) = task_re
                .captures(line)
                .and_then(|captures| captures.get(1))
                .map(|matched| matched.as_str().to_string())
            {
                if !tasks.contains(&task) {
                    tasks.push(task);
                }
            }
        }
        if tasks.len() >= 20 {
            break;
        }
    }
    tasks
}

fn ci_file_signals(path: &Path, root: &Path, text: &str) -> Vec<String> {
    let kind = ci_file_kind(path, root);
    let mut signals = Vec::new();
    match kind.as_str() {
        "GitHub Actions" | "GitLab CI" | "Azure Pipelines" | "Bitbucket Pipelines" | "CircleCI" => {
            let jobs = ci_yaml_jobs(text);
            let commands = ci_yaml_commands(text);
            if !jobs.is_empty() {
                signals.push(format!("jobs: {}", jobs.join(", ")));
            }
            if !commands.is_empty() {
                signals.push(format!(
                    "run: {}",
                    commands.into_iter().take(6).collect::<Vec<_>>().join(" | ")
                ));
            }
        }
        "Makefile" => {
            let targets = make_targets(text);
            if !targets.is_empty() {
                signals.push(format!("targets: {}", targets.join(", ")));
            }
        }
        "justfile" => {
            let recipes = just_recipes(text);
            if !recipes.is_empty() {
                signals.push(format!("recipes: {}", recipes.join(", ")));
            }
        }
        "Taskfile" => {
            let tasks = taskfile_tasks(text);
            if !tasks.is_empty() {
                signals.push(format!("tasks: {}", tasks.join(", ")));
            }
        }
        _ => {}
    }
    signals
}

fn task_file_commands(root: &Path) -> Vec<String> {
    let mut commands = Vec::new();
    for rel_path in CI_DIRECT_REL_PATHS {
        let path = root.join(rel_path);
        if !path.is_file() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        match ci_file_kind(&path, root).as_str() {
            "Makefile" => {
                let targets = make_targets(&text);
                for target in ["test", "check", "lint", "build"] {
                    if targets.iter().any(|item| item == target) {
                        commands.push(format!("make {target}"));
                    }
                }
            }
            "justfile" => {
                let recipes = just_recipes(&text);
                for recipe in ["test", "check", "lint", "build", "fmt", "format"] {
                    if recipes.iter().any(|item| item == recipe) {
                        commands.push(format!("just {recipe}"));
                    }
                }
            }
            "Taskfile" => {
                let tasks = taskfile_tasks(&text);
                for task in ["test", "check", "lint", "build", "fmt", "format"] {
                    if tasks.iter().any(|item| item == task) {
                        commands.push(format!("task {task}"));
                    }
                }
            }
            _ => {}
        }
    }
    let mut deduped = Vec::new();
    for command in commands {
        if !deduped.contains(&command) {
            deduped.push(command);
        }
    }
    deduped
}

fn skill_description(content: &str) -> String {
    for line in content.lines().take(80) {
        let stripped = line.trim();
        if stripped.to_lowercase().starts_with("description:") {
            return stripped
                .split_once(':')
                .map(|(_, value)| value.trim().trim_matches(['"', '\'']).to_string())
                .unwrap_or_default();
        }
    }
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.chars().take(240).collect())
        .unwrap_or_default()
}

fn skill_sample_files(skill_dir: &Path, max_files: usize) -> Vec<PathBuf> {
    if max_files == 0 {
        return Vec::new();
    }
    let mut files = Vec::new();
    walk_files(skill_dir, usize::MAX, &mut |path| {
        if path.file_name().and_then(|name| name.to_str()) != Some("SKILL.md") {
            files.push(path.to_path_buf());
        }
        files.len() < max_files
    });
    files.sort();
    files
}

fn context_pack_paths(
    root: &Path,
    explicit_paths: &[PathBuf],
    include_changed: bool,
    max_files: usize,
) -> Vec<PathBuf> {
    let mut targets = Vec::new();
    for path in explicit_paths {
        if path.is_file() && !targets.contains(path) {
            targets.push(path.clone());
        }
    }
    if include_changed {
        for path in git_changed_files(root) {
            if path.is_file() && !targets.contains(&path) {
                targets.push(path);
            }
            if targets.len() >= max_files {
                break;
            }
        }
    }
    targets.truncate(max_files);
    targets
}

fn context_pack_file_section(
    root: &Path,
    path: &Path,
    max_symbols: usize,
    max_imports: usize,
) -> Vec<String> {
    let rel = relative_display(path, root);
    let Ok(text) = fs::read_to_string(path) else {
        return vec![
            format!("### {rel}"),
            "error: unable to read file".to_string(),
        ];
    };
    let line_count = text.lines().count();
    let bytes = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    let language = path
        .extension()
        .and_then(|ext| ext.to_str())
        .and_then(language_for_ext)
        .unwrap_or("text");
    let mut section = vec![
        format!("### {rel}"),
        format!("lines: {line_count}"),
        format!("bytes: {bytes}"),
        format!("language: {language}"),
        String::new(),
        "imports:".to_string(),
    ];
    let imports = if max_imports == 0 {
        Vec::new()
    } else {
        extract_imports(&text, max_imports).unwrap_or_default()
    };
    if imports.is_empty() {
        section.push("(none)".to_string());
    } else {
        section.extend(imports.into_iter().map(|item| {
            format!(
                "- {}: {} {} | {}",
                item.line, item.kind, item.spec, item.signature
            )
        }));
    }
    section.extend([String::new(), "outline:".to_string()]);
    let outline = extract_outline(&text, max_symbols).unwrap_or_default();
    if outline.is_empty() {
        section.push("(none)".to_string());
    } else {
        section.extend(outline.into_iter().map(|item| format!("- {item}")));
    }
    let (tests, same_name, references, _scanned) = scan_related_files(path, root, 20);
    let related = tests
        .into_iter()
        .take(8)
        .chain(same_name.into_iter().take(4))
        .collect::<Vec<_>>();
    section.extend([String::new(), "related:".to_string()]);
    if related.is_empty() {
        section.push("(none)".to_string());
    } else {
        section.extend(
            related
                .iter()
                .map(|candidate| format!("- {}", relative_display(candidate, root))),
        );
    }
    if !references.is_empty() {
        section.extend([String::new(), "references:".to_string()]);
        section.extend(
            references
                .into_iter()
                .take(8)
                .map(|item| format!("- {item}")),
        );
    }
    section
}

fn focused_context_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();
    for ch in query.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$' | '.' | '/' | ':' | '-') {
            current.push(ch);
            continue;
        }
        let cleaned = current.trim_matches(['.', '/', ':', '-', '_', '$']);
        if cleaned.len() >= 3 && !terms.iter().any(|term| term == cleaned) {
            terms.push(cleaned.to_string());
        }
        current.clear();
    }
    let cleaned = current.trim_matches(['.', '/', ':', '-', '_', '$']);
    if cleaned.len() >= 3 && !terms.iter().any(|term| term == cleaned) {
        terms.push(cleaned.to_string());
    }
    terms.truncate(16);
    terms
}

fn focused_context_snippets(text: &str, terms: &[String], max_snippets: usize) -> Vec<String> {
    if terms.is_empty() || max_snippets == 0 {
        return Vec::new();
    }
    let lines = text.lines().collect::<Vec<_>>();
    let mut snippets = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let lowered = line.to_lowercase();
        if !terms.iter().any(|term| lowered.contains(term)) {
            continue;
        }
        let start = index.saturating_sub(1);
        let end = (index + 2).min(lines.len());
        let body = (start..end)
            .map(|line_no| format!("{}: {}", line_no + 1, outline_signature(lines[line_no])))
            .collect::<Vec<_>>()
            .join("\n");
        snippets.push(body);
        if snippets.len() >= max_snippets {
            break;
        }
    }
    snippets
}

fn focused_context_candidates(
    root: &Path,
    query: &str,
    seed_paths: &[PathBuf],
    max_files: usize,
    max_scan_files: usize,
) -> (Vec<(i64, PathBuf, Vec<String>)>, usize) {
    let terms = focused_context_terms(query);
    let mut candidates: HashMap<PathBuf, (i64, Vec<String>)> = HashMap::new();
    for path in seed_paths {
        if path.is_file() && is_text_source(path) {
            candidates.insert(path.clone(), (200, vec!["explicit target".to_string()]));
        }
    }
    let mut scanned = 0usize;
    walk_files(root, max_scan_files, &mut |path| {
        if !is_text_source(path) {
            return true;
        }
        scanned += 1;
        let rel = relative_display(path, root).to_lowercase();
        let (mut score, mut reasons) = candidates.remove(path).unwrap_or((0, Vec::new()));
        if seed_paths
            .iter()
            .any(|seed| seed.is_dir() && path.starts_with(seed))
        {
            score += 160;
            reasons.push("inside explicit target directory".to_string());
        }
        for term in &terms {
            if rel.contains(term) {
                score += 30;
                if reasons.len() < 4 {
                    reasons.push(format!("path matches `{term}`"));
                }
            }
        }
        if !terms.is_empty() {
            if let Ok(text) = fs::read_to_string(path) {
                let lowered = text
                    .chars()
                    .take(200_000)
                    .collect::<String>()
                    .to_lowercase();
                for term in &terms {
                    let count = lowered.matches(term).count();
                    if count > 0 {
                        score += (count as i64 * 4).min(40);
                        if reasons.len() < 4 {
                            reasons.push(format!("content matches `{term}` x{count}"));
                        }
                    }
                }
            }
        }
        if score > 0 {
            candidates.insert(path.to_path_buf(), (score, reasons));
        }
        true
    });
    let mut ranked = candidates
        .into_iter()
        .map(|(path, (score, reasons))| (score, path, reasons))
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    ranked.truncate(max_files);
    (ranked, scanned)
}

fn change_review_risks(changed: &[PathBuf]) -> Vec<String> {
    let mut risks = Vec::new();
    if changed.is_empty() {
        return risks;
    }
    let source_changed = changed.iter().any(|path| is_code_source(path));
    let tests_changed = changed.iter().any(|path| is_probable_test_file(path));
    if source_changed && !tests_changed {
        risks.push("source files changed without tests changed in this review scope".to_string());
    }
    if changed.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| manifest_names().contains(&name))
            || path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "gradle" | "kts"))
    }) {
        risks.push("manifest or build configuration changed".to_string());
    }
    let lowered = changed
        .iter()
        .map(|path| path_slash(path).to_lowercase())
        .collect::<Vec<_>>();
    if lowered.iter().any(|path| {
        path.contains("migration")
            || path.contains("schema")
            || path.contains("database")
            || path.contains("/db/")
    }) {
        risks.push("database/schema-related files changed".to_string());
    }
    if lowered.iter().any(|path| {
        path.contains(".env")
            || path.contains("secret")
            || path.contains("credential")
            || path.contains("token")
    }) {
        risks.push("sensitive configuration or credential-looking path changed".to_string());
    }
    if changed.iter().any(|path| {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext, "strings" | "xml"))
            && path_slash(path).contains("values")
    }) {
        risks.push(
            "localized resources changed; check fallback/default locale consistency".to_string(),
        );
    }
    risks
}

const DELETED_TEST_ASSERT_PATTERNS: &[&str] = &[
    "assert",
    "expect(",
    "xctassert",
    "assertthat",
    ".tobe",
    ".toequal",
    "should",
];

fn looks_like_hard_coded_secret_assignment(line: &str) -> bool {
    let Some((left, right)) = line.split_once('=') else {
        return false;
    };
    let left = left.trim_end();
    let right = right.trim_start();
    if left.ends_with(['=', '!', '<', '>']) || right.starts_with('=') {
        return false;
    }
    let key = left
        .rsplit(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let sensitive_key = [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "private_key",
        "client_secret",
        "access_key",
    ]
    .iter()
    .any(|candidate| key == *candidate || key.ends_with(&format!("_{candidate}")));
    if !sensitive_key {
        return false;
    }
    let Some(quote) = right.chars().next().filter(|value| matches!(value, '\'' | '"')) else {
        return false;
    };
    let value = right[quote.len_utf8()..]
        .split(quote)
        .next()
        .unwrap_or_default()
        .trim();
    !value.is_empty()
}

fn diff_review_finding(
    path: &str,
    line_no: usize,
    line: &str,
    deleted: bool,
    staged: bool,
) -> Option<String> {
    let stripped = line.trim();
    if stripped.is_empty() {
        return None;
    }
    let lowered = stripped.to_lowercase();
    let origin = if staged { "staged " } else { "" };
    let location = format!("{origin}{path}:{line_no}");
    let sample = outline_signature(stripped);
    if deleted {
        let path_obj = Path::new(path);
        if is_probable_test_file(path_obj)
            && DELETED_TEST_ASSERT_PATTERNS
                .iter()
                .any(|pattern| lowered.contains(pattern))
        {
            return Some(format!(
                "- medium: {location} deleted test assertion or expectation | {sample}"
            ));
        }
        if is_probable_test_file(path_obj)
            && (lowered.contains("test") || lowered.contains("it(") || lowered.contains("spec"))
        {
            return Some(format!(
                "- medium: {location} deleted test case structure | {sample}"
            ));
        }
        return None;
    }
    if looks_like_hard_coded_secret_assignment(stripped) {
        return Some(format!(
            "- high: {location} adds a hard-coded credential or secret | {sample}"
        ));
    }
    if lowered.contains("verify=false")
        || lowered.contains("rejectunauthorized: false")
        || lowered.contains("node_tls_reject_unauthorized")
    {
        return Some(format!(
            "- high: {location} disables TLS or certificate verification | {sample}"
        ));
    }
    if lowered.contains("dangerouslysetinnerhtml")
        || (lowered.contains(".innerhtml") && stripped.contains('='))
    {
        return Some(format!(
            "- high: {location} adds raw HTML injection surface | {sample}"
        ));
    }
    if (lowered.starts_with("except") && lowered.ends_with(':'))
        || (lowered.starts_with("catch") && (lowered.contains("error") || lowered.contains("err")))
    {
        return Some(format!(
            "- medium: {location} adds broad exception handler; ensure errors are not swallowed | {sample}"
        ));
    }
    if (lowered.contains("pass") || lowered.contains("return none"))
        && (lowered.contains("except") || lowered.contains("catch"))
    {
        return Some(format!(
            "- medium: {location} may swallow exception handling | {sample}"
        ));
    }
    if lowered.contains("console.log(")
        || lowered.contains("debugger")
        || lowered.contains("pdb.set_trace")
    {
        return Some(format!(
            "- medium: {location} adds debug-only statement | {sample}"
        ));
    }
    if lowered.contains("todo") || lowered.contains("fixme") {
        return Some(format!(
            "- low: {location} adds unresolved TODO/FIXME | {sample}"
        ));
    }
    if lowered.contains("sleep(") || lowered.contains("thread.sleep(") {
        return Some(format!(
            "- medium: {location} adds fixed sleep; prefer deterministic wait or synchronization | {sample}"
        ));
    }
    None
}

fn parse_diff_hunk_start(token: &str) -> usize {
    token
        .trim_start_matches(['-', '+'])
        .split(',')
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
}

fn scan_diff_review_text(
    diff_text: &str,
    staged: bool,
    max_findings: usize,
    findings: &mut Vec<String>,
    scanned: &mut usize,
) -> bool {
    let mut current_path = String::new();
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    for raw_line in diff_text.lines() {
        if let Some(path) = raw_line.strip_prefix("+++ ") {
            current_path = path.trim().to_string();
            if let Some(stripped) = current_path.strip_prefix("b/") {
                current_path = stripped.to_string();
            }
            continue;
        }
        if raw_line.starts_with("@@ ") {
            let parts = raw_line.split_whitespace().collect::<Vec<_>>();
            if parts.len() >= 3 {
                old_line = parse_diff_hunk_start(parts[1]);
                new_line = parse_diff_hunk_start(parts[2]);
            }
            continue;
        }
        if current_path.is_empty() || current_path == "/dev/null" {
            continue;
        }
        if raw_line.starts_with('+') && !raw_line.starts_with("+++") {
            *scanned += 1;
            if let Some(finding) =
                diff_review_finding(&current_path, new_line, &raw_line[1..], false, staged)
            {
                if !findings.contains(&finding) {
                    findings.push(finding);
                }
            }
            new_line += 1;
        } else if raw_line.starts_with('-') && !raw_line.starts_with("---") {
            *scanned += 1;
            if let Some(finding) =
                diff_review_finding(&current_path, old_line, &raw_line[1..], true, staged)
            {
                if !findings.contains(&finding) {
                    findings.push(finding);
                }
            }
            old_line += 1;
        } else if raw_line.starts_with(' ') {
            old_line += 1;
            new_line += 1;
        }
        if findings.len() > max_findings {
            return true;
        }
    }
    false
}

fn diff_review_findings(
    root: &Path,
    paths: &[PathBuf],
    include_staged: bool,
    max_findings: usize,
) -> (Vec<String>, usize, bool) {
    let pathspecs = paths
        .iter()
        .filter_map(|path| path.strip_prefix(root).ok().map(path_slash))
        .collect::<Vec<_>>();
    let mut findings = Vec::new();
    let mut scanned = 0usize;
    for staged in [false, true] {
        if staged && !include_staged {
            continue;
        }
        let base = if staged {
            ["diff", "--unified=0", "--cached"].as_slice()
        } else {
            ["diff", "--unified=0"].as_slice()
        };
        let args = git_args_with_pathspecs(base, &pathspecs);
        let Ok(output) = run_git_strings(root, &args) else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let diff_text = String::from_utf8_lossy(&output.stdout);
        if scan_diff_review_text(
            &diff_text,
            staged,
            max_findings,
            &mut findings,
            &mut scanned,
        ) {
            findings.truncate(max_findings);
            return (findings, scanned, true);
        }
    }
    let truncated = findings.len() > max_findings;
    findings.truncate(max_findings);
    (findings, scanned, truncated)
}

fn verification_severity(text: &str) -> &'static str {
    let lowered = text.to_lowercase();
    if [
        "merge conflict",
        "conflict marker",
        "invalid json",
        "invalid toml",
        "invalid yaml",
        "broken relative import",
        "hard-coded secret",
        "credential",
        "token",
        "sensitive configuration",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "high";
    }
    if [
        "without tests",
        "debugger",
        "print call left",
        "console.log",
        "pdb.set_trace",
        "manifest or build",
        "database/schema",
        "localized resources",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
    {
        return "medium";
    }
    "low"
}

fn verification_findings(
    target_paths: &[PathBuf],
    issues: &[String],
    risks: &[String],
    max_findings: usize,
) -> (Vec<String>, bool) {
    let mut findings = Vec::new();
    findings.extend(
        issues
            .iter()
            .map(|issue| format!("- {}: {issue}", verification_severity(issue))),
    );
    findings.extend(
        risks
            .iter()
            .map(|risk| format!("- {}: {risk}", verification_severity(risk))),
    );
    if target_paths.len() > 20 {
        findings.push(format!(
            "- medium: large change touches {} files; broaden validation beyond focused tests",
            target_paths.len()
        ));
    }
    let mut deduped = Vec::new();
    for finding in findings {
        if !deduped.contains(&finding) {
            deduped.push(finding);
        }
    }
    let truncated = deduped.len() > max_findings;
    deduped.truncate(max_findings);
    (deduped, truncated)
}

fn verification_verdict(findings: &[String]) -> &'static str {
    if findings
        .iter()
        .any(|finding| finding.starts_with("- high:"))
    {
        "blocked"
    } else if findings.is_empty() {
        "pass"
    } else {
        "attention_needed"
    }
}

fn handoff_plan_entries(root: &Path, max_plans: usize) -> Vec<String> {
    let plans_dir = root.join(".opencode").join("plans");
    let Ok(entries) = fs::read_dir(&plans_dir) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            let is_markdown = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "md");
            (metadata.is_file() && is_markdown).then_some((path, metadata))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| {
        b.1.modified()
            .ok()
            .cmp(&a.1.modified().ok())
            .then_with(|| a.0.cmp(&b.0))
    });
    files
        .into_iter()
        .take(max_plans)
        .filter_map(|(path, metadata)| {
            let slug = path.file_stem()?.to_string_lossy();
            let digest = file_sha256_hex(&path).ok()?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or_default();
            Some(format!(
                "- {slug} path={} sha256={digest} bytes={} mtime={modified}",
                relative_display(&path, root),
                metadata.len()
            ))
        })
        .collect()
}

fn dependency_impact(root: &Path, paths: &[PathBuf], max_entries: usize) -> Vec<String> {
    let mut impacts = Vec::new();
    for path in paths {
        if !path.is_file() || !is_code_source(path) {
            continue;
        }
        let remaining = max_entries.saturating_sub(impacts.len()).max(1);
        let (importers, scanned) = scan_importers(root, path, remaining);
        if importers.is_empty() {
            continue;
        }
        impacts.push(format!(
            "- {}: imported by {} local file(s); scanned_files={}",
            relative_display(path, root),
            importers.len(),
            scanned
        ));
        impacts.extend(importers.into_iter().map(|item| format!("  - {item}")));
        if impacts.len() >= max_entries {
            break;
        }
    }
    impacts.truncate(max_entries);
    impacts
}

fn changed_symbols_for_review(root: &Path, max_entries: usize) -> (Vec<String>, usize, usize) {
    let Ok(git_root_output) = run_git(root, &["rev-parse", "--show-toplevel"]) else {
        return (Vec::new(), 0, 0);
    };
    if !git_root_output.status.success() {
        return (Vec::new(), 0, 0);
    }
    let git_root_text = String::from_utf8_lossy(&git_root_output.stdout)
        .trim()
        .to_string();
    let git_root = PathBuf::from(&git_root_text)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&git_root_text));
    let mut pathspecs = Vec::new();
    if let Ok(root_scope) = root.strip_prefix(&git_root) {
        let root_scope = path_slash(root_scope);
        if !root_scope.is_empty() {
            pathspecs.push(root_scope);
        }
    }
    if pathspecs.is_empty() {
        pathspecs.push(".".to_string());
    }
    let unstaged_args = git_args_with_pathspecs(&["diff", "--unified=0"], &pathspecs);
    let staged_args = git_args_with_pathspecs(&["diff", "--cached", "--unified=0"], &pathspecs);
    let mut changed_lines = HashMap::new();
    if let Ok(unstaged) = run_git_strings(&git_root, &unstaged_args) {
        if unstaged.status.success() {
            changed_lines = parse_diff_changed_lines(&String::from_utf8_lossy(&unstaged.stdout));
        }
    }
    if let Ok(staged) = run_git_strings(&git_root, &staged_args) {
        if staged.status.success() {
            merge_changed_lines(
                &mut changed_lines,
                parse_diff_changed_lines(&String::from_utf8_lossy(&staged.stdout)),
            );
        }
    }
    let mut untracked_count = 0usize;
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
    let (entries, scanned) = changed_symbol_entries(&git_root, &changed_lines, max_entries);
    (entries, scanned, untracked_count)
}

fn git_status_path(line: &str) -> Option<&str> {
    if line.len() < 4 {
        return None;
    }
    let mut path = line[3..].trim();
    if let Some((_old_path, new_path)) = path.rsplit_once(" -> ") {
        path = new_path.trim();
    }
    let path = path.trim_matches('"');
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

fn git_changed_files(root: &Path) -> Vec<PathBuf> {
    let Ok(git_root_output) = run_git(root, &["rev-parse", "--show-toplevel"]) else {
        return Vec::new();
    };
    if !git_root_output.status.success() {
        return Vec::new();
    }
    let git_root_text = String::from_utf8_lossy(&git_root_output.stdout)
        .trim()
        .to_string();
    if git_root_text.is_empty() {
        return Vec::new();
    }
    let git_root = PathBuf::from(git_root_text);
    let Ok(status) = run_git(
        root,
        &[
            "-c",
            "status.relativePaths=false",
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            ".",
        ],
    ) else {
        return Vec::new();
    };
    if !status.status.success() {
        return Vec::new();
    }
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut changed = Vec::new();
    for line in String::from_utf8_lossy(&status.stdout).lines() {
        let Some(path_text) = git_status_path(line) else {
            continue;
        };
        let candidate = git_root.join(path_text);
        let candidate = candidate.canonicalize().unwrap_or(candidate);
        if candidate.is_file() && candidate.starts_with(&root) && !changed.contains(&candidate) {
            changed.push(candidate);
        }
    }
    changed
}

fn compact_name(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_probable_test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let has_test_dir = path.components().any(|component| {
        let text = component.as_os_str().to_string_lossy().to_lowercase();
        text == "test" || text == "tests"
    });
    name.starts_with("test_")
        || name.ends_with("_test.py")
        || name.ends_with("_tests.py")
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.ends_with("test.swift")
        || name.ends_with("tests.swift")
        || name.ends_with("test.kt")
        || name.ends_with("tests.kt")
        || has_test_dir
}

fn is_likely_test_for(target: &Path, candidate: &Path) -> bool {
    if target == candidate || !is_probable_test_file(candidate) {
        return false;
    }
    let Some(target_stem) = target
        .file_stem()
        .map(|value| compact_name(&value.to_string_lossy()))
    else {
        return false;
    };
    let Some(candidate_stem) = candidate
        .file_stem()
        .map(|value| compact_name(&value.to_string_lossy()))
    else {
        return false;
    };
    candidate_stem.contains(&target_stem)
        || candidate_stem == format!("test{target_stem}")
        || candidate_stem == format!("{target_stem}test")
        || candidate_stem == format!("{target_stem}tests")
        || candidate_stem == format!("{target_stem}spec")
}

fn is_text_source(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| {
            language_for_ext(ext).is_some()
                || matches!(ext, "json" | "toml" | "yaml" | "yml" | "md" | "txt")
        })
}

fn is_code_source(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| language_for_ext(ext).is_some())
}

fn related_reference_terms(target: &Path, root: &Path) -> Vec<String> {
    let mut terms = Vec::new();
    if let Some(stem) = target.file_stem().and_then(|value| value.to_str()) {
        terms.push(stem.to_string());
    }
    if let Some(name) = target.file_name().and_then(|value| value.to_str()) {
        terms.push(name.to_string());
    }
    if let Ok(rel) = target.strip_prefix(root) {
        let mut rel = rel.to_path_buf();
        rel.set_extension("");
        let slash = path_slash(&rel);
        terms.push(slash.clone());
        terms.push(slash.replace('/', "."));
    }
    terms.sort();
    terms.dedup();
    terms
}

fn scan_related_files(
    target: &Path,
    root: &Path,
    max_results: usize,
) -> (Vec<PathBuf>, Vec<PathBuf>, Vec<String>, usize) {
    let mut tests = Vec::new();
    let mut same_name = Vec::new();
    let mut references = Vec::new();
    let mut scanned = 0usize;
    let terms = related_reference_terms(target, root);
    walk_files(root, usize::MAX, &mut |path| {
        if path == target {
            return tests.len() + same_name.len() + references.len() < max_results;
        }
        scanned += 1;
        if is_likely_test_for(target, path) && !tests.contains(&path.to_path_buf()) {
            tests.push(path.to_path_buf());
        }
        if path.file_stem() == target.file_stem() && !same_name.contains(&path.to_path_buf()) {
            same_name.push(path.to_path_buf());
        }
        if is_text_source(path) && references.len() < max_results {
            if let Ok(text) = fs::read_to_string(path) {
                for (line_no, line) in text.lines().enumerate() {
                    if terms.iter().any(|term| line.contains(term)) {
                        references.push(format!(
                            "{}:{}: {}",
                            path.display(),
                            line_no + 1,
                            outline_signature(line)
                        ));
                        break;
                    }
                }
            }
        }
        tests.len() + same_name.len() + references.len() < max_results
    });
    (tests, same_name, references, scanned)
}

fn outline_signature(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() > 160 {
        format!("{}...", trimmed.chars().take(157).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

fn extract_outline(text: &str, max_symbols: usize) -> Result<Vec<String>, ToolOutcome> {
    let patterns = outline_patterns();
    let mut symbols = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        for (kind, regex) in patterns {
            let Some(captures) = regex.captures(line) else {
                continue;
            };
            let Some(name) = captures.get(1) else {
                continue;
            };
            if skip_outline_symbol(kind, name.as_str(), line) {
                continue;
            }
            symbols.push(format!(
                "{}: {} {} | {}",
                line_no + 1,
                kind,
                name.as_str(),
                outline_signature(line)
            ));
            break;
        }
        if symbols.len() >= max_symbols {
            symbols.push("...[outline truncated]".to_string());
            break;
        }
    }
    Ok(symbols)
}

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

fn format_numbered_lines(text: &str, start_line: usize, show_line_numbers: bool) -> String {
    if !show_line_numbers {
        return text.to_string();
    }
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let width = 4usize.max((start_line + lines.len() - 1).to_string().len());
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| format!("{:>width$} | {}", start_line + index, line, width = width))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_unified_diff(path: &Path, before: &str, after: &str, max_chars: usize) -> String {
    if before == after {
        return "(no changes)".to_string();
    }
    let before_lines = before.lines().collect::<Vec<_>>();
    let after_lines = after.lines().collect::<Vec<_>>();
    let mut prefix = 0usize;
    while prefix < before_lines.len()
        && prefix < after_lines.len()
        && before_lines[prefix] == after_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < before_lines.len().saturating_sub(prefix)
        && suffix < after_lines.len().saturating_sub(prefix)
        && before_lines[before_lines.len() - 1 - suffix]
            == after_lines[after_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let context = 3usize;
    let old_start = prefix.saturating_sub(context);
    let new_start = prefix.saturating_sub(context);
    let old_changed_end = before_lines.len().saturating_sub(suffix);
    let new_changed_end = after_lines.len().saturating_sub(suffix);
    let old_end = (old_changed_end + context).min(before_lines.len());
    let new_end = (new_changed_end + context).min(after_lines.len());
    let old_count = old_end.saturating_sub(old_start);
    let new_count = new_end.saturating_sub(new_start);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let mut diff = format!(
        "--- a/{name}\n+++ b/{name}\n@@ -{},{} +{},{} @@\n",
        old_start + 1,
        old_count,
        new_start + 1,
        new_count
    );
    for line in &before_lines[old_start..prefix] {
        diff.push(' ');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &before_lines[prefix..old_changed_end] {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &after_lines[prefix..new_changed_end] {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in &before_lines[old_changed_end..old_end] {
        diff.push(' ');
        diff.push_str(line);
        diff.push('\n');
    }
    if diff.chars().count() > max_chars {
        format!(
            "{}\n...[diff truncated]",
            diff.chars().take(max_chars).collect::<String>().trim_end()
        )
    } else {
        diff
    }
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for index in 0..16 {
            let offset = index * 4;
            w[index] = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    h.iter().map(|value| format!("{value:08x}")).collect()
}

fn file_sha256_hex(path: &Path) -> Result<String, ToolOutcome> {
    fs::read(path)
        .map(|data| sha256_hex(&data))
        .map_err(|err| error(format!("Read failed: {err}")))
}

fn expected_sha256(args: &Map<String, Value>) -> Result<Option<String>, ToolOutcome> {
    let Some(value) = args.get("expected_sha256") else {
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return Err(error(
            "expected_sha256 must be a 64-character hex SHA-256 digest".to_string(),
        ));
    };
    let trimmed = text.trim();
    if trimmed.len() != 64 || !trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(error(
            "expected_sha256 must be a 64-character hex SHA-256 digest".to_string(),
        ));
    }
    Ok(Some(trimmed.to_ascii_lowercase()))
}

fn check_expected_sha256(path: &Path, args: &Map<String, Value>) -> Result<(), ToolOutcome> {
    let Some(expected) = expected_sha256(args)? else {
        return Ok(());
    };
    let actual = file_sha256_hex(path)?;
    if actual != expected {
        return Err(error(format!(
            "expected_sha256 mismatch for {}: expected {expected}, actual {actual}; read the latest file before editing",
            path.display()
        )));
    }
    Ok(())
}

fn expected_sha256_by_path(
    args: &Map<String, Value>,
) -> Result<Option<&Map<String, Value>>, ToolOutcome> {
    let Some(value) = args.get("expected_sha256_by_path") else {
        return Ok(None);
    };
    let Some(map) = value.as_object() else {
        return Err(error(
            "expected_sha256_by_path must be an object".to_string(),
        ));
    };
    for (path, digest) in map {
        if path.trim().is_empty() {
            return Err(error(
                "expected_sha256_by_path keys must be non-empty paths".to_string(),
            ));
        }
        let Some(text) = digest.as_str() else {
            return Err(error(
                "expected_sha256_by_path values must be 64-character hex SHA-256 digests"
                    .to_string(),
            ));
        };
        let trimmed = text.trim();
        if trimmed.len() != 64 || !trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(error(
                "expected_sha256_by_path values must be 64-character hex SHA-256 digests"
                    .to_string(),
            ));
        }
    }
    Ok(Some(map))
}

fn check_expected_sha256_by_path(
    root: &Path,
    target: &Path,
    args: &Map<String, Value>,
) -> Result<(), ToolOutcome> {
    let Some(map) = expected_sha256_by_path(args)? else {
        return Ok(());
    };
    let rel = target
        .strip_prefix(root)
        .map(path_slash)
        .unwrap_or_else(|_| target.display().to_string());
    let absolute = target.display().to_string();
    let expected = map
        .get(&rel)
        .or_else(|| map.get(&absolute))
        .or_else(|| map.get(&path_slash(target)))
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| error(format!("expected_sha256_by_path missing digest for {rel}")))?;
    let actual = file_sha256_hex(target)?;
    if actual != expected {
        return Err(error(format!(
            "expected_sha256_by_path mismatch for {rel}: expected {expected}, actual {actual}; rerun dry_run before applying"
        )));
    }
    Ok(())
}

fn file_state_key(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn refresh_file_access_state(path: &Path) {
    // Mutation/formatting/rollback is not a returned observation. Invalidate
    // the preimage rather than pretending the model saw all retained content.
    forget_file_access_state(path);
}

fn forget_file_access_state(path: &Path) {
    let Some(scope) = current_file_read_scope_key() else { return; };
    let key = (scope, file_state_key(path));
    let state = FILE_ACCESS_STATE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = state.lock() {
        guard.remove(&key);
    }
}

fn range_is_covered(ranges: &[(usize, usize)], requested: (usize, usize)) -> bool {
    requested.0 <= requested.1
        && ranges
            .iter()
            .any(|range| range.0 <= requested.0 && range.1 >= requested.1)
}

fn existing_file_access_snapshot(
    target: &Path,
    args: &Map<String, Value>,
) -> Result<Option<FileAccessState>, ToolOutcome> {
    check_expected_sha256(target, args)?;
    if !target.exists() {
        return Ok(None);
    }
    // Read-before-write evidence is a per-agent-run invariant. Direct library
    // calls have no run scope and therefore cannot safely share a synthetic
    // global read cache. Production tool dispatch always enters the context's
    // SessionPermissionScope before reaching this guard.
    let Some(scope) = current_file_read_scope_key() else { return Ok(None); };
    let key = (scope, file_state_key(target));
    let state = FILE_ACCESS_STATE.get_or_init(|| Mutex::new(HashMap::new()));
    let snapshot = state
        .lock()
        .ok()
        .and_then(|guard| guard.get(&key).cloned())
        .ok_or_else(|| {
            error(format!(
                "File has not been read yet. Read it first before editing: {}",
                target.display()
            ))
        })?;
    let actual_sha = file_sha256_hex(target)?;
    let actual_modified = fs::metadata(target).and_then(|metadata| metadata.modified())
        .map_err(|err| error(format!("Read mtime failed for {}: {err}", target.display())))?;
    if actual_sha != snapshot.version.sha256 || actual_modified != snapshot.version.modified {
        return Err(error(format!(
            "File has been modified since it was read. Read it again before editing: {}",
            target.display()
        )));
    }
    Ok(Some(snapshot))
}

fn guard_existing_file_write(
    target: &Path,
    args: &Map<String, Value>,
    require_full_read: bool,
    line_range: Option<(usize, usize)>,
) -> Result<(), ToolOutcome> {
    let Some(snapshot) = existing_file_access_snapshot(target, args)? else {
        return Ok(());
    };
    let range_ok = line_range
        .map(|range| range_is_covered(&snapshot.ranges, range))
        .unwrap_or(false);
    if require_full_read && !snapshot.full_read {
        return Err(error(format!(
            "File was only partially read. Read the full file before editing: {}",
            target.display()
        )));
    }
    if !require_full_read && !snapshot.full_read && !range_ok {
        return Err(error(format!(
            "File edit range was not read. Read the target range before editing: {}",
            target.display()
        )));
    }
    Ok(())
}

fn no_effect_edit_outcome(target: &Path) -> ToolOutcome {
    error(format!(
        "Edit rejected: the requested replacement would not change {}\n\
edit_no_effect: true\n\
tool_execution_skipped: true\n\
deterministic_failure_category: edit_no_effect\n\
next_action: inspect the exact current text and provide a materially different replacement",
        target.display()
    ))
}

fn guard_exact_edit_existing_file_write(
    target: &Path,
    args: &Map<String, Value>,
    _source: &str,
    _old_text: &str,
) -> Result<(), ToolOutcome> {
    // Exact replacement tools validate old_text against the current file and
    // compare the complete preflight snapshot again while holding the write
    // lock. That provides optimistic concurrency without forcing a redundant
    // full-file read through the tool API.
    check_expected_sha256(target, args)
}

fn exact_match_offsets(source: &str, old_text: &str) -> Vec<usize> {
    source
        .match_indices(old_text)
        .map(|(offset, _)| offset)
        .collect()
}

fn source_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn exact_edit_preflight_failure(
    target: &Path,
    source: &str,
    old_text: &str,
    replace_all: bool,
) -> ToolOutcome {
    let offsets = exact_match_offsets(source, old_text);
    let match_lines = offsets
        .iter()
        .take(8)
        .map(|offset| source_line_for_offset(source, *offset).to_string())
        .collect::<Vec<_>>();
    let anchor = old_text
        .lines()
        .map(str::trim)
        .filter(|line| line.chars().count() >= 4)
        .max_by_key(|line| line.chars().count());
    let candidate_lines = if offsets.is_empty() {
        anchor
            .map(|anchor| {
                source
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| line.contains(anchor))
                    .take(8)
                    .map(|(index, _)| (index + 1).to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let sha256 = file_sha256_hex(target).unwrap_or_else(|_| "unavailable".to_string());
    let reason = if offsets.is_empty() {
        "old_text_not_found"
    } else if !replace_all {
        "old_text_not_unique"
    } else {
        "invalid_exact_edit"
    };
    error(format!(
        "Exact edit preflight failed: {reason}\n\
edit_match_count: {}\n\
match_lines: {}\n\
candidate_anchor_lines: {}\n\
current_sha256: {sha256}\n\
next_action: use the reported line locations to provide a unique old_text, use replace_all=true together with expected_replacements=N only when every occurrence is intended, or use edit_file_lines for a known range; do not repeat unchanged arguments.",
        offsets.len(),
        if match_lines.is_empty() {
            "(none)".to_string()
        } else {
            match_lines.join(",")
        },
        if candidate_lines.is_empty() {
            "(none)".to_string()
        } else {
            candidate_lines.join(",")
        },
    ))
}
