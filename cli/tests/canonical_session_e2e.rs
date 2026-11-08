use rusqlite::Connection;
#[path = "support/assistant_journal.rs"]
mod assistant_journal;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

struct MockLlm {
    endpoint: String,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockLlm {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let worker_stop = Arc::clone(&stop);
        let worker_requests = Arc::clone(&requests);
        let thread = thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        serve(stream, &worker_requests);
                    }
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
            requests,
            thread: Some(thread),
        }
    }
}

impl Drop for MockLlm {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            if let Err(failure) = thread.join() {
                // Preserve a worker failure during normal teardown, but do not
                // abort the process with a second panic while the primary CLI
                // assertion is already unwinding (and carries its diagnostics).
                if !std::thread::panicking() {
                    std::panic::resume_unwind(failure);
                }
            }
        }
    }
}

fn read_http_request(reader: &mut impl Read) -> std::io::Result<Option<(String, Vec<u8>)>> {
    use std::io::{Error, ErrorKind};
    let invalid = |message| Error::new(ErrorKind::InvalidData, message);
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let (header_end, content_length, first_line) = loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) if request.is_empty() => return Ok(None),
            Ok(0) => return Err(Error::new(ErrorKind::UnexpectedEof, "partial HTTP headers")),
            Ok(count) => count,
            Err(error)
                if request.is_empty()
                    && matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) =>
            {
                // A transport can open then abandon an idle pooled connection.
                // It is not a model request and must not consume a fixture turn.
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        request.extend_from_slice(&buffer[..count]);
        let Some(header_end) = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|offset| offset + 4)
        else {
            continue;
        };
        let headers = std::str::from_utf8(&request[..header_end])
            .map_err(|_| invalid("non UTF-8 HTTP headers"))?;
        let first_line = headers
            .lines()
            .next()
            .ok_or_else(|| invalid("missing request line"))?;
        let mut content_length = None;
        for line in headers.lines().skip(1).filter(|line| !line.is_empty()) {
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| invalid("invalid HTTP header"))?;
            if name.eq_ignore_ascii_case("content-length") {
                let length = value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| invalid("invalid Content-Length"))?;
                if content_length.replace(length).is_some() {
                    return Err(invalid("duplicate Content-Length"));
                }
            }
            if name.eq_ignore_ascii_case("transfer-encoding") {
                return Err(invalid("fixture expects a length-delimited JSON request"));
            }
        }
        let content_length = match content_length {
            Some(length) => length,
            None if first_line == "GET /v1/models HTTP/1.1" => 0,
            None => return Err(invalid("model request missing Content-Length")),
        };
        break (header_end, content_length, first_line.to_owned());
    };
    let expected = header_end
        .checked_add(content_length)
        .ok_or_else(|| invalid("HTTP length overflow"))?;
    while request.len() < expected {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Err(Error::new(ErrorKind::UnexpectedEof, "partial HTTP body"));
        }
        request.extend_from_slice(&buffer[..count]);
    }
    if request.len() != expected {
        return Err(invalid("unexpected bytes after HTTP body"));
    }
    Ok(Some((first_line, request[header_end..].to_vec())))
}

fn serve(mut stream: TcpStream, requests: &AtomicUsize) {
    // Accepted sockets inherit a nonblocking listener on some platforms.
    // A partial WouldBlock is not the end of a JSON request.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let Some((first_line, body)) =
        read_http_request(&mut stream).expect("fixture receives a complete HTTP request")
    else {
        return;
    };
    if first_line == "GET /v1/models HTTP/1.1" {
        let body = r#"{"data":[{"id":"e2e-model","context_window":65536}]}"#;
        write_response(&mut stream, "application/json", body);
        return;
    }
    assert_eq!(first_line, "POST /v1/chat/completions HTTP/1.1");
    let payload: Value =
        serde_json::from_slice(&body).expect("fixture receives a complete model request");
    requests.fetch_add(1, Ordering::SeqCst);
    let answer =
        fixture_answer(&payload).expect("model request contains an accepted fixture user input");
    let escaped = serde_json::to_string(answer).unwrap();
    let body = format!(
        "data: {{\"choices\":[{{\"delta\":{{\"content\":{escaped}}},\"finish_reason\":null}}]}}\n\n\
         data: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
         data: [DONE]\n\n"
    );
    write_response(&mut stream, "text/event-stream", &body);
}

#[test]
fn fixture_http_reader_ignores_only_empty_connections_and_rejects_partial_requests() {
    use std::io::Cursor;
    assert!(read_http_request(&mut Cursor::new(b"")).unwrap().is_none());
    let parsed = read_http_request(&mut Cursor::new(
        b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}",
    ))
    .unwrap()
    .unwrap();
    assert_eq!(parsed.0, "POST /v1/chat/completions HTTP/1.1");
    assert_eq!(parsed.1, b"{}");
    for partial in [
        &b"POST /v1/chat/completions HTTP/1.1\r\n"[..],
        &b"POST /v1/chat/completions HTTP/1.1\r\n\r\n{}"[..],
        &b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 3\r\n\r\n{}"[..],
        &b"POST /v1/chat/completions HTTP/1.1\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}"
            [..],
    ] {
        assert!(read_http_request(&mut Cursor::new(partial)).is_err());
    }
}

fn fixture_answer(request: &Value) -> Option<&'static str> {
    // Core also sends role=user observations after the accepted input. They
    // are not new user requests. Recognize only this fixture's exact inputs,
    // independently of HTTP/request counts or the number of model rounds.
    request
        .get("messages")?
        .as_array()?
        .iter()
        .rev()
        .find_map(
            |message| match (message["role"].as_str(), message["content"].as_str()) {
                (Some("user"), Some("first user message")) => Some("first persisted answer"),
                (Some("user"), Some("second user message")) => Some("second appended answer"),
                _ => None,
            },
        )
}

