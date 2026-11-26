// SPDX-License-Identifier: Apache-2.0
//! Capability-probed OS sandbox:
//! - Linux: `bwrap` (bubblewrap) with user-namespace probe so unprivileged containers never break
//! - macOS: Apple Seatbelt (`/usr/bin/sandbox-exec`) with strict SBPL workspace/tmp write allowlist
//!   and optional network denial
//! - Windows: clean shell resolution (`pwsh.exe` / `powershell.exe` / Git Bash / `cmd.exe /C`)

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use nola_types::PermissionProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    /// Use native OS sandbox (`bwrap` on Linux, Seatbelt on macOS) if available; otherwise run directly.
    Auto,
    /// Require native OS sandbox (fail closed if unavailable).
    Required,
    /// Disable OS sandbox wrapper.
    Disabled,
}

impl SandboxMode {
    pub fn from_env() -> Self {
        match std::env::var("NOLA_NATIVE_OS_SANDBOX")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "required" | "strict" => Self::Required,
            "off" | "disabled" | "0" | "false" => Self::Disabled,
            _ => Self::Auto,
        }
    }
}

/// Probe once whether `bwrap --unshare-user` actually succeeds on this kernel/container.
pub fn probe_bwrap_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        #[cfg(target_os = "linux")]
        {
            Command::new("bwrap")
                .args([
                    "--ro-bind",
                    "/",
                    "/",
                    "--dev",
                    "/dev",
                    "--proc",
                    "/proc",
                    "--unshare-user",
                    "--",
                    "true",
                ])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    })
}

/// Probe once whether macOS `/usr/bin/sandbox-exec` (Seatbelt) actually succeeds on this host.
pub fn probe_seatbelt_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            if !Path::new("/usr/bin/sandbox-exec").is_file() {
                return false;
            }
            Command::new("/usr/bin/sandbox-exec")
                .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    })
}

/// Build an Apple Seatbelt SBPL profile allowing read-only system access + read/write
/// inside `workspace_root`, `/tmp`, and `TMPDIR`, protecting `.git/hooks` + `.git/config`
/// under non-AllowAll profiles, and toggling `(allow network*)` vs `(deny network*)`.
///
/// All paths are JSON-encoded literals so spaces/quotes cannot inject SBPL syntax.
pub fn build_seatbelt_profile(
    workspace_root: &Path,
    profile: PermissionProfile,
    network_allowed: bool,
) -> Result<String, String> {
    use std::collections::BTreeSet;

    let encode_path = |p: &Path| -> Result<String, String> {
        let s = p
            .to_str()
            .ok_or_else(|| format!("sandbox path is not valid UTF-8: {}", p.display()))?;
        serde_json::to_string(s).map_err(|e| format!("failed to encode SBPL path: {e}"))
    };

    // Start from default allow (preserves process/mach/sysctl/read access needed by bash/python/cargo)
    // and lock down all filesystem writes globally, then re-allow only workspace + tmp + stdio devices.
    // In Apple Seatbelt SBPL, the LAST matching rule wins.
    let mut sbpl = String::from("(version 1)\n(allow default)\n(deny file-write*)\n");

    let mut writable_dirs: BTreeSet<std::path::PathBuf> = BTreeSet::new();
    writable_dirs.insert(std::path::PathBuf::from("/tmp"));
    writable_dirs.insert(std::path::PathBuf::from("/private/tmp"));
    let sys_tmp = std::env::temp_dir();
    writable_dirs.insert(sys_tmp.clone());
    if let Ok(canon_tmp) = std::fs::canonicalize(&sys_tmp) {
        writable_dirs.insert(canon_tmp);
    }
    if let Ok(tmpdir_env) = std::env::var("TMPDIR") {
        let td = std::path::PathBuf::from(tmpdir_env);
        if !td.as_os_str().is_empty() {
            writable_dirs.insert(td.clone());
            if let Ok(canon) = std::fs::canonicalize(&td) {
                writable_dirs.insert(canon);
            }
        }
    }
    writable_dirs.insert(workspace_root.to_path_buf());
    if let Ok(canon_ws) = std::fs::canonicalize(workspace_root) {
        writable_dirs.insert(canon_ws);
    }

    for dir in &writable_dirs {
        sbpl.push_str(&format!(
            "(allow file-write* (subpath {}))\n",
            encode_path(dir)?
        ));
    }

    // Standard device nodes required for shell redirections and terminals.
    sbpl.push_str(
        "(allow file-write* \
            (literal \"/dev/null\") \
            (literal \"/dev/zero\") \
            (literal \"/dev/tty\") \
            (literal \"/dev/ptmx\") \
            (literal \"/dev/dtracehelper\") \
            (regex #\"^/dev/ttys[0-9]+$\"))\n",
    );

    // Protect `.git/hooks` and `.git/config` under non-AllowAll profiles (including ancestor unlink).
    if profile != PermissionProfile::AllowAll {
        let mut protected: BTreeSet<std::path::PathBuf> = BTreeSet::new();
        for base in [
            Some(workspace_root.to_path_buf()),
            std::fs::canonicalize(workspace_root).ok(),
        ]
        .into_iter()
        .flatten()
        {
            let hooks = base.join(".git/hooks");
            if hooks.exists() {
                protected.insert(hooks);
            }
            let cfg = base.join(".git/config");
            if cfg.exists() {
                protected.insert(cfg);
            }
        }
        let mut ancestors: BTreeSet<std::path::PathBuf> = BTreeSet::new();
        for p in &protected {
            let enc = encode_path(p)?;
            sbpl.push_str(&format!("(deny file-write* (subpath {enc}))\n"));
            sbpl.push_str(&format!("(deny file-link (subpath {enc}))\n"));
            ancestors.extend(p.ancestors().map(Path::to_path_buf));
        }
        for anc in ancestors {
            if !anc.as_os_str().is_empty() {
                let enc = encode_path(&anc)?;
                sbpl.push_str(&format!("(deny file-write-unlink (literal {enc}))\n"));
            }
        }
    }

    if network_allowed {
        sbpl.push_str("(allow network*)\n");
    } else {
        sbpl.push_str("(deny network*)\n");
    }

    Ok(sbpl)
}

