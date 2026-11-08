// SPDX-License-Identifier: Apache-2.0
//! Real IPC checks for fresh, non-persistent projection reads.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use nola_protocol::{
    CommandEnvelope, CommandPayload, CommandReceipt, CommandReceiptOutcome,
    APPLICATION_SERVICE_PROTOCOL_VERSION,
};
use nola_service_client::ApplicationServiceClient;
use nola_store::{CreateConversationInput, NolaStore};
use serde_json::{json, Value};

struct Service(Child);

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(database: &Path, socket: &Path) -> Service {
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(database)
            .arg("--socket")
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let client = ApplicationServiceClient::with_socket(database, socket);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return service;
        }
        assert!(
            service.0.try_wait().unwrap().is_none(),
            "service exited at startup"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("service startup timed out");
}

fn request(
    socket: &Path,
    id: &str,
    kind: &str,
    arguments: Value,
    expected_revision: Option<u64>,
) -> CommandReceiptOutcome {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    serde_json::to_writer(
        &mut stream,
        &CommandEnvelope {
            protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
            client_id: "projection-query-e2e".into(),
            request_id: id.into(),
            expected_revision,
            command: CommandPayload {
                command_type: kind.into(),
                arguments,
            },
        },
    )
    .unwrap();
    stream.write_all(b"\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let receipt: CommandReceipt = serde_json::from_str(&line).unwrap();
    receipt.validate().unwrap();
    assert_eq!(receipt.request_id, id);
    assert_eq!(receipt.client_id, "projection-query-e2e");
    receipt.outcome
}

fn committed(outcome: CommandReceiptOutcome) -> Value {
    match outcome {
        CommandReceiptOutcome::Committed { result, .. } => result,
        other => panic!("unexpected response: {other:?}"),
    }
}

fn window_arguments() -> Value {
    json!({"conversation_id":"chat", "before_ordinal":null,
        "limit":64, "content_byte_limit":512000})
}

fn receipt_count(database: &rusqlite::Connection) -> i64 {
    database
        .query_row("SELECT count(*) FROM command_receipts", [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn data_version(database: &rusqlite::Connection) -> i64 {
    database
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn reasoning_resolution_is_a_pure_query_across_real_ipc() {
    let directory = tempfile::tempdir_in("/tmp").unwrap();
    let path = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    drop(NolaStore::open(&path).unwrap());
    let _service = start(&path, &socket);
    let observer = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let before = (receipt_count(&observer), data_version(&observer));
    let arguments = json!({
        "declaration":{"control":"effort","efforts":["low","high"]},
        "protocol":"responses","output_limit":16384,"supports_reasoning":true,
        "requested":null,"tier":"balanced"
    });
    for _ in 0..3 {
        assert_eq!(committed(request(&socket, "reasoning-query", "resolve_model_reasoning", arguments.clone(), None)),
            json!({"selection":{"kind":"effort","value":"low"}}));
    }
    assert!(matches!(request(&socket, "reasoning-revision", "resolve_model_reasoning", arguments.clone(), Some(1)),
        CommandReceiptOutcome::Rejected { .. }));
    let mut invalid = arguments.clone();
    invalid["supports_reasoning"] = json!(false);
    assert!(matches!(request(&socket, "reasoning-rejected", "resolve_model_reasoning", invalid, None),
        CommandReceiptOutcome::Rejected { .. }));
    let mut budget = arguments;
    budget["declaration"] = json!({"control":"budget","budgets":[1024,4096,8192]});
    budget["protocol"] = json!("anthropic");
    budget["output_limit"] = json!(4096);
    budget["tier"] = json!("concise");
    assert_eq!(committed(request(&socket, "reasoning-budget", "resolve_model_reasoning", budget, None)),
        json!({"selection":{"kind":"budget","tokens":1024}}));
    assert_eq!((receipt_count(&observer), data_version(&observer)), before);
}

#[test]
fn idle_window_and_inspector_queries_do_not_write_receipts_or_database_pages() {
    let directory = tempfile::tempdir_in("/tmp").unwrap();
    let path = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "chat".into(),
            title: "Initial".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    drop(store);
    let service = start(&path, &socket);
    let observer =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let initial_receipts = receipt_count(&observer);
    let initial_version = data_version(&observer);
    let window = committed(request(
        &socket,
        "window",
        "load_conversation_projection_window_v1",
        window_arguments(),
        None,
    ));
    for index in 0..128 {
        let mut args = window_arguments();
        args["after_sequence"] = window["event_sequence"].clone();
        assert!(committed(request(
            &socket,
            &format!("idle-{index}"),
            "load_conversation_projection_window_update_v1",
            args,
            None
        ))
        .is_null());
    }
    assert!(committed(request(
        &socket,
        "permission",
        "load_conversation_permission_projection_v1",
        json!({"conversation_id":"chat","run_id":"permission-read-run","scope":{"type":"current_run"}}),
        None
    ))
    .is_null());
    let usage = committed(request(
        &socket,
        "usage",
        "load_conversation_context_usage_v1",
        json!({"conversation_id":"chat"}),
        None,
    ));
    assert_eq!(usage, window["projection"]["context_usage"]);
    assert_eq!(usage["used_tokens"], 0);
    assert!(matches!(
        request(
            &socket,
            "bad-revision",
            "load_conversation_projection_window_v1",
            window_arguments(),
            Some(1)
        ),
        CommandReceiptOutcome::Rejected { .. }
    ));
    let mut invalid = window_arguments();
    invalid["limit"] = json!(0);
    assert!(matches!(
        request(
            &socket,
            "invalid-window",
            "load_conversation_projection_window_v1",
            invalid,
            None
        ),
        CommandReceiptOutcome::Rejected { .. }
    ));
    assert_eq!(
        receipt_count(&observer),
        initial_receipts,
        "queries persisted receipts"
    );
    assert_eq!(
        data_version(&observer),
        initial_version,
        "queries committed database writes"
    );
    drop(service);
}

#[test]
fn query_ids_remain_fresh_while_mutations_replay_across_service_restart() {
    let directory = tempfile::tempdir_in("/tmp").unwrap();
    let path = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "chat".into(),
            title: "Initial".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    drop(store);
    let service = start(&path, &socket);
    let query = || {
        committed(request(
            &socket,
            "same-query-id",
            "load_conversation_projection_window_v1",
            window_arguments(),
            None,
        ))
    };
    let before = query();
    let rename = json!({"conversation_id":"chat", "title":"Renamed", "updated_at_ms":2});
    let mutation = committed(request(
        &socket,
        "rename",
        "rename_conversation",
        rename.clone(),
        None,
    ));
    let after = query();
    assert_ne!(after, before, "query replayed stale content");
    assert!(after["event_sequence"].as_u64().unwrap() > before["event_sequence"].as_u64().unwrap());
    drop(service);
    let _ = std::fs::remove_file(&socket);
    let restarted = start(&path, &socket);
    assert_eq!(query(), after);
    assert_eq!(
        committed(request(
            &socket,
            "rename",
            "rename_conversation",
            rename,
            None
        )),
        mutation
    );
    assert_eq!(
        query(),
        after,
        "replayed mutation emitted another projection change"
    );
    assert!(committed(request(
        &socket,
        "lookup-query",
        "command_receipt",
        json!({"request_id":"same-query-id"}),
        None
    ))
    .is_null());
    assert!(!committed(request(
        &socket,
        "lookup-mutation",
        "command_receipt",
        json!({"request_id":"rename"}),
        None
    ))
    .is_null());
    let observer =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(receipt_count(&observer), 1);
    drop(restarted);
}

#[test]
fn committed_mutation_wakes_conversation_and_list_waiters_without_blocking_writes() {
    let directory = tempfile::tempdir_in("/tmp").unwrap();
    let path = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "chat".into(),
            title: "Initial".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    drop(store);
    let _service = start(&path, &socket);
    let (finished, responses) = std::sync::mpsc::channel();
    let mut waiters = Vec::new();
    for (index, scope) in [
        json!({"type":"conversation", "conversation_id":"chat"}),
        json!({"type":"conversations"}),
    ]
    .into_iter()
    .enumerate()
    {
        let socket = socket.clone();
        let finished = finished.clone();
        waiters.push(std::thread::spawn(move || {
            let response = committed(request(
                &socket,
                &format!("wait-{index}"),
                "projection_event_suffix",
                json!({"scope":scope, "after":{"scope":scope,"sequence":0},
                    "limit":32, "wait_ms":5000}),
                None,
            ));
            finished.send(response).unwrap();
        }));
    }
    drop(finished);
    assert!(
        matches!(
            responses.recv_timeout(Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "wait returned before any commit"
    );
    let started = Instant::now();
    committed(request(
        &socket,
        "rename-wake",
        "rename_conversation",
        json!({"conversation_id":"chat", "title":"Wake readers", "updated_at_ms":2}),
        None,
    ));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "waiting reader blocked the writer"
    );
    for _ in 0..2 {
        let response = responses
            .recv_timeout(Duration::from_secs(2))
            .expect("commit must wake every matching waiter before its five-second deadline");
        let suffix: nola_protocol::EventSuffix = serde_json::from_value(response).unwrap();
        suffix.validate().unwrap();
        assert!(!suffix.events.is_empty());
        assert!(suffix.next_cursor.sequence > 0);
    }
    for waiter in waiters {
        waiter.join().unwrap();
    }
}

#[test]
fn cursor_wait_returns_only_a_fresh_wake_hint_and_rejects_future_cursors() {
    let directory = tempfile::tempdir_in("/tmp").unwrap();
    let path = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "chat".into(),
            title: "Initial".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    drop(store);
    let _service = start(&path, &socket);
    let scope = json!({"type":"conversation", "conversation_id":"chat"});
    let initial = committed(request(
        &socket,
        "initial-cursor",
        "projection_cursor_wait",
        json!({"after":{"scope":scope,"sequence":0},"wait_ms":25}),
        None,
    ));
    assert_eq!(initial, json!({"scope":scope,"sequence":0}));
    let (finished, response) = std::sync::mpsc::channel();
    let waiter_socket = socket.clone();
    let waiter_scope = scope.clone();
    let waiter = std::thread::spawn(move || {
        finished
            .send(committed(request(
                &waiter_socket,
                "cursor-wait",
                "projection_cursor_wait",
                json!({"after":{"scope":waiter_scope,"sequence":0},"wait_ms":5000}),
                None,
            )))
            .unwrap();
    });
    assert!(matches!(
        response.recv_timeout(Duration::from_millis(100)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    committed(request(
        &socket,
        "rename-cursor",
        "rename_conversation",
        json!({"conversation_id":"chat","title":"Changed","updated_at_ms":2}),
        None,
    ));
    let hint = response
        .recv_timeout(Duration::from_secs(2))
        .expect("cursor waiter must wake on commit");
    waiter.join().unwrap();
    let cursor: nola_protocol::ProjectionCursor = serde_json::from_value(hint.clone()).unwrap();
    cursor.validate().unwrap();
    assert!(cursor.sequence > 0);
    assert!(hint.to_string().len() < 256);
    let window = committed(request(
        &socket,
        "read-after-hint",
        "load_conversation_projection_window_v1",
        window_arguments(),
        None,
    ));
    assert_eq!(window["event_sequence"], cursor.sequence);
    let again = committed(request(
        &socket,
        "cursor-wait",
        "projection_cursor_wait",
        json!({"after":hint,"wait_ms":0}),
        None,
    ));
    assert_eq!(again, hint);
    for args in [
        json!({"after":{"scope":scope,"sequence":cursor.sequence + 1},"wait_ms":0}),
        json!({"after":hint,"wait_ms":30001}),
    ] {
        assert!(matches!(
            request(
                &socket,
                "invalid-cursor",
                "projection_cursor_wait",
                args,
                None
            ),
            CommandReceiptOutcome::Rejected { .. }
        ));
    }
    let observer =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(
        receipt_count(&observer),
        1,
        "wake hints must not create durable receipts"
    );
}

#[test]
fn disconnected_clients_release_server_waiters_without_waiting_for_deadlines() {
    let directory = tempfile::tempdir_in("/tmp").unwrap();
    let path = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "chat".into(),
            title: "Initial".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    drop(store);
    let _service = start(&path, &socket);
    let scope = json!({"type":"conversation","conversation_id":"chat"});
    let mut clients = Vec::new();
    for index in 0..8 {
        let mut client = UnixStream::connect(&socket).unwrap();
        let (kind, arguments) = if index % 2 == 0 {
            (
                "projection_cursor_wait",
                json!({"after":{"scope":scope,"sequence":0},"wait_ms":30000}),
            )
        } else {
            (
                "projection_event_suffix",
                json!({"scope":scope,
                "after":{"scope":scope,"sequence":0},"wait_ms":30000,"limit":32}),
            )
        };
        serde_json::to_writer(
            &mut client,
            &CommandEnvelope {
                protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
                client_id: "disconnect-e2e".into(),
                request_id: format!("wait-{index}"),
                expected_revision: None,
                command: CommandPayload {
                    command_type: kind.into(),
                    arguments,
                },
            },
        )
        .unwrap();
        client.write_all(b"\n").unwrap();
        clients.push(client);
    }
    let wait_for_count = |expected: u64| {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let health = committed(request(
                &socket,
                "health-waiters",
                "health",
                json!({}),
                None,
            ));
            if health["projection_waiters"] == expected {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "expected {expected} server waiters, observed {health}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    wait_for_count(8);
    drop(clients);
    wait_for_count(0);
    let observer =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    assert_eq!(receipt_count(&observer), 0);
    committed(request(
        &socket,
        "rename-after-disconnect",
        "rename_conversation",
        json!({"conversation_id":"chat","title":"Still running","updated_at_ms":2}),
        None,
    ));
}
