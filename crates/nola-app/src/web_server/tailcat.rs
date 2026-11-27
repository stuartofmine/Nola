// SPDX-License-Identifier: Apache-2.0
//! Tailcat verified helper binary management, ingress process supervision, and ephemeral transport leases.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nola_types::new_id;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::auth::ct_eq;

// --- Tailcat Verified Binary, Ingress Management & Ephemeral Transport Leases ---

pub(super) const TAILCAT_GRANT_SECONDS: u64 = 20 * 60;
pub(super) const TAILCAT_GLOBAL_LEASE_LIMIT: usize = 32;
pub(super) const TAILCAT_PER_OWNER_LEASE_LIMIT: usize = 8;
pub(super) const TAILCAT_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub(super) struct TailcatConfig {
    pub(super) ingress: bool,
    pub(super) helper_path: Option<PathBuf>,
    pub(super) helper_sha256: Option<String>,
    pub(super) web_port: u16,
}

impl TailcatConfig {
    pub(super) fn is_available(&self) -> bool {
        self.helper_path.is_some()
    }
}

pub(super) fn normalize_windows_verbatim(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

pub(super) fn verified_tailcat_command(
    program: &Path,
    expected_sha256: Option<&str>,
) -> Result<Command, String> {
    if !program.is_absolute()
        || program
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("An absolute Tailcat transport executable path without '..' is required".into());
    }
    if let Some(expected) = expected_sha256 {
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Tailcat helper SHA-256 must be exactly 64 hex characters".into());
        }
    }
    let canonical = program.canonicalize().map_err(|_| {
        "Tailcat transport executable is unavailable; no alternate program was selected"
    })?;
    if !canonical.is_file() {
        return Err("Tailcat transport executable must be an exact regular-file path".into());
    }
    if let Some(expected) = expected_sha256 {
        let mut file =
            fs::File::open(&canonical).map_err(|_| "Cannot open Tailcat transport executable")?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)
            .map_err(|_| "Cannot hash Tailcat transport executable")?;
        let actual = format!("{:x}", hasher.finalize());
        if !ct_eq(&actual, &expected.to_ascii_lowercase()) {
            return Err("Tailcat transport executable SHA-256 mismatch; launch refused".into());
        }
    }
    let exec_path = normalize_windows_verbatim(&canonical);
    let mut command = Command::new(exec_path);
    command.env_clear();
    #[cfg(not(windows))]
    command.env("PATH", "/usr/bin:/bin");
    #[cfg(windows)]
    {
        let system_root = std::env::var_os("SystemRoot")
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| std::ffi::OsString::from(r"C:\Windows"));
        command
            .env("PATH", r"C:\Windows\System32;C:\Windows")
            .env("SystemRoot", system_root);
        for key in ["TEMP", "TMP"] {
            if let Some(v) = std::env::var_os(key).filter(|val| !val.is_empty()) {
                command.env(key, v);
            }
        }
    }
    Ok(command)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TailcatLeaseCommand {
    Renew,
    Stop,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct TailcatLeaseError {
    pub(super) status: u16,
    pub(super) code: &'static str,
    pub(super) detail: Option<String>,
}

impl TailcatLeaseError {
    pub(super) fn new(status: u16, code: &'static str) -> Self {
        Self {
            status,
            code,
            detail: None,
        }
    }

    pub(super) fn with_detail(status: u16, code: &'static str, detail: Option<String>) -> Self {
        Self {
            status,
            code,
            detail,
        }
    }
}

pub(super) struct TailcatLease {
    pub(super) owner: String,
    pub(super) control: mpsc::Sender<TailcatLeaseCommand>,
    pub(super) finished: Arc<(Mutex<Option<bool>>, Condvar)>,
}

#[derive(Default)]
pub(super) struct TailcatLeaseManager {
    pub(super) leases: Mutex<HashMap<String, TailcatLease>>,
}

pub(super) fn sanitize_tailcat_helper_detail(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 96 {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("nodekey:")
        || lower.contains("psk")
    {
        return None;
    }
    if trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'))
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub(super) fn extract_tailcat_helper_error_detail(frame: &Value) -> Option<String> {
    if frame.get("protocolVersion").and_then(Value::as_u64) != Some(1)
        || frame.get("event").and_then(Value::as_str) != Some("error")
    {
        return None;
    }
    let stage = frame
        .get("stage")
        .and_then(Value::as_str)
        .and_then(sanitize_tailcat_helper_detail)?;
    let detail = frame
        .get("detail")
        .and_then(Value::as_str)
        .and_then(sanitize_tailcat_helper_detail);
    Some(match detail {
        Some(d) => format!("{stage}:{d}"),
        None => stage,
    })
}

pub(super) fn wait_or_kill_tailcat_child(
    child: &Mutex<Child>,
    grace: Duration,
) -> std::io::Result<std::process::ExitStatus> {
    let deadline = Instant::now() + grace;
    loop {
        {
            let mut guard = child.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(status) = guard.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                let _ = guard.kill();
                return guard.wait();
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl TailcatLeaseManager {
    pub(super) fn create_lease(
        self: &Arc<Self>,
        cfg: &TailcatConfig,
        host_id: &str,
        owner: String,
        client_public_key: &str,
    ) -> Result<Value, TailcatLeaseError> {
        self.create_lease_with_options(
            cfg,
            host_id,
            owner,
            client_public_key,
            Duration::from_secs(35),
            Duration::from_secs(TAILCAT_GRANT_SECONDS),
            TAILCAT_SHUTDOWN_GRACE,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_lease_with_options(
        self: &Arc<Self>,
        cfg: &TailcatConfig,
        host_id: &str,
        owner: String,
        client_public_key: &str,
        startup_timeout: Duration,
        grant_duration: Duration,
        shutdown_grace: Duration,
    ) -> Result<Value, TailcatLeaseError> {
        let Some(ref program) = cfg.helper_path else {
            return Err(TailcatLeaseError::new(409, "tailcat_not_available"));
        };
        if client_public_key.len() > 128 || !client_public_key.starts_with("nodekey:") {
            return Err(TailcatLeaseError::new(400, "invalid_transport_public_key"));
        }
        let mut command = verified_tailcat_command(program, cfg.helper_sha256.as_deref())
            .map_err(|_| TailcatLeaseError::new(503, "tailcat_helper_verification_failed"))?;

        let id = new_id();
        let (control_tx, control_rx) = mpsc::channel();
        let finished = Arc::new((Mutex::new(None), Condvar::new()));
        {
            let mut leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
            if leases.len() >= TAILCAT_GLOBAL_LEASE_LIMIT {
                return Err(TailcatLeaseError::new(503, "transport_capacity_reached"));
            }
            let owner_leases = leases.values().filter(|l| l.owner == owner).count();
            if owner_leases >= TAILCAT_PER_OWNER_LEASE_LIMIT {
                return Err(TailcatLeaseError::new(
                    429,
                    "transport_owner_capacity_reached",
                ));
            }
            leases.insert(
                id.clone(),
                TailcatLease {
                    owner,
                    control: control_tx,
                    finished: Arc::clone(&finished),
                },
            );
        }

        let mut child = match command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                self.leases
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                return Err(TailcatLeaseError::new(503, "tailcat_start_failed"));
            }
        };
        let mut input = child.stdin.take().expect("tailcat helper stdin");
        let output = child.stdout.take().expect("tailcat helper stdout");
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = stderr.take(4096).read_to_end(&mut sink);
            });
        }
        let child = Arc::new(Mutex::new(child));
        let child_for_timeout = Arc::clone(&child);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let grant_id = id.clone();
        let host_owned = host_id.to_string();
        let pub_owned = client_public_key.to_string();
        let origin = format!("http://127.0.0.1:{}", cfg.web_port);
        let mgr = Arc::clone(self);
        std::thread::spawn(move || {
            let frame = json!({
                "version": 1,
                "kind": "host",
                "host": {
                    "origin": origin,
                    "expectedHostId": host_owned,
                    "allowedClientKeys": [pub_owned]
                }
            });
            let startup: Result<String, Option<String>> = (|| {
                serde_json::to_writer(&mut input, &frame).map_err(|_| None)?;
                input
                    .write_all(b"\n")
                    .and_then(|_| input.flush())
                    .map_err(|_| None)?;
                let mut line = String::new();
                std::io::BufReader::new(output)
                    .take(64 * 1024)
                    .read_line(&mut line)
                    .map_err(|_| None)?;
                let parsed: Value = serde_json::from_str(&line).map_err(|_| None)?;
                if parsed.get("protocolVersion").and_then(Value::as_u64) != Some(1)
                    || parsed.get("event").and_then(Value::as_str) != Some("ready")
                {
                    return Err(extract_tailcat_helper_error_detail(&parsed));
                }
                parsed
                    .get("address")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .ok_or(None)
            })();
            let started = startup.is_ok();
            if ready_tx.send(startup).is_ok() && started {
                let mut deadline = Instant::now() + grant_duration;
                loop {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    match control_rx.recv_timeout(deadline.saturating_duration_since(now)) {
                        Ok(TailcatLeaseCommand::Renew) => {
                            deadline = Instant::now() + grant_duration;
                        }
                        Ok(TailcatLeaseCommand::Stop)
                        | Err(mpsc::RecvTimeoutError::Timeout)
                        | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            }
            let _ = input.write_all(b"{\"version\":1,\"kind\":\"stop\"}\n");
            let _ = input.flush();
            drop(input);
            let exit = wait_or_kill_tailcat_child(&child, shutdown_grace);
            mgr.finish_lease(&grant_id, &finished, exit);
        });

        let address = match ready_rx.recv_timeout(startup_timeout) {
            Ok(Ok(addr)) => addr,
            Ok(Err(detail)) => {
                return Err(TailcatLeaseError::with_detail(
                    503,
                    "tailcat_start_failed",
                    detail,
                ));
            }
            Err(_) => {
                let mut guard = child_for_timeout.lock().unwrap_or_else(|e| e.into_inner());
                let _ = guard.kill();
                drop(guard);
                self.leases
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                return Err(TailcatLeaseError::new(503, "tailcat_start_timeout"));
            }
        };
        Ok(json!({
            "version": 1,
            "leaseId": id,
            "address": address,
            "hostId": host_id,
            "expiresIn": TAILCAT_GRANT_SECONDS
        }))
    }

    pub(super) fn finish_lease(
        &self,
        id: &str,
        finished: &(Mutex<Option<bool>>, Condvar),
        exit: std::io::Result<std::process::ExitStatus>,
    ) {
        let (state, wake) = finished;
        *state.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(exit.as_ref().is_ok_and(|s| s.success()));
        wake.notify_all();
        self.leases
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    pub(super) fn renew_lease(&self, owner: &str, id: &str) -> Result<Value, TailcatLeaseError> {
        let leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
        let lease = leases
            .get(id)
            .ok_or_else(|| TailcatLeaseError::new(404, "transport_lease_unavailable"))?;
        if lease.owner != owner {
            return Err(TailcatLeaseError::new(
                403,
                "transport_lease_owner_mismatch",
            ));
        }
        lease
            .control
            .send(TailcatLeaseCommand::Renew)
            .map_err(|_| TailcatLeaseError::new(404, "transport_lease_unavailable"))?;
        Ok(json!({
            "version": 1,
            "leaseId": id,
            "renewed": true,
            "expiresIn": TAILCAT_GRANT_SECONDS
        }))
    }

    pub(super) fn close_lease(&self, owner: &str, id: &str) -> Result<Value, TailcatLeaseError> {
        let finished = {
            let leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
            let lease = leases
                .get(id)
                .ok_or_else(|| TailcatLeaseError::new(404, "transport_lease_unavailable"))?;
            if lease.owner != owner {
                return Err(TailcatLeaseError::new(
                    403,
                    "transport_lease_owner_mismatch",
                ));
            }
            let _ = lease.control.send(TailcatLeaseCommand::Stop);
            Arc::clone(&lease.finished)
        };
        let (state, wake) = &*finished;
        let guard = state.lock().unwrap_or_else(|e| e.into_inner());
        let (state, _) = wake
            .wait_timeout_while(guard, TAILCAT_SHUTDOWN_GRACE, |v| v.is_none())
            .unwrap_or_else(|e| e.into_inner());
        Ok(json!({
            "closed": *state == Some(true),
            "closing": state.is_none(),
            "stopUnconfirmed": *state == Some(false),
            "runStateChanged": false
        }))
    }

    pub(super) fn shutdown_all(&self) {
        let active: Vec<Arc<(Mutex<Option<bool>>, Condvar)>> = {
            let leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
            leases
                .values()
                .map(|l| {
                    let _ = l.control.send(TailcatLeaseCommand::Stop);
                    Arc::clone(&l.finished)
                })
                .collect()
        };
        let deadline = Instant::now() + TAILCAT_SHUTDOWN_GRACE;
        for finished in active {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let (state, wake) = &*finished;
            let guard = state.lock().unwrap_or_else(|e| e.into_inner());
            let _ = wake
                .wait_timeout_while(guard, remaining, |v| v.is_none())
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

pub(super) struct TailcatIngressChild {
    pub(super) stdin: Option<std::process::ChildStdin>,
    pub(super) child: Mutex<Child>,
}

impl TailcatIngressChild {
    pub(super) fn spawn(cfg: &TailcatConfig, host_id: &str) -> Result<Self, String> {
        let Some(ref helper) = cfg.helper_path else {
            return Err("--tailcat-ingress requires --tailcat-helper".into());
        };
        let mut cmd = verified_tailcat_command(helper, cfg.helper_sha256.as_deref())?;
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed spawning Tailcat ingress helper: {e}"))?;
        let mut stdin = child.stdin.take();
        if let Some(ref mut input) = stdin {
            let init = json!({
                "version": 1,
                "kind": "ingress",
                "host": {
                    "origin": format!("http://127.0.0.1:{}", cfg.web_port),
                    "expectedHostId": host_id
                }
            });
            let _ = serde_json::to_writer(&mut *input, &init);
            let _ = input.write_all(b"\n");
            let _ = input.flush();
        }
        if let Some(stdout) = child.stdout.take() {
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = stdout.take(64 * 1024).read_to_end(&mut sink);
            });
        }
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = stderr.take(16 * 1024).read_to_end(&mut sink);
            });
        }
        Ok(Self {
            stdin,
            child: Mutex::new(child),
        })
    }

    pub(super) fn shutdown(mut self) {
        if let Some(mut input) = self.stdin.take() {
            let _ = input.write_all(b"{\"version\":1,\"kind\":\"stop\"}\n");
            let _ = input.flush();
        }
        let _ = wait_or_kill_tailcat_child(&self.child, TAILCAT_SHUTDOWN_GRACE);
    }
}

