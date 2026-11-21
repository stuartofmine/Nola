// SPDX-License-Identifier: Apache-2.0
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use url::Url;

const PREFERRED_PORT_START: u16 = 28_420;
const PREFERRED_PORT_SPAN: u16 = 1_000;
const PREFERRED_PORT_PROBES: u16 = 64;
const MAX_DIAGNOSTIC_LOG_BYTES: usize = 16 * 1024;
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_millis(2500);
static PORT_TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub struct Host {
    child: Child,
    stdin: Option<ChildStdin>,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop();
    }
}

fn canonical_backend(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "cpu" => Some("cpu"),
        "vulkan" => Some("vulkan"),
        "cuda" => Some("cuda"),
        _ => None,
    }
}

fn update_target_for_backend(arch: &str, backend: &str) -> String {
    match (arch, canonical_backend(backend)) {
        ("x86_64", Some(valid)) => format!("windows-x86_64-{valid}"),
        _ => format!("windows-{arch}-cpu"),
    }
}

fn wait_for_port_available(port: u16, timeout: Duration) -> Result<(), String> {
    if port == 0 {
        return Ok(());
    }
    let deadline = Instant::now() + timeout;
    loop {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                drop(listener);
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                if Instant::now() >= deadline {
                    return Err(crate::native::label("hostPortInUse").into());
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                return Err(crate::native::label("hostPortInUse").into());
            }
        }
    }
}

pub fn read_port(path: &Path) -> Result<u16, String> {
    match fs::read_to_string(path) {
        Ok(value) => value
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| crate::native::label("hostPortInUse").into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(_) => Err(crate::native::label("hostPortInUse").into()),
    }
}

fn write_port_atomic(path: &Path, port: u16) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let seq = PORT_TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("host-port");
    let tmp = parent.join(format!("{file_name}.tmp.{}.{seq}", std::process::id()));
    let write_result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        write!(file, "{port}")?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    Ok(())
}

fn select_preferred_port() -> u16 {
    let seed = (std::process::id() as u16)
        .wrapping_add(PORT_TEMP_SEQ.fetch_add(1, Ordering::Relaxed) as u16);
    let start_offset = seed % PREFERRED_PORT_SPAN;
    for step in 0..PREFERRED_PORT_PROBES {
        let candidate = PREFERRED_PORT_START + ((start_offset + step) % PREFERRED_PORT_SPAN);
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", candidate)) {
            drop(listener);
            return candidate;
        }
    }
    0
}

fn choose_bind_port(port_file: &Path, timeout: Duration) -> (u16, u16) {
    let saved_port = read_port(port_file).unwrap_or(0);
    if saved_port != 0 && wait_for_port_available(saved_port, timeout).is_ok() {
        return (saved_port, saved_port);
    }
    (saved_port, select_preferred_port())
}

fn handshake(data: &[u8], expected: u16) -> Option<Url> {
    let value: Value = serde_json::from_slice(data).ok()?;
    if value["type"] != "server.started"
        || value["protocol_version"] != 1
        || value["loopback"] != true
    {
        return None;
    }
    let url = Url::parse(value["origin"].as_str()?).ok()?;
    let port = url.port()?;
    if port == 0
        || (expected != 0 && expected != port)
        || url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(url)
}

#[cfg(windows)]
fn own_tree(child: &Child) -> std::io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::*;
    unsafe {
        let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if raw.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = OwnedHandle::from_raw_handle(raw);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        ) == 0
            || AssignProcessToJobObject(raw, child.as_raw_handle()) == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }
}

