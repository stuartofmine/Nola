use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Host { child: Child, port: u16 }
impl Drop for Host { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }
impl Host {
    fn start(root: &std::path::Path) -> Self {
        std::fs::create_dir_all(root.join("app/static/ui")).unwrap();
        std::fs::write(root.join("app/static/ui/index.html"), "<!doctype html><html>settings-svelte-fixture</html>").unwrap();
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut host = Self { port, child: Command::new(env!("CARGO_BIN_EXE_nola"))
            .args(["web", "--host", "127.0.0.1", "--port", &port.to_string(), "--web-root", root.to_str().unwrap()])
            .env("NOLA_DB_PATH", root.join("settings.sqlite")).env("NOLA_HOME", root).current_dir(root)
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap() };
        for _ in 0..1000 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() { return host; }
            assert!(host.child.try_wait().unwrap().is_none());
            std::thread::sleep(Duration::from_millis(30));
        }
        panic!("host did not start");
    }
    fn raw(&self, method: &str, path: &str, body: &str, cookie: &str) -> (u16, String, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nCookie: {cookie}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", self.port, body.len()).unwrap();
        let mut response = String::new(); stream.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let body = if headers.to_ascii_lowercase().contains("transfer-encoding: chunked") {
            let mut remaining = body; let mut decoded = String::new();
            loop {
                let (size, rest) = remaining.split_once("\r\n").unwrap();
                let size = usize::from_str_radix(size.split(';').next().unwrap(), 16).unwrap();
                if size == 0 { break; }
                decoded.push_str(&rest[..size]); remaining = &rest[size + 2..];
            }
            decoded
        } else { body.into() };
        (headers.split_whitespace().nth(1).unwrap().parse().unwrap(), headers.into(), body)
    }
    fn json(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let (status, _, body) = self.raw(method, path, &body.to_string(), "");
        (status, serde_json::from_str(&body).unwrap())
    }
}

