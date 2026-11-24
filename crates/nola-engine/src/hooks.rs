// SPDX-License-Identifier: Apache-2.0
//! Workspace-trust-gated lifecycle hooks (`PreToolUse`, `PostToolUse`, `SessionStart`, `Stop`).
//! Guarantees clean child process termination on timeout (no detached Mutex deadlock!).

use std::fs;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nola_types::{CommandHookConfig, HookDecision, HookEvent};
use serde_json::{json, Value};

const MAX_HOOK_PIPE_BYTES: u64 = 256 * 1024;

#[derive(Debug, Default, Clone)]
pub struct HookRunner {
    hooks: Vec<CommandHookConfig>,
    workspace_root: PathBuf,
}

impl HookRunner {
    /// Load user hooks (`~/.nola/hooks.json`) and, ONLY when `workspace_trusted == true`,
    /// project hooks (`<ws>/.nola/hooks.json`).
    pub fn load(workspace_root: &Path, user_home: Option<&Path>, workspace_trusted: bool) -> Self {
        let mut hooks = Vec::new();
        if let Some(home) = user_home {
            hooks.extend(read_hooks_file(&home.join(".nola/hooks.json")));
        }
        if workspace_trusted {
            hooks.extend(read_hooks_file(&workspace_root.join(".nola/hooks.json")));
        }
        Self {
            hooks,
            workspace_root: workspace_root.to_path_buf(),
        }
    }

    pub fn from_configs(workspace_root: PathBuf, hooks: Vec<CommandHookConfig>) -> Self {
        Self {
            hooks,
            workspace_root,
        }
    }

    pub fn run_event(
        &self,
        event: HookEvent,
        tool_name: &str,
        payload: &Value,
    ) -> Vec<HookDecision> {
        let mut decisions = Vec::new();
        let mut current_payload = payload.clone();
        for hook in &self.hooks {
            if hook.event != event || !matcher_matches(&hook.matcher, tool_name) {
                continue;
            }
            let input_json = json!({
                "event": event,
                "tool_name": tool_name,
                "workspace_root": self.workspace_root.display().to_string(),
                "payload": current_payload
            });
            let decision = execute_single_hook(hook, &self.workspace_root, &input_json);
            if let HookDecision::ModifyInput { ref input } = decision {
                current_payload = input.clone();
            }
            let is_block = matches!(decision, HookDecision::Block { .. });
            decisions.push(decision);
            if is_block {
                break;
            }
        }
        decisions
    }
}

fn read_hooks_file(path: &Path) -> Vec<CommandHookConfig> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn matcher_matches(matcher: &str, tool_name: &str) -> bool {
    let trimmed = matcher.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return true;
    }
    trimmed
        .split('|')
        .map(str::trim)
        .any(|part| part == "*" || part == tool_name)
}

fn read_capped(mut reader: impl Read) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = reader.by_ref().take(MAX_HOOK_PIPE_BYTES).read_to_end(&mut buf);
    // Drain any excess bytes so child never stalls on SIGPIPE / full pipe while exiting.
    let mut discard = [0u8; 4096];
    while let Ok(n) = reader.read(&mut discard) {
        if n == 0 {
            break;
        }
    }
    buf
}

fn execute_single_hook(
    hook: &CommandHookConfig,
    workspace_root: &Path,
    input_json: &Value,
) -> HookDecision {
    let mut cmd = Command::new("/bin/bash");
    cmd.arg("-lc")
        .arg(&hook.command)
        .current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return if hook.fail_closed {
                HookDecision::Block {
                    reason: format!("hook failed to spawn (fail_closed): {e}"),
                }
            } else {
                HookDecision::Allow
            };
        }
    };
    let pgid = child.id() as libc::pid_t;

    // Write stdin and drain stdout/stderr concurrently to avoid pipe deadlocks on large I/O!
    let stdin_handle = child.stdin.take().map(|mut stdin| {
        let bytes = serde_json::to_vec(input_json).unwrap_or_default();
        thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        })
    });
    let stdout_handle = child.stdout.take().map(|out| thread::spawn(move || read_capped(out)));
    let stderr_handle = child.stderr.take().map(|err| thread::spawn(move || read_capped(err)));

    let timeout = Duration::from_millis(hook.timeout_ms.clamp(100, 60_000));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    unsafe {
                        let _ = libc::kill(-pgid, libc::SIGKILL);
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    if let Some(h) = stdin_handle {
                        let _ = h.join();
                    }
                    if let Some(h) = stdout_handle {
                        let _ = h.join();
                    }
                    if let Some(h) = stderr_handle {
                        let _ = h.join();
                    }
                    return if hook.fail_closed {
                        HookDecision::Block {
                            reason: format!("hook timed out after {}ms (fail_closed)", hook.timeout_ms),
                        }
                    } else {
                        HookDecision::Allow
                    };
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(_) => break None,
        }
    };

    if let Some(h) = stdin_handle {
        let _ = h.join();
    }
    let stdout_bytes = stdout_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    let stderr_bytes = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    let Some(status) = status else {
        return if hook.fail_closed {
            HookDecision::Block {
                reason: "hook wait failed (fail_closed)".into(),
            }
        } else {
            HookDecision::Allow
        };
    };

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
        if hook.fail_closed || status.code() == Some(2) {
            return HookDecision::Block {
                reason: if stderr.is_empty() {
                    format!("hook exited with status {:?}", status.code())
                } else {
                    stderr
                },
            };
        }
        return HookDecision::Allow;
    }

    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return HookDecision::Allow;
    }
    match serde_json::from_str::<HookDecision>(trimmed) {
        Ok(decision) => decision,
        Err(e) => {
            if hook.fail_closed {
                HookDecision::Block {
                    reason: format!("hook emitted invalid JSON (fail_closed): {e}"),
                }
            } else {
                HookDecision::Allow
            }
        }
    }
}
