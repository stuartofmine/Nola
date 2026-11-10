use rusqlite::Connection;
#[path = "support/assistant_journal.rs"]
mod assistant_journal;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct MockLlm {
    endpoint: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    model_requests: Arc<AtomicUsize>,
}

fn fixture_provider_diagnostics(root: &std::path::Path) -> String {
    let Ok(mut file) = std::fs::File::open(root.join("provider-debug.log")) else {
        return "provider diagnostic log unavailable".into();
    };
    let length = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(length.saturating_sub(64 * 1024)));
    let mut bytes = Vec::new();
    let _ = file.take(64 * 1024).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).lines().rev()
        .filter(|line| line.contains("failure") || line.contains("details="))
        .take(8).map(|line| line.chars().take(2000).collect::<String>())
        .collect::<Vec<_>>().join("\n")
}

impl MockLlm {
    fn start() -> Self {
        Self::with_workspace_request(None)
    }

    fn with_workspace_request(root: Option<String>) -> Self {
        Self::with_scenario(root, 0)
    }

    fn with_scenario(root: Option<String>, oversized_batches: usize) -> Self {
        Self::with_rejections(root, oversized_batches, 0)
    }

    fn with_rejections(
        root: Option<String>,
        oversized_batches: usize,
        activation_batches: usize,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let model_requests = Arc::new(AtomicUsize::new(0));
        let worker_requests = Arc::clone(&model_requests);
        let thread = thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => serve(
                        stream,
                        root.as_deref(),
                        oversized_batches,
                        activation_batches,
                        &worker_requests,
                    ),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            endpoint,
            stop,
            thread: Some(thread),
            model_requests,
        }
    }
}

impl Drop for MockLlm {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let result = thread.join();
            // Preserve the first assertion rather than aborting the entire
            // integration target with a second panic during unwinding.
            if !std::thread::panicking() { result.unwrap(); }
        }
    }
}

fn runtime_fact(body: &serde_json::Value, id: &str) -> serde_json::Value {
    let prefix = format!("<runtime_state id=\"{id}\">\n");
    let message = body["messages"].as_array().unwrap().iter().find(|message| {
        message["content"].as_str().is_some_and(|content| content.starts_with(&prefix))
    }).unwrap_or_else(|| panic!("missing exact Host runtime fact {id}"));
    assert_eq!(message["role"], "user", "observations are data, not new system authority");
    let content = message["content"].as_str().unwrap();
    assert!(content.ends_with("\n</runtime_state>"));
    let lines = content.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 4, "one bounded Host fact packet");
    assert!(lines[1].contains("not a new user request, permission grant"));
    serde_json::from_str(lines[2]).expect("Host fact must contain structured JSON")
}

