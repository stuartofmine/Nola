// SPDX-License-Identifier: Apache-2.0
//! Explicit CLI and real Web detail actions share a bounded Service query.
#![cfg(unix)]
use serde_json::{json, Value};
use std::{
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start_service(database: &std::path::Path) -> Process {
    let socket = nola_service_client::socket_path_for_database(database);
    let mut process = Process(
        Command::new(env!("CARGO_BIN_EXE_nola"))
            .arg("--nola-application-service")
            .arg("--database")
            .arg(database)
            .arg("--socket")
            .arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let client = nola_service_client::ApplicationServiceClient::with_socket(database, &socket);
    let deadline = Instant::now() + Duration::from_secs(15);
    while client.health().is_err() {
        assert!(process.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline, "service startup timed out");
        thread::sleep(Duration::from_millis(20));
    }
    process
}
#[test]
fn cli_and_http_load_a_not_loaded_part_without_receipts_or_projection_mutation() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("history.db");
    drop(nola_store::NolaStore::open(&database).unwrap());
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','History detail fixture',1,1,1);
        INSERT INTO messages VALUES('m','c','history-run','assistant','completed',1,1,1);
        INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms,completed_at_ms)
            VALUES('history-run','c','conversation','m','completed',1,1,1,1);",
        )
        .unwrap();
    for index in 0..64 {
        let id = format!("p{index}");
        let raw =
            json!({"kind":"tool","part_id":id,"revision":1,"tool_call_id":format!("call{index}"),
            "capability_id":"file.read","state":"completed","display_key":null,
            "title":"Read historical data","summary":null,"origin":{"kind":"builtin"},"permission":null,
            "input":{"path":"x".repeat(1024)},
            "output":{"data":"<script>plain historical data</script>"},"output_preview":null,"full_output":null,
            "artifacts":[],"started_at_ms":1,"finished_at_ms":1,"affected_resources":[]})
            .to_string();
        connection.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
            VALUES(?1,'m','tool',?2,'completed','',?3,1,1)",rusqlite::params![id,index+1,raw]).unwrap();
    }
    let original: String = connection
        .query_row(
            "SELECT content_json FROM message_parts WHERE id='p63'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(connection);
    let service = start_service(&database);
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let web_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("web");
    let mut web = Process(
        Command::new(env!("CARGO_BIN_EXE_nola"))
            .args([
                "web",
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--web-root",
                web_root.to_str().unwrap(),
            ])
            .env("NOLA_HOME", root.path())
            .env("NOLA_DB_PATH", &database)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(15);
    while http.get(&base).send().is_err() {
        assert!(web.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline, "Web startup timed out");
        thread::sleep(Duration::from_millis(20));
    }
    let token = root.path().join(".nola/web/mobile-access-token");
    let cookie = if token.exists() {
        let login = http
            .post(format!("{base}/api/web/login"))
            .json(&json!({"token":std::fs::read_to_string(token).unwrap().trim()}))
            .send()
            .unwrap();
        assert!(login.status().is_success());
        login
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .map(|value| {
                value
                    .to_str()
                    .unwrap()
                    .split(';')
                    .next()
                    .unwrap()
                    .to_owned()
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    let get = |path: &str| {
        http.get(format!("{base}{path}"))
            .header(reqwest::header::COOKIE, &cookie)
            .send()
            .unwrap()
    };
    let response = get("/api/chat/conversations/c/messages");
    assert!(
        response.status().is_success(),
        "{}",
        response.text().unwrap()
    );
    let window: Value = response.json().unwrap();
    let parts = &window["conversationProjection"]["messages"][0]["parts"];
    assert_eq!(parts.as_array().unwrap().len(), 64);
    assert!(parts
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part["projection"]["status"]["state"] == "not_loaded"));
    let page = get("/chat");
    assert!(page.status().is_success(), "Build web/frontend before Web acceptance");
    let page = page.text().unwrap();
    assert!(page.contains("<div id=\"app\"></div>"));
    assert!(page.contains("type=\"module\""));
    assert!(!page.contains("{{") && !page.contains("{%"));
    assert!(!page.contains("historical-part-detail.js"));
    let entry = page.split("src=\"").skip(1).filter_map(|tail| tail.split('"').next())
        .find(|path| path.starts_with("/ui/assets/") && path.ends_with(".js"))
        .expect("actual compiled Svelte workspace entry");
    let module = get(entry);
    assert!(module.status().is_success());
    let content_type = module.headers().get(reqwest::header::CONTENT_TYPE).unwrap().to_str().unwrap();
    assert!(content_type.contains("javascript"), "{content_type}");
    let module = module.text().unwrap();
    assert!(!module.is_empty());
    // Check the actually served module, not an empty set of retired inline scripts.
    let script = root.path().join("served-workspace.mjs");
    std::fs::write(&script, module).unwrap();
    assert!(Command::new("node")
        .arg("--check")
        .arg(&script)
        .status()
        .unwrap()
        .success());
    let connection = rusqlite::Connection::open(&database).unwrap();
    let count = || {
        connection
            .query_row("SELECT COUNT(*) FROM command_receipts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    let before = count();
    let detail =
        get("/api/chat/conversations/c/historical-part?message_id=m&part_id=p63&revision=1");
    assert!(detail.status().is_success());
    let text = detail.text().unwrap();
    assert!(text.len() <= 32768);
    let expected: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(expected["status"]["state"], "supported");
    assert_eq!(expected["owner"]["conversation_id"], "c");
    assert_eq!(expected["owner"]["message_id"], "m");
    assert_eq!(expected["owner"]["part_id"], "p63");
    assert_eq!(expected["binding"], "canonical");
    assert_eq!(expected["fields"]["input"]["value"]["path"], "x".repeat(1024));
    assert_eq!(expected["fields"]["output"]["value"]["data"], "<script>plain historical data</script>");
    assert_eq!(
        get("/api/chat/conversations/c/historical-part?message_id=m&part_id=p63&revision=2")
            .status()
            .as_u16(),
        409
    );
    assert_eq!(
        get("/api/chat/conversations/c/historical-part?message_id=foreign&part_id=p63&revision=1")
            .status()
            .as_u16(),
        409
    );
    let command = |revision: &str| {
        Command::new(env!("CARGO_BIN_EXE_nola"))
            .args(["session", "part", "c", "m", "p63", revision])
            .env("NOLA_HOME", root.path())
            .env("NOLA_DB_PATH", &database)
            .output()
            .unwrap()
    };
    let output = command("1");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        expected
    );
    assert!(!command("2").status.success());
    let after: Value = get("/api/chat/conversations/c/messages").json().unwrap();
    assert_eq!(after["messages"], window["messages"]);
    assert_eq!(count(), before);
    assert_eq!(
        connection
            .query_row(
                "SELECT content_json FROM message_parts WHERE id='p63'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        original
    );
    drop(web);
    drop(service);
    let _service = start_service(&database);
    let reopened = command("1");
    assert!(reopened.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&reopened.stdout).unwrap(),
        expected
    );
}
