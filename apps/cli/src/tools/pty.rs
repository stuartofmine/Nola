// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Codex-inspired native PTY execution (`pty_exec`, `pty_write`, `pty_screenshot`,
//   `pty_close`) with built-in VT100 terminal emulation and pure-Rust PNG screenshot rendering.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use super::{error, ok, resolve_allowed, ToolContext, ToolOutcome};

const MAX_PTY_SESSIONS: usize = 8;
const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 36;
const MIN_COLS: u16 = 20;
const MAX_COLS: u16 = 300;
const MIN_ROWS: u16 = 5;
const MAX_ROWS: u16 = 120;
const DEFAULT_EXEC_YIELD_MS: u64 = 1000;
const DEFAULT_WRITE_YIELD_MS: u64 = 500;
const MIN_YIELD_MS: u64 = 50;
const MAX_YIELD_MS: u64 = 30_000;
const MAX_OUTPUT_BUFFER_BYTES: usize = 256 * 1024;
const MAX_DELTA_BYTES: usize = 32 * 1024;

pub(super) fn session_cwd(session_id: &str) -> Option<String> {
    #[cfg(unix)]
    {
        unix_impl::manager().session_cwd(session_id)
    }
    #[cfg(not(unix))]
    {
        let _ = session_id;
        None
    }
}

pub(super) fn pty_exec(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    #[cfg(unix)]
    {
        unix_impl::pty_exec(args, ctx)
    }
    #[cfg(not(unix))]
    {
        let _ = (args, ctx);
        error("Native PTY execution is only supported on Unix platforms.".to_string())
    }
}

pub(super) fn pty_write(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    #[cfg(unix)]
    {
        unix_impl::pty_write(args, ctx)
    }
    #[cfg(not(unix))]
    {
        let _ = (args, ctx);
        error("Native PTY execution is only supported on Unix platforms.".to_string())
    }
}

pub(super) fn pty_screenshot(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    #[cfg(unix)]
    {
        unix_impl::pty_screenshot(args, ctx)
    }
    #[cfg(not(unix))]
    {
        let _ = (args, ctx);
        error("Native PTY execution is only supported on Unix platforms.".to_string())
    }
}

pub(super) fn pty_close(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
    #[cfg(unix)]
    {
        unix_impl::pty_close(args, ctx)
    }
    #[cfg(not(unix))]
    {
        let _ = (args, ctx);
        error("Native PTY execution is only supported on Unix platforms.".to_string())
    }
}

fn parse_clamped_u16(args: &Map<String, Value>, key: &str, default: u16, min: u16, max: u16) -> u16 {
    args.get(key)
        .and_then(Value::as_u64)
        .map(|v| v.clamp(min as u64, max as u64) as u16)
        .unwrap_or(default)
}

fn parse_clamped_u64(args: &Map<String, Value>, key: &str, default: u64, min: u64, max: u64) -> u64 {
    args.get(key)
        .and_then(Value::as_u64)
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

fn resolve_pty_cwd(args: &Map<String, Value>, ctx: &ToolContext) -> Result<PathBuf, String> {
    if let Some(raw) = args.get("cwd").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        let resolved = if !ctx.allowed_dirs.is_empty() {
            resolve_allowed(raw, &ctx.allowed_dirs, true).map_err(|outcome| outcome.text)?
        } else {
            let candidate = PathBuf::from(raw);
            if candidate.is_absolute() {
                candidate
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(candidate)
            }
        };
        if !resolved.is_dir() {
            return Err(format!("Working directory does not exist or is not a directory: {}", resolved.display()));
        }
        return Ok(resolved);
    }

    if let Some(first) = ctx.allowed_dirs.first() {
        if first.is_dir() {
            return Ok(first.clone());
        }
    }
    std::env::current_dir().map_err(|err| format!("Failed to determine working directory: {err}"))
}

fn resolve_screenshot_path(session_id: &str, args: &Map<String, Value>, ctx: &ToolContext) -> Result<PathBuf, String> {
    if let Some(raw) = args
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let resolved = if !ctx.allowed_dirs.is_empty() {
            resolve_allowed(raw, &ctx.allowed_dirs, false).map_err(|outcome| outcome.text)?
        } else {
            PathBuf::from(raw)
        };
        if let Some(parent) = resolved.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("Failed to create screenshot directory {}: {err}", parent.display()))?;
            }
        }
        return Ok(resolved);
    }

    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let safe_id: String = session_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let dir = std::env::temp_dir();
    Ok(dir.join(format!("nola-pty-{safe_id}-{millis}.png")))
}

