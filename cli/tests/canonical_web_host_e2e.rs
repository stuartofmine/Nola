use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct MockLlm {
    endpoint: String,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl MockLlm {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        counter.fetch_add(1, Ordering::SeqCst);
                        respond_to_llm(stream);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("Fixture model accept failed: {error}"),
                }
            }
        });
        Self {
            endpoint: format!("http://{address}/v1"),
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn stop(&mut self) -> thread::Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().map(thread::JoinHandle::join).unwrap_or(Ok(()))
    }
}

impl Drop for MockLlm {
    fn drop(&mut self) {
        if self.stop().is_err() && !thread::panicking() {
            panic!("Fixture model worker failed before teardown");
        }
    }
}

/// Exact owned Child handles, not a numeric PID discovered after its owner exits.
struct OwnedFixtureChild(Option<Child>);

impl OwnedFixtureChild {
    fn stop(&mut self) -> std::io::Result<()> {
        if let Some(child) = self.0.as_mut() {
            if child.try_wait()?.is_none() { child.kill()?; }
            child.wait()?;
            self.0.take();
        }
        Ok(())
    }
}

impl Drop for OwnedFixtureChild {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            if !thread::panicking() { panic!("Fixture child teardown failed: {error}"); }
            eprintln!("Fixture child teardown failed during panic: {error}");
        }
    }
}

fn start_owned_fixture_service(database: &std::path::Path) -> OwnedFixtureChild {
    let client = nola_service_client::ApplicationServiceClient::for_database(database);
    let socket = client.socket_path().to_path_buf();
    let child = Command::new(env!("CARGO_BIN_EXE_nola"))
        .arg("--nola-application-service")
        .arg("--database").arg(database)
        .arg("--socket").arg(&socket)
        .arg("--parent-pid").arg(std::process::id().to_string())
        .env("NOLA_DB_PATH", database)
        .env("NOLA_HOME", database.parent().unwrap())
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::inherit())
        .spawn().unwrap();
    let mut owned = OwnedFixtureChild(Some(child));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(owned.0.as_mut().unwrap().try_wait().unwrap().is_none(),
            "Owned fixture Service exited before readiness");
        if client.health().is_ok() { return owned; }
        assert!(Instant::now() < deadline, "Owned fixture Service did not become ready");
        thread::sleep(Duration::from_millis(10));
    }
}

fn respond_to_llm(mut stream: TcpStream) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer).unwrap_or(0);
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(str::trim)
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
    let answer = "## Durable reply\n\n- alpha\n- beta";
    let escaped = serde_json::to_string(answer).unwrap();
    let body = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":{escaped}}},\"finish_reason\":null}}]}}\n\n\
         data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
         data: [DONE]\n\n"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start_web(database: &std::path::Path, port: u16) -> Child {
    let web_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("web");
    start_web_with_root(database, port, &web_root)
}

fn start_web_with_root(database: &std::path::Path, port: u16, web_root: &std::path::Path) -> Child {
    start_web_with_root_and_log(database, port, web_root, None)
}

fn start_web_with_root_and_log(database: &std::path::Path, port: u16, web_root: &std::path::Path, log: Option<&std::path::Path>) -> Child {
    let log = log.map(|path| std::fs::File::create(path).unwrap());
    let mut child = Command::new(env!("CARGO_BIN_EXE_nola"))
        .args([
            "web",
            "--no-skills",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--web-root",
            web_root.to_str().unwrap(),
        ])
        .env("NOLA_DB_PATH", database)
        .env("NOLA_HOME", database.parent().unwrap())
        .stdout(log.as_ref().map(|file| Stdio::from(file.try_clone().unwrap())).unwrap_or_else(Stdio::null))
        .stderr(log.map(Stdio::from).unwrap_or_else(Stdio::null))
        .spawn()
        .unwrap();
    for _ in 0..1000 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return child;
        }
        if let Some(status) = child.try_wait().unwrap() { panic!("Web Host exited before binding: {status}"); }
        thread::sleep(Duration::from_millis(30));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("Web Host did not start");
}

