// SPDX-License-Identifier: Apache-2.0
// Git object I/O for selected-resource operations, not workflow admission.

fn git_process_preservation(
    cwd: &Path,
    allowed_dirs: &[PathBuf],
) -> Result<ProcessPreservation, String> {
    let mut guard = ProcessPreservation::default();
    if unrestricted_resource_policy() { return Ok(guard); }
    let mut roots = std::collections::BTreeSet::new();
    for directory in std::iter::once(cwd).chain(allowed_dirs.iter().map(PathBuf::as_path)) {
        if let Some(root) = directory
            .ancestors()
            .find(|path| path.join(".git").exists())
        {
            roots.insert(root.canonicalize().map_err(|error| error.to_string())?);
        }
    }
    for root in roots {
        // Staged objects cannot safely be consumed by an opaque operation.
        // Explicit selection belongs to the Git adapter, not command parsing.
        if git_command_output(&root, &["diff", "--cached", "--name-only", "-z"])?.is_empty() {
            continue;
        }
        // Linked worktrees have a .git pointer file as well as shared metadata.
        // Changing the pointer must not redirect a protected repository.
        guard.read_only.insert(
            root.join(".git")
                .canonicalize()
                .map_err(|error| error.to_string())?,
        );
        for selector in ["--absolute-git-dir", "--git-common-dir"] {
            let path = PathBuf::from(git_commit_text(&root, &["rev-parse", selector])?);
            let path = if path.is_absolute() {
                path
            } else {
                root.join(path)
            };
            guard
                .read_only
                .insert(path.canonicalize().map_err(|error| error.to_string())?);
        }
        guard.recovery_capabilities.insert("git_commit".into());
    }
    if guard.active() {
        guard
            .environment
            .insert("GIT_OPTIONAL_LOCKS".into(), "0".into());
        guard.guidance.push("Existing staged Git state is protected. For selected-file commits, inspect the git_commit schema and supply its explicit selection; it preserves unrelated staged entries. Do not clear, replace, or move the existing index.".into());
    }
    Ok(guard)
}

fn git_operation_command(root: &Path) -> Command {
    // Host-owned tool boundary. Never resolve git/helper executables through
    // a repository-controlled PATH or inherit Git config/loader overrides.
    #[cfg(not(windows))]
    let program = "/usr/bin/git";
    #[cfg(windows)]
    let program = r"C:\Program Files\Git\cmd\git.exe";
    let mut command = Command::new(program);
    command.env_clear();
    #[cfg(not(windows))]
    command.env("PATH", "/usr/bin:/bin");
    #[cfg(windows)]
    command.env("PATH", r"C:\Windows\System32;C:\Windows;C:\Program Files\Git\cmd").env("SystemRoot", r"C:\Windows");
    // Retain only the Host's user-config location, not runtime credential,
    // executable, index, worktree or GIT_CONFIG_* environment overrides.
    for key in ["HOME", "XDG_CONFIG_HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(key) {command.env(key, value);}
    }
    command.current_dir(root).arg("--literal-pathspecs")
        .args(["-c", "core.fsmonitor=false"])
        .env("LANG", "C").env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

fn git_command_output(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    git_operation_output(git_operation_command(root), arguments)
}

fn git_private_index_output(
    root: &Path,
    index: &Path,
    arguments: &[&str],
) -> Result<Vec<u8>, String> {
    let mut command = git_operation_command(root);
    command.env("GIT_INDEX_FILE", git_path_argument(index)?);
    git_operation_output(command, arguments)
}

fn git_operation_output(mut command: Command, arguments: &[&str]) -> Result<Vec<u8>, String> {
    let output = command
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Git operation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}

fn git_index_entries(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let bytes = git_command_output(root, &["ls-files", "--stage", "-z"])?;
    let mut entries = BTreeMap::new();
    for entry in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let entry = std::str::from_utf8(entry)
            .map_err(|_| "Non-UTF8 index path is not representable by this tool")?;
        let (object, path) = entry.split_once('\t').ok_or("Malformed index entry")?;
        if object.split_whitespace().nth(2) != Some("0") {
            return Err(
                "Index contains unresolved conflicts; this operation does not resolve merges"
                    .into(),
            );
        }
        entries.insert(path.into(), object.into());
    }
    Ok(entries)
}

fn git_path_argument(path: &Path) -> Result<String, String> {
    path.to_str().ok_or("Git path must be UTF-8")?;
    Ok(crate::platform_paths::shell_path(path))
}