#[test]
fn settings_real_host_permission_worker_and_session_contracts() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::start(root.path());
    for method in ["GET", "HEAD"] {
        let (status, _, body) = host.raw(method, "/settings", "", "");
        assert_eq!(status, 200);
        if method == "GET" { assert!(body.contains("settings-svelte-fixture")); } else { assert!(body.is_empty()); }
    }
    let (status, seeded) = host.json("POST", "/api/agent/settings", json!({"agent_llm_model":"fixture-model","temperature":0.37,"custom_system_prompt":"preserve me"}));
    assert_eq!(status, 200, "{seeded}");
    let (status, seeded) = host.json("POST", "/api/agent/settings", json!({"mcp_baseline":[],"mcp_servers":[{"server_id":"settings-fixture","name":"Disabled fixture","enabled":false,"endpoint":"http://127.0.0.1:1"}]}));
    assert_eq!(status, 200, "{seeded}");
    assert_eq!(seeded["mcp_servers"].as_array().unwrap().len(), 1);
    for policy in ["cautious", "allow_all"] {
        let (status, settings) = host.json("POST", "/api/agent/settings", json!({"permission_profile":policy,"allowed_dirs":[root.path()],"bash_prefixes":["git status"],"host_tools_enabled":false}));
        assert_eq!(status, 200, "{settings}"); assert_eq!(settings["permission_profile"], policy);
        for field in ["agent_llm_model", "temperature", "custom_system_prompt", "mcp_servers"] { assert_eq!(settings[field], seeded[field], "{field} changed"); }
        assert_eq!(settings["network_policy"], if policy == "allow_all" { "allow" } else { "ask" });
    }
    for invalid in ["moderate", "allow-all", "unknown", ""] {
        assert_eq!(host.json("POST", "/api/agent/settings", json!({"permission_profile":invalid})).0, 422);
    }
    assert_eq!(host.json("POST", "/api/agent/settings", json!({"network_policy":"typo"})).0, 422);
    let (_, settings) = host.json("GET", "/api/agent/settings", Value::Null);
    assert_eq!(settings["permission_profile"], "allow_all");

    let (_, initial) = host.json("GET", "/api/messaging-channel-worker", Value::Null);
    assert_eq!(initial["enabled"], false); assert_eq!(initial["has_access_token"], false);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let mut requests = String::new();
        for (status, body) in [("400 Bad Request", "secret-fixture-new-token provider sensitive response"), ("200 OK", r#"{"connection":{"id":"fixture","enabled":false}}"#)] {
            let mut accepted = None;
            for _ in 0..1000 {
                if let Ok((stream, _)) = listener.accept() { accepted = Some(stream); break; }
                std::thread::sleep(Duration::from_millis(10));
            }
            let mut stream = accepted.expect("mock connection request");
            stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            let mut bytes = [0; 8192]; let count = stream.read(&mut bytes).unwrap();
            requests.push_str(&String::from_utf8_lossy(&bytes[..count]));
            write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    let old = "http://127.0.0.1:1";
    assert_eq!(host.json("PUT", "/api/messaging-channel-worker", json!({"enabled":false,"backend_url":old,"access_token":"secret-fixture-old-token","execution_backend":"nola"})).0, 200);
    assert_eq!(host.json("PUT", "/api/messaging-channel-worker", json!({"enabled":false,"backend_url":endpoint})).0, 400);
    let (_, unchanged) = host.json("GET", "/api/messaging-channel-worker", Value::Null);
    assert_eq!(unchanged["backend_url"], old);
    assert_eq!(host.json("PUT", "/api/messaging-channel-worker", json!({"clear_access_token":true,"access_token":"conflict"})).0, 400);
    let (status, updated) = host.json("PUT", "/api/messaging-channel-worker", json!({"enabled":false,"backend_url":endpoint,"access_token":"secret-fixture-new-token"}));
    assert_eq!(status, 200); assert_eq!(updated["has_access_token"], true); assert!(updated.get("access_token").is_none());
    let (status, failure) = host.json("PUT", "/api/messaging-channels", json!({"id":"fixture","provider":"telegram","enabled":false}));
    assert_eq!(status, 400); assert!(!failure.to_string().contains("secret-fixture")); assert!(!failure.to_string().contains("provider sensitive"));
    let (status, saved) = host.json("PUT", "/api/messaging-channels", json!({"id":"fixture","provider":"telegram","enabled":false}));
    assert_eq!(status, 200); assert_eq!(saved["connection"]["enabled"], false);
    let request = worker.join().unwrap();
    assert!(request.contains("secret-fixture-new-token")); assert!(!request.contains("secret-fixture-old-token"));
    let (status, cleared) = host.json("PUT", "/api/messaging-channel-worker", json!({"enabled":false,"clear_access_token":true,"backend_url":"http://127.0.0.1:2"}));
    assert_eq!(status, 200); assert_eq!(cleared["has_access_token"], false);
    assert_eq!(host.json("GET", "/api/messaging-channels", Value::Null).0, 400);

    let token = std::fs::read_to_string(root.path().join(".nola/web/mobile-access-token")).unwrap();
    let (status, headers, _) = host.raw("POST", "/api/web/login", &json!({"token":token.trim()}).to_string(), "");
    assert_eq!(status, 200);
    let cookie = headers.lines().filter_map(|line| line.split_once(':')).filter(|(key, _)| key.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, value)| value.trim().split(';').next().unwrap()).collect::<Vec<_>>().join("; ");
    let (_, _, session) = host.raw("GET", "/api/web/session", "", &cookie);
    assert_eq!(serde_json::from_str::<Value>(&session).unwrap()["sessionAuthenticated"], true);
    let (status, headers, _) = host.raw("POST", "/api/web/logout", "{}", &cookie);
    assert_eq!(status, 200); assert!(headers.contains("Max-Age=0"));
    let (_, _, session) = host.raw("GET", "/api/web/session", "", &cookie);
    let session: Value = serde_json::from_str(&session).unwrap();
    assert_eq!(session["sessionAuthenticated"], false);
    assert_eq!(session["localTrust"], true); assert_eq!(session["authenticated"], true);
}