#[test]
fn migrated_pages_serve_svelte_for_get_and_head_on_real_host() {
    struct Fixture { root: std::path::PathBuf, host: Option<Child> }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(child) = self.host.as_mut() { let _ = child.kill(); let _ = child.wait(); }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    let root = std::env::temp_dir().join(format!("nola-svelte-route-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(root.join("app/static/ui")).unwrap();
    let mut fixture = Fixture { root, host: None };
    let html = "<!doctype html><html><body>nola-svelte-route-fixture</body></html>";
    std::fs::write(fixture.root.join("app/static/ui/index.html"), html).unwrap();
    // A complete old template tree must never become a second rendering path,
    // including when the single Svelte build entry is unavailable.
    std::fs::create_dir_all(fixture.root.join("app/templates")).unwrap();
    for name in ["base", "dashboard", "models", "model_test", "capabilities", "results", "chat",
        "login", "register", "data_sources", "settings", "scheduled_actions"] {
        std::fs::write(fixture.root.join(format!("app/templates/{name}.html")),
            "<!doctype html><html>FORBIDDEN_LEGACY_TEMPLATE</html>").unwrap();
    }
    let port = free_port();
    fixture.host = Some(start_web_with_root(&fixture.root.join("test.sqlite"), port, &fixture.root));
    let pages = ["/dashboard", "/models", "/model-test", "/capabilities", "/results", "/chat", "/login",
        "/data-sources", "/settings", "/scheduled-actions"];
    for path in pages {
        for method in ["GET", "HEAD"] {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"), "{method} {path}: {response}");
            assert!(response.to_ascii_lowercase().contains("content-type: text/html"));
            let (_, body) = response.split_once("\r\n\r\n").unwrap();
            assert_eq!(body, if method == "HEAD" { "" } else { html }, "{method} {path}");
        }
    }
    let home = request(port, "GET", "/api/home", "");
    assert_eq!(home["tasks"], serde_json::json!([]));
    assert_eq!(home["stats"]["totalTasks"], 0);

    for (path, target) in [("/", "/chat"), ("/register", "/login")] {
        for method in ["GET", "HEAD"] {
            let mut stream=TcpStream::connect(("127.0.0.1",port)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            write!(stream,"{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").unwrap();
            let mut response=String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 303"),"{response}");
            assert!(response.to_ascii_lowercase().contains(&format!("location: {target}\r\n")),"{response}");
        }
    }

    std::fs::remove_file(fixture.root.join("app/static/ui/index.html")).unwrap();
    for path in pages {
        for method in ["GET", "HEAD"] {
            let mut stream=TcpStream::connect(("127.0.0.1",port)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            write!(stream,"{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").unwrap();
            let mut response=String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 503"),"{method} {path}: {response}");
            assert!(!response.contains("FORBIDDEN_LEGACY_TEMPLATE"));
            let (_,body)=response.split_once("\r\n\r\n").unwrap();
            if method=="HEAD" {assert!(body.is_empty(),"{response}");}
            else {assert!(body.contains("Web UI is not built"),"{response}");}
        }
    }
}

fn request(port: u16, method: &str, path: &str, body: &str) -> Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "unexpected response: {response}"
    );
    serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

#[cfg(unix)]
#[test]
fn endpoint_credentials_are_private_isolated_and_used_by_real_inference() {
    use serde_json::json;
    use std::sync::{Mutex, atomic::AtomicBool};
    struct Host(Child);
    impl Drop for Host { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
    struct Provider { endpoint: String, headers: Arc<Mutex<Vec<String>>>, requests: Arc<Mutex<Vec<String>>>, stop: Arc<AtomicBool>, worker: Option<thread::JoinHandle<()>> }
    impl Drop for Provider {
        fn drop(&mut self) { self.stop.store(true, Ordering::SeqCst); if let Some(worker) = self.worker.take() { worker.join().unwrap(); } }
    }
    fn provider() -> Provider {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let headers = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&headers);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_log = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else { thread::sleep(Duration::from_millis(10)); continue; };
                stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 8192];
                loop {
                    let count = stream.read(&mut buffer).unwrap_or(0);
                    if count == 0 { break; }
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]);
                        let length = header.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|value| value.trim().parse::<usize>().ok())).unwrap_or(0);
                        if bytes.len() >= end + 4 + length { break; }
                    }
                }
                // A closed preconnected socket is not an HTTP request and has
                // no Authorization header to audit.
                if bytes.is_empty() { continue; }
                let request = String::from_utf8_lossy(&bytes);
                let authorization = request.lines().find_map(|line| line.split_once(':').filter(|(key, _)| key.eq_ignore_ascii_case("authorization")).map(|(_, value)| value.trim().to_string())).unwrap_or_default();
                let authenticated = !authorization.is_empty();
                request_log.lock().unwrap().push(format!("{} credential={}", request.lines().next().unwrap_or("empty"), match authorization.as_str() {
                    "Bearer fixture-vault-key-a" => "a", "Bearer fixture-vault-key-b" => "b", "Bearer fixture-vault-key-a-2" => "a2", "" => "none", _ => "other",
                }));
                recorded.lock().unwrap().push(authorization);
                let payload: Value = request.split_once("\r\n\r\n").and_then(|(_, body)| serde_json::from_str(body).ok()).unwrap_or(Value::Null);
                let receipts = payload["messages"].as_array().into_iter().flatten()
                    .filter_map(|message| message["content"].as_str())
                    .find_map(|content| content.split_once("Current-turn Host-confirmed successful mutation call IDs: "))
                    .and_then(|(_, rest)| rest.split_once(". IDs establish"))
                    .and_then(|(raw, _)| serde_json::from_str::<Value>(raw).ok());
                let content = if let Some(receipts) = receipts {
                    json!({"decision":"accept","outcome":"answered","effect_call_ids":receipts,"constraints":[],"items":[{"label":"Fixture greeting","eligibility":"eligible","evidence":"Fixture model answered the greeting"}],"violations":[]}).to_string()
                } else { "Credential fixture reply".into() };
                let (status, content_type, body) = if authenticated {
                    ("200 OK", "text/event-stream", format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n", json!({"choices":[{"delta":{"content":content},"finish_reason":null}]}), json!({"choices":[{"delta":{},"finish_reason":"stop"}]})))
                } else { ("401 Unauthorized", "application/json", "{\"error\":{\"message\":\"missing endpoint credential\"}}".into()) };
                let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        Provider { endpoint, headers, requests, stop, worker: Some(worker) }
    }
    fn http(port: u16, method: &str, path: &str, body: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        let mut response = String::new(); stream.read_to_string(&mut response).unwrap(); response
    }
    fn mutation(snapshot: &Value, action: &str, secret: Option<&str>) -> Value {
        let mut value = json!({"expectedRevisions":snapshot["expectedRevisions"],"baseURL":snapshot["baseURL"],"wireProtocol":snapshot["wireProtocol"],"credentialRevision":snapshot["credentialRevision"],"action":action});
        if let Some(secret) = secret { value["secret"] = json!(secret); }
        value
    }
    struct Inference { wire: String, text: String }
    fn infer(port: u16, profile: &str) -> Inference {
        let id = uuid::Uuid::new_v4().to_string();
        let health = request(port, "GET", "/api/health", "");
        let mut body = json!({"modelProfileId":profile,"messages":[{"role":"user","content":"Reply with a short greeting."}],
            "agentMode":false,"max_iterations":2,"submission_id":id,"user_message_id":format!("{id}-user"),
            "assistant_message_id":format!("{id}-assistant"),"conversationId":format!("{id}-conversation"),
            "expected_host_id":health["host"]["id"],"client_id":"credential-fixture"});
        let identity = json!({"source_host_id":body["expected_host_id"],"client_id":body["client_id"],
            "conversation_id":body["conversationId"],"submission_id":body["submission_id"],
            "user_message_id":body["user_message_id"],"assistant_message_id":body["assistant_message_id"]});
        let prepared = request(port,"POST","/api/chat/agent/v2/submission",
            &json!({"operation":"prepare","identity":identity,"request":body}).to_string());
        body["request_digest"] = prepared["result"]["request_digest"].clone();
        let wire = http(port, "POST", "/api/chat/agent/v2/stream", &body.to_string());
        assert!(!wire.contains("fixture-vault-key"));
        if !wire.starts_with("HTTP/1.1 200") { return Inference { wire, text: String::new() }; }
        let observed = request(port, "POST", "/api/chat/agent/v2/submission", &json!({
            "operation":"observe", "identity":identity, "request_digest":body["request_digest"]
        }).to_string());
        assert_eq!(observed["source_host_id"], identity["source_host_id"]);
        let receipt = &observed["result"];
        assert_eq!(receipt["protocol_version"], 1);
        assert_eq!(receipt["identity"], identity);
        assert_eq!(receipt["request_digest"], body["request_digest"]);
        if receipt["phase"] != "admitted" { return Inference { wire, text: String::new() }; }
        let owner = receipt["admission"]["identity"].clone();
        assert_eq!(owner["run_id"], identity["submission_id"]);
        assert_eq!(owner["conversation_id"], identity["conversation_id"]);
        assert_eq!(owner["assistant_message_id"], identity["assistant_message_id"]);
        let journal = |query: Value| {
            let response = request(port,"POST","/api/chat/agent/v2/journal",&query.to_string());
            assert_eq!(response["source_host_id"], identity["source_host_id"]);
            assert!(!response.to_string().contains("fixture-vault-key"));
            response["result"].clone()
        };
        let seal = journal(json!({"operation":"terminal_seal","conversation_id":owner["conversation_id"],"run_id":owner["run_id"]}));
        assert_eq!(seal["identity"], owner, "EOF is not a terminal seal");
        if seal["result"]["value"]["kind"] != "answered" { return Inference { wire, text: String::new() }; }
        let head = journal(json!({"operation":"head","identity":owner}));
        assert_eq!(head["identity"], owner);
        let mut manifest_after = 0u64;
        let mut sealed_head = None;
        loop {
            let page = journal(json!({"operation":"terminal_manifest","seal":seal,
                "after_entry":manifest_after.to_string(),"max_records":32,"max_encoded_bytes":262144}));
            assert_eq!(page["seal"], seal);
            assert_eq!(page["after_entry"], manifest_after.to_string());
            let entries = page["entries"].as_array().unwrap();
            let next = page["next_entry"].as_str().unwrap().parse::<u64>().unwrap();
            assert_eq!(next.checked_sub(manifest_after).unwrap(), entries.len() as u64);
            for entry in entries {
                if entry["kind"] == "assistant" && entry["through"]["identity"] == owner {
                    assert!(sealed_head.replace(entry["through"].clone()).is_none());
                }
            }
            if page["complete"] == true {
                assert_eq!(page["next_entry"], seal["entry_count"]);
                break;
            }
            assert!(next > manifest_after);
            manifest_after = next;
        }
        assert_eq!(sealed_head.as_ref(), Some(&head));
        let snapshot = journal(json!({"operation":"open_snapshot","at":head}));
        // Release the immutable snapshot even if a validation assertion fails.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut text = String::new();
            let mut after: Option<String> = None;
            let mut cursors = std::collections::HashSet::new();
            loop {
                let page = journal(json!({"operation":"parts","snapshot":snapshot,"after_part_id":after,
                    "max_records":32,"max_encoded_bytes":262144}));
                assert_eq!(page["at"], head);
                for part in page["parts"].as_array().unwrap() {
                    assert_eq!(part["identity"], owner);
                    if part["metadata"]["kind"] != "text" || part["metadata"]["data"]["visibility"] != "public" { continue; }
                    for content in part["bodies"].as_array().unwrap().iter().filter(|body| body["field"] == "markdown") {
                        let mut offset = 0u64;
                        loop {
                            let range = journal(json!({"operation":"range","snapshot":snapshot,"part_id":part["part_id"],
                                "content":content,"offset_bytes":offset.to_string(),"limit_bytes":32768}));
                            assert_eq!(range["offset_bytes"], offset.to_string());
                            assert_eq!(range["total_utf8_bytes"], content["utf8_bytes"]);
                            let next = range["next_offset_bytes"].as_str().unwrap().parse::<u64>().unwrap();
                            let total = content["utf8_bytes"].as_str().unwrap().parse::<u64>().unwrap();
                            let value = range["text"].as_str().unwrap();
                            assert!(next >= offset && next <= total && value.len() <= 32768);
                            assert_eq!(next - offset, value.len() as u64);
                            assert_eq!(range["end_of_content"], next == total);
                            text.push_str(value);
                            if next == total { break; }
                            assert!(next > offset);
                            offset = next;
                        }
                    }
                }
                after = page["next_part_id"].as_str().map(str::to_owned);
                let Some(cursor) = &after else { break };
                assert!(cursors.insert(cursor.clone()), "directory cursor cycle");
            }
            text
        }));
        journal(json!({"operation":"release_snapshot","snapshot":snapshot}));
        let text = result.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        Inference { wire, text }
    }
    let a = provider(); let b = provider();
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("test.sqlite");
    let mut store = nola_store::NolaStore::open(&database).unwrap();
    for (id, endpoint) in [("a", &a.endpoint), ("b", &b.endpoint)] {
        store.upsert_model_profile(&nola_store::StoredModelProfile {
            id:id.into(), display_name:id.into(), provider:"openai-compatible".into(), model_name:"fixture".into(), runtime:"remote".into(),
            source_json:json!({"endpointID":id,"endpointName":id,"baseURL":endpoint,"wireProtocol":"chat_completions","endpointEnabled":true,"modelEnabled":true,"capabilities":["chat"],
                "executionCapabilities":nola_application::ModelExecutionCapabilities {
                    sampling_parameters:true, ..Default::default()
                }}).to_string(),
            parameters_json:json!({"temperature":0.25,"maxOutputTokens":16384}).to_string(), selected:id=="a", enabled:true, revision:0, created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
        }).unwrap();
    }
    drop(store);
    let port = free_port();
    let host_log = root.path().join("host.log");
    let web_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("web");
    let _host = Host(start_web_with_root_and_log(&database, port, &web_root, Some(&host_log)));
    let initial_profiles = request(port, "GET", "/api/model-profiles", "");
    let mut snapshots = Vec::new();
    for (id, secret) in [("a", "fixture-vault-key-a"), ("b", "fixture-vault-key-b")] {
        let path = format!("/api/model-endpoints/{id}/credential");
        let initial = request(port, "GET", &path, "");
        assert_eq!(initial["credentialRevision"], 0); assert_eq!(initial["hasCredential"], false);
        let body = mutation(&initial, "replace", Some(secret));
        let saved = request(port, "POST", &path, &body.to_string());
        assert_eq!(saved["credentialRevision"], 1); assert_eq!(saved["hasCredential"], true);
        assert!(!saved.to_string().contains(secret));
        let stale = http(port, "POST", &path, &body.to_string());
        assert!(stale.starts_with("HTTP/1.1 409")); assert!(!stale.contains(secret));
        let malformed = http(port, "POST", &path, &format!("{{invalid-json-{secret}"));
        assert!(malformed.starts_with("HTTP/1.1 400")); assert!(!malformed.contains(secret));
        snapshots.push(saved);
    }
    assert_eq!(request(port, "GET", "/api/model-profiles", ""), initial_profiles, "Host secrets cannot mutate canonical profiles");
    for id in ["a", "b"] {
        let response = infer(port, id);
        assert!(response.wire.starts_with("HTTP/1.1 200"), "Inference failed: {}", response.wire);
        assert!(response.text.contains("Credential fixture reply"), "No committed fixture model output: {}", response.wire);
        assert!(!response.text.contains("fixture-vault-key"));
    }
    assert!(a.headers.lock().unwrap().iter().all(|value| value == "Bearer fixture-vault-key-a"));
    assert!(b.headers.lock().unwrap().iter().all(|value| value == "Bearer fixture-vault-key-b"), "Provider B requests: {:?}", b.requests.lock().unwrap());
    assert!(!a.headers.lock().unwrap().is_empty()); assert!(!b.headers.lock().unwrap().is_empty());
    let replaced = request(port, "POST", "/api/model-endpoints/a/credential", &mutation(&snapshots[0], "replace", Some("fixture-vault-key-a-2")).to_string());
    assert_eq!(replaced["credentialRevision"], 2);
    assert!(infer(port, "a").text.contains("Credential fixture reply"));
    assert_eq!(a.headers.lock().unwrap().last().map(String::as_str), Some("Bearer fixture-vault-key-a-2"));
    // A compatible legacy key must not spring back to life after explicit clear.
    assert!(http(port, "POST", "/api/agent/settings", &json!({"agent_llm_endpoint":b.endpoint,"agent_llm_api_key":"fixture-legacy-key"}).to_string()).starts_with("HTTP/1.1 200"));
    let cleared = request(port, "POST", "/api/model-endpoints/b/credential", &mutation(&snapshots[1], "clear", None).to_string());
    assert_eq!(cleared["hasCredential"], false); assert_eq!(cleared["credentialRevision"], 2);
    let before = b.headers.lock().unwrap().len();
    let failed = infer(port, "b");
    assert!(!failed.text.contains("Credential fixture reply"));
    assert!(b.headers.lock().unwrap()[before..].iter().all(String::is_empty));
    let current = request(port, "GET", "/api/model-endpoints/a/credential", "");
    request(port, "POST", "/api/model-endpoints/a/configuration", &json!({"expectedRevisions":current["expectedRevisions"],"name":"Moved","baseURL":b.endpoint,"wireProtocol":"chat_completions","enabled":true}).to_string());
    let moved = request(port, "GET", "/api/model-endpoints/a/credential", "");
    assert_eq!(moved["hasCredential"], false);
    let moved_catalog = request(port, "GET", "/api/model-profiles", "");
    let moved_profiles = moved_catalog["profiles"].as_array().unwrap();
    let moved_profile = moved_profiles.iter().find(|profile| profile["id"] == "a").unwrap();
    assert!(moved_profile["executionCapabilities"].is_null());
    assert_eq!(moved_profile["executionCapabilityStatus"], "needs_confirmation");
    let before_unconfirmed = b.headers.lock().unwrap().len();
    let unconfirmed = infer(port, "a");
    assert!(unconfirmed.wire.starts_with("HTTP/1.1 422"), "{}", unconfirmed.wire);
    assert!(unconfirmed.wire.contains("model_execution_capabilities_required"));
    assert_eq!(b.headers.lock().unwrap().len(), before_unconfirmed,
        "An unconfirmed route must not reach the provider");
    assert_eq!(request(port, "GET", "/api/model-profiles", ""), moved_catalog,
        "Admission rejection must not write the catalog");

    // This fixture provider explicitly supports sampling on the new address.
    // Confirm its concrete route through the same full-catalog CAS as Model
    // Center, before continuing the independent credential-isolation check.
    let expected: serde_json::Map<String, Value> = moved_profiles.iter()
        .map(|profile| (profile["id"].as_str().unwrap().to_owned(), profile["revision"].clone())).collect();
    let endpoints: Vec<Value> = moved_profiles.iter().map(|profile| json!({
        "id":profile["endpoint"]["id"], "name":profile["endpoint"]["name"],
        "baseURL":profile["endpoint"]["baseURL"], "wireProtocol":profile["endpoint"]["wireProtocol"],
        "enabled":profile["endpoint"]["enabled"], "models":[{
            "id":profile["id"], "modelName":profile["modelName"], "displayName":profile["displayName"],
            "enabled":profile["enabled"], "priority":if profile["id"] == "a" {100} else {50},
            "capabilities":profile["capabilities"], "reasoning":profile["reasoning"],
            "parameters":profile["parameters"],
            "executionCapabilities":nola_application::ModelExecutionCapabilities {
                sampling_parameters:true, ..Default::default()
            }
        }]
    })).collect();
    let confirmed = request(port, "POST", "/api/model-profiles/catalog/replace",
        &json!({"expectedRevisions":expected,"endpoints":endpoints}).to_string());
    assert_eq!(confirmed["profiles"].as_array().unwrap().iter()
        .find(|profile| profile["id"] == "a").unwrap()["executionCapabilityStatus"], "text_only");
    assert_eq!(request(port, "GET", "/api/model-endpoints/a/credential", "")["hasCredential"], false);
    let stale_identity = http(port, "POST", "/api/model-endpoints/a/credential", &mutation(&current, "replace", Some("fixture-vault-key-a-new")).to_string());
    assert!(stale_identity.starts_with("HTTP/1.1 409"));
    assert!(!stale_identity.contains("fixture-vault-key"));
    let before = b.headers.lock().unwrap().len();
    let failed = infer(port, "a");
    assert!(!failed.text.contains("Credential fixture reply"));
    assert!(b.headers.lock().unwrap().len() > before,
        "After explicit route confirmation, this check must exercise the provider's missing-credential rejection");
    assert!(b.headers.lock().unwrap()[before..].iter().all(String::is_empty));
    for path in [database.clone(), database.with_file_name("test.sqlite-wal")] {
        if let Ok(canonical) = std::fs::read(path) {
            assert!(!String::from_utf8_lossy(&canonical).contains("fixture-vault-key"), "New credentials cannot enter canonical storage/receipts or WAL");
        }
    }
    assert!(!std::fs::read_to_string(host_log).unwrap().contains("fixture-vault-key"), "Host stdout/stderr must not contain credential values");
}

