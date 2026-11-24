// SPDX-License-Identifier: Apache-2.0
//! Native interactive PTY sessions (`pty_exec`, `pty_write`, `pty_screenshot`, `pty_close`)
//! backed by POSIX `openpty` + `vt100` terminal screen emulation.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nola_types::{CapabilityDomain, PermissionProfile, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
use serde_json::{json, Value};

use crate::file_pipeline::{path_within_workspace, resolve_path};
use crate::sandbox::SandboxMode;
use crate::{ToolContext, ToolHandler};

const MAX_PTY_SESSIONS: usize = 8;
const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 36;

/// Thread-safe manager for interactive PTY terminal sessions.
#[derive(Default)]
pub struct PtyManager {
    #[cfg(unix)]
    sessions: Mutex<BTreeMap<String, Arc<unix_pty::PtySession>>>,
    next_id: Mutex<u64>,
}

impl PtyManager {
    pub fn list_sessions(&self) -> Vec<Value> {
        #[cfg(unix)]
        {
            let Ok(guard) = self.sessions.lock() else {
                return Vec::new();
            };
            guard
                .iter()
                .map(|(id, s)| {
                    json!({
                        "session_id": id,
                        "cwd": s.cwd.display().to_string(),
                        "running": s.is_running(),
                        "exit_code": s.exit_code(),
                    })
                })
                .collect()
        }
        #[cfg(not(unix))]
        {
            Vec::new()
        }
    }

    pub fn close_all(&self) {
        #[cfg(unix)]
        {
            if let Ok(mut guard) = self.sessions.lock() {
                for (_, s) in std::mem::take(&mut *guard) {
                    s.close();
                }
            }
        }
    }

    fn alloc_id(&self) -> String {
        let mut n = self.next_id.lock().unwrap();
        *n += 1;
        format!("pty_{}", *n)
    }
}

#[cfg(unix)]
mod unix_pty {
    use super::*;
    use std::ffi::CString;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::unix::io::FromRawFd;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    pub struct PtySession {
        pub cwd: PathBuf,
        pid: libc::pid_t,
        master_writer: Mutex<File>,
        parser: Arc<Mutex<vt100::Parser>>,
        unread_delta: Arc<Mutex<VecDeque<u8>>>,
        running: Arc<AtomicBool>,
        reaped: Arc<Mutex<bool>>,
        exit_code: Arc<Mutex<Option<i32>>>,
    }

