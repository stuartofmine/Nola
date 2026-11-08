// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Multiplexes authenticated newline-delimited control frames sent by native Hosts.
// - Boundary: This is the sole stdin reader after the desktop Host launch request is hydrated.

use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_CONTROL_FRAME_BYTES: usize = 64 * 1024;
const MAX_BUFFERED_PERMISSION_DECISIONS: usize = 64;
const MAX_BUFFERED_DEVICE_TOOL_RESULTS: usize = 64;

#[derive(Default)]
struct DesktopHostControlState {
    started: bool,
    closed: bool,
    session_id: Option<String>,
    permission_decisions: HashMap<String, Value>,
    device_tool_results: HashMap<String, Value>,
}

static CONTROL_STATE: OnceLock<(Mutex<DesktopHostControlState>, Condvar)> = OnceLock::new();

fn control_state() -> &'static (Mutex<DesktopHostControlState>, Condvar) {
    CONTROL_STATE.get_or_init(|| {
        (
            Mutex::new(DesktopHostControlState::default()),
            Condvar::new(),
        )
    })
}

pub fn start_if_needed() -> Result<(), String> {
    if std::env::var_os("NOLA_DESKTOP_HOST").is_none() {
        return Ok(());
    }
    let (lock, _) = control_state();
    {
        let mut state = lock
            .lock()
            .map_err(|_| "desktop Host control state lock is poisoned".to_string())?;
        if state.started {
            return Ok(());
        }
        state.started = true;
    }
    std::thread::Builder::new()
        .name("nola-desktop-host-control".to_string())
        .spawn(read_control_frames)
        .map(|_| ())
        .map_err(|error| format!("failed to start desktop Host control reader: {error}"))
}

/// Binds the one canonical conversation owned by this desktop Host process.
/// Tool execution may run on worker threads, so this identity cannot live
/// only in the caller thread's permission-session guard.
pub fn bind_session_id(session_id: &str) -> Result<(), String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("desktop Host control session id is empty".to_string());
    }
    let (lock, _) = control_state();
    let mut state = lock
        .lock()
        .map_err(|_| "desktop Host control state lock is poisoned".to_string())?;
    if let Some(existing) = state.session_id.as_deref() {
        if existing != session_id {
            return Err("desktop Host control session id changed inside one process".to_string());
        }
        return Ok(());
    }
    state.session_id = Some(session_id.to_string());
    Ok(())
}

pub fn active_session_id() -> Option<String> {
    let (lock, _) = control_state();
    let state = lock.lock().ok()?;
    (state.started && !state.closed)
        .then(|| state.session_id.clone())
        .flatten()
}

pub fn is_active() -> bool {
    let (lock, _) = control_state();
    lock.lock().map(|state| state.started).unwrap_or(false)
}

/// Private Host-to-Host admission barrier, never a model-visible tool. A fresh
/// nonce is checked for each physical native request, including transport retries.
pub(crate) fn validate_native_model_channel(
    route: &nola_host_protocol::HostNativeModelRouteBinding,
    endpoint: &str,
    model: &str,
    credential: &str,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    crate::model_route::validate_native_transport(route, endpoint, model, credential)?;
    let session_id = active_session_id().ok_or("native model control channel is unavailable")?;
    let request_id = uuid::Uuid::new_v4().to_string();
    crate::native_host_output::emit(serde_json::json!({
        "type": "device_tool_request", "request_id": request_id, "session_id": session_id,
        "tool_name": "__nola_validate_native_model_channel",
        "arguments": {"native_model_route": route, "endpoint": endpoint, "model": model,
            "credential_sha256": format!("{:x}", Sha256::digest(credential.as_bytes()))}
    })).map_err(|_| "native model channel validation could not be delivered")?;
    let response = wait_for_device_tool_result(&request_id, Duration::from_secs(15))
        .ok_or("native model control channel disconnected or timed out")?;
    validate_native_model_channel_response(&response, &request_id, &session_id)
}