fn signal_graceful_shutdown(pid: u32) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
        let _ = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid);
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn stop_child_gracefully(child: &mut Child, stdin: &mut Option<ChildStdin>, grace: Duration) {
    // Closing the stdin pipe first signals EOF to cooperative children.
    drop(stdin.take());
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    signal_graceful_shutdown(child.id());
    let deadline = Instant::now() + grace;
    loop {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) if Instant::now() >= deadline => break,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Host {
    pub fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    pub fn stop(&mut self) {
        stop_child_gracefully(&mut self.child, &mut self.stdin, GRACEFUL_STOP_TIMEOUT);
    }

    pub fn start(resources: &Path, data: &Path, workspace: &Path) -> Result<(Self, Url), String> {
        fs::create_dir_all(data)
            .and_then(|_| fs::create_dir_all(workspace))
            .map_err(|_| crate::native::label("hostStartFailed").to_string())?;
        let port_file = data.join("host-port");
        let (saved_port, target_port) = choose_bind_port(&port_file, Duration::from_millis(1500));
        let (host, origin) = match Self::spawn_and_handshake(resources, data, workspace, target_port)
        {
            Ok(started) => started,
            Err(_) if target_port != 0 => Self::spawn_and_handshake(resources, data, workspace, 0)?,
            Err(err) => return Err(err),
        };
        let bound_port = origin.port().unwrap();
        if bound_port != saved_port || read_port(&port_file).ok() != Some(bound_port) {
            write_port_atomic(&port_file, bound_port)
                .map_err(|_| crate::native::label("hostStartFailed").to_string())?;
        }
        Ok((host, origin))
    }

    fn spawn_and_handshake(
        resources: &Path,
        data: &Path,
        workspace: &Path,
        port: u16,
    ) -> Result<(Self, Url), String> {
        let log_path = data.join("host-startup.log");
        let bin = resources.join("bin");
        let mut command = Command::new(bin.join("nola.exe"));
        #[cfg(windows)]
        {
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            std::os::windows::process::CommandExt::creation_flags(
                &mut command,
                CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP,
            );
        }
        let native_bytes = fs::read(resources.join("native-runtime.json")).map_err(|e| {
            record_startup_outcome(
                &log_path,
                &format!("Startup failed: missing native-runtime.json ({:?})", e.kind()),
            );
            crate::native::label("hostRuntimeMissing").to_string()
        })?;
        let native: Value = serde_json::from_slice(&native_bytes).map_err(|_| {
            record_startup_outcome(
                &log_path,
                "Startup failed: native-runtime.json is malformed",
            );
            crate::native::label("hostRuntimeMissing").to_string()
        })?;
        let raw_backend = native["backend"].as_str().ok_or_else(|| {
            record_startup_outcome(
                &log_path,
                "Startup failed: native-runtime.json is missing inference backend",
            );
            crate::native::label("hostRuntimeMissing").to_string()
        })?;
        let backend = canonical_backend(raw_backend).unwrap_or("cpu");
        let update_target = update_target_for_backend(std::env::consts::ARCH, backend);
        let mut child = command
            .args([
                "web",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--web-root",
            ])
            .arg(resources.join("web"))
            .arg("--json")
            .current_dir(workspace)
            .env("NOLA_AGENT_HOST_PATH", bin.join("nola-agent-host.exe"))
            .env("NOLA_LLAMA_SERVER", bin.join("llama-server.exe"))
            .env("NOLA_LLAMA_BACKEND", backend)
            .env("NOLA_DESKTOP_PLATFORM", "windows")
            .env("NOLA_UPDATE_TARGET", update_target)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                record_startup_outcome(
                    &log_path,
                    &format!("Startup failed: could not launch packaged Host ({:?})", e.kind()),
                );
                crate::native::label("hostLaunchFailed").to_string()
            })?;
        #[cfg(windows)]
        let job = own_tree(&child).map_err(|e| {
            let _ = child.kill();
            let _ = child.wait();
            record_startup_outcome(
                &log_path,
                &format!("Startup failed: could not supervise Host ({:?})", e.kind()),
            );
            crate::native::label("hostSupervisionFailed").to_string()
        })?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let mut host = Host {
            child,
            stdin,
            #[cfg(windows)]
            _job: job,
        };
        let (tx, rx) = mpsc::sync_channel(1);
        // The first line contains a credential. Parse it only in native memory and drain the pipe.
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut bytes = Vec::new();
            let value = reader
                .by_ref()
                .take(65537)
                .read_until(b'\n', &mut bytes)
                .ok()
                .filter(|n| *n <= 65536)
                .and_then(|_| handshake(&bytes, port));
            let _ = tx.send(value);
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
        });
        let startup_complete = Arc::new(AtomicBool::new(false));
        let diag_complete = Arc::clone(&startup_complete);
        let diag_log = log_path.clone();
        thread::spawn(move || capture_diagnostics(stderr, diag_log, diag_complete));
        let origin = match rx.recv_timeout(Duration::from_secs(45)) {
            Ok(Some(origin)) => {
                startup_complete.store(true, Ordering::Release);
                origin
            }
            Ok(None) => {
                startup_complete.store(true, Ordering::Release);
                let outcome = match host.child.try_wait() {
                    Ok(Some(status)) => match status.code() {
                        Some(code) => {
                            format!("Startup failed: Host exited with code {code} before handshake")
                        }
                        None => "Startup failed: Host terminated before handshake".to_string(),
                    },
                    _ => "Startup failed: Host closed stdout without a valid handshake".to_string(),
                };
                record_startup_outcome(&log_path, &outcome);
                return Err(crate::native::label("hostStartFailed").into());
            }
            Err(_) => {
                startup_complete.store(true, Ordering::Release);
                record_startup_outcome(
                    &log_path,
                    "Startup failed: timed out waiting for Host startup handshake",
                );
                return Err(crate::native::label("hostStartFailed").into());
            }
        };
        Ok((host, origin))
    }
}