    impl PtySession {
        pub fn spawn(command: &str, cwd: PathBuf, cols: u16, rows: u16) -> Result<Self, String> {
            if !cwd.is_dir() {
                return Err(format!("pty cwd is not a directory: {}", cwd.display()));
            }
            let c_shell = CString::new("/bin/bash").unwrap();
            let c_flag = CString::new("-lc").unwrap();
            let c_cmd = CString::new(command).map_err(|_| "command contains NUL byte".to_string())?;
            let c_cwd = CString::new(cwd.to_string_lossy().as_bytes())
                .map_err(|_| "cwd contains NUL byte".to_string())?;

            let mut master_fd: libc::c_int = -1;
            let mut slave_fd: libc::c_int = -1;
            let mut ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let rc = unsafe {
                libc::openpty(
                    &mut master_fd,
                    &mut slave_fd,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut ws,
                )
            };
            if rc != 0 {
                return Err(format!(
                    "openpty failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            unsafe {
                libc::fcntl(master_fd, libc::F_SETFD, libc::FD_CLOEXEC);
                libc::fcntl(slave_fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }

            let pid = unsafe { libc::fork() };
            if pid < 0 {
                unsafe {
                    libc::close(master_fd);
                    libc::close(slave_fd);
                }
                return Err(format!("fork failed: {}", std::io::Error::last_os_error()));
            }
            if pid == 0 {
                unsafe {
                    libc::close(master_fd);
                    libc::setsid();
                    libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0);
                    libc::dup2(slave_fd, 0);
                    libc::dup2(slave_fd, 1);
                    libc::dup2(slave_fd, 2);
                    // Close all inherited FDs > 2 to avoid leaking sibling PTYs/pipes
                    let max_fd = libc::sysconf(libc::_SC_OPEN_MAX).clamp(64, 4096) as libc::c_int;
                    for fd in 3..max_fd {
                        libc::close(fd);
                    }
                    if libc::chdir(c_cwd.as_ptr()) != 0 {
                        libc::_exit(126);
                    }
                    let argv: [*const libc::c_char; 4] = [
                        c_shell.as_ptr(),
                        c_flag.as_ptr(),
                        c_cmd.as_ptr(),
                        std::ptr::null(),
                    ];
                    libc::execv(c_shell.as_ptr(), argv.as_ptr());
                    libc::_exit(127);
                }
            }

            unsafe {
                libc::close(slave_fd);
            }
            let master_read_fd = unsafe { libc::dup(master_fd) };
            if master_read_fd < 0 {
                unsafe {
                    libc::close(master_fd);
                    libc::kill(pid, libc::SIGKILL);
                    let mut st = 0;
                    libc::waitpid(pid, &mut st, 0);
                }
                return Err("dup(master_fd) failed".into());
            }
            unsafe {
                libc::fcntl(master_read_fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }

            let mut reader = unsafe { File::from_raw_fd(master_read_fd) };
            let writer = unsafe { File::from_raw_fd(master_fd) };

            let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 200)));
            let unread_delta = Arc::new(Mutex::new(VecDeque::new()));
            let running = Arc::new(AtomicBool::new(true));
            let reaped = Arc::new(Mutex::new(false));
            let exit_code = Arc::new(Mutex::new(None));

            let p_bg = Arc::clone(&parser);
            let r_bg = Arc::clone(&unread_delta);
            let run_bg = Arc::clone(&running);
            let reaped_bg = Arc::clone(&reaped);
            let ec_bg = Arc::clone(&exit_code);
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut p) = p_bg.lock() {
                        p.process(&buf[..n]);
                    }
                    if let Ok(mut q) = r_bg.lock() {
                        q.extend(&buf[..n]);
                        while q.len() > 256 * 1024 {
                            q.pop_front();
                        }
                    }
                }
                let mut status: libc::c_int = 0;
                unsafe {
                    libc::waitpid(pid, &mut status, 0);
                }
                let mut r_guard = reaped_bg.lock().unwrap();
                *r_guard = true;
                let code = if libc::WIFEXITED(status) {
                    libc::WEXITSTATUS(status)
                } else {
                    -1
                };
                *ec_bg.lock().unwrap() = Some(code);
                run_bg.store(false, Ordering::SeqCst);
            });

            Ok(Self {
                cwd,
                pid,
                master_writer: Mutex::new(writer),
                parser,
                unread_delta,
                running,
                reaped,
                exit_code,
            })
        }

        pub fn write_input(&self, data: &str) -> Result<(), String> {
            let mut w = self.master_writer.lock().unwrap();
            w.write_all(data.as_bytes())
                .and_then(|_| w.flush())
                .map_err(|e| format!("pty write failed: {e}"))
        }

        pub fn wait_yield(&self, yield_ms: u64) {
            let deadline = Instant::now() + Duration::from_millis(yield_ms.clamp(20, 30_000));
            while Instant::now() < deadline && self.is_running() {
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        pub fn screen_text(&self) -> String {
            let p = self.parser.lock().unwrap();
            p.screen().contents()
        }

        pub fn read_delta(&self) -> String {
            let mut q = self.unread_delta.lock().unwrap();
            let bytes: Vec<u8> = q.drain(..).collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }

        pub fn is_running(&self) -> bool {
            self.running.load(Ordering::SeqCst)
        }

        pub fn exit_code(&self) -> Option<i32> {
            *self.exit_code.lock().unwrap()
        }

        pub fn close(&self) {
            let reaped = self.reaped.lock().unwrap();
            if *reaped || !self.is_running() {
                return;
            }
            unsafe {
                libc::kill(-self.pid, libc::SIGTERM);
                libc::kill(self.pid, libc::SIGTERM);
            }
            drop(reaped);
            std::thread::sleep(Duration::from_millis(50));
            let reaped = self.reaped.lock().unwrap();
            if !*reaped && self.is_running() {
                unsafe {
                    libc::kill(-self.pid, libc::SIGKILL);
                    libc::kill(self.pid, libc::SIGKILL);
                }
            }
        }
    }

    impl Drop for PtySession {
        fn drop(&mut self) {
            self.close();
        }
    }
}

/// Unified interactive PTY tool (`exec`, `write`, `screenshot`, `list`, `close`).
pub struct PtyExecTool;