pub(crate) fn resolve_goal_native_model_channel(
    goal_id: &str, route: &nola_host_protocol::HostNativeModelRouteBinding,
) -> Result<(String, String), String> {
    let session = active_session_id().ok_or("native Goal model channel is unavailable")?;
    let nonce = uuid::Uuid::new_v4().to_string();
    crate::native_host_output::emit(serde_json::json!({
        "type":"device_tool_request", "request_id":nonce, "session_id":session,
        "tool_name":"__nola_resolve_goal_model_channel",
        "arguments":{"goal_id":goal_id,"native_model_route":route},
    })).map_err(|_| "native Goal model request could not be delivered")?;
    let response = wait_for_device_tool_result(&nonce, Duration::from_secs(300))
        .ok_or("native Goal model preparation disconnected or timed out")?;
    parse_goal_native_channel_response(&response, &nonce, &session, route)
}

fn parse_goal_native_channel_response(response: &Value, nonce: &str, session: &str,
    route: &nola_host_protocol::HostNativeModelRouteBinding) -> Result<(String, String), String> {
    validate_native_model_channel_response(response, nonce, session)?;
    let result = &response["result"];
    let returned: nola_host_protocol::HostNativeModelRouteBinding = serde_json::from_value(result["native_model_route"].clone())
        .map_err(|_| "native Goal model response is invalid")?;
    if returned != *route { return Err("native Goal model selection changed".into()); }
    let endpoint = result["endpoint"].as_str().ok_or("native Goal endpoint is absent")?;
    let credential = result["api_key"].as_str().ok_or("native Goal credential is absent")?;
    let model = result["model"].as_str().ok_or("native Goal model is absent")?;
    crate::model_route::validate_native_transport(route, endpoint, model, credential)?;
    Ok((endpoint.into(), credential.into()))
}

fn validate_native_model_channel_response(response: &Value, request_id: &str, session_id: &str)
    -> Result<(), String>
{
    if response.get("type").and_then(Value::as_str) != Some("device_tool_result")
        || response.get("request_id").and_then(Value::as_str) != Some(request_id)
        || response.get("session_id").and_then(Value::as_str) != Some(session_id)
        || response.get("result").and_then(|value| value.get("authorized")).and_then(Value::as_bool) != Some(true)
    {
        return Err("native model channel is revoked or no longer matches the admitted profile".into());
    }
    Ok(())
}

/// Release every native interaction wait after execution cancellation. This
/// closes transport, not a permission decision: Core's Interrupt submission
/// remains the only owner of the durable cancelled state.
pub fn interrupt_pending_waits() {
    let (lock, changed) = control_state();
    if let Ok(mut state) = lock.lock() {
        state.closed = true;
        state.permission_decisions.clear();
        state.device_tool_results.clear();
        changed.notify_all();
    }
}

pub fn wait_for_permission_decision(
    request_id: &str,
    timeout: Duration,
) -> Result<Value, nola_host_protocol::PermissionTransportInterruptionReason> {
    use nola_host_protocol::PermissionTransportInterruptionReason::{
        Disconnected, Timeout, Unavailable,
    };
    let request_id = request_id.trim();
    if request_id.is_empty() || !is_active() {
        return Err(Unavailable);
    }
    let deadline = Instant::now() + timeout;
    let (lock, changed) = control_state();
    let mut state = lock.lock().map_err(|_| Unavailable)?;
    loop {
        if state.closed {
            return Err(Disconnected);
        }
        if let Some(decision) = state.permission_decisions.remove(request_id) {
            return Ok(decision);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Timeout);
        }
        let (next, _) = changed
            .wait_timeout(state, remaining)
            .map_err(|_| Unavailable)?;
        state = next;
    }
}

pub fn wait_for_device_tool_result(request_id: &str, timeout: Duration) -> Option<Value> {
    let request_id = request_id.trim();
    if request_id.is_empty() || !is_active() {
        return None;
    }
    let deadline = Instant::now() + timeout;
    let (lock, changed) = control_state();
    let mut state = lock.lock().ok()?;
    loop {
        if state.closed {
            return None;
        }
        if let Some(result) = state.device_tool_results.remove(request_id) {
            return Some(result);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let (next, _) = changed.wait_timeout(state, remaining).ok()?;
        state = next;
    }
}


fn read_control_frames() {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    loop {
        let line = match read_bounded_control_line(&mut reader, MAX_CONTROL_FRAME_BYTES) {
            Ok(BoundedControlLine::Line(line)) => line,
            Ok(BoundedControlLine::Oversized) => {
                eprintln!("[desktop-host-control] rejected an oversized control frame");
                continue;
            }
            Ok(BoundedControlLine::Eof) | Err(_) => break,
        };
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            eprintln!("[desktop-host-control] rejected malformed JSON");
            continue;
        };
        if let Err(error) = dispatch_control_frame(frame) {
            eprintln!("[desktop-host-control] {error}");
        }
    }
    let (lock, changed) = control_state();
    if let Ok(mut state) = lock.lock() {
        state.closed = true;
        changed.notify_all();
    }
}

