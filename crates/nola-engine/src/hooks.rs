// SPDX-License-Identifier: Apache-2.0
//! Workspace-trust-gated lifecycle hooks (`PreToolUse`, `PostToolUse`, `SessionStart`, `Stop`).
//! Guarantees clean child process termination on timeout (no detached Mutex deadlock!).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nola_types::{CommandHookConfig, HookDecision, HookEvent};
use serde_json::{json, Value};

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
        for hook in &self.hooks {
            if hook.event != event || !matcher_matches(&hook.matcher, tool_name) {
                continue;
            }
            let input_json = json!({
                "event": event,
                "tool_name": tool_name,
                "workspace_root": self.workspace_root.display().to_string(),
                "payload": payload
            });
            let decision = execute_single_hook(hook, &self.workspace_root, &input_json);
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
        .stderr(Stdio::piped());

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

    if let Some(mut stdin) = child.stdin.take() {
        let bytes = serde_json::to_vec(input_json).unwrap_or_default();
        let _ = stdin.write_all(&bytes);
    }

    let timeout = Duration::from_millis(hook.timeout_ms.clamp(100, 60_000));
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return if hook.fail_closed {
                        HookDecision::Block {
                            reason: format!("hook timed out after {}ms (fail_closed)", hook.timeout_ms),
                        }
                    } else {
                        HookDecision::Allow
                    };
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(_) => break,
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            return if hook.fail_closed {
                HookDecision::Block {
                    reason: format!("hook wait failed (fail_closed): {e}"),
                }
            } else {
                HookDecision::Allow
            };
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if hook.fail_closed || output.status.code() == Some(2) {
            return HookDecision::Block {
                reason: if stderr.is_empty() {
                    format!("hook exited with status {:?}", output.status.code())
                } else {
                    stderr
                },
            };
        }
        return HookDecision::Allow;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return HookDecision::Allow;
    }
    serde_json::from_str::<HookDecision>(trimmed).unwrap_or(HookDecision::Allow)
}
