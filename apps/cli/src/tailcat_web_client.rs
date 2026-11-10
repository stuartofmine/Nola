// SPDX-License-Identifier: Apache-2.0
//! CLI-owned optional remote Web transport. No local Agent/Core authority,
//! credential storage, browser auth substitution or business request replay.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Options {
    origin: reqwest::Url,
    host: String,
    program: PathBuf,
    digest: String,
    control: bool,
    json: bool,
}
const USAGE:&str="nola web --connect-host <Host origin> --expected-host-id <ID> --tailcat-helper <absolute executable> --tailcat-helper-sha256 <SHA256> --token-stdin [--control-stdin] [--json]";

fn parse(args: &[String]) -> Result<Options, String> {
    let mut origin = None;
    let mut host = None;
    let mut program = None;
    let mut digest = None;
    let mut token_input = false;
    let mut control = false;
    let mut json = false;
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--connect-host" => origin = Some(args.next().ok_or(USAGE)?.clone()),
            "--expected-host-id" => host = Some(args.next().ok_or(USAGE)?.clone()),
            "--tailcat-helper" => program = Some(PathBuf::from(args.next().ok_or(USAGE)?)),
            "--tailcat-helper-sha256" => digest = Some(args.next().ok_or(USAGE)?.clone()),
            "--token-stdin" => token_input = true,
            "--control-stdin" => control = true,
            "--json" => json = true,
            _ => return Err(USAGE.into()),
        }
    }
    if !token_input {
        return Err("Provide the existing Nola Host access token through --token-stdin, never argv or environment".into());
    }
    let origin = validated_origin(&origin.ok_or(USAGE)?)?;
    let host = host.filter(|value| !value.trim().is_empty()).ok_or(USAGE)?;
    let program = program.ok_or(USAGE)?;
    let digest = digest.ok_or(USAGE)?;
    crate::tailcat_transport::verified_command(&program, &digest)?;
    Ok(Options {
        origin,
        host,
        program,
        digest,
        control,
        json,
    })
}

fn validated_origin(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "Invalid Host origin")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("Use a root Host origin without credentials, query or fragment".into());
    }
    let private = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        Some(url::Host::Ipv6(ip)) => {
            ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
        }
        Some(url::Host::Domain(name)) => {
            name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local")
        }
        None => false,
    };
    if url.scheme() != "https" && !(url.scheme() == "http" && private) {
        return Err("Host bootstrap requires HTTPS or private-network HTTP".into());
    }
    Ok(url)
}

struct SignalRegistrations(Vec<signal_hook::SigId>);
impl Drop for SignalRegistrations {
    fn drop(&mut self) {
        for id in self.0.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

struct Bridge {
    child: Child,
    input: Option<ChildStdin>,
    events: mpsc::Receiver<Result<Value, ()>>,
}
impl Bridge {
    fn start(mut command: Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "Tailcat helper launch failed")?;
        let input = child.stdin.take();
        let output = child.stdout.take().expect("private helper output");
        let (send, events) = mpsc::channel();
        std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut line = String::new();
                let value = match output.by_ref().take(65537).read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) if line.len() <= 65536 => serde_json::from_str(&line).map_err(|_| ()),
                    _ => Err(()),
                };
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            events,
        })
    }
    fn send(&mut self, value: Value) -> Result<(), String> {
        let input = self.input.as_mut().ok_or("Transport control closed")?;
        serde_json::to_writer(&mut *input, &value).map_err(|_| "Transport control failed")?;
        input
            .write_all(b"\n")
            .and_then(|_| input.flush())
            .map_err(|_| "Transport control failed".into())
    }
    fn event(&mut self, kind: &str, stop: &AtomicBool) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(35);
        loop {
            if stop.load(Ordering::SeqCst) {
                return Err("Web transport startup cancelled".into());
            }
            match self.events.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(value)) if value["protocolVersion"] == 1 && value["event"] == kind => {
                    return Ok(value)
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Tailcat helper startup failed".into())
                }
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(_) => return Err("Tailcat helper startup timed out".into()),
            }
        }
    }
    fn close(&mut self) -> Result<(), String> {
        // EOF closes startup as well as active transport. No signal is sent.
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return if status.success() {
                        Ok(())
                    } else {
                        Err("Tailcat helper exited with failure".into())
                    }
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => return Err(
                    "Tailcat helper stop remains unconfirmed; no Run cancellation ACK is implied"
                        .into(),
                ),
            }
        }
    }
}