#[cfg(unix)]
mod unix_impl {
    use super::*;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Child, Command, Stdio};
    use std::thread;

    struct PtyState {
        parser: vt100::Parser,
        output_buf: Vec<u8>,
        read_cursor: usize,
        cols: u16,
        rows: u16,
        eof: bool,
        exit_code: Option<i32>,
    }

    pub(super) struct PtySession {
        pub(super) id: String,
        pub(super) cwd: PathBuf,
        master_writer: Mutex<Option<File>>,
        child: Mutex<Option<Child>>,
        child_pid: i32,
        state: Mutex<PtyState>,
        condvar: Condvar,
    }

    impl PtySession {
        fn refresh_exit_status(&self) -> Option<i32> {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(code) = state.exit_code {
                return Some(code);
            }
            drop(state);

            let mut child_guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(child) = child_guard.as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    let code = status
                        .code()
                        .unwrap_or_else(|| status.signal().map(|sig| 128 + sig).unwrap_or(-1));
                    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.exit_code = Some(code);
                    self.condvar.notify_all();
                    return Some(code);
                }
            }
            None
        }

        fn wait_for_output(&self, yield_time_ms: u64) {
            let deadline = Instant::now() + Duration::from_millis(yield_time_ms);
            let mut exit_grace_deadline: Option<Instant> = None;

            loop {
                let exited = self.refresh_exit_status().is_some();
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if exited && state.eof {
                    break;
                }
                if exited {
                    let grace = *exit_grace_deadline
                        .get_or_insert_with(|| Instant::now() + Duration::from_millis(80));
                    if Instant::now() >= grace {
                        break;
                    }
                }
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let remaining = deadline.saturating_duration_since(now);
                let step = remaining.min(Duration::from_millis(40));
                let (guard, _) = self
                    .condvar
                    .wait_timeout(state, step)
                    .unwrap_or_else(|e| e.into_inner());
                state = guard;
                if exited && state.eof {
                    break;
                }
            }
            let _ = self.refresh_exit_status();
        }

        fn take_snapshot(&self) -> (bool, Option<i32>, u16, u16, String, String) {
            let _ = self.refresh_exit_status();
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let start = state.read_cursor.min(state.output_buf.len());
            let delta_slice = &state.output_buf[start..];
            let delta = if delta_slice.len() > MAX_DELTA_BYTES {
                String::from_utf8_lossy(&delta_slice[delta_slice.len() - MAX_DELTA_BYTES..]).into_owned()
            } else {
                String::from_utf8_lossy(delta_slice).into_owned()
            };
            state.read_cursor = state.output_buf.len();
            let screen = state.parser.screen().contents();
            let running = state.exit_code.is_none();
            (running, state.exit_code, state.cols, state.rows, delta, screen)
        }

        fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
            let writer_guard = self.master_writer.lock().unwrap_or_else(|e| e.into_inner());
            let Some(file) = writer_guard.as_ref() else {
                return Err("PTY session master descriptor is already closed.".to_string());
            };
            let winsize = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let rc = unsafe { libc::ioctl(file.as_raw_fd(), libc::TIOCSWINSZ as _, &winsize) };
            if rc != 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!("Failed to resize PTY (TIOCSWINSZ): {err}"));
            }
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.parser.set_size(rows, cols);
            state.cols = cols;
            state.rows = rows;
            Ok(())
        }

        fn write_bytes(&self, data: &[u8]) -> Result<(), String> {
            let mut writer_guard = self.master_writer.lock().unwrap_or_else(|e| e.into_inner());
            let Some(file) = writer_guard.as_mut() else {
                return Err("PTY session master descriptor is already closed.".to_string());
            };
            file.write_all(data)
                .and_then(|_| file.flush())
                .map_err(|err| format!("Failed to write to PTY: {err}"))
        }

        fn terminate(&self) -> Option<i32> {
            if self.refresh_exit_status().is_none() && self.child_pid > 0 {
                // Signal the process group first (since child called setsid), then the pid directly.
                unsafe {
                    libc::kill(-self.child_pid, libc::SIGHUP);
                    libc::kill(self.child_pid, libc::SIGHUP);
                }
                let deadline = Instant::now() + Duration::from_millis(250);
                while Instant::now() < deadline {
                    if self.refresh_exit_status().is_some() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                if self.refresh_exit_status().is_none() {
                    unsafe {
                        libc::kill(-self.child_pid, libc::SIGKILL);
                        libc::kill(self.child_pid, libc::SIGKILL);
                    }
                }
            }

            // Reap child process to prevent zombies.
            let mut child_guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(mut child) = child_guard.take() {
                if let Ok(status) = child.wait() {
                    let code = status
                        .code()
                        .unwrap_or_else(|| status.signal().map(|sig| 128 + sig).unwrap_or(-1));
                    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.exit_code.get_or_insert(code);
                }
            }

            // Close master writer FD.
            if let Ok(mut writer_guard) = self.master_writer.lock() {
                let _ = writer_guard.take();
            }

            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.exit_code
        }
    }

    pub(super) struct PtySessionManager {
        next_id: AtomicU64,
        sessions: Mutex<HashMap<String, Arc<PtySession>>>,
    }

    impl PtySessionManager {
        fn new() -> Self {
            Self {
                next_id: AtomicU64::new(1),
                sessions: Mutex::new(HashMap::new()),
            }
        }

        pub(super) fn session_cwd(&self, session_id: &str) -> Option<String> {
            let sessions = self.sessions.lock().ok()?;
            sessions
                .get(session_id)
                .map(|s| s.cwd.to_string_lossy().into_owned())
        }

        fn get(&self, session_id: &str) -> Result<Arc<PtySession>, String> {
            let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sessions
                .get(session_id)
                .cloned()
                .ok_or_else(|| format!("Unknown PTY session '{session_id}'."))
        }

        fn spawn(
            &self,
            command: &str,
            cwd: PathBuf,
            cols: u16,
            rows: u16,
        ) -> Result<Arc<PtySession>, String> {
            {
                let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
                if sessions.len() >= MAX_PTY_SESSIONS {
                    return Err(format!(
                        "Maximum concurrent PTY sessions ({MAX_PTY_SESSIONS}) reached. Close an existing session with pty_close first."
                    ));
                }
            }

            let mut master_fd: RawFd = -1;
            let mut slave_fd: RawFd = -1;
            let mut winsize = libc::winsize {
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
                    &mut winsize,
                )
            };
            if rc != 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!("openpty failed: {err}"));
            }

            // Ensure window size is applied on master FD and set FD_CLOEXEC on master.
            unsafe {
                libc::ioctl(master_fd, libc::TIOCSWINSZ as _, &winsize);
                let flags = libc::fcntl(master_fd, libc::F_GETFD);
                if flags >= 0 {
                    libc::fcntl(master_fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                }
            }

            let mut cmd = Command::new("/bin/sh");
            cmd.arg("-c")
                .arg(command)
                .current_dir(&cwd)
                .env("TERM", "xterm-256color")
                .env("COLUMNS", cols.to_string())
                .env("LINES", rows.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            let child_master_fd = master_fd;
            let child_slave_fd = slave_fd;
            unsafe {
                cmd.pre_exec(move || {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::ioctl(child_slave_fd, libc::TIOCSCTTY as _, 0) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::dup2(child_slave_fd, libc::STDIN_FILENO) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::dup2(child_slave_fd, libc::STDOUT_FILENO) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::dup2(child_slave_fd, libc::STDERR_FILENO) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if child_slave_fd > libc::STDERR_FILENO {
                        libc::close(child_slave_fd);
                    }
                    if child_master_fd > libc::STDERR_FILENO {
                        libc::close(child_master_fd);
                    }
                    Ok(())
                });
            }

            let child = match cmd.spawn() {
                Ok(child) => child,
                Err(err) => {
                    unsafe {
                        libc::close(slave_fd);
                        libc::close(master_fd);
                    }
                    return Err(format!("Failed to spawn PTY child process: {err}"));
                }
            };

            // Parent must close slave FD immediately so EOF arrives when the child exits.
            unsafe {
                libc::close(slave_fd);
            }

            let master_file = unsafe { File::from_raw_fd(master_fd) };
            let mut reader_file = match master_file.try_clone() {
                Ok(cloned) => cloned,
                Err(err) => {
                    let mut child = child;
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("Failed to clone PTY master descriptor: {err}"));
                }
            };

            let id = format!("pty-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
            let child_pid = child.id() as i32;

            let session = Arc::new(PtySession {
                id: id.clone(),
                cwd,
                master_writer: Mutex::new(Some(master_file)),
                child: Mutex::new(Some(child)),
                child_pid,
                state: Mutex::new(PtyState {
                    parser: vt100::Parser::new(rows, cols, 0),
                    output_buf: Vec::new(),
                    read_cursor: 0,
                    cols,
                    rows,
                    eof: false,
                    exit_code: None,
                }),
                condvar: Condvar::new(),
            });

            {
                let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
                if sessions.len() >= MAX_PTY_SESSIONS {
                    drop(sessions);
                    session.terminate();
                    return Err(format!(
                        "Maximum concurrent PTY sessions ({MAX_PTY_SESSIONS}) reached. Close an existing session with pty_close first."
                    ));
                }
                sessions.insert(id.clone(), Arc::clone(&session));
            }

            let reader_session = Arc::clone(&session);
            let _ = thread::Builder::new()
                .name(format!("nola-pty-reader-{id}"))
                .spawn(move || {
                    let mut buf = [0u8; 4096];
                    loop {
                        match reader_file.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                let mut state = reader_session
                                    .state
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                state.parser.process(&buf[..n]);
                                state.output_buf.extend_from_slice(&buf[..n]);
                                if state.output_buf.len() > MAX_OUTPUT_BUFFER_BYTES {
                                    let excess = state.output_buf.len() - MAX_OUTPUT_BUFFER_BYTES;
                                    state.output_buf.drain(..excess);
                                    state.read_cursor = state.read_cursor.saturating_sub(excess);
                                }
                                reader_session.condvar.notify_all();
                            }
                            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                            Err(_) => break,
                        }
                    }
                    let mut state = reader_session
                        .state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    state.eof = true;
                    reader_session.condvar.notify_all();
                });

            Ok(session)
        }

        fn remove(&self, session_id: &str) -> Result<Arc<PtySession>, String> {
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sessions
                .remove(session_id)
                .ok_or_else(|| format!("Unknown PTY session '{session_id}'."))
        }
    }

    pub(super) fn manager() -> &'static PtySessionManager {
        static MANAGER: OnceLock<PtySessionManager> = OnceLock::new();
        MANAGER.get_or_init(PtySessionManager::new)
    }

    pub(super) fn pty_exec(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
        let Some(command) = args
            .get("command")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        else {
            return error("Missing required non-empty 'command' argument.".to_string());
        };
        let cwd = match resolve_pty_cwd(args, ctx) {
            Ok(cwd) => cwd,
            Err(msg) => return error(msg),
        };
        let cols = parse_clamped_u16(args, "cols", DEFAULT_COLS, MIN_COLS, MAX_COLS);
        let rows = parse_clamped_u16(args, "rows", DEFAULT_ROWS, MIN_ROWS, MAX_ROWS);
        let yield_time_ms =
            parse_clamped_u64(args, "yield_time_ms", DEFAULT_EXEC_YIELD_MS, MIN_YIELD_MS, MAX_YIELD_MS);

        let session = match manager().spawn(command, cwd, cols, rows) {
            Ok(session) => session,
            Err(msg) => return error(msg),
        };
        session.wait_for_output(yield_time_ms);
        let (running, exit_code, cols, rows, output, screen) = session.take_snapshot();

        let payload = json!({
            "session_id": session.id,
            "running": running,
            "exit_code": exit_code,
            "cols": cols,
            "rows": rows,
            "output": output,
            "screen": screen,
        });
        let display = if running {
            format!("PTY {} running ({}x{})", session.id, cols, rows)
        } else {
            format!("PTY {} exited ({:?})", session.id, exit_code)
        };
        ok(
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()),
            display,
        )
    }

    pub(super) fn pty_write(args: &Map<String, Value>, _ctx: &ToolContext) -> ToolOutcome {
        let Some(session_id) = args
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        else {
            return error("Missing required 'session_id' argument.".to_string());
        };
        let session = match manager().get(session_id) {
            Ok(session) => session,
            Err(msg) => return error(msg),
        };

        let has_cols = args.get("cols").and_then(Value::as_u64).is_some();
        let has_rows = args.get("rows").and_then(Value::as_u64).is_some();
        if has_cols || has_rows {
            let (cur_cols, cur_rows) = {
                let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
                (state.cols, state.rows)
            };
            let new_cols = parse_clamped_u16(args, "cols", cur_cols, MIN_COLS, MAX_COLS);
            let new_rows = parse_clamped_u16(args, "rows", cur_rows, MIN_ROWS, MAX_ROWS);
            if let Err(msg) = session.resize(new_cols, new_rows) {
                return error(msg);
            }
        }

        if let Some(chars) = args.get("chars").and_then(Value::as_str) {
            if !chars.is_empty() {
                if let Err(msg) = session.write_bytes(chars.as_bytes()) {
                    return error(msg);
                }
            }
        }

        let yield_time_ms =
            parse_clamped_u64(args, "yield_time_ms", DEFAULT_WRITE_YIELD_MS, MIN_YIELD_MS, MAX_YIELD_MS);
        session.wait_for_output(yield_time_ms);
        let (running, exit_code, cols, rows, output, screen) = session.take_snapshot();

        let payload = json!({
            "session_id": session.id,
            "running": running,
            "exit_code": exit_code,
            "cols": cols,
            "rows": rows,
            "output": output,
            "screen": screen,
        });
        let display = if running {
            format!("PTY {} updated ({}x{})", session.id, cols, rows)
        } else {
            format!("PTY {} exited ({:?})", session.id, exit_code)
        };
        ok(
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()),
            display,
        )
    }

    pub(super) fn pty_screenshot(args: &Map<String, Value>, ctx: &ToolContext) -> ToolOutcome {
        let Some(session_id) = args
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        else {
            return error("Missing required 'session_id' argument.".to_string());
        };
        let session = match manager().get(session_id) {
            Ok(session) => session,
            Err(msg) => return error(msg),
        };
        let path = match resolve_screenshot_path(session_id, args, ctx) {
            Ok(path) => path,
            Err(msg) => return error(msg),
        };

        let _ = session.refresh_exit_status();
        let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        let screen = state.parser.screen();
        let screen_text = screen.contents();
        let cols = state.cols;
        let rows = state.rows;
        let running = state.exit_code.is_none();
        let exit_code = state.exit_code;

        let render_result = render_screen_to_png(screen, &path);
        drop(state);

        match render_result {
            Ok((width, height)) => {
                let payload = json!({
                    "session_id": session.id,
                    "path": path.display().to_string(),
                    "width": width,
                    "height": height,
                    "cols": cols,
                    "rows": rows,
                    "running": running,
                    "exit_code": exit_code,
                    "screen": screen_text,
                });
                let mut outcome = ok(
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()),
                    format!("PTY screenshot {} ({width}x{height})", path.display()),
                )
                .with_file_artifact(&path, "screenshot");
                if let Some(artifact) = outcome.artifacts.last_mut() {
                    artifact.mime_type = Some("image/png".into());
                    artifact.name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                }
                outcome
            }
            Err(msg) => error(msg),
        }
    }

    pub(super) fn pty_close(args: &Map<String, Value>, _ctx: &ToolContext) -> ToolOutcome {
        let Some(session_id) = args
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        else {
            return error("Missing required 'session_id' argument.".to_string());
        };
        let session = match manager().remove(session_id) {
            Ok(session) => session,
            Err(msg) => return error(msg),
        };
        let exit_code = session.terminate();
        let payload = json!({
            "session_id": session.id,
            "closed": true,
            "exit_code": exit_code,
        });
        ok(
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string()),
            format!("Closed PTY {}", session.id),
        )
    }
}

