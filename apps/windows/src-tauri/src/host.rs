// SPDX-License-Identifier: Apache-2.0
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use url::Url;

pub struct Host {
    child: Child,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn read_port(path: &Path) -> Result<u16, String> {
    match fs::read_to_string(path) {
        Ok(value) => value
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| "Saved Host port is invalid. Check the desktop data folder.".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(format!("Cannot read the saved Host port: {e}")),
    }
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

impl Host {
    pub fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    pub fn start(resources: &Path, data: &Path, workspace: &Path) -> Result<(Self, Url), String> {
        fs::create_dir_all(data)
            .and_then(|_| fs::create_dir_all(workspace))
            .map_err(|e| e.to_string())?;
        let port_file = data.join("host-port");
        let port = read_port(&port_file)?;
        let bin = resources.join("bin");
        let mut command = Command::new(bin.join("nola.exe"));
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x08000000);
        let native: Value = serde_json::from_slice(
            &fs::read(resources.join("native-runtime.json"))
                .map_err(|e| format!("Missing packaged runtime: {e}"))?,
        )
        .map_err(|e| e.to_string())?;
        let backend = native["backend"]
            .as_str()
            .ok_or("Missing inference backend")?;
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
            .env(
                "NOLA_UPDATE_TARGET",
                format!("windows-{}-{backend}", std::env::consts::ARCH),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Cannot launch the packaged Host: {e}"))?;
        #[cfg(windows)]
        let job = own_tree(&child).map_err(|e| {
            let _ = child.kill();
            let _ = child.wait();
            format!("Cannot supervise the local Host: {e}")
        })?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let host = Host {
            child,
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
        let log = data.join("host-startup.log");
        thread::spawn(move || capture_diagnostics(stderr, log));
        let origin = rx
            .recv_timeout(Duration::from_secs(45))
            .ok()
            .flatten()
            .ok_or(
                "The local Host did not start. See host-startup.log in the desktop data folder.",
            )?;
        if port == 0 {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&port_file)
                .map_err(|e| e.to_string())?;
            write!(file, "{}", origin.port().unwrap())
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        Ok((host, origin))
    }
}

fn capture_diagnostics(stderr: impl Read, path: PathBuf) {
    // Persist only known startup failure categories, never raw process output.
    let mut log = fs::File::create(path).ok();
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        let category = if line.contains("incompatible") {
            "Local data format is incompatible"
        } else if line.contains("already in use") {
            "Saved Host port is already in use"
        } else if line.contains("Application Service") {
            "Local Application Service failed to start or respond"
        } else if line.contains("denied") {
            "Local resource access was denied"
        } else {
            continue;
        };
        if let Some(file) = &mut log {
            let _ = writeln!(file, "{category}");
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
}