#[test]
fn fixture_answers_follow_accepted_input_not_trailing_runtime_observations() {
    let mut request = serde_json::json!({"messages":[
        {"role":"user","content":"first user message"},
        {"role":"user","content":"Current Core runtime facts only"},
        {"role":"user","content":"Core task observations"}
    ]});
    assert_eq!(fixture_answer(&request), Some("first persisted answer"));
    let messages = request["messages"].as_array_mut().unwrap();
    messages.extend([
        serde_json::json!({"role":"user","content":"second user message"}),
        serde_json::json!({"role":"user","content":"Core task observations"}),
    ]);
    assert_eq!(fixture_answer(&request), Some("second appended answer"));
    assert_eq!(
        fixture_answer(&serde_json::json!({"messages":[
            {"role":"user","content":"unrecognized fixture input"}
        ]})),
        None
    );
}

fn write_response(stream: &mut TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

fn run_turn(database: &std::path::Path, endpoint: &str, prompt: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nola"))
        .args([
            prompt,
            "--endpoint",
            endpoint,
            "--api-key",
            "e2e-key",
            "--model",
            "e2e-model",
            "--session",
            "canonical-e2e",
            "--no-tools",
            "--json",
            "--lang",
            "en",
        ])
        .env("NOLA_DB_PATH", database)
        .env("NOLA_HOME", database.parent().unwrap().join("home"))
        .env("NOLA_LONG_TERM_MEMORY_ENABLED", "0")
        .env("NOLA_CLI_INTERACTION_MODE", "never")
        .env("NOLA_CLI_NETWORK_POLICY", "deny")
        .output()
        .unwrap()
}

#[test]
fn two_real_cli_processes_commit_complete_turns_through_the_application_service() {
    let directory = std::env::temp_dir().join(format!(
        "nola-canonical-cli-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let database = directory.join("nola.db");
    let llm = MockLlm::start();

    let first = run_turn(&database, &llm.endpoint, "first user message");
    assert!(
        first.status.success(),
        "first CLI process failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let second = run_turn(&database, &llm.endpoint, "second user message");
    assert!(
        second.status.success(),
        "second CLI process failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(llm.requests.load(Ordering::SeqCst) >= 2);

    let connection = Connection::open(&database).unwrap();
    let canonical_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = 'canonical-e2e'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        canonical_count, 4,
        "two turns must append user and assistant messages"
    );
    let completed_runs: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM runs r JOIN run_results rr ON rr.run_id=r.id
             WHERE r.conversation_id = 'canonical-e2e' AND r.status = 'sealed'
               AND json_extract(rr.result_json,'$.value.kind') = 'answered'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        completed_runs, 2,
        "each real process must close its durable run"
    );
    let run_configs = connection
        .prepare(
            "SELECT p.config_json, r.invocation_config_digest
             FROM runs r JOIN run_invocation_payloads p ON p.run_id = r.id
             WHERE r.conversation_id = 'canonical-e2e'
             ORDER BY r.created_at_ms, r.id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(run_configs.len(), 2);
    for (config, digest) in &run_configs {
        let config = config
            .as_ref()
            .expect("cold recovery retains the frozen invocation configuration");
        assert_eq!(&format!("{:x}", Sha256::digest(config.as_bytes())), digest);
        let _: nola_core::agent::AgentCoreConfig = serde_json::from_str(config).unwrap();
    }
    let unfinished_assistants: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = 'canonical-e2e' AND role = 'assistant' AND status != 'completed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        unfinished_assistants, 0,
        "no queued assistant may survive completion"
    );
    let pending_effects: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM effect_outbox o JOIN runs r ON r.id = o.run_id WHERE r.conversation_id = 'canonical-e2e' AND o.acknowledged_at_ms IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pending_effects, 0,
        "successful Host effects must be acknowledged"
    );

    let mut statement = connection
        .prepare(
            "SELECT m.role, m.id, m.run_id, p.content_text
             FROM messages m
             JOIN message_parts p ON p.message_id = m.id
             WHERE m.conversation_id = 'canonical-e2e'
             ORDER BY m.ordinal, p.ordinal",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let contents = rows
        .iter()
        .map(|row| {
            if row.0 == "assistant" {
                assert!(
                    row.3.is_empty(),
                    "assistant bodies must not be duplicated into cumulative SQL text"
                );
                assistant_journal::public_text(&database, "canonical-e2e", &row.2, &row.1)
            } else {
                row.3.clone()
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        contents,
        vec![
            "first user message",
            "first persisted answer",
            "second user message",
            "second appended answer"
        ]
    );
    assert_eq!(rows.iter().filter(|row| row.0 == "assistant").count(), 2);

    let canonical_checkpoint: String = connection
        .query_row(
            "SELECT checkpoint_json FROM core_checkpoint_bodies WHERE conversation_id = 'canonical-e2e'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let checkpoint_value = serde_json::from_str::<Value>(&canonical_checkpoint).unwrap();
    assert!(checkpoint_value.get("state").is_some());
    assert!(
        checkpoint_value.get("core_config").is_none()
            && checkpoint_value.get("invocation_config").is_none(),
        "checkpoint must contain Core state only; Run owns the immutable invocation config"
    );
    let retired_checkpoint = directory
        .join("cli")
        .join("sessions")
        .join("canonical-e2e.agent-v2.checkpoint.json");
    assert!(
        !retired_checkpoint.exists(),
        "production CLI must not recreate the retired checkpoint file"
    );
    drop(statement);
    drop(connection);
    std::fs::remove_dir_all(directory).unwrap();
}