// ============================================================================
// Pure-Rust VT100 -> PNG Screenshot Renderer
// ============================================================================

#[cfg(any(feature = "terminal-ui", test))]
fn render_screen_to_png(screen: &vt100::Screen, path: &Path) -> Result<(u32, u32), String> {
    use image::{Rgb, RgbImage};

    const CELL_W: u32 = 8;
    const CELL_H: u32 = 16;
    const PAD: u32 = 12;
    const DEFAULT_BG: [u8; 3] = [22, 24, 33];
    const DEFAULT_FG: [u8; 3] = [216, 222, 233];
    const CURSOR_BG: [u8; 3] = [122, 162, 247];

    let (rows, cols) = screen.size();
    let img_w = cols as u32 * CELL_W + PAD * 2;
    let img_h = rows as u32 * CELL_H + PAD * 2;

    let mut img = RgbImage::from_pixel(img_w, img_h, Rgb(DEFAULT_BG));
    let (cursor_row, cursor_col) = screen.cursor_position();
    let show_cursor = !screen.hide_cursor();

    for row in 0..rows {
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let cell_span = if cell.is_wide() && col + 1 < cols { 2 } else { 1 };
            let base_x = PAD + col as u32 * CELL_W;
            let base_y = PAD + row as u32 * CELL_H;

            let mut fg = resolve_vt100_color(cell.fgcolor(), DEFAULT_FG, cell.bold());
            let mut bg = resolve_vt100_color(cell.bgcolor(), DEFAULT_BG, false);
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
            }
            if show_cursor && row == cursor_row && col == cursor_col {
                bg = CURSOR_BG;
                fg = DEFAULT_BG;
            }

            // Fill cell background.
            let fill_w = CELL_W * cell_span;
            for py in 0..CELL_H {
                for px in 0..fill_w {
                    img.put_pixel(base_x + px, base_y + py, Rgb(bg));
                }
            }

            // Render glyph.
            let contents = cell.contents();
            if let Some(ch) = contents.chars().next() {
                if ch != ' ' && ch != '\0' {
                    draw_glyph(&mut img, base_x, base_y, ch, Rgb(fg), cell.bold(), cell.is_wide());
                }
            }

            // Underline attribute.
            if cell.underline() {
                for px in 0..fill_w {
                    img.put_pixel(base_x + px, base_y + CELL_H - 2, Rgb(fg));
                }
            }
        }
    }

    img.save_with_format(path, image::ImageFormat::Png)
        .map_err(|err| format!("Failed to save PTY PNG screenshot to {}: {err}", path.display()))?;
    Ok((img_w, img_h))
}