#[test]
fn remote_endpoint_configuration_uses_canonical_profiles_over_real_http() {
    struct Host(Child);
    impl Drop for Host { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("test.sqlite");
    let mut store = nola_store::NolaStore::open(&database).unwrap();
    for id in ["first", "second"] {
        store.upsert_model_profile(&nola_store::StoredModelProfile {
            id: id.into(), display_name: id.into(), provider: "openai-compatible".into(), model_name: "fixture".into(), runtime: "remote".into(),
            source_json: serde_json::json!({"endpointID":"endpoint/with space", "endpointName":"Before", "baseURL":"https://fixture.invalid/v1", "wireProtocol":"responses", "endpointEnabled":true, "modelEnabled":true, "capabilities":["chat"], "apiKey":"private-fixture-key"}).to_string(),
            parameters_json: serde_json::json!({"temperature":0.25,"maxOutputTokens":16384}).to_string(),
            selected: id == "first", enabled: true, revision: 0, created_at_ms: 1, updated_at_ms: 1, deleted_at_ms: None,
        }).unwrap();
    }
    drop(store);
    let port = free_port();
    let _host = Host(start_web(&database, port));
    let initial = request(port, "GET", "/api/model-profiles", "");
    let revisions: serde_json::Map<String, Value> = initial["profiles"].as_array().unwrap().iter()
        .map(|profile| (profile["id"].as_str().unwrap().to_owned(), profile["revision"].clone())).collect();
    let body = serde_json::json!({"expectedRevisions":revisions,"name":"Edited endpoint","baseURL":"https://edited.fixture/v2","wireProtocol":"anthropic","enabled":false}).to_string();
    let saved = request(port, "POST", "/api/model-endpoints/endpoint%2Fwith%20space/configuration", &body);
    assert_eq!(saved["profiles"].as_array().unwrap().len(), 2);
    assert!(!saved.to_string().contains("private-fixture-key"));
    for profile in saved["profiles"].as_array().unwrap() {
        assert_eq!(profile["endpoint"]["name"], "Edited endpoint");
        assert_eq!(profile["endpoint"]["baseURL"], "https://edited.fixture/v2");
        assert_eq!(profile["endpoint"]["enabled"], false);
        assert_eq!(profile["endpoint"]["wireProtocol"], "anthropic");
        assert_eq!(profile["parameters"]["temperature"], 0.25);
    }
    assert_eq!(request(port, "GET", "/api/model-profiles", ""), saved);
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(stream, "POST /api/model-endpoints/endpoint%2Fwith%20space/configuration HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 409"), "Stale endpoint write: {response}");
    assert_eq!(request(port, "GET", "/api/model-profiles", ""), saved);
}

