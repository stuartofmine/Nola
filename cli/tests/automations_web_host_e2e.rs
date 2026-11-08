use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Host(Child, u16);
impl Drop for Host { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
impl Host {
    fn start(root: &std::path::Path) -> Self {
        std::fs::create_dir_all(root.join("app/static/ui")).unwrap();
        std::fs::write(root.join("app/static/ui/index.html"), "<!doctype html><html>automation-svelte-fixture</html>").unwrap();
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut host = Self(Command::new(env!("CARGO_BIN_EXE_nola"))
            .args(["web", "--host", "127.0.0.1", "--port", &port.to_string(), "--web-root", root.to_str().unwrap()])
            .env("NOLA_HOME", root).env("NOLA_DB_PATH", root.join("fixture.sqlite")).current_dir(root)
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap(), port);
        for _ in 0..1000 {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() { return host; }
            assert!(host.0.try_wait().unwrap().is_none());
            std::thread::sleep(Duration::from_millis(30));
        }
        panic!("host did not start");
    }
    fn request(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.1)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        let body = body.to_string();
        write!(stream, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", self.1, body.len()).unwrap();
        let mut response = String::new(); stream.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let body = if headers.to_ascii_lowercase().contains("transfer-encoding: chunked") {
            let mut rest = body; let mut result = String::new();
            loop { let (size, next) = rest.split_once("\r\n").unwrap(); let size = usize::from_str_radix(size,16).unwrap(); if size == 0 { break; } result.push_str(&next[..size]); rest = &next[size+2..]; }
            result
        } else { body.into() };
        (headers.split_whitespace().nth(1).unwrap().parse().unwrap(), serde_json::from_str(&body).unwrap())
    }
}

#[test]
fn source_automation_contract_is_explicit_and_approval_does_not_run_script() {
    let root = tempfile::tempdir().unwrap();
    let host = Host::start(root.path());
    for method in ["GET", "HEAD"] {
        let mut stream = TcpStream::connect(("127.0.0.1", host.1)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        write!(stream, "{method} /scheduled-actions HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n", host.1).unwrap();
        let mut response = String::new(); stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        let (_, body) = response.split_once("\r\n\r\n").unwrap();
        if method == "GET" { assert!(body.contains("automation-svelte-fixture")); } else { assert!(body.is_empty()); }
    }
    let (_, list) = host.request("GET", "/api/scheduled-actions", Value::Null);
    assert_eq!(list["executionCapabilities"]["scheduledDispatch"], false);
    assert_eq!(list["executionCapabilities"]["promptAutomations"], true);
    assert_eq!(list["actions"], json!([]));
    // A harmless fixture script that is NEVER executed by this test.
    let spec = json!({"kind":"agent","spec":{"prompt":"Inspect the isolated fixture once",
        "execution_scope":{"kind":"local_workspace","root":root.path()},"permission_profile":"cautious",
        "model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}});
    let draft = json!({"name":"Fixture script","format":"script","cwd":root.path(),"script":"printf should-not-run","frequency":"weekly","weekday":2,"time":"09:15","timezone":"local","executionSpec":spec});
    for change in [json!({"time":"25:00"}), json!({"time":"9:00"}), json!({"time":"12:99"}), json!({"weekday":0}), json!({"weekday":8}), json!({"weekday":null}), json!({"timezone":"UTC"}), json!({"frequency":"hourly"}), json!({"format":"nola.mac.automation.v1"})] {
        let mut invalid = draft.clone();
        for (key, value) in change.as_object().unwrap() { invalid[key] = value.clone(); }
        assert_eq!(host.request("POST", "/api/scheduled-actions", invalid).0, 400);
    }
    let (status, created) = host.request("POST", "/api/scheduled-actions", draft.clone());
    assert_eq!(status, 200, "{created}");
    let id = created["action"]["id"].as_str().unwrap();
    let path = format!("/api/scheduled-actions/{id}");
    assert_eq!(created["action"]["enabled"], false);
    assert_eq!(created["action"]["scriptApproved"], false);
    assert_eq!(created["action"]["timezone"], "local");
    assert_eq!(host.request("PATCH", &path, json!({"enabled":true,"expectedRevision":created["action"]["revision"]})).0, 409);
    assert_eq!(host.request("PATCH", &path, json!({"enabled":false,"time":"10:00"})).0, 400);
    assert_eq!(host.request("POST", &format!("{path}/run-now"), json!({})).0, 400);
    let (status, _) = host.request("POST", &format!("{path}/run-now"), json!({"occurrenceKey":"disabled-script","expectedRevision":created["action"]["revision"]}));
    assert_eq!(status, 409);
    assert_eq!(host.request("POST", &format!("{path}/approve-script"), json!({"confirmSafety":false})).0, 400);
    assert_eq!(host.request("POST", &format!("{path}/approve-script"), json!({"confirmSafety":true})).0, 400);
    let reviewed_revision = created["action"]["revision"].as_i64().unwrap();
    let mut edited = draft.clone();
    edited["time"] = json!("10:30");
    let (_, updated) = host.request("PATCH", &path, json!({"definition":edited,"expectedRevision":reviewed_revision}));
    // Even an intervening update to the same reviewed definition invalidates stale approval.
    let current_revision = updated["action"]["revision"].as_i64().unwrap();
    assert!(current_revision > reviewed_revision);
    let (status, stale) = host.request("POST", &format!("{path}/approve-script"), json!({"confirmSafety":true,"expectedRevision":reviewed_revision}));
    assert_eq!(status, 409); assert_eq!(stale["code"], "automation_revision_conflict");
    assert_eq!(host.request("PATCH", &path, json!({"enabled":false,"expectedRevision":reviewed_revision})).0, 409);
    let (status, approved) = host.request("POST", &format!("{path}/approve-script"), json!({"confirmSafety":true,"expectedRevision":current_revision}));
    assert_eq!(status, 200); assert_eq!(approved["action"]["scriptApproved"], true);
    assert_eq!(host.request("POST", &format!("{path}/approve-script"), json!({"confirmSafety":true,"expectedRevision":current_revision})).0, 409);
    assert_eq!(host.request("PATCH", &path, json!({"enabled":false,"expectedRevision":approved["action"]["revision"]})).0, 200);
    let (_, results) = host.request("GET", "/api/results", Value::Null);
    assert!(results.to_string().find("should-not-run").is_none());
    assert_eq!(host.request("POST", "/api/scheduled-actions/missing/run-now", json!({"occurrenceKey":"missing","expectedRevision":1})).0, 404);
    assert_eq!(host.request("PATCH", "/api/scheduled-actions/missing", json!({"enabled":false,"expectedRevision":1})).0, 404);
    assert_eq!(host.request("DELETE", &path, Value::Null).0, 200);
    assert_eq!(host.request("DELETE", &path, Value::Null).0, 404);
    assert_eq!(host.request("GET", "/api/scheduled-actions", Value::Null).1["actions"], json!([]));
    // An unavailable catalog identity fails admission before any model or script.
    let fixture = root.path().join("source.txt");
    std::fs::write(&fixture, "isolated automation fixture").unwrap();
    assert_eq!(host.request("POST", "/api/agent/settings", json!({"enable_local_dev_tools":true,"allowed_dirs":[root.path()]})).0, 200);
    let mut unavailable = spec.clone();
    unavailable["spec"]["skill_refs"] = json!(["fixture-skill-that-does-not-exist"]);
    let (_, source) = host.request("POST", "/api/scheduled-actions", json!({"name":"Fixture file","format":"file","path":fixture,"frequency":"daily","time":"23:59","executionSpec":unavailable}));
    let source_id = source["action"]["id"].as_str().unwrap();
    let (status, run) = host.request("POST", &format!("/api/scheduled-actions/{source_id}/run-now"), json!({"occurrenceKey":"unavailable-skill","expectedRevision":source["action"]["revision"]}));
    assert_eq!(status, 409); assert_eq!(run["code"], "automation_skill_unavailable", "{run}");
    let (_, definitions) = host.request("GET", "/api/scheduled-actions", Value::Null);
    let definition = definitions["actions"].as_array().unwrap().iter().find(|action| action["id"] == source_id).unwrap();
    assert_eq!(definition["revision"], source["action"]["revision"]);
    assert_eq!(definition["updatedAt"], source["action"]["updatedAt"]);
    let (_, results) = host.request("GET", "/api/results", Value::Null);
    assert!(!results["results"].as_array().unwrap().iter().any(|result| result["sourceId"] == source_id));
}