fn serve(
    mut stream: TcpStream,
    workspace: Option<&str>,
    oversized_batches: usize,
    activation_batches: usize,
    requests: &AtomicUsize,
) {
    // Accepted sockets can inherit the listener's nonblocking mode (macOS).
    // This HTTP fixture deliberately uses blocking reads with a deadline.
    #[cfg(unix)] {
        use std::os::fd::AsRawFd;
        let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFL) };
        eprintln!("fixture_http accepted_nonblocking={}", flags & libc::O_NONBLOCK != 0);
    }
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut expected_length = None;
    let mut read_end = "complete".to_string();
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => { read_end = "eof".into(); break; }
            Ok(count) => {
                request.extend_from_slice(&buffer[..count]);
                if expected_length.is_none() {
                    if let Some(header_end) = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|position| position + 4)
                    {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':').and_then(|(name, value)| {
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                            })
                            .unwrap_or(0);
                        expected_length = Some(header_end + content_length);
                    }
                }
                if expected_length.is_some_and(|length| request.len() >= length) {
                    break;
                }
            }
            Err(error) => { read_end = format!("read_error:{:?}", error.kind()); break; }
        }
    }
    let request_line = String::from_utf8_lossy(&request)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let header_end = request.windows(4).position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4).unwrap_or(0);
    let framing = String::from_utf8_lossy(&request[..header_end]).lines()
        .filter(|line| line.split_once(':').is_some_and(|(name, _)|
            name.eq_ignore_ascii_case("content-length") || name.eq_ignore_ascii_case("transfer-encoding")))
        .map(str::to_owned).collect::<Vec<_>>();
    eprintln!("fixture_http local={:?} request={request_line:?} expected={expected_length:?} received={} read_end={read_end} framing={framing:?}",
        stream.local_addr(), request.len());
    if request_line.contains("/models") {
        write_response(
            &mut stream,
            "application/json",
            r#"{"data":[{"id":"desktop-e2e-model","context_window":65536}]}"#,
        );
        return;
    }
    if request_line.is_empty() {
        return;
    }
    assert!(expected_length.is_some_and(|length| request.len() >= length),
        "fixture must not answer an incomplete request: expected={expected_length:?}, received={}, read_end={read_end}", request.len());
    assert!(request_line.contains("/chat/completions"), "{request_line}");
    let model_request = requests.fetch_add(1, Ordering::SeqCst);
    let body: serde_json::Value = serde_json::from_slice(&request[header_end..]).unwrap();
    if oversized_batches > 0 && model_request > 0 {
        assert_eq!(
            body["parallel_tool_calls"], false,
            "capacity recovery must disable provider-side parallel calls"
        );
        let fact = runtime_fact(&body, "rejected-tool-batch");
        assert_eq!(fact["failure"], "model_tool_batch_limit_exceeded");
        assert_eq!(fact["executed_calls"], 0);
        assert_eq!(fact["next_response_max_native_calls"], 1);
        assert_eq!(fact["guidance"], nola_agent_engine::agent::model_tool_batch::model_tool_batch_guidance());
    }
    if activation_batches > 0 && model_request > 0 {
        let fact = runtime_fact(&body, "rejected-activation-batch");
        assert_eq!(fact["failure"], "activation_batch_requires_replan");
        assert_eq!(fact["executed_calls"], 0);
        assert_eq!(fact["activation_batch_limit"], 1);
    }
    if model_request < activation_batches {
        let calls = [
            ("Skill", serde_json::json!({"skill": "fixture"})),
            ("write_file", serde_json::json!({"path": "must-not-exist.txt", "content": "unapproved companion"})),
        ].into_iter().enumerate().map(|(index, (name, arguments))| serde_json::json!({
            "index": index, "id": format!("activation-call-{index}"), "type": "function",
            "function": {"name": name, "arguments": arguments.to_string()}
        })).collect::<Vec<_>>();
        let chunk = serde_json::json!({"choices": [{"delta": {"tool_calls": calls}, "finish_reason": "tool_calls"}]});
        write_response(
            &mut stream,
            "text/event-stream",
            &format!("data: {chunk}\n\ndata: [DONE]\n\n"),
        );
        return;
    }
    if model_request < oversized_batches {
        let calls = (0..121).map(|index| serde_json::json!({
            "index": index, "id": format!("call-{index}"), "type": "function",
            "function": {"name": "read_file", "arguments": serde_json::json!({"path": format!("file-{index}.rs")}).to_string()}
        })).collect::<Vec<_>>();
        let chunk = serde_json::json!({"choices": [{"delta": {"tool_calls": calls}, "finish_reason": null}]});
        write_response(
            &mut stream,
            "text/event-stream",
            &format!("data: {chunk}\n\ndata: [DONE]\n\n"),
        );
        return;
    }
    if let Some(root) = workspace
        .filter(|_| !String::from_utf8_lossy(&request).contains("workspace_access_granted"))
    {
        let chunk = serde_json::json!({"choices": [{"delta": {"tool_calls": [{
            "index": 0, "id": "workspace-call", "type": "function",
            "function": {"name": "workspace_access", "arguments": serde_json::json!({"root": root}).to_string()}
        }]}, "finish_reason": null}]});
        let body = format!("data: {chunk}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n");
        write_response(&mut stream, "text/event-stream", &body);
        return;
    }
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"complete \"},\"finish_reason\":null}]}\n\n\
                data: {\"choices\":[{\"delta\":{\"content\":\"canonical answer\"},\"finish_reason\":null}]}\n\n\
                data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                data: [DONE]\n\n";
    write_response(&mut stream, "text/event-stream", body);
}