#[cfg(not(any(feature = "terminal-ui", test)))]
fn render_screen_to_png(_screen: &vt100::Screen, _path: &Path) -> Result<(u32, u32), String> {
    Err("PTY PNG screenshot rendering requires the 'terminal-ui' feature.".to_string())
}

#[cfg(any(feature = "terminal-ui", test))]
fn resolve_vt100_color(color: vt100::Color, default: [u8; 3], bold: bool) -> [u8; 3] {
    match color {
        vt100::Color::Default => {
            if bold {
                [
                    default[0].saturating_add(24),
                    default[1].saturating_add(24),
                    default[2].saturating_add(24),
                ]
            } else {
                default
            }
        }
        vt100::Color::Idx(idx) => {
            let effective_idx = if bold && idx < 8 { idx + 8 } else { idx };
            ansi_256_color(effective_idx)
        }
        vt100::Color::Rgb(r, g, b) => [r, g, b],
    }
}

#[cfg(any(feature = "terminal-ui", test))]
fn ansi_256_color(idx: u8) -> [u8; 3] {
    const ANSI16: [[u8; 3]; 16] = [
        [30, 34, 42],    // 0 Black
        [224, 108, 117], // 1 Red
        [152, 195, 121], // 2 Green
        [229, 192, 123], // 3 Yellow
        [97, 175, 239],  // 4 Blue
        [198, 120, 221], // 5 Magenta
        [86, 182, 194],  // 6 Cyan
        [171, 178, 191], // 7 White
        [92, 99, 112],   // 8 Bright Black
        [244, 135, 145], // 9 Bright Red
        [177, 220, 146], // 10 Bright Green
        [245, 212, 146], // 11 Bright Yellow
        [128, 196, 255], // 12 Bright Blue
        [218, 148, 240], // 13 Bright Magenta
        [116, 208, 220], // 14 Bright Cyan
        [240, 244, 248], // 15 Bright White
    ];
    if idx < 16 {
        return ANSI16[idx as usize];
    }
    if idx < 232 {
        let cube = idx - 16;
        let r = cube / 36;
        let g = (cube % 36) / 6;
        let b = cube % 6;
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        return [LEVELS[r as usize], LEVELS[g as usize], LEVELS[b as usize]];
    }
    let gray = 8 + (idx - 232) * 10;
    [gray, gray, gray]
}