fn exchange(
    client: &reqwest::blocking::Client,
    origin: &reqwest::Url,
    token: &str,
    payload: Value,
    timeout: Duration,
) -> Result<Value, String> {
    use nola_core::mobile_crypto::*;
    let endpoint = origin
        .join("/api/mobile/transport/tailcat")
        .map_err(|_| "Invalid Host origin")?;
    let body = serde_json::to_vec(&encrypt_mobile_json(&payload, token)?)
        .map_err(|_| "Cannot encode transport request")?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "System clock unavailable")?
        .as_secs();
    let nonce = generate_protocol_nonce("web-client");
    let digest = mobile_body_sha256(&body);
    let proof = mobile_auth_proof(token, "POST", endpoint.path(), timestamp, &nonce, &digest);
    let response = client
        .post(endpoint)
        .timeout(timeout)
        .header("Content-Type", "application/json")
        .header("X-Nola-Mobile-Timestamp", timestamp)
        .header("X-Nola-Mobile-Nonce", nonce)
        .header("X-Nola-Mobile-Body-SHA256", digest)
        .header("X-Nola-Mobile-Auth", proof)
        .body(body)
        .send()
        .map_err(|_| "Host transport request failed")?;
    if !response.status().is_success() {
        return Err(format!(
            "Host transport refused HTTP {}",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "Host transport response failed")?;
    if bytes.len() > 65536 {
        return Err("Host transport response exceeds limit".into());
    }
    let envelope =
        serde_json::from_slice(&bytes).map_err(|_| "Host transport response is not encrypted")?;
    decrypt_mobile_json(&envelope, token)
}

pub(crate) fn run_if_requested(args: &[String]) -> Option<Result<(), String>> {
    if args.first().map(String::as_str) != Some("web")
        || !args.iter().any(|arg| arg == "--connect-host")
    {
        return None;
    }
    Some(run(args))
}
fn run(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    if std::io::stdin().is_terminal() {
        return Err(
            "--token-stdin requires private piped input; terminal echo is not used for credentials"
                .into(),
        );
    }
    let mut reader = BufReader::new(std::io::stdin());
    let mut token = String::new();
    reader
        .by_ref()
        .take(8193)
        .read_line(&mut token)
        .map_err(|_| "Cannot read Host credential")?;
    if token.len() > 8192 || token.trim().is_empty() {
        return Err("Invalid Host credential input".into());
    }
    let token = token.trim().to_owned();
    let stop = Arc::new(AtomicBool::new(false));
    let mut signals = SignalRegistrations(Vec::new());
    let interrupt = signal_hook::flag::register(signal_hook::consts::SIGINT, stop.clone())
        .map_err(|_| "Cannot register Web transport lifecycle")?;
    signals.0.push(interrupt);
    let terminate = signal_hook::flag::register(signal_hook::consts::SIGTERM, stop.clone())
        .map_err(|_| "Cannot register Web transport lifecycle")?;
    signals.0.push(terminate);
    if options.control {
        let stop = stop.clone();
        std::thread::spawn(move || {
            // The private owner channel accepts only stop or EOF, never prompts.
            let mut line = String::new();
            let _ = reader.take(64).read_line(&mut line);
            stop.store(true, Ordering::SeqCst);
        });
    }
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(40))
        .build()
        .map_err(|_| "Cannot initialize Host transport")?;
    let status = (|| -> Result<Value, String> {
        let response = client
            .get(options.origin.join("/api/web/session").unwrap())
            .timeout(Duration::from_secs(8))
            .send()
            .map_err(|_| "Host identity inspection unavailable")?;
        if !response.status().is_success() {
            return Err("Host identity inspection refused".into());
        }
        let mut bytes = Vec::new();
        response
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot inspect Host identity")?;
        if bytes.len() > 65536 {
            return Err("Host identity response exceeds limit".into());
        }
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Host identity response".into())
    })()?;
    if status["hostId"].as_str() != Some(&options.host)
        || status["localTrust"] != false
        || status["tailcatAvailable"] != true
    {
        return Err("Expected authenticated Tailcat Host was not verified".into());
    }
    let mut bridge = Bridge::start(crate::tailcat_transport::verified_command(
        &options.program,
        &options.digest,
    )?)?;
    let mut lease = None;
    let result = (|| -> Result<(), String> {
        bridge.send(json!({"version":1,"kind":"client"}))?;
        let identity = bridge.event("identity", &stop)?;
        let public = identity["clientPublicKey"]
            .as_str()
            .ok_or("Missing ephemeral client identity")?;
        let requested = Instant::now();
        let grant = exchange(
            &client,
            &options.origin,
            &token,
            json!({"operation":"connect","expectedHostId":options.host,"clientPublicKey":public}),
            Duration::from_secs(40),
        )?;
        if grant["version"] != 1 || grant["hostId"].as_str() != Some(&options.host) {
            return Err("Host grant identity mismatch".into());
        }
        let id = grant["leaseId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("Missing temporary lease")?
            .to_owned();
        lease = Some(id);
        let seconds = grant["expiresIn"]
            .as_u64()
            .filter(|value| (31..=1200).contains(value))
            .ok_or("Invalid temporary lease expiry")?;
        let address = grant["address"]
            .as_str()
            .ok_or("Missing temporary transport address")?;
        bridge.send(
            json!({"version":1,"kind":"connect","address":address,"expectedHostId":options.host}),
        )?;
        let ready = bridge.event("ready", &stop)?;
        let local = ready["origin"].as_str().ok_or("Missing local Web origin")?;
        let local = validated_origin(local)?;
        if local.host_str() != Some("127.0.0.1")
            || local.scheme() != "http"
            || local.port().is_none()
        {
            return Err("Unsafe local Web endpoint".into());
        }
        let deadline = requested + Duration::from_secs(seconds);
        if options.json {
            println!(
                "{}",
                json!({"type":"web.remote.started","protocol_version":1,"origin":local.as_str(),"host_id":options.host,"transport":"tailcat","auth":"web_session","expires_in":deadline.saturating_duration_since(Instant::now()).as_secs()})
            );
        } else {
            println!("Nola Web client: {local}\nNormal Host sign-in is required. Temporary connection; Ctrl-C to close.");
        }
        while !stop.load(Ordering::SeqCst) {
            if Instant::now() >= deadline {
                return Err(
                    "Temporary Web connection expired; reconnect. Host Tasks were not cancelled"
                        .into(),
                );
            }
            if bridge
                .child
                .try_wait()
                .map_err(|_| "Cannot inspect owned transport")?
                .is_some()
            {
                return Err("Web transport exited; no Run stop ACK is implied".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    })();
    let closed = bridge.close();
    // Best effort remote release, using the same frozen Host credential. The
    // finite lease remains the bound when bootstrap is no longer reachable.
    if let Some(id) = lease {
        let _ = exchange(
            &client,
            &options.origin,
            &token,
            json!({"operation":"close","leaseId":id}),
            Duration::from_secs(6),
        );
    }
    result.and(closed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_web_origin_is_exact_and_does_not_accept_credentials_or_public_http() {
        for raw in ["http://10.0.2.2:5001", "https://host.example"] {
            assert!(validated_origin(raw).is_ok());
        }
        for raw in [
            "http://10.0.0.1.example.com",
            "http://8.8.8.8",
            "https://user:secret@host.example",
            "https://host.example/api",
            "https://host.example/?token=secret",
        ] {
            assert!(validated_origin(raw).is_err());
        }
        assert!(run_if_requested(&["web".into()]).is_none());
    }
}