fn write_response(stream: &mut TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

#[test]
fn nonblocking_listener_accepts_a_fragmented_request_without_premature_close() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let (stream, _) = loop {
        match listener.accept() {
            Ok(accepted) => break accepted,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
                && std::time::Instant::now() < deadline => thread::sleep(Duration::from_millis(1)),
            Err(error) => panic!("fixture accept failed: {error}"),
        }
    };
    let requests = Arc::new(AtomicUsize::new(0));
    let worker_requests = Arc::clone(&requests);
    let server = thread::spawn(move || serve(stream, None, 0, 0, &worker_requests));
    client.write_all(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n").unwrap();
    thread::sleep(Duration::from_millis(25));
    client.write_all(b"{}").unwrap();
    let mut response = String::new();
    client.read_to_string(&mut response).unwrap();
    server.join().unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(response.contains("canonical answer"));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

fn run_desktop_host(
    root: &std::path::Path,
    database: &std::path::Path,
    llm: &MockLlm,
    payload: serde_json::Value,
) {
    let expected_run = payload["submission_id"].as_str().unwrap().to_owned();
    let output = run_desktop_host_result(root, database, llm, payload);
    assert!(
        output.status.success(),
        "desktop Agent Host failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let frames = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect::<Vec<_>>();
    let first_control = frames
        .iter()
        .find(|frame| frame["type"] == "core_permission_projection_updated")
        .expect("admitted Core control frame");
    assert!(
        first_control["turn_id"]
            .as_str()
            .is_some_and(|turn| !turn.is_empty()),
        "idle preflight state must not escape before this submission's admission: {first_control}"
    );
    for frame in &frames {
        if frame["type"] == "assistant_journal_committed_v1" {
            assert_eq!(
                frame["through"]["identity"]["run_id"], expected_run,
                "recovery of an older Run must not publish into this submission stream"
            );
        }
        if frame["type"] == "assistant_run_terminal_committed_v1" {
            assert_eq!(frame["run_id"], expected_run);
        }
    }
}

fn configure_native_payload(payload: &mut serde_json::Value, database: &std::path::Path, session: &str) {
    payload["api_key"] = serde_json::json!("e2e-fixture-key");
    payload["model_protocol"] = serde_json::json!("chat_completions");
    payload["model_capabilities"] = serde_json::json!({"native_tool_calls":true,
        "parallel_tool_calls":true,"stream_text":true,"stream_tool_calls":true});
    if payload.get("permission_profile").is_none() {
        payload["permission_profile"] = serde_json::json!("cautious");
    }
    let store = nola_store::NolaStore::open(database).unwrap();
    let current = store.load_conversation_message_window(session, None, 1, 1).unwrap();
    payload["expected_configuration_revision"] = serde_json::json!(
        current.as_ref().map(|window| window.conversation.configuration_revision));
    payload["expected_parent_message_id"] = serde_json::json!(current.as_ref()
        .and_then(|window| window.conversation.messages.last()).map(|message| message.id.clone()));
}

fn assert_run_result_kind(connection: &Connection, run: &str, expected: &str) {
    let (status, raw): (String,String) = connection.query_row(
        "SELECT r.status,rr.result_json FROM runs r JOIN run_results rr ON rr.run_id=r.id WHERE r.id=?1",
        [run], |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    assert_eq!(status,"sealed");
    let result: nola_domain::run_result::RunResult = serde_json::from_str(&raw).unwrap();
    assert_eq!(result.run_id,run);
    result.validate().unwrap();
    assert_eq!(serde_json::to_value(result.value).unwrap()["kind"],expected);
}

fn configure_native_tool_receipt(command: &mut Command, root: &std::path::Path, session: &str) {
    use base64::Engine as _;
    use nola_core::tool_policy::*;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let identity = RuntimeIdentity {host_id:Some("local".into()),client_id:Some("mac".into()),session_id:Some(session.into())};
    let tools = ["read_file", "write_file", "workspace_access", "tool_search", "Skill"];
    let input = ToolPolicyInput {
        platform:ToolPlatform::Macos,now_ms:now,expected_identity:identity.clone(),
        runtime:RuntimeCapabilityManifest {
            manifest_id:"native-e2e-fixture".into(),manifest_version:1,
            issued_at_ms:now,expires_at_ms:now+300_000,
            host_id:identity.host_id.clone(),client_id:identity.client_id.clone(),session_id:identity.session_id.clone(),
            routes:vec![ToolRoute::AgentHost],capabilities:vec!["files_read".into(),"files_write".into()],
            authorizations:[("files_read".into(),CapabilityAuthorization::Granted),
                ("files_write".into(),CapabilityAuthorization::Granted)].into(),
            tools:tools.iter().map(|name| RuntimeToolManifest {tool_id:(*name).into(),handler_id:Some((*name).into()),
                schema:serde_json::json!({"type":"function","function":{"name":name,"parameters":{"type":"object"}}}),
                route:Some(ToolRoute::AgentHost),available:true}).collect(),
            ..Default::default()
        },
        scoped_grants:vec![ScopedCapabilityGrant {capability:"files_read".into(),route:Some(ToolRoute::AgentHost),
            scope:Some(root.to_string_lossy().into()),expires_at_ms:now+300_000}],
        ..Default::default()
    };
    let output = resolve_tool_policy(&input);
    for name in tools {
        assert!(output.enabled_tools.iter().any(|tool|
            tool.tool_id == nola_core::tool_catalog::normalize_tool_id(name)
                && tool.schema.pointer("/function/name").and_then(serde_json::Value::as_str) == Some(name)),
            "fixture handler unavailable: {name}: {:?}",output.disabled_tools);
    }
    let mut receipt = serde_json::to_value(&output).unwrap();
    receipt["expected_identity"] = serde_json::to_value(identity).unwrap();
    command.env("NOLA_CORE_TOOL_POLICY_RECEIPT",base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&receipt).unwrap()))
        .env("NOLA_CORE_TOOL_POLICY_DIGEST",output.policy_digest)
        .env("NOLA_CORE_TOOL_MANIFEST_DIGEST",output.manifest_digest);
}

fn run_desktop_host_result(
    root: &std::path::Path,
    database: &std::path::Path,
    llm: &MockLlm,
    mut payload: serde_json::Value,
) -> std::process::Output {
    configure_native_payload(&mut payload, database, "mac-e2e-conversation");
    let mut command = Command::new(env!("CARGO_BIN_EXE_nola-agent-host"));
    configure_native_tool_receipt(&mut command, root, "mac-e2e-conversation");
    let mut child = command
        .args([
            "run",
            "--json",
            "--host-request-stdin",
            "--session",
            "mac-e2e-conversation",
            "--project",
            root.to_str().unwrap(),
            "--endpoint",
            &llm.endpoint,
            "--model",
            "desktop-e2e-model",
        ])
        .env("NOLA_DESKTOP_HOST", "1")
        .env("NOLA_CLIENT_SURFACE", "mac")
        .env("NOLA_DB_PATH", database)
        .env("NOLA_CLI_DEBUG", "1")
        .env("NOLA_CLI_DEBUG_LOG_PATH", root.join("provider-debug.log"))
        .env("NOLA_LONG_TERM_MEMORY_ENABLED", "0")
        .env("NOLA_CLI_INTERACTION_MODE", "never")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{payload}").unwrap();
    child.wait_with_output().unwrap()
}

#[test]
#[cfg(unix)]
fn stopping_native_permission_wait_persists_cancelled_turn_before_host_exit() {
    native_permission_handoff(false);
}

#[test]
#[cfg(unix)]
fn native_permission_handoff_resumes_without_a_following_emitter_event() {
    native_permission_handoff(true);
}

#[cfg(unix)]
fn native_permission_handoff(approve: bool) {
    use std::io::{BufRead, BufReader};
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("cancel.db");
    let llm = MockLlm::with_workspace_request(Some(root.path().to_str().unwrap().to_string()));
    let mut command = Command::new(env!("CARGO_BIN_EXE_nola-agent-host"));
    configure_native_tool_receipt(&mut command, root.path(), "cancel-session");
    let mut child = command
        .args([
            "run",
            "--json",
            "--host-request-stdin",
            "--session",
            "cancel-session",
            "--project",
            root.path().to_str().unwrap(),
            "--endpoint",
            &llm.endpoint,
            "--model",
            "desktop-e2e-model",
        ])
        .env("NOLA_DESKTOP_HOST", "1")
        .env("NOLA_CLIENT_SURFACE", "mac")
        .env("NOLA_NATIVE_PERMISSION_MODE", "1")
        .env("NOLA_DB_PATH", &database)
        .env("NOLA_CLI_DEBUG", "1")
        .env("NOLA_CLI_DEBUG_LOG_PATH", root.path().join("provider-debug.log"))
        .env("NOLA_LONG_TERM_MEMORY_ENABLED", "0")
        .env("NOLA_CLI_INTERACTION_MODE", "never")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut payload = serde_json::json!({
        "prompt": "inspect the requested directory", "submission_id": "cancel-turn",
        "user_message_id": "cancel-user", "assistant_message_id": "cancel-assistant",
        "input_origin": "interactive_user", "permission_profile": "cautious",
    });
    payload["permission_rules"] = serde_json::to_value(vec![nola_core::agent::permissions::PermissionRule {
        capability_pattern:nola_core::agent::tool_permission_domain("workspace_access").unwrap(),
        resource_pattern:"*".into(),decision:nola_core::agent::permissions::PermissionPolicyDecision::Ask,
        source:nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
    }]).unwrap();
    configure_native_payload(&mut payload, &database, "cancel-session");
    writeln!(
        input,
        "{}",
        payload
    )
    .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (events, received) = std::sync::mpsc::channel();
    let recent = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let reader_recent = recent.clone();
    let mut reader = Some(thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let mut tail = reader_recent.lock().unwrap();
            tail.push_back(line.chars().take(2048).collect::<String>());
            if tail.len() > 16 {
                tail.pop_front();
            }
            drop(tail);
            let _ = events.send(line);
        }
    }));
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut challenge = None;
    let mut projection = None;
    let mut sequences = std::collections::HashMap::new();
    while let Ok(line) =
        received.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
    {
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) {
            if let (Some(trace), Some(sequence)) = (
                event.pointer("/trace/trace_id").and_then(|v| v.as_str()),
                event.pointer("/trace/sequence").and_then(|v| v.as_u64()),
            ) {
                let last = sequences.entry(trace.to_string()).or_insert(0);
                assert!(
                    sequence > *last,
                    "Native stream must retain strict trace order"
                );
                *last = sequence;
            }
            if event["type"] == "permission_request" {
                challenge = Some(event.clone());
            }
            if event["type"] == "core_permission_projection_updated" {
                projection = Some(event);
            }
            if let (Some(challenge), Some(projection)) = (&challenge, &projection) {
                if projection
                    .pointer("/permission_projection/pending")
                    .and_then(|v| v.as_array())
                    .is_some_and(|pending| {
                        pending.iter().any(|p| {
                            p["request_id"] == challenge["request_id"]
                                && p["call_id"] == challenge["tool_call_id"]
                        })
                    })
                {
                    break;
                }
            }
        }
    }
    let ready = challenge
        .as_ref()
        .zip(projection.as_ref())
        .is_some_and(|(c, p)| {
            p.pointer("/permission_projection/pending")
                .and_then(|v| v.as_array())
                .is_some_and(|pending| {
                    pending.iter().any(|p| {
                        p["request_id"] == c["request_id"] && p["call_id"] == c["tool_call_id"]
                    })
                })
        });
    if !ready {
        let _ = child.kill();
        let status = child.wait();
        if let Some(reader) = reader.take() {
            let _ = reader.join();
        }
        let recent = recent.lock().unwrap().iter().cloned().collect::<Vec<_>>();
        let diagnostics = fixture_provider_diagnostics(root.path());
        let retained = root.keep();
        panic!("Host did not deliver BOTH challenge and matching projection before waiting; no user response or later event may be needed to flush them; status={status:?}; retained={}; challenge={challenge:?}; projection={projection:?}; recent={recent:?}; diagnostics={diagnostics}", retained.display());
    }
    if approve {
        let challenge = challenge.unwrap();
        let mut response = serde_json::Map::new();
        for key in [
            "protocol_version",
            "nonce",
            "request_id",
            "host_id",
            "client_id",
            "session_id",
            "tool_name",
        ] {
            response.insert(key.to_string(), challenge[key].clone());
        }
        response.insert(
            "resolution".into(),
            serde_json::json!({"type":"authorization", "decision":"allow_once"}),
        );
        writeln!(input, "{}", serde_json::Value::Object(response)).unwrap();
        input.flush().unwrap();
    } else {
        // Keep stdin open: Stop is a signal, not EOF or a fabricated deny response.
        assert!(Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success());
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(if approve { 15 } else { 4 });
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            if !status.success() {
                use std::os::unix::process::ExitStatusExt;
                reader.take().unwrap().join().unwrap();
                let tail = recent
                    .lock()
                    .unwrap()
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                panic!("Host permission {} did not settle successfully: status={status:?}, code={:?}, signal={:?}; bounded recent stdout:\n{tail}",
                    if approve {"approval"} else {"cancellation"},status.code(),status.signal());
            }
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Permission interaction failed to settle within its deadline");
        }
        thread::sleep(Duration::from_millis(10));
    }
    drop(input);
    reader.take().unwrap().join().unwrap();
    let db = Connection::open(database).unwrap();
    let run: String = db
        .query_row("SELECT status FROM runs WHERE id='cancel-turn'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let expected = if approve { "completed" } else { "cancelled" };
    assert_eq!(run, "sealed");
    assert_run_result_kind(&db, "cancel-turn", if approve { "answered" } else { "cancelled" });
    let message: String = db
        .query_row(
            "SELECT status FROM messages WHERE id='cancel-assistant'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(message, expected);
    let open_parts: i64 = db.query_row("SELECT COUNT(*) FROM message_parts WHERE message_id='cancel-assistant' AND state IN ('open','running','waiting_for_permission')", [], |r| r.get(0)).unwrap();
    assert_eq!(
        open_parts, 0,
        "Cancellation must settle pending Parts as well as Run status"
    );
    let pending_effects: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM effect_outbox WHERE run_id='cancel-turn' AND status='pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        pending_effects, 0,
        "Cancelled run must not retain a deliverable permission effect"
    );
    if !approve {
        let interrupts: i64 = db.query_row(
            "SELECT COUNT(*) FROM host_event_replay_order WHERE run_id='cancel-turn'
             AND submission_json IS NOT NULL
             AND json_extract(json_extract(submission_json,'$.submission_json'),'$.type')='interrupt'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(interrupts, 1, "one foreground stop must commit exactly one formal Interrupt, regardless of boundary-poll ordering");
        let tool_starts: i64 = db.query_row(
            "SELECT COUNT(*) FROM effect_delivery_attempts a JOIN effect_outbox e ON e.effect_id=a.effect_id
             WHERE e.run_id='cancel-turn' AND e.effect_kind='execute_tool'
             AND json_extract(a.record_json,'$.execution_started_at_ms') IS NOT NULL",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(tool_starts, 0, "cancelling an undecided permission must never start its tool");
    }
}

#[test]
fn desktop_agent_host_owns_one_complete_canonical_turn_with_application_ids() {
    let root = std::env::temp_dir().join(format!(
        "nola-desktop-canonical-e2e-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("nola-product.db");
    let llm = MockLlm::start();
    let payload = serde_json::json!({
        "prompt": "persist this complete response",
        "submission_id": "turn-mac-e2e-42",
        "user_message_id": "user-mac-e2e-42",
        "assistant_message_id": "assistant-mac-e2e-42",
        "input_origin": "interactive_user"
    });

    run_desktop_host(&root, &database, &llm, payload);

    let connection = Connection::open(&database).unwrap();
    let messages: Vec<(String, String, String, String)> = connection
        .prepare(
            "SELECT id, COALESCE(run_id, ''), role, status
             FROM messages WHERE conversation_id = ?1 ORDER BY ordinal",
        )
        .unwrap()
        .query_map(["mac-e2e-conversation"], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        messages.len(),
        2,
        "one Turn must create exactly two messages"
    );
    assert_eq!(messages[0].0, "user-mac-e2e-42");
    assert_eq!(messages[0].2, "user");
    assert_eq!(messages[1].0, "assistant-mac-e2e-42");
    assert_eq!(messages[1].1, "turn-mac-e2e-42");
    assert_eq!(messages[1].2, "assistant");
    assert_eq!(messages[1].3, "completed");

    let run: (String, String) = connection
        .query_row(
            "SELECT id, status FROM runs WHERE conversation_id = ?1",
            ["mac-e2e-conversation"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        run,
        ("turn-mac-e2e-42".to_string(), "sealed".to_string())
    );
    assert_run_result_kind(&connection, "turn-mac-e2e-42", "answered");

    let complete_text = assistant_journal::public_text(
        &database,
        "mac-e2e-conversation",
        "turn-mac-e2e-42",
        "assistant-mac-e2e-42",
    );
    assert_eq!(complete_text, "complete canonical answer");
    drop(connection);

    // A second real Host process must resume from the typed canonical Parts,
    // not reinterpret their JSON as provider messages or append a duplicate
    // snapshot of the first answer.
    run_desktop_host(
        &root,
        &database,
        &llm,
        serde_json::json!({
            "prompt": "continue without duplicating history",
            "submission_id": "turn-mac-e2e-43",
            "user_message_id": "user-mac-e2e-43",
            "assistant_message_id": "assistant-mac-e2e-43",
            "input_origin": "interactive_user"
        }),
    );
    let reopened = Connection::open(&database).unwrap();
    let message_count: i64 = reopened
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id=?1",
            ["mac-e2e-conversation"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(message_count, 4, "two Runs must own exactly four messages");
    let assistant_texts = reopened
        .prepare(
            "SELECT m.run_id,m.id FROM messages m
             WHERE m.conversation_id=?1 AND m.role='assistant'
             ORDER BY m.ordinal",
        )
        .unwrap()
        .query_map(["mac-e2e-conversation"], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(|(run, message)| {
            assistant_journal::public_text(&database, "mac-e2e-conversation", &run, &message)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        assistant_texts,
        vec!["complete canonical answer", "complete canonical answer"],
        "each assistant answer must be persisted once and in Run order"
    );
    drop(reopened);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn desktop_host_replans_oversized_batch_without_persisting_or_executing_a_prefix() {
    let root = std::env::temp_dir().join(format!(
        "nola-batch-e2e-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("fixture.db");
    let llm = MockLlm::with_scenario(None, 1);
    run_desktop_host(
        &root,
        &database,
        &llm,
        serde_json::json!({
            "prompt": "继续审计 / continue reviewing",
            "submission_id": "batch-turn", "user_message_id": "batch-user",
            "assistant_message_id": "batch-assistant", "input_origin": "interactive_user"
        }),
    );
    assert_eq!(llm.model_requests.load(Ordering::SeqCst), 2);
    let connection = Connection::open(&database).unwrap();
    let tools: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM message_parts WHERE kind='tool'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        tools, 0,
        "rejected drafts must not become canonical tool cards"
    );
    let status: String = connection
        .query_row("SELECT status FROM runs WHERE id='batch-turn'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "sealed");
    assert_run_result_kind(&connection, "batch-turn", "answered");
}

#[test]
fn desktop_host_stops_replanning_when_tool_batch_correction_budget_is_exhausted() {
    let root = std::env::temp_dir().join(format!(
        "nola-batch-exhausted-e2e-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("fixture.db");
    let llm = MockLlm::with_scenario(None, usize::MAX);
    let output = run_desktop_host_result(
        &root,
        &database,
        &llm,
        serde_json::json!({
            "prompt": "继续审计", "submission_id": "batch-turn", "user_message_id": "batch-user",
            "assistant_message_id": "batch-assistant", "input_origin": "interactive_user"
        }),
    );
    assert_eq!(llm.model_requests.load(Ordering::SeqCst), 3);
    assert!(String::from_utf8_lossy(&output.stdout).contains("model_tool_batch_limit_exceeded"));
    let connection = Connection::open(&database).unwrap();
    let status: String = connection
        .query_row("SELECT status FROM runs WHERE id='batch-turn'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(status, "sealed");
    assert_run_result_kind(&connection, "batch-turn", "failed");
    let tools: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM message_parts WHERE kind='tool'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tools, 0);
}

#[test]
fn desktop_host_activation_batch_replans_atomically_and_has_a_bounded_failure_path() {
    for (rejections, expected_status, expected_requests) in
        [(1, "completed", 2), (usize::MAX, "failed", 3)]
    {
        let fixture = tempfile::tempdir().unwrap();
        let database = fixture.path().join("fixture.db");
        let llm = MockLlm::with_rejections(None, 0, rejections);
        let output = run_desktop_host_result(
            fixture.path(),
            &database,
            &llm,
            serde_json::json!({
                "prompt": "继续处理 / current design review",
                "submission_id": "activation-turn", "user_message_id": "activation-user",
                "assistant_message_id": "activation-assistant", "input_origin": "interactive_user"
            }),
        );
        let requests = llm.model_requests.load(Ordering::SeqCst);
        if requests != expected_requests {
            let diagnostics = fixture_provider_diagnostics(fixture.path());
            let retained = fixture.keep();
            panic!("activation batch expected {expected_requests} model requests, got {requests}; status={}; retained={}; diagnostics={diagnostics}",
                output.status, retained.display());
        }
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("activation_batch_requires_replan")
        );
        assert!(!fixture.path().join("must-not-exist.txt").exists());
        let connection = Connection::open(&database).unwrap();
        let tools: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM message_parts WHERE kind='tool'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tools, 0, "no rejected call becomes a canonical tool card");
        let status: String = connection
            .query_row(
                "SELECT status FROM runs WHERE id='activation-turn'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "sealed");
        assert_run_result_kind(&connection, "activation-turn", if expected_status == "completed" { "answered" } else { "failed" });
    }
}
