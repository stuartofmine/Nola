// SPDX-License-Identifier: Apache-2.0
// Ephemeral network grants only. Nola authentication, Run ownership and ACKs
// remain unchanged; no transport key or grant is written to the store.
const TAILCAT_GRANT_SECONDS: u64 = 20 * 60;
struct TailcatLease {
    owner: String,
    stop: mpsc::Sender<()>,
    finished: Arc<(Mutex<Option<bool>>, std::sync::Condvar)>,
}
static TAILCAT_LEASES: OnceLock<Mutex<HashMap<String, TailcatLease>>> = OnceLock::new();
fn tailcat_leases() -> &'static Mutex<HashMap<String, TailcatLease>> {
    TAILCAT_LEASES.get_or_init(|| Mutex::new(HashMap::new()))
}
fn tailcat_owner(token: &str) -> String {
    web_secret_digest(token)
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
                Err((409, "transport_host_identity_mismatch"))
            } else if let Some(public) = body.get("clientPublicKey").and_then(Value::as_str) {
                create_tailcat_lease(config, owner, public)
            } else {
                Err((400, "client_public_key_required"))
            }
        }
        Some("close") => body
            .get("leaseId")
            .and_then(Value::as_str)
            .ok_or((400, "transport_lease_required"))
            .and_then(|lease| close_tailcat_lease(&owner, lease)),
        _ => Err((400, "unknown_transport_operation")),
    };
    match result {
        Ok(value) => mobile_encrypted_json_response(StatusCode(200), value, &token),
        Err((status, code)) => {
            mobile_encrypted_json_response(StatusCode(status), json!({"error":code}), &token)
        }
    }
}

fn create_tailcat_lease(
    config: &Config,
    owner: String,
    public: &str,
) -> Result<Value, (u16, &'static str)> {
    let (program, digest) = config
        .tailcat_helper
        .as_ref()
        .ok_or((409, "tailcat_not_available"))?;
    if public.len() > 128 || !public.starts_with("nodekey:") {
        return Err((400, "invalid_transport_public_key"));
    }
    let mut command = crate::tailcat_transport::verified_command(program, digest)
        .map_err(|_| (503, "tailcat_helper_verification_failed"))?;
    let id = uuid::Uuid::new_v4().to_string();
    let (stop, control) = mpsc::channel();
    let finished = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
    {
        let mut leases = tailcat_leases()
            .lock()
            .map_err(|_| (503, "transport_registry_unavailable"))?;
        // A bounded number of ephemeral network processes, not a model budget.
        if leases.len() >= 32 {
            return Err((503, "transport_capacity_reached"));
        }
        leases.insert(
            id.clone(),
            TailcatLease {
                owner,
                stop,
                finished: finished.clone(),
            },
        );
    }
    let mut child = match command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            tailcat_leases().lock().unwrap().remove(&id);
            return Err((503, "tailcat_start_failed"));
        }
    };
    let mut input = child.stdin.take().expect("private transport stdin");
    let output = child.stdout.take().expect("private transport stdout");
    let (ready, result) = mpsc::sync_channel(1);
    let grant_id = id.clone();
    let host = host_identity().to_owned();
    let public = public.to_owned();
    let origin = format!("http://127.0.0.1:{}", config.web_port);
    thread::spawn(move || {
        use std::io::{BufRead, Write};
        let frame = json!({"version":1,"kind":"host","host":{"origin":origin,"expectedHostId":host,"allowedClientKeys":[public]}});
        let startup = (|| -> Result<String, ()> {
            serde_json::to_writer(&mut input, &frame).map_err(|_| ())?;
            input
                .write_all(b"\n")
                .and_then(|_| input.flush())
                .map_err(|_| ())?;
            let mut line = String::new();
            std::io::BufReader::new(output)
                .take(64 * 1024)
                .read_line(&mut line)
                .map_err(|_| ())?;
            let frame: Value = serde_json::from_str(&line).map_err(|_| ())?;
            if frame.get("protocolVersion").and_then(Value::as_u64) != Some(1)
                || frame.get("event").and_then(Value::as_str) != Some("ready")
            {
                return Err(());
            }
            frame
                .get("address")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .ok_or(())
        })();
        let started = startup.is_ok();
        if ready.send(startup).is_ok() && started {
            let _ = control.recv_timeout(StdDuration::from_secs(TAILCAT_GRANT_SECONDS));
        }
        let _ = input.write_all(b"{\"version\":1,\"kind\":\"stop\"}\n");
        let _ = input.flush();
        drop(input);
        finish_tailcat_lease(&grant_id, &finished, child.wait());
    });
    // Verified helper has its own30s startup deadline. A dropped HTTP receiver
    // also drops the ready receiver; the owner then closes the helper.
    let address = result
        .recv_timeout(StdDuration::from_secs(35))
        .map_err(|_| (503, "tailcat_start_timeout"))?
        .map_err(|_| (503, "tailcat_start_failed"))?;
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
    *state.lock().unwrap() = Some(exit.as_ref().is_ok_and(|status| status.success()));
    wake.notify_all();
    // A reaped child no longer consumes a grant slot, including on an error exit.
    if exit.is_ok() {
        tailcat_leases().lock().unwrap().remove(id);
    }
}

