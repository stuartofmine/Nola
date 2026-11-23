// SPDX-License-Identifier: Apache-2.0
// Ephemeral network grants only. Nola authentication, Run ownership and ACKs
// remain unchanged; no transport key or grant is written to the store.
const TAILCAT_GRANT_SECONDS: u64 = 20 * 60;
const TAILCAT_GLOBAL_LEASE_LIMIT: usize = 32;
const TAILCAT_PER_OWNER_LEASE_LIMIT: usize = 8;
const TAILCAT_SHUTDOWN_GRACE: StdDuration = StdDuration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
enum TailcatLeaseCommand {
    Renew,
    Stop,
}

#[derive(Debug, PartialEq, Eq)]
struct TailcatLeaseError {
    status: u16,
    code: &'static str,
    detail: Option<String>,
}

impl TailcatLeaseError {
    fn new(status: u16, code: &'static str) -> Self {
        Self {
            status,
            code,
            detail: None,
        }
    }

    fn with_detail(status: u16, code: &'static str, detail: Option<String>) -> Self {
        Self {
            status,
            code,
            detail,
        }
    }
}

struct TailcatLease {
    owner: String,
    control: mpsc::Sender<TailcatLeaseCommand>,
    finished: Arc<(Mutex<Option<bool>>, std::sync::Condvar)>,
}

static TAILCAT_LEASES: OnceLock<Mutex<HashMap<String, TailcatLease>>> = OnceLock::new();

fn tailcat_leases() -> &'static Mutex<HashMap<String, TailcatLease>> {
    TAILCAT_LEASES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_tailcat_leases() -> std::sync::MutexGuard<'static, HashMap<String, TailcatLease>> {
    tailcat_leases()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

fn tailcat_owner(token: &str) -> String {
    web_secret_digest(token)
}

fn sanitize_helper_detail(raw: &str) -> Option<String> {
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

fn extract_helper_error_detail(frame: &Value) -> Option<String> {
    if frame.get("protocolVersion").and_then(Value::as_u64) != Some(1)
        || frame.get("event").and_then(Value::as_str) != Some("error")
    {
        return None;
    }
    let stage = frame
        .get("stage")
        .and_then(Value::as_str)
        .and_then(sanitize_helper_detail)?;
    let detail = frame
        .get("detail")
        .and_then(Value::as_str)
        .and_then(sanitize_helper_detail);
    Some(match detail {
        Some(detail) => format!("{stage}:{detail}"),
        None => stage,
    })
}

fn mobile_tailcat_transport_response(
    request: &mut Request,
    config: &Config,
) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error":"unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let owner = tailcat_owner(&token);
    let result = match body.get("operation").and_then(Value::as_str) {
        Some("connect") => {
            if body.get("expectedHostId").and_then(Value::as_str) != Some(host_identity()) {
                Err(TailcatLeaseError::new(409, "transport_host_identity_mismatch"))
            } else if let Some(public) = body.get("clientPublicKey").and_then(Value::as_str) {
                create_tailcat_lease(config, owner, public)
            } else {
                Err(TailcatLeaseError::new(400, "client_public_key_required"))
            }
        }
        Some("renew") => body
            .get("leaseId")
            .and_then(Value::as_str)
            .ok_or_else(|| TailcatLeaseError::new(400, "transport_lease_required"))
            .and_then(|lease| renew_tailcat_lease(&owner, lease)),
        Some("close") => body
            .get("leaseId")
            .and_then(Value::as_str)
            .ok_or_else(|| TailcatLeaseError::new(400, "transport_lease_required"))
            .and_then(|lease| close_tailcat_lease(&owner, lease)),
        _ => Err(TailcatLeaseError::new(400, "unknown_transport_operation")),
    };
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200), value, &token),
        Err(err) => {
            let payload = match err.detail {
                Some(detail) => json!({"error": err.code, "detail": detail}),
                None => json!({"error": err.code}),
            };
            mobile_encrypted_json_response(StatusCode(err.status), payload, &token)
        }
    }
}

fn create_tailcat_lease(
    config: &Config,
    owner: String,
    public: &str,
) -> Result<Value, TailcatLeaseError> {
    create_tailcat_lease_with_options(
        config,
        owner,
        public,
        StdDuration::from_secs(35),
        StdDuration::from_secs(TAILCAT_GRANT_SECONDS),
        TAILCAT_SHUTDOWN_GRACE,
    )
}

fn create_tailcat_lease_with_timeout(
    config: &Config,
    owner: String,
    public: &str,
    startup_timeout: StdDuration,
) -> Result<Value, TailcatLeaseError> {
    create_tailcat_lease_with_options(
        config,
        owner,
        public,
        startup_timeout,
        StdDuration::from_secs(TAILCAT_GRANT_SECONDS),
        TAILCAT_SHUTDOWN_GRACE,
    )
}

