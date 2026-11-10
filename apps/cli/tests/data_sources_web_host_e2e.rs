//! Isolated real HTTP coverage for the host-owned data source contract.
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Host { child: Child, port: u16 }
impl Drop for Host {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}
impl Host {
    fn start(root: &std::path::Path) -> Self {
        std::fs::create_dir_all(root.join("app/static/ui")).unwrap();
        std::fs::write(root.join("app/static/ui/index.html"), "<!doctype html><html><body>data-source-svelte-shell</body></html>").unwrap();
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut host = Self { port, child: Command::new(env!("CARGO_BIN_EXE_nola"))
            .args(["web", "--host", "127.0.0.1", "--port", &port.to_string(), "--web-root", root.to_str().unwrap()])
            .env("NOLA_DB_PATH", root.join("fixture.sqlite"))
            .env("NOLA_HOME", root).current_dir(root)
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap() };
        for _ in 0..1000 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() { return host; }
            assert!(host.child.try_wait().unwrap().is_none(), "host exited before binding");
            std::thread::sleep(Duration::from_millis(30));
        }
        panic!("host did not bind");
    }
    fn request(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let body = body.to_string();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", self.port, body.len()).unwrap();
        let mut response = String::new(); stream.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").expect("HTTP response");
        let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).expect("JSON response"))
    }
}