#[cfg(target_os = "windows")]
fn find_windows_program(name: &str) -> Option<std::path::PathBuf> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn build_unsandboxed_shell_command(shell_cmd: &str, cwd: &Path) -> Command {
    // Prefer PowerShell 7 (`pwsh.exe`), then Windows PowerShell (`powershell.exe`),
    // then Git Bash (`bash.exe`), and finally fall back to `cmd.exe /C`.
    if let Some(pwsh) = find_windows_program("pwsh.exe") {
        let mut cmd = Command::new(pwsh);
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", shell_cmd])
            .current_dir(cwd);
        return cmd;
    }
    if let Some(ps) = find_windows_program("powershell.exe") {
        let mut cmd = Command::new(ps);
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", shell_cmd])
            .current_dir(cwd);
        return cmd;
    }
    for git_bash in [
        Path::new(r"C:\Program Files\Git\bin\bash.exe"),
        Path::new(r"C:\Program Files (x86)\Git\bin\bash.exe"),
    ] {
        if git_bash.is_file() {
            let mut cmd = Command::new(git_bash);
            cmd.args(["-lc", shell_cmd]).current_dir(cwd);
            return cmd;
        }
    }
    if let Some(bash) = find_windows_program("bash.exe") {
        let mut cmd = Command::new(bash);
        cmd.args(["-lc", shell_cmd]).current_dir(cwd);
        return cmd;
    }
    let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
    let mut cmd = Command::new(comspec);
    cmd.args(["/C", shell_cmd]).current_dir(cwd);
    cmd
}

#[cfg(not(target_os = "windows"))]
fn build_unsandboxed_shell_command(shell_cmd: &str, cwd: &Path) -> Command {
    let mut cmd = Command::new("/bin/bash");
    cmd.arg("-lc").arg(shell_cmd).current_dir(cwd);
    cmd
}