fn wait_or_kill_child(
    child: &Mutex<std::process::Child>,
    grace: StdDuration,
) -> std::io::Result<std::process::ExitStatus> {
    let deadline = Instant::now() + grace;
    loop {
        {
            let mut guard = child.lock().unwrap_or_else(|err| err.into_inner());
            if let Some(status) = guard.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                let _ = guard.kill();
                return guard.wait();
            }
        }
        thread::sleep(StdDuration::from_millis(20));
    }
}

fn create_tailcat_lease_with_options(
    config: &Config,
    owner: String,
    public: &str,
    startup_timeout: StdDuration,
    grant_duration: StdDuration,
    shutdown_grace: StdDuration,
) -> Result<Value, TailcatLeaseError> {
    let (program, digest) = config
        .tailcat_helper
        .as_ref()
        .ok_or_else(|| TailcatLeaseError::new(409, "tailcat_not_available"))?;
    if public.len() > 128 || !public.starts_with("nodekey:") {
        return Err(TailcatLeaseError::new(400, "invalid_transport_public_key"));
    }
    let mut command = crate::tailcat_transport::verified_command(program, digest)
        .map_err(|_| TailcatLeaseError::new(503, "tailcat_helper_verification_failed"))?;
    let id = uuid::Uuid::new_v4().to_string();
    let (control_tx, control_rx) = mpsc::channel();
    let finished = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
    {
        let mut leases = lock_tailcat_leases();
        // A bounded number of ephemeral network processes, not a model budget.
        if leases.len() >= TAILCAT_GLOBAL_LEASE_LIMIT {
            return Err(TailcatLeaseError::new(503, "transport_capacity_reached"));
        }
        let owner_leases = leases
            .values()
            .filter(|lease| lease.owner == owner)
            .count();
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
                finished: finished.clone(),
            },
        );
    }
    let mut child = match command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            lock_tailcat_leases().remove(&id);
            return Err(TailcatLeaseError::new(503, "tailcat_start_failed"));
        }
    };
    let mut input = child.stdin.take().expect("private transport stdin");
    let output = child.stdout.take().expect("private transport stdout");
    let stderr = child.stderr.take();
    if let Some(stderr) = stderr {
        // Drain bounded stderr so a chatty helper can never block on pipe writes.
        thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = stderr.take(4096).read_to_end(&mut sink);
        });
    }
    let child = Arc::new(Mutex::new(child));
    let child_for_timeout = child.clone();
    let (ready, result) = mpsc::sync_channel(1);
    let grant_id = id.clone();
    let host = host_identity().to_owned();
    let public = public.to_owned();
    let origin = format!("http://127.0.0.1:{}", config.web_port);
    thread::spawn(move || {
        use std::io::{BufRead, Write};
        let frame = json!({"version":1,"kind":"host","host":{"origin":origin,"expectedHostId":host,"allowedClientKeys":[public]}});
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
            let frame: Value = serde_json::from_str(&line).map_err(|_| None)?;
            if frame.get("protocolVersion").and_then(Value::as_u64) != Some(1)
                || frame.get("event").and_then(Value::as_str) != Some("ready")
            {
                return Err(extract_helper_error_detail(&frame));
            }
            frame
                .get("address")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .ok_or(None)
        })();
        let started = startup.is_ok();
        if ready.send(startup).is_ok() && started {
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
        let exit = wait_or_kill_child(&child, shutdown_grace);
        finish_tailcat_lease(&grant_id, &finished, exit);
    });
    // Verified helper has its own 30s startup deadline. A dropped HTTP receiver
    // also drops the ready receiver; the owner then closes the helper.
    let address = match result.recv_timeout(startup_timeout) {
        Ok(Ok(address)) => address,
        Ok(Err(detail)) => {
            return Err(TailcatLeaseError::with_detail(
                503,
                "tailcat_start_failed",
                detail,
            ));
        }
        Err(_) => {
            let mut guard = child_for_timeout
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            let _ = guard.kill();
            drop(guard);
            lock_tailcat_leases().remove(&id);
            return Err(TailcatLeaseError::new(503, "tailcat_start_timeout"));
        }
    };
    Ok(
        json!({"version":1,"leaseId":id,"address":address,"hostId":host_identity(),"expiresIn":TAILCAT_GRANT_SECONDS}),
    )
}

fn finish_tailcat_lease(
    id: &str,
    finished: &(Mutex<Option<bool>>, std::sync::Condvar),
    exit: std::io::Result<std::process::ExitStatus>,
) {
    let (state, wake) = finished;
    *state.lock().unwrap_or_else(|err| err.into_inner()) =
        Some(exit.as_ref().is_ok_and(|status| status.success()));
    wake.notify_all();
    // Always release the grant slot once the worker finishes reaping/killing the child.
    lock_tailcat_leases().remove(id);
}

