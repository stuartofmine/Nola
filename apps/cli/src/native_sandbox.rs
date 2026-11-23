// SPDX-License-Identifier: Apache-2.0
//! Zero-dependency OS-level process sandbox (macOS Seatbelt `sandbox-exec`, Linux `bwrap`)
//! and shared child-process environment scrubbing helpers.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const SENSITIVE_ENV_KEYS: &[&str] = &[
    "NOLA_MOBILE_PAIRING_SECRET",
    "NOLA_CORE_TOOL_POLICY_RECEIPT",
    "NOLA_WEB_TOKEN",
    "NOLA_API_KEY",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "DEEPSEEK_API_KEY",
    "OPENROUTER_API_KEY",
    "XAI_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "BASH_ENV",
    "ENV",
];

const DEFAULT_SENSITIVE_READ_SUBPATHS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".docker",
    ".nola",
    "Library/Application Support/Nola",
    "Library/Keychains",
    ".git-credentials",
    ".netrc",
    ".config/gh",
    ".pgpass",
    ".zsh_history",
    ".bash_history",
];

const DEFAULT_SENSITIVE_WRITE_SUBPATHS: &[&str] = &[
    "Library/LaunchAgents",
    ".zshenv",
    ".zprofile",
    ".bash_profile",
    ".bashrc",
    ".profile",
];

/// Canonicalizes a path for OS sandbox rules, resolving symlinks on the longest
/// existing ancestor when the target path does not yet exist (e.g. macOS
/// `/var` -> `/private/var` and `/tmp` -> `/private/tmp`).
pub(crate) fn canonicalize_for_sandbox(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let mut current = path.to_path_buf();
    let mut tail = Vec::new();
    while let Some(name) = current.file_name() {
        tail.push(name.to_os_string());
        if !current.pop() {
            break;
        }
        if let Ok(mut canonical_ancestor) = current.canonicalize() {
            for part in tail.into_iter().rev() {
                canonical_ancestor.push(part);
            }
            return canonical_ancestor;
        }
    }
    // Normalize lexical `.` components without hitting the filesystem if no ancestor existed.
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if normalized.as_os_str().is_empty() {
        path.to_path_buf()
    } else {
        normalized
    }
}

/// Removes sensitive host secrets, API keys, and shell startup hooks from a child
/// process environment.
pub(crate) fn scrub_sensitive_env(cmd: &mut Command) {
    for key in SENSITIVE_ENV_KEYS {
        cmd.env_remove(key);
    }
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("BASH_FUNC_") {
            cmd.env_remove(name);
        }
    }
}

/// Returns true when the explicit debugging/compatibility escape hatch
/// `NOLA_NATIVE_OS_SANDBOX=off` is set.
pub(crate) fn native_os_sandbox_disabled_by_env() -> bool {
    std::env::var("NOLA_NATIVE_OS_SANDBOX")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| matches!(value.as_str(), "off" | "0" | "false" | "disabled"))
}

