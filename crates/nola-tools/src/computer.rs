// SPDX-License-Identifier: Apache-2.0
//! Native Computer Use desktop automation tool (`computer`) wrapping the full
//! `computer_helper.py` runtime cleanly:
//! - Normalizes action names to `computer_<action>` and passes `{"action", "args"}`
//! - Materializes helper inside a private `0700` directory (never writable from sandbox!)
//! - Enforces subprocess timeout + process-group kill
//! - Extracts `Screenshot: <path>` into `MediaAttachment` and real errors from `parsed["text"]`.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use nola_types::{
    sha256_hex, CapabilityDomain, MediaAttachment, ToolExecutionResult, ToolSchema, ToolSpec,
    ToolTier,
};
use serde_json::{json, Value};

use crate::{ToolContext, ToolHandler};

const COMPUTER_HELPER_PY: &str = include_str!("computer_helper.py");
const COMPUTER_TIMEOUT_SECS: u64 = 30;

fn materialize_private_helper() -> Result<PathBuf, String> {
    let base = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(".nola/runtime");
    fs::create_dir_all(&base).map_err(|e| format!("failed to create helper dir: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&base, fs::Permissions::from_mode(0o700));
    }
    let hash = &sha256_hex(COMPUTER_HELPER_PY.as_bytes())[..12];
    let path = base.join(format!("computer_helper_{hash}.py"));
    if !path.is_file() {
        fs::write(&path, COMPUTER_HELPER_PY.as_bytes())
            .map_err(|e| format!("failed to write computer helper: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
    }
    Ok(path)
}

pub struct ComputerTool;

impl ToolHandler for ComputerTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "computer".into(),
                description: "Control the local desktop GUI (screenshot, observe, accessibility_snapshot, list_windows, list_apps, click, type_text, press_key, hotkey, scroll, clipboard, wait) via native OS automation.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "description": "Computer action (e.g. status, observe, screenshot, list_displays, list_windows, list_apps, accessibility_snapshot, click, type_text, press_key, hotkey, scroll, get_clipboard, set_clipboard, wait)"
                        },
                        "params": {
                            "type": "object",
                            "description": "Action-specific parameters (x, y, text, keys, app, window, etc.)"
                        }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Process,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(action) = args.get("action").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'action'", None);
        };
        let helper = match materialize_private_helper() {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::err_structured("helper_error", e, None),
        };
        let helper_action = if action.starts_with("computer_") {
            action.to_string()
        } else {
            format!("computer_{action}")
        };
        let params = args
            .get("params")
            .or_else(|| args.get("args"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let request = json!({
            "action": helper_action,
            "args": params
        });

        let mut cmd = Command::new("python3");
        cmd.arg(&helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolExecutionResult::err_structured(
                    "computer_spawn_error",
                    format!("python3 computer helper failed to start: {e}"),
                    None,
                )
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(request.to_string().as_bytes());
        }

        let deadline = Instant::now() + Duration::from_secs(COMPUTER_TIMEOUT_SECS);
        let timed_out = loop {
            match child.try_wait() {
                Ok(Some(_)) => break false,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        #[cfg(unix)]
                        unsafe {
                            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
                        }
                        let _ = child.kill();
                        let _ = child.wait();
                        break true;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break false,
            }
        };
        if timed_out {
            return ToolExecutionResult::err_structured(
                "computer_timeout",
                format!("computer action '{action}' timed out after {COMPUTER_TIMEOUT_SECS}s"),
                None,
            );
        }

        let output = match child.wait_with_output() {
            Ok(o) => o,
            Err(e) => return ToolExecutionResult::err_structured("computer_io_error", e.to_string(), None),
        };

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() && stdout.is_empty() {
            return ToolExecutionResult::err_structured(
                "computer_failed",
                if stderr.is_empty() {
                    format!("computer helper exited with {:?}", output.status.code())
                } else {
                    stderr
                },
                None,
            );
        }

        let parsed: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|_| json!({"ok": output.status.success(), "text": stdout, "stderr": stderr}));

        // Extract screenshot path from structured field OR `Screenshot: <path>` line in `text`
        let shot_path = parsed
            .get("screenshot_path")
            .or_else(|| parsed.get("path"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .or_else(|| {
                parsed.get("text").and_then(|v| v.as_str()).and_then(|t| {
                    t.lines()
                        .find_map(|l| l.trim().strip_prefix("Screenshot:").map(|p| p.trim().to_string()))
                })
            });

        let mut media = Vec::new();
        if let Some(sp) = shot_path.as_deref() {
            if sp.ends_with(".png") {
                if let Ok(bytes) = fs::read(sp) {
                    if bytes.len() <= 20 * 1024 * 1024 {
                        media.push(MediaAttachment {
                            mime_type: "image/png".into(),
                            filename: "desktop_screenshot.png".into(),
                            sha256: sha256_hex(&bytes),
                            raw_bytes_len: bytes.len(),
                            bytes_base64: B64.encode(&bytes),
                        });
                    }
                }
            }
        }

        let is_err = parsed
            .get("ok")
            .and_then(|v| v.as_bool())
            .map(|ok| !ok)
            .unwrap_or(!output.status.success());
        if is_err {
            let msg = parsed
                .get("error")
                .or_else(|| parsed.get("message"))
                .or_else(|| parsed.get("text"))
                .or_else(|| parsed.get("display"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("computer action failed")
                .to_string();
            ToolExecutionResult::err_structured("computer_error", msg, Some(parsed))
        } else {
            ToolExecutionResult::ok_structured(parsed, format!("Computer action '{action}' completed"))
                .with_media(media)
        }
    }
}