#[derive(Debug, Eq, PartialEq)]
enum BoundedControlLine {
    Eof,
    Line(String),
    Oversized,
}

fn read_bounded_control_line<R: BufRead>(
    reader: &mut R,
    maximum_bytes: usize,
) -> io::Result<BoundedControlLine> {
    // Include room for CRLF without letting `read_until` allocate the rest of
    // an attacker-controlled or accidentally unterminated native frame.
    let read_limit = maximum_bytes.saturating_add(2);
    let mut bytes = Vec::with_capacity(read_limit.min(8 * 1024));
    let mut limited =
        std::io::Read::take(&mut *reader, u64::try_from(read_limit).unwrap_or(u64::MAX));
    let count = limited.read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(BoundedControlLine::Eof);
    }

    let terminated = bytes.last() == Some(&b'\n');
    if terminated {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    let oversized = bytes.len() > maximum_bytes || (!terminated && count == read_limit);
    if oversized && !terminated {
        drain_through_newline(reader)?;
    }
    if oversized {
        return Ok(BoundedControlLine::Oversized);
    }
    String::from_utf8(bytes)
        .map(BoundedControlLine::Line)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn drain_through_newline<R: BufRead>(reader: &mut R) -> io::Result<()> {
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(());
        }
        let consumed = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |position| position + 1);
        let terminated = consumed <= buffer.len() && buffer.get(consumed - 1) == Some(&b'\n');
        reader.consume(consumed);
        if terminated {
            return Ok(());
        }
    }
}

fn dispatch_control_frame(frame: Value) -> Result<(), String> {
    match frame.get("type").and_then(Value::as_str) {
        Some("goal_control") => dispatch_goal_control(&frame),
        Some("request_task_cancellation") => dispatch_task_cancellation(&frame),
        Some("device_tool_result") => buffer_device_tool_result(frame),
        Some(other) => Err(format!("unsupported control frame type: {other}")),
        None => buffer_permission_decision(frame),
    }
}

fn dispatch_goal_control(frame: &Value) -> Result<(), String> {
    let session = required_text(frame, "session_id")?;
    if active_session_id().as_deref() != Some(session) {
        return Err("Goal control does not belong to the bound native session".into());
    }
    let control: crate::goal_control_port::NativeGoalControl = serde_json::from_value(
        frame.get("control").cloned().ok_or("Goal control is missing")?,
    ).map_err(|error| format!("invalid Goal control: {error}"))?;
    crate::goal_control_port::submit_existing(control.input(session.into())).map(|_| ())
}

fn dispatch_task_cancellation(frame: &Value) -> Result<(), String> {
    let session_id = required_text(frame, "session_id")?;
    let task_id = required_text(frame, "task_id")?;
    crate::core_agent_runtime::cancel_agent_core_task(session_id, task_id)
}

fn buffer_permission_decision(frame: Value) -> Result<(), String> {
    let request_id = required_text(&frame, "request_id")?.to_string();
    serde_json::from_value::<nola_core::host_auth::HostPermissionResponse>(frame.clone())
        .map_err(|error| format!("invalid permission control frame: {error}"))?;
    let (lock, changed) = control_state();
    let mut state = lock
        .lock()
        .map_err(|_| "desktop Host control state lock is poisoned".to_string())?;
    if state.permission_decisions.len() >= MAX_BUFFERED_PERMISSION_DECISIONS
        && !state.permission_decisions.contains_key(&request_id)
    {
        return Err("too many buffered permission decisions".to_string());
    }
    state.permission_decisions.insert(request_id, frame);
    changed.notify_all();
    Ok(())
}

fn buffer_device_tool_result(frame: Value) -> Result<(), String> {
    let request_id = required_text(&frame, "request_id")?.to_string();
    let (lock, changed) = control_state();
    let mut state = lock
        .lock()
        .map_err(|_| "desktop Host control state lock is poisoned".to_string())?;
    if state.device_tool_results.len() >= MAX_BUFFERED_DEVICE_TOOL_RESULTS
        && !state.device_tool_results.contains_key(&request_id)
    {
        return Err("too many buffered device tool results".to_string());
    }
    state.device_tool_results.insert(request_id, frame);
    changed.notify_all();
    Ok(())
}


