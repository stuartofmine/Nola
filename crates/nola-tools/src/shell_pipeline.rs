// SPDX-License-Identifier: Apache-2.0
//! Unified `ShellExecutionPipeline` (`bash` + `bash_control`):
//! - Capability-probed `bwrap` sandbox (auto-disables in containers lacking userns).
//! - Foreground execution with timeout + process-group clean termination.
//! - Background job management (`status` / `wait` / `stop`) returning full incremental stdout/stderr.
//! - `output_artifacts` pre/post SHA-256 hashing -> authoritative `VersionTransition` effects!
//! - `validation_snapshot` ignoring `.git`, `__pycache__`, `target`, `node_modules`, `.venv`.

use std::collections::{BTreeMap, VecDeque};
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

use crate::file_pipeline::{path_within_workspace, resolve_path};
use crate::sandbox::build_shell_command;
use crate::{ToolContext, ToolHandler};

const MAX_CAPTURED_BYTES: usize = 256 * 1024;
const FOREGROUND_HEAD_BYTES: usize = 64 * 1024;
const FOREGROUND_TAIL_BYTES: usize = 192 * 1024;

/// Head+Tail bounded pipe accumulator so huge build/test logs NEVER lose the final traceback!
#[derive(Default)]
struct HeadTailCapture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total_bytes: usize,
}

impl HeadTailCapture {
    fn push(&mut self, data: &[u8]) {
        self.total_bytes = self.total_bytes.saturating_add(data.len());
        let head_rem = FOREGROUND_HEAD_BYTES.saturating_sub(self.head.len());
        let to_head = data.len().min(head_rem);
        if to_head > 0 {
            self.head.extend_from_slice(&data[..to_head]);
        }
        let rest = &data[to_head..];
        if !rest.is_empty() {
            if rest.len() >= FOREGROUND_TAIL_BYTES {
                self.tail.clear();
                self.tail
                    .extend(&rest[rest.len() - FOREGROUND_TAIL_BYTES..]);
            } else {
                let overflow = (self.tail.len() + rest.len()).saturating_sub(FOREGROUND_TAIL_BYTES);
                if overflow > 0 {
                    self.tail.drain(..overflow);
                }
                self.tail.extend(rest);
            }
        }
    }

    fn into_string_and_omitted(self) -> (String, usize) {
        let kept = self.head.len() + self.tail.len();
        let omitted = self.total_bytes.saturating_sub(kept);
        let head_s = strip_ansi(&String::from_utf8_lossy(&self.head));
        if self.tail.is_empty() {
            return (head_s, 0);
        }
        let tail_vec: Vec<u8> = self.tail.into_iter().collect();
        let tail_s = strip_ansi(&String::from_utf8_lossy(&tail_vec));
        if omitted == 0 {
            (format!("{head_s}{tail_s}"), 0)
        } else {
            (
                format!("{head_s}\n... [{omitted} bytes omitted in middle] ...\n{tail_s}"),
                omitted,
            )
        }
    }
}

/// Strip ANSI CSI/OSC escape sequences so colored test/compiler logs stay compact and clean.
pub fn strip_ansi(input: &str) -> String {
    if !input.contains('\x1b') {
        return input.to_string();
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'[' => {
                    // CSI sequence: ESC [ ... final byte 0x40..=0x7E
                    i += 2;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += 1;
                    }
                    continue;
                }
                b']' => {
                    // OSC sequence: ESC ] ... BEL or ESC \
                    i += 2;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                    continue;
                }
                _ => {
                    i += 2;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn drain_head_tail(reader: &mut dyn Read, target: &Mutex<HeadTailCapture>) {
    let mut chunk = [0u8; 4096];
    while let Ok(n) = reader.read(&mut chunk) {
        if n == 0 {
            break;
        }
        if let Ok(mut cap) = target.lock() {
            cap.push(&chunk[..n]);
        }
    }
}

fn kill_process_group(child: &mut Child) {
    #[cfg(unix)]
    {
        let pgid = child.id() as libc::pid_t;
        if pgid > 0 {
            unsafe {
                libc::kill(-pgid, libc::SIGTERM);
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn configure_process_group(cmd: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
}

struct BackgroundJob {
    command: String,
    child: Mutex<Child>,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    stdout_cursor: Mutex<usize>,
    stderr_cursor: Mutex<usize>,
    exit_code: Mutex<Option<i32>>,
}

/// Manages foreground and background shell processes cleanly (kills process groups on Drop).
#[derive(Default)]
pub struct ShellExecutionPipeline {
    jobs: Mutex<BTreeMap<String, Arc<BackgroundJob>>>,
    next_id: Mutex<u64>,
}

impl Drop for ShellExecutionPipeline {
    fn drop(&mut self) {
        if let Ok(jobs) = self.jobs.lock() {
            for job in jobs.values() {
                if job.exit_code.lock().unwrap().is_none() {
                    let mut ch = job.child.lock().unwrap();
                    kill_process_group(&mut ch);
                }
            }
        }
    }
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
        configure_process_group(&mut cmd);
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
            kill_process_group(&mut ch);
            *job.exit_code.lock().unwrap() = Some(-1);
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
        if ctx.permission_profile != nola_types::PermissionProfile::AllowAll
            && !path_within_workspace(&ctx.workspace_root, &cwd)
        {
            return ToolExecutionResult::err_structured(
                "workspace_escape",
                format!("shell cwd '{}' escapes workspace boundary", cwd.display()),
                None,
            );
        }
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
            &ctx.workspace_root,
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

        configure_process_group(&mut cmd);
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

        let started = Instant::now();
        let out_buf = Arc::new(Mutex::new(HeadTailCapture::default()));
        let err_buf = Arc::new(Mutex::new(HeadTailCapture::default()));
        let t_out = child.stdout.take().map(|mut s| {
            let b = Arc::clone(&out_buf);
            thread::spawn(move || drain_head_tail(&mut s, &b))
        });
        let t_err = child.stderr.take().map(|mut s| {
            let b = Arc::clone(&err_buf);
            thread::spawn(move || drain_head_tail(&mut s, &b))
        });

        let deadline = started + Duration::from_secs(timeout_secs);
        let mut timed_out = false;
        let exit_code = loop {
            match child.try_wait() {
                Ok(Some(st)) => break st.code().unwrap_or(-1),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        kill_process_group(&mut child);
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
        let duration_ms = started.elapsed().as_millis() as u64;

        let (stdout, stdout_omitted) =
            std::mem::take(&mut *out_buf.lock().unwrap()).into_string_and_omitted();
        let (stderr, stderr_omitted) =
            std::mem::take(&mut *err_buf.lock().unwrap()).into_string_and_omitted();

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

        let mut data = json!({
            "exit_code": exit_code,
            "timed_out": timed_out,
            "duration_ms": duration_ms,
            "stdout": stdout,
            "stderr": stderr,
            "effects": effects
        });
        if stdout_omitted > 0 || stderr_omitted > 0 {
            data["omitted_bytes"] = json!({"stdout": stdout_omitted, "stderr": stderr_omitted});
        }
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