/// Build a shell `Command` wrapped in the host OS sandbox when appropriate:
/// - Linux: `bwrap` binding ONLY `workspace_root` (never arbitrary `cwd`), isolating `/tmp` via
///   `--tmpfs`, and protecting `.git/hooks` + `.git/config` under non-AllowAll profiles.
/// - macOS: `/usr/bin/sandbox-exec -p <SBPL> /bin/bash -lc <cmd>`.
/// - Windows: native shell resolution (`pwsh.exe` / `powershell.exe` / Git Bash / `cmd.exe /C`).
pub fn build_shell_command(
    shell_cmd: &str,
    workspace_root: &Path,
    cwd: &Path,
    mode: SandboxMode,
    profile: PermissionProfile,
    network_allowed: bool,
) -> Result<Command, String> {
    let effective_mode = match mode {
        SandboxMode::Auto => SandboxMode::from_env(),
        other => other,
    };

    #[cfg(target_os = "macos")]
    {
        let use_seatbelt = match effective_mode {
            SandboxMode::Disabled => false,
            SandboxMode::Required => {
                if !probe_seatbelt_available() {
                    return Err(
                        "NOLA_NATIVE_OS_SANDBOX=required, but macOS /usr/bin/sandbox-exec probe failed on this host"
                            .into(),
                    );
                }
                true
            }
            SandboxMode::Auto => {
                probe_seatbelt_available() && profile != PermissionProfile::AllowAll
            }
        };

        if !use_seatbelt {
            return Ok(build_unsandboxed_shell_command(shell_cmd, cwd));
        }

        let sbpl = build_seatbelt_profile(workspace_root, profile, network_allowed)?;
        let mut cmd = Command::new("/usr/bin/sandbox-exec");
        cmd.args(["-p", &sbpl, "/bin/bash", "-lc", shell_cmd])
            .current_dir(cwd);
        return Ok(cmd);
    }

    #[cfg(target_os = "linux")]
    {
        let use_bwrap = match effective_mode {
            SandboxMode::Disabled => false,
            SandboxMode::Required => {
                if !probe_bwrap_available() {
                    return Err(
                        "NOLA_NATIVE_OS_SANDBOX=required, but bwrap user-namespace probe failed on this host"
                            .into(),
                    );
                }
                true
            }
            SandboxMode::Auto => probe_bwrap_available() && profile != PermissionProfile::AllowAll,
        };

        if !use_bwrap {
            return Ok(build_unsandboxed_shell_command(shell_cmd, cwd));
        }

        let mut cmd = Command::new("bwrap");
        cmd.args(["--die-with-parent", "--unshare-user", "--ro-bind", "/", "/"]);
        cmd.args(["--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp"]);
        if Path::new("/dev/shm").exists() {
            cmd.args(["--bind", "/dev/shm", "/dev/shm"]);
        }
        if Path::new("/dev/pts").exists() {
            cmd.args(["--dev-bind", "/dev/pts", "/dev/pts"]);
        }
        let ws_str = workspace_root.to_string_lossy().into_owned();
        let cwd_str = cwd.to_string_lossy().into_owned();
        cmd.args(["--bind", &ws_str, &ws_str]);
        if profile != PermissionProfile::AllowAll {
            let git_hooks = workspace_root.join(".git/hooks");
            if git_hooks.exists() {
                let s = git_hooks.to_string_lossy().into_owned();
                cmd.args(["--ro-bind", &s, &s]);
            }
            let git_config = workspace_root.join(".git/config");
            if git_config.exists() {
                let s = git_config.to_string_lossy().into_owned();
                cmd.args(["--ro-bind", &s, &s]);
            }
        }
        if !network_allowed {
            cmd.arg("--unshare-net");
        }
        cmd.args(["--chdir", &cwd_str, "--", "/bin/bash", "-lc", shell_cmd]);
        return Ok(cmd);
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (workspace_root, profile, network_allowed);
        if effective_mode == SandboxMode::Required {
            return Err(
                "NOLA_NATIVE_OS_SANDBOX=required, but no native OS sandbox backend is available on this platform"
                    .into(),
            );
        }
        Ok(build_unsandboxed_shell_command(shell_cmd, cwd))
    }
}