#[test]
fn data_sources_real_host_crud_validation_and_policy_contract() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("records.json");
    std::fs::write(&file, r#"[{"count":7,"privateValue":"fixture-never-return-content"}]"#).unwrap();
    let host = Host::start(root.path());
    for method in ["GET", "HEAD"] {
        let mut stream = TcpStream::connect(("127.0.0.1", host.port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        write!(stream, "{method} /data-sources HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", host.port).unwrap();
        let mut response = String::new(); stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        let (_, body) = response.split_once("\r\n\r\n").unwrap();
        if method == "GET" { assert!(body.contains("data-source-svelte-shell")); }
        else { assert!(body.is_empty()); }
    }
    let (status, empty) = host.request("GET", "/api/data-sources", Value::Null);
    assert_eq!(status, 200); assert_eq!(empty["sources"], json!([])); assert_eq!(empty["localFirst"], true);
    let (status, usage) = host.request("GET", "/api/data-sources/usage", Value::Null);
    assert_eq!(status, 200); assert_eq!(usage["usageRecords"], json!([]));
    // Real HTTP connector, without external network access or provider credentials.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let location = format!("http://{}/records", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let provider = std::thread::spawn(move || {
        for _ in 0..1000 {
            if let Ok((mut stream, _)) = listener.accept() {
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = [0; 4096]; let _ = stream.read(&mut request);
                let body = r#"[{"value":1}]"#;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("HTTP source was never requested");
    });
    let (status, http_source) = host.request("POST", "/api/data-sources", json!({"name":"HTTP fixture","kind":"http","location":location,"accessPolicy":"blocked"}));
    provider.join().unwrap();
    assert_eq!(status, 201, "{http_source}");
    assert_eq!(http_source["source"]["validationContract"]["readRecipe"]["method"], "httpGet");
    assert_eq!(host.request("DELETE", &format!("/api/data-sources/{}", http_source["source"]["id"].as_str().unwrap()), Value::Null).0, 200);
    let mut draft = json!({"name":"Fixture", "kind":"localFile", "location":file, "accessPolicy":"conversationOptIn", "maximumAgeMinutes":60});
    let (status, created) = host.request("POST", "/api/data-sources", draft.clone());
    assert_eq!(status, 201, "{created}"); assert_eq!(created["tested"], true);
    assert!(!created.to_string().contains("fixture-never-return-content"));
    let id = created["source"]["id"].as_str().unwrap();
    let path = format!("/api/data-sources/{id}");
    assert_eq!(created["source"]["validationContract"]["validationState"], "verified");
    let (_, listed) = host.request("GET", "/api/data-sources?conversationId=fixture-conversation", Value::Null);
    assert_eq!(listed["sources"][0]["source"]["id"], id);
    assert_eq!(listed["sources"][0]["conversationOverride"], Value::Null);
    assert!(listed["sources"][0]["resolution"].is_object());
    let (status, conversation) = host.request("POST", "/api/chat/conversations", json!({"title":"Scoped source"}));
    assert_eq!(status, 200, "{conversation}");
    let conversation_id = conversation["conversationId"].as_str().unwrap();
    let scoped_path = format!("/api/chat/conversations/{conversation_id}/data-sources/{id}");
    let list_path = format!("/api/data-sources?conversationId={conversation_id}");
    assert_eq!(host.request("PATCH", &format!("/api/chat/conversations/missing/data-sources/{id}"), json!({"enabled":true})).0, 404);
    let (_, inherited) = host.request("GET", &list_path, Value::Null);
    assert_eq!(inherited["sources"][0]["conversationOverride"], Value::Null);
    assert_eq!(inherited["sources"][0]["resolution"]["reason"], "conversation_opt_in_required");
    let (status, enabled) = host.request("PATCH", &scoped_path, json!({"enabled":true}));
    assert_eq!(status, 200, "{enabled}");
    assert_eq!(enabled["source"]["conversationOverride"], true);
    assert_eq!(enabled["source"]["resolution"]["reason"], "ready");
    let (_, persisted) = host.request("GET", &list_path, Value::Null);
    assert_eq!(persisted["sources"][0]["conversationOverride"], true);
    let (status, restored) = host.request("PATCH", &scoped_path, json!({"enabled":null}));
    assert_eq!(status, 200, "{restored}");
    assert_eq!(restored["source"]["conversationOverride"], Value::Null);
    assert_eq!(restored["source"]["resolution"]["reason"], "conversation_opt_in_required");
    for policy in ["defaultEnabled", "askWhenRelevant", "blocked", "conversationOptIn"] {
        draft["accessPolicy"] = json!(policy);
        let (status, updated) = host.request("PATCH", &path, draft.clone());
        assert_eq!(status, 200, "{updated}"); assert_eq!(updated["source"]["accessPolicy"], policy);
        if policy == "blocked" {
            let (_, attempted) = host.request("PATCH", &scoped_path, json!({"enabled":true}));
            assert_eq!(attempted["source"]["resolution"]["reason"], "blocked");
            assert_eq!(attempted["source"]["resolution"]["usable"], false);
            assert_eq!(host.request("PATCH", &scoped_path, json!({"enabled":null})).0, 200);
        }
    }
    let (status, tested) = host.request("POST", &format!("{path}/test"), json!({}));
    assert_eq!(status, 200, "{tested}"); assert_eq!(tested["tested"], true);
    for change in [json!({"accessPolicy":"allow_all"}), json!({"kind":"platformAPI"}), json!({"maximumAgeMinutes":-1})] {
        let mut invalid = draft.clone();
        for (key, value) in change.as_object().unwrap() { invalid[key] = value.clone(); }
        assert_eq!(host.request("PATCH", &path, invalid).0, 400);
    }
    let (_, after_invalid) = host.request("GET", "/api/data-sources", Value::Null);
    assert_eq!(after_invalid["sources"][0]["source"]["accessPolicy"], "conversationOptIn");
    assert_eq!(host.request("PATCH", "/api/data-sources/missing", draft.clone()).0, 400);
    assert_eq!(host.request("POST", "/api/data-sources/missing/test", json!({})).0, 400);
    assert_eq!(host.request("DELETE", "/api/data-sources/missing", Value::Null).0, 404);
    for location in [root.path().join("absent").to_string_lossy().into_owned(), "https://example.invalid/".into()] {
        let mut invalid = draft.clone(); invalid["location"] = json!(location);
        let (status, failure) = host.request("POST", "/api/data-sources", invalid);
        assert_eq!(status, 400); assert_eq!(failure["tested"], false);
    }
    // A failed validation is persisted as failed, not a successful readable source.
    let (_, after_failures) = host.request("GET", "/api/data-sources", Value::Null);
    assert!(after_failures["sources"].as_array().unwrap().iter().any(|entry| entry["source"]["validationContract"]["validationState"] == "failed"));
    let mut retry = draft.clone(); retry["id"] = json!("stable-retry-id"); retry["location"] = json!(root.path().join("missing-retry"));
    assert_eq!(host.request("POST", "/api/data-sources", retry.clone()).0, 400);
    retry["location"] = json!(file);
    assert_eq!(host.request("POST", "/api/data-sources", retry).0, 201);
    let (_, after_retry) = host.request("GET", "/api/data-sources", Value::Null);
    let retried = after_retry["sources"].as_array().unwrap().iter().filter(|entry| entry["source"]["id"] == "stable-retry-id").collect::<Vec<_>>();
    assert_eq!(retried.len(), 1);
    assert_eq!(retried[0]["source"]["validationContract"]["validationState"], "verified");
    let vault = root.path().join(".nola-host-secrets");
    std::fs::create_dir_all(&vault).unwrap();
    let private = vault.join("fixture-secret.txt");
    std::fs::write(&private, "private-fixture-must-not-leak").unwrap();
    let mut forbidden = draft.clone(); forbidden["location"] = json!(private);
    let (status, response) = host.request("POST", "/api/data-sources", forbidden.clone());
    assert_eq!(status, 400); assert_eq!(response["tested"], false);
    assert!(!response.to_string().contains("private-fixture-must-not-leak"));
    #[cfg(unix)] {
        let alias = root.path().join("innocent.txt");
        std::os::unix::fs::symlink(&private, &alias).unwrap();
        forbidden["location"] = json!(alias);
        assert_eq!(host.request("POST", "/api/data-sources", forbidden).0, 400);
    }
    let (status, deleted) = host.request("DELETE", &path, Value::Null);
    assert_eq!(status, 200); assert_eq!(deleted, json!({"deleted":true,"id":id}));
    assert_eq!(host.request("DELETE", &path, Value::Null).0, 404);
    let (_, remaining) = host.request("GET", "/api/data-sources", Value::Null);
    assert!(remaining["sources"].as_array().unwrap().iter().all(|entry| entry["source"]["id"] != id));
}