/// Returns true when a native OS sandbox backend is available on this platform
/// and not disabled via `NOLA_NATIVE_OS_SANDBOX=off`.
pub(crate) fn native_os_sandbox_available() -> bool {
    if native_os_sandbox_disabled_by_env() {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        Path::new("/usr/bin/sandbox-exec").is_file()
    }
    #[cfg(target_os = "linux")]
    {
        find_in_path("bwrap").is_some()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

#[cfg(target_os = "linux")]
fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn push_unique_canonical(target: &mut Vec<PathBuf>, path: &Path, reject_root: bool) {
    let canonical = canonicalize_for_sandbox(path);
    if reject_root && canonical.parent().is_none() {
        return;
    }
    if !target.iter().any(|existing| existing == &canonical) {
        target.push(canonical);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeOsSandboxPolicy {
    pub workspace_root: PathBuf,
    pub allowed_write_roots: Vec<PathBuf>,
    pub allowed_read_roots: Vec<PathBuf>,
    pub deny_read: Vec<PathBuf>,
    pub deny_write: Vec<PathBuf>,
    pub allow_network: bool,
}

impl NativeOsSandboxPolicy {
    /// Builds a default workspace-scoped OS sandbox policy with canonicalized
    /// writable roots, default sensitive read denials, and default sensitive write denials.
    pub(crate) fn for_workspace(workspace_root: &Path, extra_write_roots: &[PathBuf]) -> Self {
        let workspace = canonicalize_for_sandbox(workspace_root);
        let mut allowed_write_roots = Vec::new();
        push_unique_canonical(&mut allowed_write_roots, &workspace, true);
        push_unique_canonical(&mut allowed_write_roots, &std::env::temp_dir(), true);
        #[cfg(unix)]
        push_unique_canonical(&mut allowed_write_roots, Path::new("/tmp"), true);
        if let Some(tmpdir) = std::env::var_os("TMPDIR").filter(|value| !value.is_empty()) {
            push_unique_canonical(&mut allowed_write_roots, Path::new(&tmpdir), true);
        }
        for extra in extra_write_roots {
            push_unique_canonical(&mut allowed_write_roots, extra, true);
        }

        let mut allowed_read_roots = Vec::new();
        for root in &allowed_write_roots {
            push_unique_canonical(&mut allowed_read_roots, root, true);
        }

        let mut deny_read = Vec::new();
        let mut deny_write = Vec::new();
        push_unique_canonical(&mut deny_write, &workspace.join(".git"), true);

        if let Some(home) = user_home_dir() {
            for rel in DEFAULT_SENSITIVE_READ_SUBPATHS {
                push_unique_canonical(&mut deny_read, &home.join(rel), true);
            }
            for rel in DEFAULT_SENSITIVE_WRITE_SUBPATHS {
                push_unique_canonical(&mut deny_write, &home.join(rel), true);
            }
        }

        Self {
            workspace_root: workspace,
            allowed_write_roots,
            allowed_read_roots,
            deny_read,
            deny_write,
            allow_network: true,
        }
    }

    fn canonicalized(&self) -> Self {
        let workspace_root = canonicalize_for_sandbox(&self.workspace_root);
        let mut allowed_write_roots = Vec::new();
        push_unique_canonical(&mut allowed_write_roots, &workspace_root, true);
        for root in &self.allowed_write_roots {
            push_unique_canonical(&mut allowed_write_roots, root, true);
        }
        let mut allowed_read_roots = Vec::new();
        for root in &self.allowed_read_roots {
            push_unique_canonical(&mut allowed_read_roots, root, true);
        }
        let mut deny_read = Vec::new();
        for path in &self.deny_read {
            push_unique_canonical(&mut deny_read, path, true);
        }
        let mut deny_write = Vec::new();
        for path in &self.deny_write {
            push_unique_canonical(&mut deny_write, path, true);
        }
        Self {
            workspace_root,
            allowed_write_roots,
            allowed_read_roots,
            deny_read,
            deny_write,
            allow_network: self.allow_network,
        }
    }
}

#[cfg(target_os = "macos")]
fn sbpl_escape_path(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("Sandbox path is not valid UTF-8: {}", path.display()))?;
    if text.contains('\0') || text.contains('\n') || text.contains('\r') {
        return Err("Sandbox path contains forbidden control characters".into());
    }
    Ok(text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Generates an Apple Seatbelt SBPL `(version 1)` profile.
/// Note: in Seatbelt SBPL, the LAST matching rule wins.
#[cfg(target_os = "macos")]
pub(crate) fn build_macos_seatbelt_profile(policy: &NativeOsSandboxPolicy) -> Result<String, String> {
    let policy = policy.canonicalized();
    let mut profile = String::new();
    profile.push_str("(version 1)\n");
    profile.push_str("(allow default)\n");
    // Allow reads by default, then deny sensitive read paths AFTER so last rule wins.
    profile.push_str("(allow file-read*)\n");
    for path in &policy.deny_read {
        let escaped = sbpl_escape_path(path)?;
        profile.push_str(&format!(
            "(deny file-read-data (subpath \"{escaped}\") (literal \"{escaped}\"))\n"
        ));
    }
    // Default-deny all writes, then allow workspace / temp / device writes,
    // then deny sensitive write paths LAST so last rule wins.
    profile.push_str("(deny file-write*)\n");
    profile.push_str("(allow file-write*\n");
    for root in &policy.allowed_write_roots {
        let escaped = sbpl_escape_path(root)?;
        profile.push_str(&format!("    (subpath \"{escaped}\")\n"));
    }
    profile.push_str("    (literal \"/dev/null\")\n");
    profile.push_str("    (literal \"/dev/tty\")\n");
    profile.push_str("    (literal \"/dev/dtracehelper\")\n");
    profile.push_str("    (literal \"/dev/ptmx\")\n");
    profile.push_str("    (regex #\"^/dev/ttys[0-9]+$\")\n");
    profile.push_str(")\n");
    for path in &policy.deny_write {
        let escaped = sbpl_escape_path(path)?;
        profile.push_str(&format!(
            "(deny file-write* (subpath \"{escaped}\") (literal \"{escaped}\"))\n"
        ));
    }
    if !policy.allow_network {
        profile.push_str("(deny network-outbound)\n");
        profile.push_str("(allow network-outbound (local ip \"localhost:*\"))\n");
    }
    Ok(profile)
}

fn copy_command_metadata(source: &Command, dest: &mut Command) {
    for (key, value) in source.get_envs() {
        match value {
            Some(v) => {
                dest.env(key, v);
            }
            None => {
                dest.env_remove(key);
            }
        }
    }
    if let Some(dir) = source.get_current_dir() {
        dest.current_dir(dir);
    }
}

/// Wraps an already-configured `Command` (program, args, env, current_dir) inside
/// the native OS sandbox (`sandbox-exec` on macOS, `bwrap` on Linux).
/// Callers should attach stdio pipes and process-group settings to the returned `Command`.
pub(crate) fn wrap_command_in_native_sandbox(
    policy: &NativeOsSandboxPolicy,
    original: &Command,
) -> Result<Command, String> {
    let program = original.get_program();
    let args: Vec<&OsStr> = original.get_args().collect();

    #[cfg(target_os = "macos")]
    {
        let profile = build_macos_seatbelt_profile(policy)?;
        let mut wrapped = Command::new("/usr/bin/sandbox-exec");
        wrapped.arg("-p").arg(profile).arg(program).args(args);
        copy_command_metadata(original, &mut wrapped);
        Ok(wrapped)
    }

    #[cfg(target_os = "linux")]
    {
        let bwrap = find_in_path("bwrap")
            .ok_or_else(|| "bwrap is not available in PATH".to_string())?;
        let policy = policy.canonicalized();
        let mut wrapped = Command::new(bwrap);
        wrapped.args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"]);
        for root in &policy.allowed_write_roots {
            if root.exists() {
                wrapped.arg("--bind").arg(root).arg(root);
            }
        }
        for path in &policy.deny_read {
            if path.is_dir() {
                wrapped.arg("--tmpfs").arg(path);
            }
        }
        for path in &policy.deny_write {
            if path.exists() {
                wrapped.arg("--ro-bind").arg(path).arg(path);
            }
        }
        if !policy.allow_network {
            wrapped.arg("--unshare-net");
        }
        wrapped.arg("--die-with-parent").arg("--").arg(program).args(args);
        copy_command_metadata(original, &mut wrapped);
        Ok(wrapped)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (policy, program, args);
        Err("Native OS sandbox is unsupported on this platform".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_for_sandbox_resolves_nonexistent_children_through_existing_ancestors() {
        let dir = tempfile::tempdir().unwrap();
        let canonical_dir = dir.path().canonicalize().unwrap();
        let missing = dir.path().join("nested").join("future.txt");
        assert_eq!(
            canonicalize_for_sandbox(&missing),
            canonical_dir.join("nested").join("future.txt")
        );
    }

    #[test]
    fn scrub_sensitive_env_removes_api_keys_tokens_and_shell_hooks() {
        let mut cmd = Command::new("/bin/echo");
        scrub_sensitive_env(&mut cmd);
        let removed: Vec<&OsStr> = cmd
            .get_envs()
            .filter_map(|(k, v)| v.is_none().then_some(k))
            .collect();
        for required in [
            "NOLA_MOBILE_PAIRING_SECRET",
            "NOLA_CORE_TOOL_POLICY_RECEIPT",
            "NOLA_WEB_TOKEN",
            "NOLA_API_KEY",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "BASH_ENV",
            "ENV",
        ] {
            assert!(
                removed.iter().any(|key| *key == OsStr::new(required)),
                "expected {required} to be scrubbed"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_seatbelt_enforces_workspace_write_sensitive_read_and_git_write_boundaries() {
        assert!(
            Path::new("/usr/bin/sandbox-exec").is_file(),
            "/usr/bin/sandbox-exec must exist on macOS"
        );
        let base = tempfile::tempdir().unwrap();
        let workspace = base.path().join("workspace");
        let outside = base.path().join("outside");
        let sensitive = base.path().join("sensitive");
        let git_hooks = workspace.join(".git").join("hooks");
        std::fs::create_dir_all(&git_hooks).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(&sensitive).unwrap();
        let secret_file = sensitive.join("secret.txt");
        std::fs::write(&secret_file, "top-secret-fixture").unwrap();

        let policy = NativeOsSandboxPolicy {
            workspace_root: workspace.clone(),
            allowed_write_roots: vec![workspace.clone()],
            allowed_read_roots: vec![workspace.clone()],
            deny_read: vec![sensitive.clone()],
            deny_write: vec![workspace.join(".git")],
            allow_network: false,
        };

        // 1. Writing inside workspace_root succeeds.
        let ok_file = workspace.join("allowed.txt");
        let mut cmd_ok = Command::new("/bin/sh");
        cmd_ok
            .arg("-c")
            .arg(format!("printf 'ok' > '{}'", ok_file.display()))
            .current_dir(&workspace);
        let out_ok = wrap_command_in_native_sandbox(&policy, &cmd_ok)
            .unwrap()
            .output()
            .unwrap();
        assert!(
            out_ok.status.success(),
            "write inside workspace should succeed, stderr: {}",
            String::from_utf8_lossy(&out_ok.stderr)
        );
        assert_eq!(std::fs::read_to_string(&ok_file).unwrap(), "ok");

        // 2. Writing outside workspace_root fails.
        let blocked_file = outside.join("blocked.txt");
        let mut cmd_outside = Command::new("/bin/sh");
        cmd_outside
            .arg("-c")
            .arg(format!("printf 'nope' > '{}'", blocked_file.display()))
            .current_dir(&workspace);
        let out_outside = wrap_command_in_native_sandbox(&policy, &cmd_outside)
            .unwrap()
            .output()
            .unwrap();
        assert!(
            !out_outside.status.success(),
            "write outside workspace must be denied by seatbelt"
        );
        assert!(!blocked_file.exists());

        // 3. Reading a denied sensitive directory fails.
        let mut cmd_read = Command::new("/bin/cat");
        cmd_read.arg(&secret_file).current_dir(&workspace);
        let out_read = wrap_command_in_native_sandbox(&policy, &cmd_read)
            .unwrap()
            .output()
            .unwrap();
        assert!(
            !out_read.status.success(),
            "reading denied sensitive directory must fail"
        );
        assert!(
            !String::from_utf8_lossy(&out_read.stdout).contains("top-secret-fixture"),
            "secret content must not leak"
        );

        // 4. Writing into <workspace>/.git/hooks/pre-commit fails.
        let hook_file = git_hooks.join("pre-commit");
        let mut cmd_git = Command::new("/bin/sh");
        cmd_git
            .arg("-c")
            .arg(format!("printf '#!/bin/sh\n' > '{}'", hook_file.display()))
            .current_dir(&workspace);
        let out_git = wrap_command_in_native_sandbox(&policy, &cmd_git)
            .unwrap()
            .output()
            .unwrap();
        assert!(
            !out_git.status.success(),
            "writing into .git/hooks/pre-commit must be denied by seatbelt"
        );
        assert!(!hook_file.exists());

        // 5. Default workspace policy permits writing into /tmp inside Seatbelt (consistent with write_file).
        let workspace_policy = NativeOsSandboxPolicy::for_workspace(&workspace, &[]);
        let tmp_probe = PathBuf::from(format!("/tmp/nola-seatbelt-probe-{}", std::process::id()));
        let mut cmd_tmp = Command::new("/bin/sh");
        cmd_tmp
            .arg("-c")
            .arg(format!("printf 'ok' > '{}'", tmp_probe.display()))
            .current_dir(&workspace);
        let out_tmp = wrap_command_in_native_sandbox(&workspace_policy, &cmd_tmp)
            .unwrap()
            .output()
            .unwrap();
        assert!(
            out_tmp.status.success(),
            "writing into /tmp must be permitted by seatbelt: {}",
            String::from_utf8_lossy(&out_tmp.stderr)
        );
        assert_eq!(std::fs::read_to_string(&tmp_probe).unwrap(), "ok");
        let _ = std::fs::remove_file(&tmp_probe);
    }
}
