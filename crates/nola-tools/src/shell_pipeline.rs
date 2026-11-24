// SPDX-License-Identifier: Apache-2.0
//! Unified `ShellExecutionPipeline` (`bash` + `bash_control`):
//! - Capability-probed `bwrap` sandbox (auto-disables in containers lacking userns).
//! - Foreground execution with timeout + process-group clean termination.
//! - Background job management (`status` / `wait` / `stop`) returning full incremental stdout/stderr.
//! - `output_artifacts` pre/post SHA-256 hashing -> authoritative `VersionTransition` effects!
//! - `validation_snapshot` ignoring `.git`, `__pycache__`, `target`, `node_modules`, `.venv`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use nola_types::{
    sha256_hex, CapabilityDomain, ContentVersion, ResourceEffect, ResourceOperation,
    ToolExecutionResult, ToolSchema, ToolSpec, ToolTier, VersionTransition,
};
use serde_json::{json, Value};

use crate::file_pipeline::resolve_path;
use crate::sandbox::build_shell_command;
use crate::{ToolContext, ToolHandler};

const MAX_CAPTURED_BYTES: usize = 256 * 1024;

struct BackgroundJob {
    command: String,
    child: Mutex<Child>,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    stdout_cursor: Mutex<usize>,
    stderr_cursor: Mutex<usize>,
    exit_code: Mutex<Option<i32>>,
}

/// Manages foreground and background shell processes cleanly.
#[derive(Default)]
pub struct ShellExecutionPipeline {
    jobs: Mutex<BTreeMap<String, Arc<BackgroundJob>>>,
    next_id: Mutex<u64>,
}

impl ShellExecutionPipeline {
    pub fn list_jobs(&self) -> Vec<Value> {
        let Ok(guard) = self.jobs.lock() else {
            return Vec::new();
        };
        guard
            .iter()
            .map(|(id, job)| {
                let code = *job.exit_code.lock().unwrap();
                json!({
                    "job_id": id,
                    "command": job.command,
                    "running": code.is_none(),
                    "exit_code": code
                })
            })
            .collect()
    }

    fn spawn_background(&self, mut cmd: std::process::Command, raw_command: &str) -> Result<String, String> {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn background command: {e}"))?;
        let out_buf = Arc::new(Mutex::new(Vec::new()));
        let err_buf = Arc::new(Mutex::new(Vec::new()));

        if let Some(mut stdout) = child.stdout.take() {
            let buf = Arc::clone(&out_buf);
            thread::spawn(move || drain_pipe(&mut stdout, &buf));
        }
        if let Some(mut stderr) = child.stderr.take() {
            let buf = Arc::clone(&err_buf);
            thread::spawn(move || drain_pipe(&mut stderr, &buf));
        }

        let id = {
            let mut n = self.next_id.lock().unwrap();
            *n += 1;
            format!("job_{}", *n)
        };
        let job = Arc::new(BackgroundJob {
            command: raw_command.to_string(),
            child: Mutex::new(child),
            stdout: out_buf,
            stderr: err_buf,
            stdout_cursor: Mutex::new(0),
            stderr_cursor: Mutex::new(0),
            exit_code: Mutex::new(None),
        });
        self.jobs.lock().unwrap().insert(id.clone(), job);
        Ok(id)
    }