#[cfg(any(feature = "terminal-ui", test))]
fn put_px(img: &mut image::RgbImage, x: u32, y: u32, color: image::Rgb<u8>) {
    if x < img.width() && y < img.height() {
        img.put_pixel(x, y, color);
    }
}

#[cfg(any(feature = "terminal-ui", test))]
fn draw_glyph(
    img: &mut image::RgbImage,
    bx: u32,
    by: u32,
    ch: char,
    fg: image::Rgb<u8>,
    bold: bool,
    wide: bool,
) {
    if draw_box_or_block_glyph(img, bx, by, ch, fg) {
        return;
    }
    if draw_braille_glyph(img, bx, by, ch, fg) {
        return;
    }
    if let Some(bitmap) = ascii_8x8_bitmap(ch) {
        // Render 8x8 bitmap cleanly into 8x16 cell (rows 1..15, vertical 2x scaling with crisp edges).
        for (row_idx, &bits) in bitmap.iter().enumerate() {
            let py = by + (row_idx as u32) * 2;
            for col_idx in 0..8u32 {
                if (bits >> (7 - col_idx)) & 1 != 0 {
                    let px = bx + col_idx;
                    put_px(img, px, py, fg);
                    put_px(img, px, py + 1, fg);
                    if bold && col_idx + 1 < 8 {
                        put_px(img, px + 1, py, fg);
                        put_px(img, px + 1, py + 1, fg);
                    }
                }
            }
        }
        return;
    }

    // Fallback outline glyph for unmapped non-ASCII characters (e.g., CJK / emoji).
    let w = if wide { 14 } else { 6 };
    for px in 1..w {
        put_px(img, bx + px, by + 3, fg);
        put_px(img, bx + px, by + 12, fg);
    }
    for py in 3..=12 {
        put_px(img, bx + 1, by + py, fg);
        put_px(img, bx + w - 1, by + py, fg);
    }
}