fn classify_diagnostic_line(line: &str) -> Option<&'static str> {
    let lower = line.to_ascii_lowercase();
    if lower.contains("incompatible") {
        Some("Local data format is incompatible")
    } else if lower.contains("already in use") || lower.contains("addrinuse") {
        Some("Saved Host port is already in use")
    } else if line.contains("Application Service") {
        Some("Local Application Service failed to start or respond")
    } else if lower.contains("denied") || lower.contains("permissiondenied") {
        Some("Local resource access was denied")
    } else if lower.contains("panicked") || lower.contains("panic") {
        Some("Local Host terminated due to an internal panic")
    } else if lower.contains("sqlite") || lower.contains("database") {
        Some("Local database could not be opened")
    } else if lower.contains("web-root")
        || lower.contains("web root")
        || lower.contains("not found")
        || lower.contains("no such file")
    {
        Some("Required packaged resource was not found")
    } else if lower.contains("failed to bind") || lower.contains("cannot assign requested address") {
        Some("Local Host could not bind its loopback address")
    } else {
        None
    }
}

fn record_startup_outcome(path: &Path, outcome: &str) {
    let current_len = fs::metadata(path).map(|meta| meta.len() as usize).unwrap_or(0);
    if current_len + outcome.len() + 1 > MAX_DIAGNOSTIC_LOG_BYTES {
        return;
    }
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{outcome}");
    }
}