fn close_tailcat_lease(owner: &str, id: &str) -> Result<Value, (u16, &'static str)> {
    let finished = {
        let leases = tailcat_leases()
            .lock()
            .map_err(|_| (503, "transport_registry_unavailable"))?;
        let lease = leases.get(id).ok_or((404, "transport_lease_unavailable"))?;
        if lease.owner != owner {
            return Err((403, "transport_lease_owner_mismatch"));
        }
        let _ = lease.stop.send(());
        lease.finished.clone()
    };
    let (state, wake) = &*finished;
    let (state, _) = wake
        .wait_timeout_while(state.lock().unwrap(), StdDuration::from_secs(5), |value| {
            value.is_none()
        })
        .unwrap();
    Ok(
        json!({"closed":*state==Some(true),"closing":state.is_none(),"stopUnconfirmed":*state==Some(false),"runStateChanged":false}),
    )
}

#[cfg(test)]
mod tailcat_lease_tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn abnormal_helper_exit_does_not_acknowledge_close_or_leak_a_slot() {
        let id = uuid::Uuid::new_v4().to_string();
        let (stop, close_requested) = mpsc::channel();
        let finished = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
        tailcat_leases().lock().unwrap().insert(
            id.clone(),
            TailcatLease {
                owner: "owner".into(),
                stop,
                finished: finished.clone(),
            },
        );
        let lease_id = id.clone();
        let closer = thread::spawn(move || close_tailcat_lease("owner", &lease_id).unwrap());
        close_requested
            .recv_timeout(StdDuration::from_secs(5))
            .unwrap();
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
        assert!(!tailcat_leases().lock().unwrap().contains_key(&id));
    }

    #[test]
    fn lease_close_requires_original_owner_and_never_fabricates_a_stop() {
        let id = uuid::Uuid::new_v4().to_string();
        let (stop, receive) = mpsc::channel();
        let finished = Arc::new((Mutex::new(Some(false)), std::sync::Condvar::new()));
        tailcat_leases().lock().unwrap().insert(
            id.clone(),
            TailcatLease {
                owner: "owner-a".into(),
                stop,
                finished: finished.clone(),
            },
        );
        assert_eq!(close_tailcat_lease("owner-b", &id).unwrap_err().0, 403);
        assert!(receive.try_recv().is_err(), "wrong owner sent a stop");
        let unknown = close_tailcat_lease("owner-a", &id).unwrap();
        assert_eq!(unknown["closed"], false);
        assert_eq!(unknown["stopUnconfirmed"], true);
        assert_eq!(unknown["runStateChanged"], false);
        *finished.0.lock().unwrap() = Some(true);
        assert_eq!(close_tailcat_lease("owner-a", &id).unwrap()["closed"], true);
        tailcat_leases().lock().unwrap().remove(&id);
    }
}
