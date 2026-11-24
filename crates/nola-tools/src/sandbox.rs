// SPDX-License-Identifier: Apache-2.0
//! Capability-probed OS sandbox (`bwrap` on Linux) that NEVER breaks inside
//! unprivileged Docker/Harbor containers lacking `CLONE_NEWUSER`.

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use nola_types::PermissionProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    /// Use native bwrap sandbox if the host kernel allows user namespaces; otherwise run directly.
    Auto,
    /// Require native bwrap sandbox (fail closed if unavailable).
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

/// Build a shell `Command` wrapped in `bwrap` when appropriate, preserving `/dev/shm`
/// and `/dev/pts` and NOT locking `.git` under `PermissionProfile::AllowAll`.
pub fn build_shell_command(
    shell_cmd: &str,
    cwd: &Path,
    mode: SandboxMode,
    profile: PermissionProfile,
    network_allowed: bool,
) -> Result<Command, String> {
    let effective_mode = match mode {
        SandboxMode::Auto => SandboxMode::from_env(),
        other => other,
    };

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
        let mut cmd = Command::new("/bin/bash");
        cmd.arg("-lc").arg(shell_cmd).current_dir(cwd);
        return Ok(cmd);
    }

    let mut cmd = Command::new("bwrap");
    cmd.args(["--die-with-parent", "--unshare-user", "--ro-bind", "/", "/"]);
    cmd.args(["--dev", "/dev", "--proc", "/proc"]);
    if Path::new("/dev/shm").exists() {
        cmd.args(["--bind", "/dev/shm", "/dev/shm"]);
    }
    if Path::new("/dev/pts").exists() {
        cmd.args(["--dev-bind", "/dev/pts", "/dev/pts"]);
    }
    if Path::new("/tmp").exists() {
        cmd.args(["--bind", "/tmp", "/tmp"]);
    }
    let cwd_str = cwd.to_string_lossy().into_owned();
    cmd.args(["--bind", &cwd_str, &cwd_str]);
    // Under cautious profiles, protect `.git/hooks` from silent modification;
    // under AllowAll (benchmarks/containers), never lock `.git`!
    let git_hooks = cwd.join(".git/hooks");
    if profile != PermissionProfile::AllowAll && git_hooks.exists() {
        let hooks_str = git_hooks.to_string_lossy().into_owned();
        cmd.args(["--ro-bind", &hooks_str, &hooks_str]);
    }
    if !network_allowed {
        cmd.arg("--unshare-net");
    }
    cmd.args(["--chdir", &cwd_str, "--", "/bin/bash", "-lc", shell_cmd]);
    Ok(cmd)
}