#[cfg(any(feature = "terminal-ui", test))]
fn draw_braille_glyph(
    img: &mut image::RgbImage,
    bx: u32,
    by: u32,
    ch: char,
    fg: image::Rgb<u8>,
) -> bool {
    let code = ch as u32;
    if !(0x2800..=0x28FF).contains(&code) {
        return false;
    }
    let mask = (code - 0x2800) as u8;
    // Standard Braille dot positions (bit -> (col, row)):
    let dots: [(u8, u32, u32); 8] = [
        (0x01, 2, 2),
        (0x02, 2, 5),
        (0x04, 2, 8),
        (0x40, 2, 11),
        (0x08, 5, 2),
        (0x10, 5, 5),
        (0x20, 5, 8),
        (0x80, 5, 11),
    ];
    for (bit, dx, dy) in dots {
        if mask & bit != 0 {
            put_px(img, bx + dx, by + dy, fg);
            put_px(img, bx + dx + 1, by + dy, fg);
            put_px(img, bx + dx, by + dy + 1, fg);
            put_px(img, bx + dx + 1, by + dy + 1, fg);
        }
    }
    true
}

#[cfg(any(feature = "terminal-ui", test))]
fn draw_box_or_block_glyph(
    img: &mut image::RgbImage,
    bx: u32,
    by: u32,
    ch: char,
    fg: image::Rgb<u8>,
) -> bool {
    let fill_rect = |img: &mut image::RgbImage, x0: u32, y0: u32, x1: u32, y1: u32| {
        for py in y0..y1 {
            for px in x0..x1 {
                put_px(img, bx + px, by + py, fg);
            }
        }
    };

    // Block elements.
    match ch {
        '█' => {
            fill_rect(img, 0, 0, 8, 16);
            return true;
        }
        '▀' => {
            fill_rect(img, 0, 0, 8, 8);
            return true;
        }
        '▄' => {
            fill_rect(img, 0, 8, 8, 16);
            return true;
        }
        '▌' => {
            fill_rect(img, 0, 0, 4, 16);
            return true;
        }
        '▐' => {
            fill_rect(img, 4, 0, 8, 16);
            return true;
        }
        '░' | '▒' | '▓' => {
            let density = match ch {
                '░' => 4,
                '▒' => 2,
                _ => 1,
            };
            for py in 0..16u32 {
                for px in 0..8u32 {
                    let draw = if density == 1 {
                        (px + py) % 4 != 0
                    } else if density == 2 {
                        (px + py) % 2 == 0
                    } else {
                        px % 2 == 0 && py % 2 == 0
                    };
                    if draw {
                        put_px(img, bx + px, by + py, fg);
                    }
                }
            }
            return true;
        }
        '•' | '●' => {
            fill_rect(img, 2, 6, 6, 10);
            return true;
        }
        '─' | '━' | '–' | '—' => {
            let h = if ch == '━' { 2 } else { 1 };
            fill_rect(img, 0, 7, 8, 7 + h);
            return true;
        }
        '│' | '┃' => {
            let w = if ch == '┃' { 2 } else { 1 };
            fill_rect(img, 3, 0, 3 + w, 16);
            return true;
        }
        '═' => {
            fill_rect(img, 0, 6, 8, 7);
            fill_rect(img, 0, 9, 8, 10);
            return true;
        }
        '║' => {
            fill_rect(img, 2, 0, 3, 16);
            fill_rect(img, 5, 0, 6, 16);
            return true;
        }
        _ => {}
    }

    // Light / heavy / rounded box-drawing connectors (left, right, up, down).
    let connectors: Option<(bool, bool, bool, bool)> = match ch {
        '┌' | '┏' | '╭' | '╔' => Some((false, true, false, true)),
        '┐' | '┓' | '╮' | '╗' => Some((true, false, false, true)),
        '└' | '┗' | '╰' | '╚' => Some((false, true, true, false)),
        '┘' | '┛' | '╯' | '╝' => Some((true, false, true, false)),
        '├' | '┣' | '╠' => Some((false, true, true, true)),
        '┤' | '┫' | '╣' => Some((true, false, true, true)),
        '┬' | '┳' | '╦' => Some((true, true, false, true)),
        '┴' | '┻' | '╩' => Some((true, true, true, false)),
        '┼' | '╋' | '╬' => Some((true, true, true, true)),
        '╴' | '╸' => Some((true, false, false, false)),
        '╶' | '╺' => Some((false, true, false, false)),
        '╵' | '╹' => Some((false, false, true, false)),
        '╷' | '╻' => Some((false, false, false, true)),
        _ => None,
    };
    if let Some((left, right, up, down)) = connectors {
        let heavy = matches!(ch, '┏' | '┓' | '┗' | '┛' | '┣' | '┫' | '┳' | '┻' | '╋' | '╸' | '╺' | '╹' | '╻');
        let t = if heavy { 2 } else { 1 };
        if left {
            fill_rect(img, 0, 7, 4 + t - 1, 7 + t);
        }
        if right {
            fill_rect(img, 3, 7, 8, 7 + t);
        }
        if up {
            fill_rect(img, 3, 0, 3 + t, 8 + t - 1);
        }
        if down {
            fill_rect(img, 3, 7, 3 + t, 16);
        }
        return true;
    }

    false
}