fn required_text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("control frame is missing {key}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn goal_native_channel_reconstruction_preserves_logical_profile_and_nonce() {
        let route: nola_host_protocol::HostNativeModelRouteBinding = serde_json::from_value(serde_json::json!({
            "adapter":"mlx","adapter_contract_revision":1,"model_id":"frozen-model",
            "profile_ref":{"profile_id":"frozen-model","admitted_revision":7},
        })).unwrap();
        let response = serde_json::json!({"type":"device_tool_result","request_id":"nonce","session_id":"session",
            "result":{"authorized":true,"native_model_route":route,"model":"frozen-model",
                "endpoint":"http://127.0.0.1:5123/v1","api_key":"new-physical-instance-secret"}});
        assert!(super::parse_goal_native_channel_response(&response,"nonce","session",&route).is_ok());
        assert!(super::parse_goal_native_channel_response(&response,"old","session",&route).is_err());
        assert!(super::parse_goal_native_channel_response(&response,"nonce","foreign",&route).is_err());
        let mut changed = response;
        changed["result"]["native_model_route"]["profile_ref"]["admitted_revision"] = 8.into();
        assert!(super::parse_goal_native_channel_response(&changed,"nonce","session",&route).is_err());
    }

    #[test]
    fn native_channel_response_requires_current_nonce_session_and_explicit_authorization() {
        let valid = serde_json::json!({"type":"device_tool_result", "request_id":"nonce",
            "session_id":"session", "result":{"authorized":true}});
        assert!(super::validate_native_model_channel_response(&valid, "nonce", "session").is_ok());
        for rejected in [
            serde_json::json!({"type":"device_tool_result", "request_id":"nonce", "session_id":"session", "result":{"authorized":false}}),
            serde_json::json!({"type":"device_tool_result", "request_id":"old", "session_id":"session", "result":{"authorized":true}}),
            serde_json::json!({"type":"device_tool_result", "request_id":"nonce", "session_id":"other", "result":{"authorized":true}}),
            serde_json::json!({"type":"device_tool_result", "request_id":"nonce", "session_id":"session", "result":{}}),
        ] {
            assert!(super::validate_native_model_channel_response(&rejected, "nonce", "session").is_err());
        }
    }
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset_control_state() {
        let (lock, _) = control_state();
        *lock.lock().expect("control state") = DesktopHostControlState::default();
    }

    #[test]
    fn bounded_control_reader_accepts_the_exact_limit_and_crlf() {
        let payload = "x".repeat(16);
        let mut reader = Cursor::new(format!("{payload}\r\nlast").into_bytes());

        assert_eq!(
            read_bounded_control_line(&mut reader, 16).unwrap(),
            BoundedControlLine::Line(payload)
        );
        assert_eq!(
            read_bounded_control_line(&mut reader, 16).unwrap(),
            BoundedControlLine::Line("last".to_string())
        );
        assert_eq!(
            read_bounded_control_line(&mut reader, 16).unwrap(),
            BoundedControlLine::Eof
        );
    }

    #[test]
    fn oversized_control_reader_drains_only_the_bad_frame() {
        let oversized = "x".repeat(128);
        let valid = r#"{"type":"user_input_response"}"#;
        let mut reader = Cursor::new(format!("{oversized}\n{valid}\n").into_bytes());

        assert_eq!(
            read_bounded_control_line(&mut reader, 16).unwrap(),
            BoundedControlLine::Oversized
        );
        assert_eq!(
            read_bounded_control_line(&mut reader, 64).unwrap(),
            BoundedControlLine::Line(valid.to_string())
        );
    }


    #[test]
    fn bound_session_identity_is_available_to_worker_threads() {
        let _serial = TEST_LOCK.lock().expect("test lock");
        reset_control_state();
        bind_session_id("conversation-1").expect("bind session");
        {
            let (lock, _) = control_state();
            lock.lock().expect("control state").started = true;
        }

        let observed = std::thread::spawn(active_session_id)
            .join()
            .expect("worker thread");

        assert_eq!(observed.as_deref(), Some("conversation-1"));
        reset_control_state();
    }


    #[test]
    fn typed_permission_response_resumes_the_matching_waiter() {
        let _serial = TEST_LOCK.lock().expect("test lock");
        reset_control_state();
        {
            let (lock, _) = control_state();
            lock.lock().expect("control state").started = true;
        }
        dispatch_control_frame(json!({
            "protocol_version": nola_core::host_auth::HOST_PERMISSION_PROTOCOL_VERSION,
            "nonce": "nonce-1",
            "request_id": "permission-1",
            "host_id": "local",
            "client_id": "mac",
            "session_id": "conversation-1",
            "tool_name": "bash",
            "resolution": {
                "type": "authorization",
                "decision": "allow_once"
            }
        }))
        .expect("buffer typed permission response");

        let response = wait_for_permission_decision("permission-1", Duration::from_millis(10))
            .expect("matching permission response");
        assert_eq!(
            response
                .pointer("/resolution/decision")
                .and_then(Value::as_str),
            Some("allow_once")
        );
        reset_control_state();
    }

    #[test]
    fn permission_transport_wait_distinguishes_timeout_disconnect_and_unavailable() {
        use nola_host_protocol::PermissionTransportInterruptionReason::{
            Disconnected, Timeout, Unavailable,
        };
        let _serial = TEST_LOCK.lock().expect("test lock");
        reset_control_state();
        assert_eq!(
            wait_for_permission_decision("pending", Duration::ZERO),
            Err(Unavailable)
        );
        {
            let (lock, _) = control_state();
            lock.lock().unwrap().started = true;
        }
        assert_eq!(
            wait_for_permission_decision("pending", Duration::from_millis(1)),
            Err(Timeout)
        );
        {
            let (lock, _) = control_state();
            lock.lock().unwrap().closed = true;
        }
        assert_eq!(
            wait_for_permission_decision("pending", Duration::from_millis(1)),
            Err(Disconnected)
        );
        reset_control_state();
    }

    #[test]
    fn cancellation_releases_all_native_waits_and_rejects_late_decisions() {
        let _serial = TEST_LOCK.lock().expect("test lock");
        reset_control_state();
        control_state().0.lock().unwrap().started = true;
        let (done, results) = std::sync::mpsc::channel();
        let permission_done = done.clone();
        let permission = std::thread::spawn(move || {
            assert_eq!(wait_for_permission_decision("pending", Duration::from_secs(300)),
                Err(nola_host_protocol::PermissionTransportInterruptionReason::Disconnected));
            permission_done.send(()).unwrap();
        });
        let device_done = done.clone();
        let device = std::thread::spawn(move || {
            assert!(wait_for_device_tool_result("device", Duration::from_secs(300)).is_none());
            device_done.send(()).unwrap();
        });
        interrupt_pending_waits();
        for _ in 0..2 {
            results.recv_timeout(Duration::from_secs(2)).expect("cancel wakes native interaction");
        }
        permission.join().unwrap();
        device.join().unwrap();
        // A racing response cannot turn a cancelled interaction into approval.
        control_state().0.lock().unwrap().permission_decisions.insert("pending".into(), json!({}));
        assert_eq!(wait_for_permission_decision("pending", Duration::ZERO),
            Err(nola_host_protocol::PermissionTransportInterruptionReason::Disconnected));
        reset_control_state();
    }

    #[test]
    fn legacy_user_question_response_has_no_process_local_authority() {
        let error = dispatch_control_frame(json!({"type":"user_input_response", "request_id":"pid-1", "answer":"yes"})).unwrap_err();
        assert!(error.contains("unsupported control frame"));
    }

    #[test]
    fn legacy_flat_permission_response_is_rejected() {
        let _serial = TEST_LOCK.lock().expect("test lock");
        reset_control_state();
        let error = dispatch_control_frame(json!({
            "protocol_version": nola_core::host_auth::HOST_PERMISSION_PROTOCOL_VERSION,
            "nonce": "nonce-2",
            "request_id": "permission-2",
            "host_id": "local",
            "client_id": "mac",
            "session_id": "conversation-2",
            "tool_name": "bash",
            "decision": "allow_once"
        }))
        .expect_err("legacy permission response must fail");
        assert!(error.contains("invalid permission control frame"));
        reset_control_state();
    }
}
