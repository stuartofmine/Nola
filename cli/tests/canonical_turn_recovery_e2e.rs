use rusqlite::Connection;
#[path = "support/assistant_journal.rs"]
mod assistant_journal;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct RecoveryLlm {
    endpoint: String,
}

impl RecoveryLlm {
    fn from_listener(listener: TcpListener) -> Self {
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                respond(stream);
            }
        });
        Self {
            endpoint: format!("http://{address}/v1"),
        }
    }
}

fn read_request(stream: &mut TcpStream) -> (String, Value) {
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = stream.read(&mut buffer).unwrap_or(0);
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        let Some(header_end) = request.windows(4).position(|value| value == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::trim)
                    .and_then(|value| value.parse::<usize>().ok())
            })
            .unwrap_or(0);
        if request.len() >= header_end + 4 + length {
            break;
        }
    }
    let body_start = request
        .windows(4)
        .position(|value| value == b"\r\n\r\n")
        .map(|index| index + 4)
        .unwrap_or(request.len());
    let payload = serde_json::from_slice::<Value>(&request[body_start..]).unwrap_or_default();
    let first_line = String::from_utf8_lossy(&request[..body_start])
        .lines().next().unwrap_or_default().to_owned();
    (first_line, payload)
}

fn fixture_answer(request: &Value) -> Option<&'static str> {
    // Runtime observations can also have role=user. Only these exact fixture
    // inputs identify the requested answer, never the last arbitrary user frame.
    request["messages"].as_array()?.iter().rev().find_map(|message| {
        match (message["role"].as_str(), message["content"].as_str()) {
            (Some("user"), Some("first interrupted turn")) => Some("recovered first answer"),
            (Some("user"), Some("second turn after recovery")) => Some("completed second answer"),
            _ => None,
        }
    })
}

#[test]
fn recovery_fixture_ignores_runtime_observations_when_selecting_its_answer() {
    let request = json!({"messages":[
        {"role":"user","content":"first interrupted turn"},
        {"role":"assistant","content":"recovered first answer"},
        {"role":"user","content":"second turn after recovery"},
        {"role":"user","content":"Current Core runtime facts only"},
        {"role":"user","content":"Core task observations"}
    ]});
    assert_eq!(fixture_answer(&request), Some("completed second answer"));
    assert_eq!(fixture_answer(&json!({"messages":[
        {"role":"user","content":"unrecognized fixture input"}
    ]})), None);
}

fn respond(mut stream: TcpStream) {
    let (first_line, payload) = read_request(&mut stream);
    if first_line.contains("/models") {
        respond_models(&mut stream);
        return;
    }
    // The interrupted client's abandoned connection may already be empty.
    if first_line.is_empty() { return; }
    let answer = fixture_answer(&payload).expect("model request contains an accepted fixture user input");
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
    // The interrupted client's queued connection may be closed already. It
    // must not kill the fixture server before the legitimate next request.
    let _ = stream.write_all(response.as_bytes());
}

fn respond_models(stream: &mut TcpStream) {
    let body = r#"{"data":[{"id":"e2e-model","context_window":65536}]}"#;
    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(response.as_bytes());
}

fn command(database: &std::path::Path, endpoint: &str, prompt: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nola"));
    command
        .args([
            prompt,
            "--endpoint",
            endpoint,
            "--api-key",
            "e2e",
            "--model",
            "e2e-model",
            "--session",
            "recovery-e2e",
            "--no-tools",
            "--json",
            "--lang",
            "en",
        ])
        .env("NOLA_DB_PATH", database)
        .env("NOLA_CLI_INTERACTION_MODE", "never");
    command
}

fn run(database: &std::path::Path, endpoint: &str, prompt: &str) -> std::process::Output {
    command(database, endpoint, prompt).output().unwrap()
}