impl ToolHandler for PtyExecTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "pty_exec".into(),
                description: "Run and interact with persistent VT100 PTY terminal sessions (action: exec|write|screenshot|list|close) for interactive CLIs, REPLs, TUIs, and SSH.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["exec", "write", "screenshot", "list", "close"], "description": "Default 'exec'" },
                        "command": { "type": "string", "description": "Command to start (for action=exec)" },
                        "session_id": { "type": "string", "description": "Target PTY session id (for write/screenshot/close)" },
                        "input": { "type": "string", "description": "Keystrokes/text to send (for action=write, e.g. 'ls\\n')" },
                        "cwd": { "type": "string" },
                        "cols": { "type": "integer", "description": "Default 120" },
                        "rows": { "type": "integer", "description": "Default 36" },
                        "yield_ms": { "type": "integer", "description": "Wait ms before capturing VT100 screen (default 500)" }
                    }
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Process,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        #[cfg(not(unix))]
        {
            let _ = (ctx, args);
            return ToolExecutionResult::err_structured(
                "unsupported_platform",
                "PTY sessions require Unix",
                None,
            );
        }
        #[cfg(unix)]
        {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("exec");
            match action {
                "list" => {
                    let list = ctx.pty.list_sessions();
                    let count = list.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "sessions": list}),
                        format!("Listed {count} PTY session(s)"),
                    )
                }
                "exec" => {
                    let Some(command) = args.get("command").and_then(|v| v.as_str()) else {
                        return ToolExecutionResult::err_structured(
                            "missing_param",
                            "pty_exec action=exec requires 'command'",
                            None,
                        );
                    };
                    if ctx.sandbox_mode == SandboxMode::Required || !ctx.network_allowed {
                        return ToolExecutionResult::err_structured(
                            "sandbox_required",
                            "pty_exec cannot run unsandboxed when strict sandbox or network isolation is required; use bash instead",
                            None,
                        );
                    }
                    let running_count = ctx
                        .pty
                        .sessions
                        .lock()
                        .unwrap()
                        .values()
                        .filter(|s| s.is_running())
                        .count();
                    if running_count >= MAX_PTY_SESSIONS {
                        return ToolExecutionResult::err_structured(
                            "pty_limit",
                            format!("maximum {MAX_PTY_SESSIONS} concurrent running PTY sessions reached"),
                            None,
                        );
                    }
                    let cwd = args
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(|p| resolve_path(&ctx.workspace_root, p))
                        .unwrap_or_else(|| ctx.workspace_root.clone());
                    if ctx.permission_profile != PermissionProfile::AllowAll
                        && !path_within_workspace(&ctx.workspace_root, &cwd)
                    {
                        return ToolExecutionResult::err_structured(
                            "workspace_escape",
                            format!("pty cwd '{}' escapes workspace boundary", cwd.display()),
                            None,
                        );
                    }
                    let cols = args
                        .get("cols")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(DEFAULT_COLS as u64)
                        .clamp(20, 300) as u16;
                    let rows = args
                        .get("rows")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(DEFAULT_ROWS as u64)
                        .clamp(5, 120) as u16;
                    let yield_ms = args
                        .get("yield_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(500);
                    match unix_pty::PtySession::spawn(command, cwd, cols, rows) {
                        Ok(session) => {
                            let session = Arc::new(session);
                            session.wait_yield(yield_ms);
                            let id = ctx.pty.alloc_id();
                            let screen = session.screen_text();
                            let delta = session.read_delta();
                            let running = session.is_running();
                            let exit_code = session.exit_code();
                            ctx.pty.sessions.lock().unwrap().insert(id.clone(), session);
                            ToolExecutionResult::ok_structured(
                                json!({
                                    "session_id": id,
                                    "running": running,
                                    "exit_code": exit_code,
                                    "screen": screen,
                                    "delta": delta
                                }),
                                format!("Started PTY {id} (running={running})"),
                            )
                        }
                        Err(e) => ToolExecutionResult::err_structured("pty_spawn_error", e, None),
                    }
                }
                "write" => {
                    let Some(id) = args.get("session_id").and_then(|v| v.as_str()) else {
                        return ToolExecutionResult::err_structured("missing_param", "missing 'session_id'", None);
                    };
                    let input = args.get("input").and_then(|v| v.as_str()).unwrap_or("");
                    let yield_ms = args.get("yield_ms").and_then(|v| v.as_u64()).unwrap_or(300);
                    let session = ctx.pty.sessions.lock().unwrap().get(id).cloned();
                    let Some(session) = session else {
                        return ToolExecutionResult::err_structured("not_found", format!("unknown PTY '{id}'"), None);
                    };
                    if let Err(e) = session.write_input(input) {
                        return ToolExecutionResult::err_structured("pty_write_error", e, None);
                    }
                    session.wait_yield(yield_ms);
                    ToolExecutionResult::ok_structured(
                        json!({
                            "session_id": id,
                            "running": session.is_running(),
                            "exit_code": session.exit_code(),
                            "screen": session.screen_text(),
                            "delta": session.read_delta()
                        }),
                        format!("Sent input to PTY {id}"),
                    )
                }
                "screenshot" => {
                    let Some(id) = args.get("session_id").and_then(|v| v.as_str()) else {
                        return ToolExecutionResult::err_structured("missing_param", "missing 'session_id'", None);
                    };
                    let session = ctx.pty.sessions.lock().unwrap().get(id).cloned();
                    let Some(session) = session else {
                        return ToolExecutionResult::err_structured("not_found", format!("unknown PTY '{id}'"), None);
                    };
                    ToolExecutionResult::ok_structured(
                        json!({
                            "session_id": id,
                            "running": session.is_running(),
                            "exit_code": session.exit_code(),
                            "screen": session.screen_text()
                        }),
                        format!("Captured VT100 screen of PTY {id}"),
                    )
                }
                "close" => {
                    let Some(id) = args.get("session_id").and_then(|v| v.as_str()) else {
                        return ToolExecutionResult::err_structured("missing_param", "missing 'session_id'", None);
                    };
                    if let Some(session) = ctx.pty.sessions.lock().unwrap().remove(id) {
                        session.close();
                        ToolExecutionResult::ok_structured(
                            json!({"session_id": id, "closed": true}),
                            format!("Closed PTY {id}"),
                        )
                    } else {
                        ToolExecutionResult::err_structured("not_found", format!("unknown PTY '{id}'"), None)
                    }
                }
                other => ToolExecutionResult::err_structured(
                    "invalid_action",
                    format!("unsupported pty_exec action '{other}'"),
                    None,
                ),
            }
        }
    }
}