fn capture_diagnostics(stderr: impl Read, path: PathBuf, startup_complete: Arc<AtomicBool>) {
    // Persist only known startup failure categories, never raw process output.
    let mut log = fs::File::create(path).ok();
    let mut bytes_written = 0usize;
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if startup_complete.load(Ordering::Acquire) {
            // Release the file handle once startup finishes and drain remaining stderr to sink.
            drop(log.take());
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
            break;
        }
        let Some(category) = classify_diagnostic_line(&line) else {
            continue;
        };
        let needed = category.len() + 1;
        if bytes_written + needed > MAX_DIAGNOSTIC_LOG_BYTES {
            log = None;
            continue;
        }
        if let Some(file) = &mut log {
            if writeln!(file, "{category}").is_ok() {
                bytes_written += needed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_the_owning_childs_loopback_origin() {
        for (origin, valid) in [
            ("http://127.0.0.1:5010", true),
            ("http://127.0.0.1:5009", false),
            ("http://user@127.0.0.1:5010", false),
            ("http://example.org:5010", false),
        ] {
            let frame = serde_json::json!({"origin":origin,"loopback":true,"type":"server.started","protocol_version":1});
            assert_eq!(
                handshake(frame.to_string().as_bytes(), 5010).is_some(),
                valid
            );
        }
    }

    #[test]
    fn validates_update_target_backend() {
        assert_eq!(
            update_target_for_backend("x86_64", "cpu"),
            "windows-x86_64-cpu"
        );
        assert_eq!(
            update_target_for_backend("x86_64", "vulkan"),
            "windows-x86_64-vulkan"
        );
        assert_eq!(
            update_target_for_backend("x86_64", "cuda"),
            "windows-x86_64-cuda"
        );
        assert_eq!(
            update_target_for_backend("x86_64", "external"),
            "windows-x86_64-cpu"
        );
        assert_eq!(
            update_target_for_backend("aarch64", "vulkan"),
            "windows-aarch64-cpu"
        );
    }

    #[test]
    fn diagnostics_are_bounded_and_stop_writing_after_startup() {
        let dir = std::env::temp_dir().join(format!(
            "nola-host-diag-{}-{}",
            std::process::id(),
            PORT_TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("host-startup.log");

        let sample = "thread 'main' panicked at foo\nSQLITE error: locked\n";
        let complete = Arc::new(AtomicBool::new(false));
        capture_diagnostics(sample.as_bytes(), log_path.clone(), Arc::clone(&complete));
        let content = fs::read_to_string(&log_path).unwrap();
        assert!(content.contains("Local Host terminated due to an internal panic"));
        assert!(content.contains("Local database could not be opened"));

        // When startup is already complete, nothing is written to the log.
        complete.store(true, Ordering::Release);
        capture_diagnostics("access denied\n".as_bytes(), log_path.clone(), complete);
        assert_eq!(fs::read_to_string(&log_path).unwrap(), "");

        // Outcome recording ensures non-empty diagnostics on failure.
        record_startup_outcome(&log_path, "Startup failed: timed out");
        assert!(fs::read_to_string(&log_path)
            .unwrap()
            .contains("Startup failed: timed out"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn stop_child_gracefully_waits_for_clean_exit_and_kills_hung_process() {
        let mut cooperative = Command::new("sh")
            .args(["-c", "trap 'exit 0' TERM; while true; do sleep 1; done"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = cooperative.stdin.take();
        stop_child_gracefully(&mut cooperative, &mut stdin, Duration::from_millis(1500));
        assert!(matches!(cooperative.try_wait(), Ok(Some(_))));

        let mut stubborn = Command::new("sh")
            .args(["-c", "trap '' TERM; while true; do sleep 1; done"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = stubborn.stdin.take();
        stop_child_gracefully(&mut stubborn, &mut stdin, Duration::from_millis(200));
        assert!(matches!(stubborn.try_wait(), Ok(Some(_))));
    }

    #[test]
    fn waits_for_pinned_port_release_before_failing() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(wait_for_port_available(port, Duration::from_millis(200)).is_err());
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            drop(listener);
        });
        assert!(wait_for_port_available(port, Duration::from_millis(1500)).is_ok());
        releaser.join().unwrap();
    }

    #[test]
    fn write_port_atomic_creates_and_replaces_without_temp_files() {
        let dir = std::env::temp_dir().join(format!(
            "nola-host-port-atomic-{}-{}",
            std::process::id(),
            PORT_TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let port_file = dir.join("host-port");
        write_port_atomic(&port_file, 28_501).unwrap();
        assert_eq!(read_port(&port_file).unwrap(), 28_501);
        write_port_atomic(&port_file, 28_502).unwrap();
        assert_eq!(read_port(&port_file).unwrap(), 28_502);
        let entries: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["host-port".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn choose_bind_port_rebinds_when_saved_port_is_occupied_or_corrupt() {
        let dir = std::env::temp_dir().join(format!(
            "nola-host-port-rebind-{}-{}",
            std::process::id(),
            PORT_TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let port_file = dir.join("host-port");

        let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let occupied_port = occupied.local_addr().unwrap().port();
        write_port_atomic(&port_file, occupied_port).unwrap();
        let (saved, target) = choose_bind_port(&port_file, Duration::from_millis(100));
        assert_eq!(saved, occupied_port);
        assert_ne!(target, occupied_port);
        assert!(target == 0 || (PREFERRED_PORT_START..PREFERRED_PORT_START + PREFERRED_PORT_SPAN).contains(&target));
        drop(occupied);

        fs::write(&port_file, "").unwrap();
        let (saved_empty, target_empty) = choose_bind_port(&port_file, Duration::from_millis(100));
        assert_eq!(saved_empty, 0);
        assert!(target_empty == 0 || (PREFERRED_PORT_START..PREFERRED_PORT_START + PREFERRED_PORT_SPAN).contains(&target_empty));

        let _ = fs::remove_dir_all(&dir);
    }
}