/// Crisp built-in 8x8 bitmap font covering printable ASCII 0x20..=0x7E.
#[cfg(any(feature = "terminal-ui", test))]
fn ascii_8x8_bitmap(ch: char) -> Option<[u8; 8]> {
    let glyph = match ch {
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        '!' => [0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x00],
        '"' => [0x6C, 0x6C, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00],
        '#' => [0x36, 0x36, 0x7F, 0x36, 0x7F, 0x36, 0x36, 0x00],
        '$' => [0x0C, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0x18, 0x00],
        '%' => [0x62, 0x64, 0x08, 0x10, 0x26, 0x46, 0x00, 0x00],
        '&' => [0x38, 0x6C, 0x38, 0x76, 0xCC, 0xCC, 0x76, 0x00],
        '\'' => [0x18, 0x18, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00],
        '(' => [0x0C, 0x18, 0x30, 0x30, 0x30, 0x18, 0x0C, 0x00],
        ')' => [0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x18, 0x30, 0x00],
        '*' => [0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0x00, 0x00],
        '+' => [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x00, 0x00],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x30],
        '-' => [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00],
        '/' => [0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x00, 0x00],
        '0' => [0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x3C, 0x00],
        '1' => [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00],
        '2' => [0x3C, 0x66, 0x06, 0x1C, 0x30, 0x60, 0x7E, 0x00],
        '3' => [0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0x00],
        '4' => [0x0C, 0x1C, 0x3C, 0x6C, 0x7E, 0x0C, 0x0C, 0x00],
        '5' => [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0x00],
        '6' => [0x3C, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x3C, 0x00],
        '7' => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x30, 0x30, 0x00],
        '8' => [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0x00],
        '9' => [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x0C, 0x38, 0x00],
        ':' => [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x00, 0x00],
        ';' => [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x30, 0x00],
        '<' => [0x0C, 0x18, 0x30, 0x60, 0x30, 0x18, 0x0C, 0x00],
        '=' => [0x00, 0x00, 0x7E, 0x00, 0x7E, 0x00, 0x00, 0x00],
        '>' => [0x30, 0x18, 0x0C, 0x06, 0x0C, 0x18, 0x30, 0x00],
        '?' => [0x3C, 0x66, 0x06, 0x1C, 0x18, 0x00, 0x18, 0x00],
        '@' => [0x3C, 0x66, 0x6E, 0x6A, 0x6E, 0x60, 0x3C, 0x00],
        'A' => [0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x00],
        'B' => [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0x00],
        'C' => [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0x00],
        'D' => [0x78, 0x6C, 0x66, 0x66, 0x66, 0x6C, 0x78, 0x00],
        'E' => [0x7E, 0x60, 0x60, 0x78, 0x60, 0x60, 0x7E, 0x00],
        'F' => [0x7E, 0x60, 0x60, 0x78, 0x60, 0x60, 0x60, 0x00],
        'G' => [0x3C, 0x66, 0x60, 0x6E, 0x66, 0x66, 0x3E, 0x00],
        'H' => [0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00],
        'I' => [0x3C, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00],
        'J' => [0x1E, 0x0C, 0x0C, 0x0C, 0x6C, 0x6C, 0x38, 0x00],
        'K' => [0x66, 0x6C, 0x78, 0x70, 0x78, 0x6C, 0x66, 0x00],
        'L' => [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0x00],
        'M' => [0x63, 0x77, 0x7F, 0x6B, 0x63, 0x63, 0x63, 0x00],
        'N' => [0x66, 0x76, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0x00],
        'O' => [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        'P' => [0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0x00],
        'Q' => [0x3C, 0x66, 0x66, 0x66, 0x6A, 0x6C, 0x36, 0x00],
        'R' => [0x7C, 0x66, 0x66, 0x7C, 0x6C, 0x66, 0x66, 0x00],
        'S' => [0x3C, 0x66, 0x60, 0x3C, 0x06, 0x66, 0x3C, 0x00],
        'T' => [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        'U' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        'V' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00],
        'W' => [0x63, 0x63, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0x00],
        'X' => [0x66, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x66, 0x00],
        'Y' => [0x66, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0x00],
        'Z' => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00],
        '[' => [0x3C, 0x30, 0x30, 0x30, 0x30, 0x30, 0x3C, 0x00],
        '\\' => [0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x00, 0x00],
        ']' => [0x3C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x3C, 0x00],
        '^' => [0x18, 0x3C, 0x66, 0x00, 0x00, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x00],
        '`' => [0x30, 0x18, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x00],
        'a' => [0x00, 0x00, 0x3C, 0x06, 0x3E, 0x66, 0x3E, 0x00],
        'b' => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x00],
        'c' => [0x00, 0x00, 0x3C, 0x60, 0x60, 0x60, 0x3C, 0x00],
        'd' => [0x06, 0x06, 0x3E, 0x66, 0x66, 0x66, 0x3E, 0x00],
        'e' => [0x00, 0x00, 0x3C, 0x66, 0x7E, 0x60, 0x3C, 0x00],
        'f' => [0x1C, 0x30, 0x7C, 0x30, 0x30, 0x30, 0x30, 0x00],
        'g' => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x3C],
        'h' => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00],
        'i' => [0x18, 0x00, 0x38, 0x18, 0x18, 0x18, 0x3C, 0x00],
        'j' => [0x0C, 0x00, 0x1C, 0x0C, 0x0C, 0x0C, 0x6C, 0x38],
        'k' => [0x60, 0x60, 0x66, 0x6C, 0x78, 0x6C, 0x66, 0x00],
        'l' => [0x38, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00],
        'm' => [0x00, 0x00, 0x76, 0x7F, 0x6B, 0x6B, 0x63, 0x00],
        'n' => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00],
        'o' => [0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0x00],
        'p' => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60],
        'q' => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x06],
        'r' => [0x00, 0x00, 0x7C, 0x66, 0x60, 0x60, 0x60, 0x00],
        's' => [0x00, 0x00, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0x00],
        't' => [0x30, 0x30, 0x7C, 0x30, 0x30, 0x30, 0x1C, 0x00],
        'u' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x3E, 0x00],
        'v' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00],
        'w' => [0x00, 0x00, 0x63, 0x6B, 0x6B, 0x7F, 0x36, 0x00],
        'x' => [0x00, 0x00, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x00],
        'y' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x3C],
        'z' => [0x00, 0x00, 0x7E, 0x0C, 0x18, 0x30, 0x7E, 0x00],
        '{' => [0x0E, 0x18, 0x18, 0x70, 0x18, 0x18, 0x0E, 0x00],
        '|' => [0x18, 0x18, 0x18, 0x00, 0x18, 0x18, 0x18, 0x00],
        '}' => [0x70, 0x18, 0x18, 0x0E, 0x18, 0x18, 0x70, 0x00],
        '~' => [0x32, 0x4C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        _ => return None,
    };
    Some(glyph)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::tools::ToolContext;

    fn test_ctx(dir: &Path) -> ToolContext {
        ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            skills: Default::default(),
            allowed_dirs: vec![dir.to_path_buf()],
            policy: crate::tools::ToolExecutionPolicy::new(
                Vec::new(),
                true,
                "allow".into(),
                Vec::new(),
            ),
            permission_scope: Default::default(),
            cancellation: Default::default(),
            plan_state: Default::default(),
            output_store: crate::tools::ToolOutputStore::for_test(),
            browser: crate::tools::BrowserRuntime::new(dir.join("browser")),
        }
    }

    fn json_map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn pty_interactive_write_screenshot_and_close() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_ctx(tmp.path());

        // 1. Spawn an interactive `cat` session inside a real PTY.
        let exec_outcome = pty_exec(
            &json_map(json!({
                "command": "cat",
                "cols": 80,
                "rows": 24,
                "yield_time_ms": 200
            })),
            &ctx,
        );
        assert!(!exec_outcome.error, "pty_exec failed: {}", exec_outcome.text);
        let exec_json: Value = serde_json::from_str(&exec_outcome.text).expect("exec json");
        let session_id = exec_json["session_id"]
            .as_str()
            .expect("session_id")
            .to_string();
        assert_eq!(exec_json["running"], Value::Bool(true));

        // 2. Write interactive input (including ANSI color sequence via printf or plain echoed line).
        let write_outcome = pty_write(
            &json_map(json!({
                "session_id": session_id,
                "chars": "hello-nola-pty\n",
                "yield_time_ms": 400
            })),
            &ctx,
        );
        assert!(!write_outcome.error, "pty_write failed: {}", write_outcome.text);
        let write_json: Value = serde_json::from_str(&write_outcome.text).expect("write json");
        assert_eq!(write_json["running"], Value::Bool(true));
        let output = write_json["output"].as_str().unwrap_or_default();
        let screen = write_json["screen"].as_str().unwrap_or_default();
        assert!(
            output.contains("hello-nola-pty"),
            "expected echoed output to contain 'hello-nola-pty', got: {output:?}"
        );
        assert!(
            screen.contains("hello-nola-pty"),
            "expected VT100 screen to contain 'hello-nola-pty', got: {screen:?}"
        );

        // 3. Capture PNG screenshot and verify PNG magic bytes.
        let png_path = tmp.path().join("pty-capture.png");
        let shot_outcome = pty_screenshot(
            &json_map(json!({
                "session_id": session_id,
                "output_path": png_path.to_string_lossy()
            })),
            &ctx,
        );
        assert!(
            !shot_outcome.error,
            "pty_screenshot failed: {}",
            shot_outcome.text
        );
        let shot_json: Value = serde_json::from_str(&shot_outcome.text).expect("screenshot json");
        assert!(shot_json["width"].as_u64().unwrap_or(0) > 0);
        assert!(shot_json["height"].as_u64().unwrap_or(0) > 0);
        assert!(
            shot_json["screen"]
                .as_str()
                .unwrap_or_default()
                .contains("hello-nola-pty")
        );

        let bytes = std::fs::read(&png_path).expect("read png file");
        assert!(
            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "screenshot does not start with PNG magic bytes"
        );
        assert_eq!(shot_outcome.artifacts.len(), 1);
        assert_eq!(
            shot_outcome.artifacts[0].mime_type.as_deref(),
            Some("image/png")
        );
        assert_eq!(shot_outcome.artifacts[0].name, "pty-capture.png");

        // 4. Close the session cleanly.
        let close_outcome = pty_close(
            &json_map(json!({
                "session_id": session_id
            })),
            &ctx,
        );
        assert!(
            !close_outcome.error,
            "pty_close failed: {}",
            close_outcome.text
        );
        let close_json: Value = serde_json::from_str(&close_outcome.text).expect("close json");
        assert_eq!(close_json["closed"], Value::Bool(true));

        // 5. Subsequent writes to the closed session must fail cleanly.
        let after_close = pty_write(
            &json_map(json!({
                "session_id": session_id,
                "chars": "nope\n"
            })),
            &ctx,
        );
        assert!(after_close.error, "expected error after pty_close");
    }
}