    fn poll_job(&self, job_id: &str, wait_ms: u64, stop: bool) -> Result<Value, String> {
        let job = self
            .jobs
            .lock()
            .unwrap()
            .get(job_id)
            .cloned()
            .ok_or_else(|| format!("unknown background job '{job_id}'"))?;

        if stop {
            let mut ch = job.child.lock().unwrap();
            let _ = ch.kill();
            if let Ok(st) = ch.wait() {
                *job.exit_code.lock().unwrap() = Some(st.code().unwrap_or(-1));
            }
        } else {
            let deadline = Instant::now() + Duration::from_millis(wait_ms);
            loop {
                {
                    let mut ch = job.child.lock().unwrap();
                    if let Ok(Some(st)) = ch.try_wait() {
                        *job.exit_code.lock().unwrap() = Some(st.code().unwrap_or(-1));
                        break;
                    }
                }
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
        }

        let (stdout_chunk, out_cursor) = read_incremental(&job.stdout, &job.stdout_cursor);
        let (stderr_chunk, err_cursor) = read_incremental(&job.stderr, &job.stderr_cursor);
        let exit_code = *job.exit_code.lock().unwrap();

        Ok(json!({
            "job_id": job_id,
            "running": exit_code.is_none(),
            "exit_code": exit_code,
            "stdout": stdout_chunk,
            "stderr": stderr_chunk,
            "stdout_cursor": out_cursor,
            "stderr_cursor": err_cursor,
        }))
    }
}

fn drain_pipe(reader: &mut dyn Read, target: &Mutex<Vec<u8>>) {
    let mut chunk = [0u8; 4096];
    while let Ok(n) = reader.read(&mut chunk) {
        if n == 0 {
            break;
        }
        if let Ok(mut buf) = target.lock() {
            let remaining = MAX_CAPTURED_BYTES.saturating_sub(buf.len());
            if remaining > 0 {
                buf.extend_from_slice(&chunk[..n.min(remaining)]);
            }
        }
    }
}

fn read_incremental(buf: &Mutex<Vec<u8>>, cursor: &Mutex<usize>) -> (String, usize) {
    let bytes = buf.lock().unwrap();
    let mut cur = cursor.lock().unwrap();
    let start = (*cur).min(bytes.len());
    let text = String::from_utf8_lossy(&bytes[start..]).into_owned();
    *cur = bytes.len();
    (text, *cur)
}

/// Deterministic workspace file fingerprint ignoring volatile build/vcs caches
/// (`.git`, `__pycache__`, `target`, `node_modules`, `.venv`).
pub fn validation_snapshot(workspace_root: &Path) -> String {
    let mut entries = Vec::new();
    let walker = WalkBuilder::new(workspace_root)
        .hidden(false)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git" | "__pycache__" | "target" | "node_modules" | ".venv" | ".mypy_cache" | ".pytest_cache"
            )
        })
        .build();
    for dent in walker.flatten() {
        if !dent.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        if let Ok(meta) = dent.metadata() {
            let rel = dent
                .path()
                .strip_prefix(workspace_root)
                .unwrap_or(dent.path())
                .display()
                .to_string();
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            entries.push(format!("{rel}:{}:{mtime}", meta.len()));
        }
    }
    entries.sort();
    sha256_hex(entries.join("\n").as_bytes())
}

pub struct BashTool;