fn renew_tailcat_lease(owner: &str, id: &str) -> Result<Value, TailcatLeaseError> {
    let leases = lock_tailcat_leases();
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
        "expiresIn": TAILCAT_GRANT_SECONDS,
    }))
}

fn close_tailcat_lease(owner: &str, id: &str) -> Result<Value, TailcatLeaseError> {
    let finished = {
        let leases = lock_tailcat_leases();
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
        lease.finished.clone()
    };
    let (state, wake) = &*finished;
    let guard = state.lock().unwrap_or_else(|err| err.into_inner());
    let (state, _) = wake
        .wait_timeout_while(guard, StdDuration::from_secs(5), |value| value.is_none())
        .unwrap_or_else(|err| err.into_inner());
    Ok(
        json!({"closed":*state==Some(true),"closing":state.is_none(),"stopUnconfirmed":*state==Some(false),"runStateChanged":false}),
    )
}

pub(crate) fn shutdown_all_tailcat_leases() {
    let active: Vec<Arc<(Mutex<Option<bool>>, std::sync::Condvar)>> = {
        let leases = lock_tailcat_leases();
        leases
            .values()
            .map(|lease| {
                let _ = lease.control.send(TailcatLeaseCommand::Stop);
                lease.finished.clone()
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
        let guard = state.lock().unwrap_or_else(|err| err.into_inner());
        let _ = wake
            .wait_timeout_while(guard, remaining, |value| value.is_none())
            .unwrap_or_else(|err| err.into_inner());
    }
}

#[cfg(test)]
mod tailcat_lease_tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn abnormal_helper_exit_does_not_acknowledge_close_or_leak_a_slot() {
        let id = uuid::Uuid::new_v4().to_string();
        let (control, close_requested) = mpsc::channel();
        let finished = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
        lock_tailcat_leases().insert(
            id.clone(),
            TailcatLease {
                owner: "owner".into(),
                control,
                finished: finished.clone(),
            },
        );
        let lease_id = id.clone();
        let closer = thread::spawn(move || close_tailcat_lease("owner", &lease_id).unwrap());
        assert_eq!(
            close_requested
                .recv_timeout(StdDuration::from_secs(5))
                .unwrap(),
            TailcatLeaseCommand::Stop
        );
        let exit = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 7"])
            .status();
        assert_eq!(exit.as_ref().unwrap().code(), Some(7));
        finish_tailcat_lease(&id, &finished, exit);
        let receipt = closer.join().unwrap();
        assert_eq!(
            receipt,
            json!({"closed":false,"closing":false,"stopUnconfirmed":true,"runStateChanged":false})
        );
        assert!(!lock_tailcat_leases().contains_key(&id));
    }

    #[test]
    fn lease_close_requires_original_owner_and_never_fabricates_a_stop() {
        let id = uuid::Uuid::new_v4().to_string();
        let (control, receive) = mpsc::channel();
        let finished = Arc::new((Mutex::new(Some(false)), std::sync::Condvar::new()));
        lock_tailcat_leases().insert(
            id.clone(),
            TailcatLease {
                owner: "owner-a".into(),
                control,
                finished: finished.clone(),
            },
        );
        assert_eq!(close_tailcat_lease("owner-b", &id).unwrap_err().status, 403);
        assert!(receive.try_recv().is_err(), "wrong owner sent a stop");
        let unknown = close_tailcat_lease("owner-a", &id).unwrap();
        assert_eq!(unknown["closed"], false);
        assert_eq!(unknown["stopUnconfirmed"], true);
        assert_eq!(unknown["runStateChanged"], false);
        *finished.0.lock().unwrap() = Some(true);
        assert_eq!(close_tailcat_lease("owner-a", &id).unwrap()["closed"], true);
        lock_tailcat_leases().remove(&id);
    }

    #[test]
    fn lease_renew_extends_for_owner_and_rejects_wrong_owner_or_missing() {
        let id = uuid::Uuid::new_v4().to_string();
        let (control, receive) = mpsc::channel();
        let finished = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
        lock_tailcat_leases().insert(
            id.clone(),
            TailcatLease {
                owner: "renew-owner-a".into(),
                control,
                finished,
            },
        );
        let wrong = renew_tailcat_lease("renew-owner-b", &id).unwrap_err();
        assert_eq!(wrong, TailcatLeaseError::new(403, "transport_lease_owner_mismatch"));
        assert!(receive.try_recv().is_err());

        let missing = renew_tailcat_lease("renew-owner-a", "missing-lease").unwrap_err();
        assert_eq!(missing, TailcatLeaseError::new(404, "transport_lease_unavailable"));

        let ok = renew_tailcat_lease("renew-owner-a", &id).unwrap();
        assert_eq!(
            ok,
            json!({
                "version": 1,
                "leaseId": id,
                "renewed": true,
                "expiresIn": TAILCAT_GRANT_SECONDS,
            })
        );
        assert_eq!(receive.try_recv().unwrap(), TailcatLeaseCommand::Renew);
        lock_tailcat_leases().remove(&id);
    }

    #[cfg(unix)]
    #[test]
    fn startup_timeout_kills_helper_and_releases_lease_slot() {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().canonicalize().unwrap().join("hang-helper.sh");
        fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
        let bytes = fs::read(&script).unwrap();
        let digest = format!("{:x}", Sha256::digest(&bytes));

        let mut config = Config::for_test().unwrap();
        config.web_port = 18080;
        config.tailcat_helper = Some((script, digest));

        let owner = format!("timeout-owner-{}", uuid::Uuid::new_v4());
        let err = create_tailcat_lease_with_timeout(
            &config,
            owner.clone(),
            "nodekey:deadbeef",
            StdDuration::from_millis(120),
        )
        .unwrap_err();
        assert_eq!(err, TailcatLeaseError::new(503, "tailcat_start_timeout"));
        assert!(!lock_tailcat_leases()
            .values()
            .any(|lease| lease.owner == owner));
    }

    #[cfg(unix)]
    #[test]
    fn hung_helper_on_stop_is_killed_after_grace_and_releases_slot() {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir
            .path()
            .canonicalize()
            .unwrap()
            .join("hung-on-stop-helper.sh");
        fs::write(
            &script,
            "#!/bin/sh\nread _line\nprintf '{\"protocolVersion\":1,\"event\":\"ready\",\"address\":\"tailcat://fixture\"}\\n'\nsleep 30\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
        let bytes = fs::read(&script).unwrap();
        let digest = format!("{:x}", Sha256::digest(&bytes));

        let mut config = Config::for_test().unwrap();
        config.web_port = 18080;
        config.tailcat_helper = Some((script, digest));

        let owner = format!("hung-stop-owner-{}", uuid::Uuid::new_v4());
        let created = create_tailcat_lease_with_options(
            &config,
            owner.clone(),
            "nodekey:deadbeef",
            StdDuration::from_secs(5),
            StdDuration::from_secs(60),
            StdDuration::from_millis(120),
        )
        .unwrap();
        let lease_id = created["leaseId"].as_str().unwrap().to_string();
        assert!(lock_tailcat_leases().contains_key(&lease_id));

        let receipt = close_tailcat_lease(&owner, &lease_id).unwrap();
        assert_eq!(receipt["closed"], false);
        assert_eq!(receipt["stopUnconfirmed"], true);
        assert!(!lock_tailcat_leases().contains_key(&lease_id));
    }

    #[cfg(unix)]
    #[test]
    fn per_owner_lease_limit_and_safe_startup_error_detail() {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().canonicalize().unwrap().join("fail-helper.sh");
        fs::write(
            &script,
            "#!/bin/sh\nread _line\nprintf '{\"protocolVersion\":1,\"event\":\"error\",\"stage\":\"host_start\",\"detail\":\"ingress_unavailable\"}\\n'\nexit 1\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
        let bytes = fs::read(&script).unwrap();
        let digest = format!("{:x}", Sha256::digest(&bytes));

        let mut config = Config::for_test().unwrap();
        config.web_port = 18080;
        config.tailcat_helper = Some((script, digest));

        let owner = format!("cap-owner-{}", uuid::Uuid::new_v4());
        let mut inserted_ids = Vec::new();
        for _ in 0..TAILCAT_PER_OWNER_LEASE_LIMIT {
            let id = uuid::Uuid::new_v4().to_string();
            let (control, _rx) = mpsc::channel();
            let finished = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
            lock_tailcat_leases().insert(
                id.clone(),
                TailcatLease {
                    owner: owner.clone(),
                    control,
                    finished,
                },
            );
            inserted_ids.push(id);
        }

        let cap_err = create_tailcat_lease(&config, owner.clone(), "nodekey:deadbeef").unwrap_err();
        assert_eq!(
            cap_err,
            TailcatLeaseError::new(429, "transport_owner_capacity_reached")
        );

        let other_owner = format!("other-owner-{}", uuid::Uuid::new_v4());
        let detail_err =
            create_tailcat_lease(&config, other_owner, "nodekey:deadbeef").unwrap_err();
        assert_eq!(
            detail_err,
            TailcatLeaseError::with_detail(
                503,
                "tailcat_start_failed",
                Some("host_start:ingress_unavailable".into()),
            )
        );

        let mut leases = lock_tailcat_leases();
        for id in inserted_ids {
            leases.remove(&id);
        }
    }
}