#[test]
fn cli_transcript_is_visible_after_real_web_host_restart() {
    let root = std::env::temp_dir().join(format!(
        "nola-web-host-e2e-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    eprintln!("Preserved Web restart fixture: {}", root.display());
    let database = root.join("nola.db");
    // Web and CLI share this explicitly owned same-build Service. Waiting for
    // a Web process alone cannot join a Service it would otherwise auto-spawn.
    let mut service = start_owned_fixture_service(&database);
    let port = free_port();
    let mut web = OwnedFixtureChild(Some(start_web(&database, port)));
    // The CLI creates its own conversation with the explicit fixture model.
    // A pre-existing Web draft deliberately has no selected model and cannot
    // be silently reconfigured from a different client's command-line defaults.
    let conversation_id = "cli-web-history";
    let mut llm = MockLlm::start();
    let cli = Command::new(env!("CARGO_BIN_EXE_nola"))
        .args([
            "persist this turn",
            "--endpoint",
            &llm.endpoint,
            "--api-key",
            "e2e",
            "--model",
            "e2e-model",
            "--session",
            conversation_id,
            "--no-tools",
            "--no-skills",
            "--json",
            "--lang",
            "en",
        ])
        .env("NOLA_DB_PATH", &database)
        .env("NOLA_HOME", &root)
        .env("NOLA_CLI_INTERACTION_MODE", "never")
        .output()
        .unwrap();
    std::fs::write(root.join("cli.stdout.jsonl"), &cli.stdout).unwrap();
    std::fs::write(root.join("cli.stderr.log"), &cli.stderr).unwrap();
    assert!(
        cli.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert!(llm.requests.load(Ordering::SeqCst) >= 1);

    let path = format!("/api/chat/conversations/{conversation_id}/messages");
    let first = request(port, "GET", &path, "");
    assert_eq!(first["messages"].as_array().unwrap().len(), 2);
    assert_eq!(
        first["messages"][1]["content"],
        "## Durable reply\n\n- alpha\n- beta"
    );
    let latest = request(port, "GET", &format!("{path}?limit=1"), "");
    assert_eq!(latest["messages"].as_array().unwrap().len(), 1);
    assert!(latest["messageWindow"]["hasMore"].as_bool().unwrap());
    let before = latest["messageWindow"]["before"].as_i64().unwrap();
    let revision = latest["messageWindow"]["projectionRevision"].as_u64().unwrap();
    let sequence = latest["messageWindow"]["eventSequence"].as_u64().unwrap();
    let older = request(port, "GET", &format!("{path}?limit=1&before={before}&expectedRevision={revision}&expectedEventSequence={sequence}"), "");
    assert_eq!(older["messages"].as_array().unwrap().len(), 1);
    assert_eq!(older["messages"][0]["messageId"], first["messages"][0]["messageId"]);
    assert_eq!(older["messageWindow"]["projectionRevision"], revision);
    assert_eq!(older["messageWindow"]["eventSequence"], sequence);
    for malformed in ["before=not-a-number", "before=-1", "before=", "before=0", "limit=0", "limit=201", "expectedRevision=oops", "expectedEventSequence=-1"] {
        let response = raw_request(port, "GET", &format!("{path}?{malformed}"), "");
        assert!(response.starts_with("HTTP/1.1 400"), "{malformed}: {response}");
    }
    for stale in [format!("expectedRevision={}", revision + 1), format!("expectedEventSequence={}", sequence + 1)] {
        let response = raw_request(port, "GET", &format!("{path}?limit=1&before={before}&{stale}"), "");
        assert!(response.starts_with("HTTP/1.1 409"), "{stale}: {response}");
        assert!(response.contains("stale_message_window"), "{response}");
        assert!(!response.contains("Durable reply"), "stale response must not contain a mixed page: {response}");
    }
    let conversation_path = format!("/api/chat/conversations/{conversation_id}");
    // Titles use the conversation metadata revision, not configuration CAS or
    // a GET on the mutation-only conversation resource.
    let conversations = request(port, "GET", "/api/chat/conversations", "");
    let current = conversations["conversations"].as_array().unwrap().iter()
        .find(|value| value["conversationId"] == conversation_id).unwrap();
    request(port, "PATCH", &conversation_path, &serde_json::json!({
        "title":"Renamed while paging", "expectedRevision":current["revision"],
    }).to_string());
    let changed = request(port, "GET", &format!("{path}?limit=1"), "");
    assert!(changed["messageWindow"]["projectionRevision"].as_u64().unwrap() > revision);
    let stale_after_mutation = raw_request(port, "GET", &format!("{path}?limit=1&before={before}&expectedRevision={revision}&expectedEventSequence={sequence}"), "");
    assert!(stale_after_mutation.starts_with("HTTP/1.1 409"), "{stale_after_mutation}");

    web.stop().unwrap();
    let mut restarted = OwnedFixtureChild(Some(start_web(&database, port)));
    let reopened = request(port, "GET", &path, "");
    assert_eq!(reopened["messages"], first["messages"]);
    let deleted = raw_request(port,"DELETE",&format!("/api/chat/conversations/{conversation_id}"),"");
    assert!(deleted.starts_with("HTTP/1.1 200"),"history-bearing Core session must close before deletion: {deleted}");
    assert!(raw_request(port,"GET",&path,"").starts_with("HTTP/1.1 404"));

    llm.stop().expect("Fixture model worker must be joined");
    restarted.stop().unwrap();
    service.stop().unwrap();
    // Keep the registered DB and derived publications as evidence on success
    // and failure. No cleanup retry may hide an unjoined writer lifecycle.
    assert!(database.is_file(), "The audited fixture database must remain available");
}

fn raw_request(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn pdf_preview_resource_is_real_host_scoped_and_read_only() {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    struct Host(Child);
    impl Drop for Host { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("pdf.sqlite");
    let port = free_port();
    let _host = Host(start_web(&database, port));
    let one = request(port,"POST","/api/chat/conversations",r#"{"title":"PDF one"}"#);
    let two = request(port,"POST","/api/chat/conversations",r#"{"title":"PDF two"}"#);
    let one_id = one["conversationId"].as_str().unwrap();
    let two_id = two["conversationId"].as_str().unwrap();
    let path = format!("/api/chat/conversations/{one_id}/resources");
    let pdf = b"%PDF-1.4\n1 0 obj <<>> endobj\n%%EOF\n";
    let payload = serde_json::json!({"displayName":"report.pdf","mimeType":"application/pdf","dataBase64":STANDARD.encode(pdf)});
    let saved = request(port,"POST",&path,&payload.to_string());
    assert_eq!(saved["conversationId"],one_id);
    assert_eq!(saved["mimeType"],"application/pdf");
    assert_eq!(saved["byteCount"],pdf.len());
    let listed = request(port,"GET",&path,"");
    assert_eq!(listed["resources"].as_array().unwrap().len(),1);
    assert_eq!(listed["resources"][0]["resourceId"],saved["resourceId"]);
    assert!(listed["resources"][0].get("dataBase64").is_none());
    assert_eq!(request(port,"GET",&format!("/api/chat/conversations/{two_id}/resources"),"")["resources"],serde_json::json!([]));
    let preview = saved["previewUrl"].as_str().unwrap();
    let response = raw_request(port,"GET",preview,"");
    assert!(response.starts_with("HTTP/1.1 200"),"{response}");
    assert!(response.to_ascii_lowercase().contains("content-type: application/pdf"));
    assert!(response.to_ascii_lowercase().contains("x-content-type-options: nosniff"));
    assert!(response.to_ascii_lowercase().contains("cache-control: no-store"));
    assert_eq!(response.split_once("\r\n\r\n").unwrap().1.as_bytes(),pdf);
    let head = raw_request(port,"HEAD",preview,"");
    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(head.to_ascii_lowercase().contains(&format!("content-length: {}",pdf.len())));
    assert_eq!(head.split_once("\r\n\r\n").unwrap().1,"");
    let cross_owner = preview.replace(one_id,two_id);
    assert!(raw_request(port,"GET",&cross_owner,"").starts_with("HTTP/1.1 404"));
    let spoofed = serde_json::json!({"displayName":"report.pdf","mimeType":"application/pdf","dataBase64":STANDARD.encode(b"<html>not pdf</html>")});
    assert!(raw_request(port,"POST",&path,&spoofed.to_string()).starts_with("HTTP/1.1 422"));
    let oversized = serde_json::json!({"displayName":"large.pdf","mimeType":"application/pdf","dataBase64":STANDARD.encode(vec![b'x';8*1024*1024+1])});
    assert!(raw_request(port,"POST",&path,&oversized.to_string()).starts_with("HTTP/1.1 413"));
    assert!(raw_request(port,"DELETE",&cross_owner,"").starts_with("HTTP/1.1 404"));
    assert!(raw_request(port,"DELETE",preview,"").starts_with("HTTP/1.1 200"));
    assert!(raw_request(port,"GET",preview,"").starts_with("HTTP/1.1 404"));
    assert_eq!(request(port,"GET",&path,"")["resources"],serde_json::json!([]));
    let saved = request(port,"POST",&path,&payload.to_string());
    let preview = saved["previewUrl"].as_str().unwrap();
    let fixture_run = uuid::Uuid::new_v4().to_string();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
        VALUES(?1,?2,'conversation','running',1,1,1)",rusqlite::params![fixture_run,one_id]).unwrap();
    let rejected_active = raw_request(port,"DELETE",&format!("/api/chat/conversations/{one_id}"),"");
    assert!(rejected_active.starts_with("HTTP/1.1 409"),"{rejected_active}");
    assert!(raw_request(port,"GET",preview,"").starts_with("HTTP/1.1 200"));
    connection.execute("UPDATE runs SET status='completed',completed_at_ms=2 WHERE id=?1",rusqlite::params![fixture_run]).unwrap();
    // A legacy-looking completion label is not an immutable sealed result.
    assert!(raw_request(port,"DELETE",&format!("/api/chat/conversations/{one_id}"),"").starts_with("HTTP/1.1 409"));
    assert!(raw_request(port,"GET",preview,"").starts_with("HTTP/1.1 200"));
    connection.execute("UPDATE runs SET status='sealed' WHERE id=?1",rusqlite::params![fixture_run]).unwrap();
    let deleted_conversation = raw_request(port,"DELETE",&format!("/api/chat/conversations/{one_id}"),"");
    assert!(deleted_conversation.starts_with("HTTP/1.1 200"),"{deleted_conversation}");
    assert!(raw_request(port,"GET",preview,"").starts_with("HTTP/1.1 404"));
}