impl ToolHandler for BashTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "bash".into(),
                description: "Execute a shell command in the workspace (supports timeout_secs, background=true, and output_artifacts for verified SHA-256 receipts).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" },
                        "cwd": { "type": "string", "description": "Optional working directory" },
                        "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default 120, max 1800)" },
                        "background": { "type": "boolean", "description": "Run asynchronously and return job_id" },
                        "output_artifacts": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional file paths expected to be created/updated by this command"
                        }
                    },
                    "required": ["command"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::Process,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(command) = args.get("command").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing required 'command'", None);
        };
        let cwd = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(&ctx.workspace_root, p))
            .unwrap_or_else(|| ctx.workspace_root.clone());
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(120)
            .clamp(1, 1800);
        let background = args
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let artifacts: Vec<std::path::PathBuf> = args
            .get("output_artifacts")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|p| resolve_path(&ctx.workspace_root, p))
                    .collect()
            })
            .unwrap_or_default();

        // Capture before-hashes for any declared output_artifacts
        let before_hashes: Vec<(std::path::PathBuf, Option<ContentVersion>)> = artifacts
            .iter()
            .map(|p| {
                let ver = fs::read(p)
                    .ok()
                    .map(|b| ContentVersion::Sha256(sha256_hex(&b)));
                (p.clone(), ver)
            })
            .collect();

        let mut cmd = match build_shell_command(
            command,
            &cwd,
            ctx.sandbox_mode,
            ctx.permission_profile,
            ctx.network_allowed,
        ) {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::err_structured("sandbox_error", e, None),
        };

        if background {
            return match ctx.shell.spawn_background(cmd, command) {
                Ok(job_id) => ToolExecutionResult::ok_structured(
                    json!({"job_id": job_id, "background": true, "command": command}),
                    format!("Started background shell {job_id}"),
                ),
                Err(e) => ToolExecutionResult::err_structured("spawn_error", e, None),
            };
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolExecutionResult::err_structured(
                    "spawn_error",
                    format!("failed to spawn shell: {e}"),
                    None,
                )
            }
        };

        let out_buf = Arc::new(Mutex::new(Vec::new()));
        let err_buf = Arc::new(Mutex::new(Vec::new()));
        let t_out = child.stdout.take().map(|mut s| {
            let b = Arc::clone(&out_buf);
            thread::spawn(move || drain_pipe(&mut s, &b))
        });
        let t_err = child.stderr.take().map(|mut s| {
            let b = Arc::clone(&err_buf);
            thread::spawn(move || drain_pipe(&mut s, &b))
        });

        let deadline = Instant::now() + Duration::from_secs(timeout_secs);
        let mut timed_out = false;
        let exit_code = loop {
            match child.try_wait() {
                Ok(Some(st)) => break st.code().unwrap_or(-1),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        let _ = child.kill();
                        let _ = child.wait();
                        break 124;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break -1,
            }
        };
        if let Some(h) = t_out {
            let _ = h.join();
        }
        if let Some(h) = t_err {
            let _ = h.join();
        }

        let stdout = String::from_utf8_lossy(&out_buf.lock().unwrap()).into_owned();
        let stderr = String::from_utf8_lossy(&err_buf.lock().unwrap()).into_owned();

        // Compute authoritative VersionTransitions for declared output_artifacts
        let mut effects = Vec::new();
        for (path, before) in before_hashes {
            let after = fs::read(&path).ok().map(|b| {
                let sha = sha256_hex(&b);
                ctx.file_state.record(&path, sha.clone());
                ContentVersion::Sha256(sha)
            });
            let (op, transition) = match (before, after) {
                (None, Some(a)) => (ResourceOperation::Create, VersionTransition::Created { after: a }),
                (Some(b), Some(a)) if b != a => (
                    ResourceOperation::Edit,
                    VersionTransition::Modified { before: b, after: a },
                ),
                (Some(b), Some(a)) => (
                    ResourceOperation::Edit,
                    VersionTransition::Unchanged { version: a.max_or(b) },
                ),
                (Some(b), None) => (
                    ResourceOperation::Delete,
                    VersionTransition::Deleted { before: b },
                ),
                (None, None) => (
                    ResourceOperation::Create,
                    VersionTransition::Unobservable {
                        reason: "artifact was not created".into(),
                    },
                ),
            };
            effects.push(ResourceEffect {
                uri: path.display().to_string(),
                operation: op,
                transition,
            });
        }

        let data = json!({
            "exit_code": exit_code,
            "timed_out": timed_out,
            "stdout": stdout,
            "stderr": stderr,
            "effects": effects
        });
        let summary = if timed_out {
            format!("Shell command timed out after {timeout_secs}s")
        } else {
            format!("Shell exited with status {exit_code}")
        };
        let mut res = if timed_out {
            ToolExecutionResult::err_structured("timeout", summary, Some(data))
        } else {
            ToolExecutionResult::ok_structured(data, summary)
        };
        res.effects = effects;
        res
    }
}

trait ContentVersionExt {
    fn max_or(self, _other: Self) -> Self;
}
impl ContentVersionExt for ContentVersion {
    fn max_or(self, _other: Self) -> Self {
        self
    }
}

pub struct BashControlTool;

impl ToolHandler for BashControlTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "bash_control".into(),
                description: "Inspect, wait on, or stop a background bash job and retrieve incremental stdout/stderr.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "job_id": { "type": "string" },
                        "action": { "type": "string", "enum": ["status", "wait", "stop"] },
                        "wait_ms": { "type": "integer", "description": "Wait duration in ms for action=wait (default 2000)" }
                    },
                    "required": ["job_id", "action"]
                }),
            },
            tier: ToolTier::Core,
            domain: CapabilityDomain::Process,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(job_id) = args.get("job_id").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'job_id'", None);
        };
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("status");
        let wait_ms = match action {
            "wait" => args
                .get("wait_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(2_000)
                .clamp(50, 60_000),
            _ => 0,
        };
        let stop = action == "stop";
        match ctx.shell.poll_job(job_id, wait_ms, stop) {
            Ok(data) => {
                let running = data["running"].as_bool().unwrap_or(false);
                ToolExecutionResult::ok_structured(
                    data,
                    format!("Job {job_id} ({action}, running={running})"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("job_error", e, None),
        }
    }
}