#[test]
fn second_process_requires_explicit_cancellation_before_admitting_new_input() {
    let root = std::env::temp_dir().join(format!(
        "nola-canonical-recovery-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("nola.db");
    let hanging_provider = TcpListener::bind("127.0.0.1:0").unwrap();
    hanging_provider.set_nonblocking(true).unwrap();
    let hanging_endpoint = format!("http://{}/v1", hanging_provider.local_addr().unwrap());
    let mut first = command(&database, &hanging_endpoint, "first interrupted turn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut physical_request = None;
    loop {
        if physical_request.is_none() {
            match hanging_provider.accept() {
                Ok((mut stream, _)) => {
                    let (first_line, payload) = read_request(&mut stream);
                    if first_line.contains("/models") {
                        respond_models(&mut stream);
                    } else {
                        assert!(first_line.starts_with("POST ") && first_line.contains("/chat/completions"));
                        assert_eq!(fixture_answer(&payload), Some("recovered first answer"));
                        // Hold the real physical inference request unanswered.
                        physical_request = Some(stream);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("fixture could not observe physical inference: {error}"),
            }
        }
        if database.exists() {
            if let Ok(connection) = Connection::open(&database) {
                let durable = connection
                    .query_row(
                        "SELECT
                            (SELECT COUNT(*) FROM messages WHERE conversation_id = 'recovery-e2e'),
                            (SELECT COUNT(*) FROM runs r
                             JOIN run_attempts a ON a.id=r.active_attempt_id AND a.run_id=r.id
                             JOIN effect_outbox o ON o.effect_id=a.effect_id AND o.run_id=r.id
                             JOIN effect_delivery_attempts d ON d.run_id=r.id AND d.effect_id=o.effect_id
                             WHERE r.conversation_id='recovery-e2e' AND r.status='running'
                               AND a.state='running' AND d.state='execution_started'
                               AND json_extract(a.record_json,'$.started_at_ms') IS NOT NULL
                               AND o.acknowledged_at_ms IS NULL)",
                        [],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .ok()
                    .is_some_and(|state| state.0 == 2 && state.1 > 0);
                if durable && physical_request.is_some() {
                    break;
                }
            }
        }
        assert!(first.try_wait().unwrap().is_none(), "first process exited before physical inference began");
        if Instant::now() >= deadline {
            let _ = first.kill();
            let _ = first.wait();
            panic!("first process never began its owned physical model attempt and delivery");
        }
        thread::sleep(Duration::from_millis(25));
    }
    first.kill().unwrap();
    let first_status = first.wait().unwrap();
    assert!(
        !first_status.success(),
        "the first process must be interrupted"
    );
    drop(physical_request);

    {
        let connection = Connection::open(&database).unwrap();
        let state: (i64, i64, String) = connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM messages WHERE conversation_id = 'recovery-e2e'),
                    (SELECT COUNT(*) FROM effect_outbox WHERE acknowledged_at_ms IS NULL),
                    (SELECT status FROM runs WHERE conversation_id = 'recovery-e2e' LIMIT 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state.0, 2);
        assert!(state.1 > 0);
        assert_eq!(state.2, "running");
    }

    // A new message is not authority to cancel the previously admitted Run.
    let (old_run, old_assistant): (String, String) = Connection::open(&database).unwrap()
        .query_row("SELECT id,assistant_message_id FROM runs WHERE conversation_id='recovery-e2e'",
            [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    let rejected = run(&database, &hanging_endpoint, "second turn after recovery");
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("previous run still owns unfinished work"));
    let connection = Connection::open(&database).unwrap();
    let unchanged: (i64,i64) = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM runs WHERE conversation_id='recovery-e2e'),
                (SELECT COUNT(*) FROM messages WHERE conversation_id='recovery-e2e')",
        [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(unchanged, (1,2));
    drop(connection);

    // Exercise the actual cold cancellation product entry, with exact original
    // Run/message ownership and no provider or tool configuration.
    let mut cancel = Command::new(env!("CARGO_BIN_EXE_nola-agent-host"))
        .args(["run", "--host-request-stdin", "--session", "recovery-e2e", "--project"])
        .arg(&root)
        .env("NOLA_DB_PATH", &database).env("NOLA_HOME", &root)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    writeln!(cancel.stdin.take().unwrap(), "{}", json!({"operation":"cancel",
        "session_id":"recovery-e2e", "expected_run_id":old_run,
        "assistant_message_id":old_assistant,"cancellation_id":"explicit-recovery-cancel"})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while cancel.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = cancel.kill(); let _ = cancel.wait();
            panic!("explicit cold cancellation did not settle");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let cancelled = cancel.wait_with_output().unwrap();
    assert!(cancelled.status.success(), "cold cancellation failed: {}",
        String::from_utf8_lossy(&cancelled.stderr));

    // The old cancellation must settle without poisoning the new Run's token.
    hanging_provider.set_nonblocking(false).unwrap();
    let llm = RecoveryLlm::from_listener(hanging_provider);
    let second = run(&database, &llm.endpoint, "second turn after recovery");
    assert!(
        second.status.success(),
        "recovery process failed:\n{}",
        String::from_utf8_lossy(&second.stderr)
    );

    let connection = Connection::open(&database).unwrap();
    let sealed_runs: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE conversation_id = 'recovery-e2e' AND status = 'sealed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sealed_runs, 2);
    let results = connection.prepare("SELECT rr.result_json FROM runs r JOIN run_results rr ON rr.run_id=r.id WHERE r.conversation_id='recovery-e2e' ORDER BY r.created_at_ms")
        .unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
        .collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(results.len(), 2);
    let old_result: nola_domain::run_result::RunResult = serde_json::from_str(&results[0]).unwrap();
    let new_result: nola_domain::run_result::RunResult = serde_json::from_str(&results[1]).unwrap();
    assert!(matches!(old_result.value, nola_domain::run_result::RunResultValue::Cancelled {
        ref cancellation_id, .. } if cancellation_id == "explicit-recovery-cancel"));
    assert!(matches!(new_result.value, nola_domain::run_result::RunResultValue::Answered { .. }));
    let message_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = 'recovery-e2e'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(message_count, 4);
    let pending: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM effect_outbox WHERE acknowledged_at_ms IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
    let part_ids = connection
        .prepare(
            "SELECT m.id,m.run_id FROM messages m
             WHERE m.conversation_id = 'recovery-e2e' AND m.role = 'assistant'
             ORDER BY m.ordinal",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let answers = part_ids.iter().map(|(message_id, run_id)| {
        assistant_journal::public_text(&database, "recovery-e2e", run_id, message_id)
    }).filter(|text| !text.is_empty()).collect::<Vec<_>>();
    assert_eq!(
        answers,
        vec!["completed second answer"]
    );
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}
