// SPDX-License-Identifier: Apache-2.0
//! Real IPC window completeness and bounded on-demand historical interpretation.
use nola_application::{ApplicationCommand, ConversationProjectionWindowV1};
use nola_domain::{
    ActivityState, HistoricalPartProjection, HistoricalProjectionStatus,
    MessagePartId, MessagePartView,
};
use nola_protocol::HistoricalProjectionCommand;
use nola_service_client::ApplicationServiceClient;
use serde_json::Value;
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(database: &Path, socket: &Path) -> (Service, ApplicationServiceClient) {
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
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return (service, client);
        }
        assert!(service.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("export service startup timed out");
}

#[test]
fn exhausted_window_has_fresh_bounded_part_queries_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("window.db");
    let socket = root.path().join("window.sock");
    drop(nola_store::NolaStore::open(&database).unwrap());
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','History',1,1,1); INSERT INTO messages VALUES('m','c',NULL,'user','completed',1,1,1)").unwrap();
    for index in 0..12 {
        let part = format!("p{index}");
        let raw = serde_json::to_string(&MessagePartView::Reasoning {
            part_id: MessagePartId(part.clone()),
            revision: 1,
            state: ActivityState::Completed,
            title: "Historical reasoning".into(),
            summary: Some("x".repeat(5000)),
        }).unwrap();
        db.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms) VALUES(?1,'m','reasoning',?2,'completed','',?3,1,1)", rusqlite::params![part,index+1,raw]).unwrap();
    }
    let (service, client) = start(&database, &socket);
    let window: ConversationProjectionWindowV1 = client
        .execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id: "c".into(),
            before_ordinal: None,
            limit: 32,
            content_byte_limit: 1024,
        })
        .unwrap();
    let parts = &window.projection.messages[0].parts;
    assert_eq!(parts.len(), 12);
    assert!(serde_json::to_vec(&window).unwrap().len() < 65536);
    let not_loaded = parts
        .iter()
        .find_map(|part| match part {
            MessagePartView::Historical { projection, .. }
                if matches!(projection.status, HistoricalProjectionStatus::NotLoaded { .. }) =>
            {
                Some(projection)
            }
            _ => None,
        })
        .unwrap();
    assert!(not_loaded.source.is_none());
    let owner = not_loaded.owner.clone();
    let query = HistoricalProjectionCommand::ReadHistoricalPart {
        owner: owner.clone(),
        expected_conversation_revision: 1,
        maximum_output_bytes: 8192,
    };
    let result: Option<HistoricalPartProjection> = client
        .execute_with_request_id("same-query", query.clone())
        .unwrap();
    assert!(matches!(
        result.as_ref().unwrap().status,
        HistoricalProjectionStatus::Supported
    ));
    assert!(serde_json::to_vec(&result).unwrap().len() <= 8192);
    assert!(client.command_receipt("same-query").unwrap().is_none());
    drop(service);
    let (service, client) = start(&database, &socket);
    let reopened: Option<HistoricalPartProjection> = client
        .execute_with_request_id("same-query", query.clone())
        .unwrap();
    assert_eq!(reopened, result);
    drop(service);
    db.execute("UPDATE conversations SET revision=2 WHERE id='c'", [])
        .unwrap();
    let (service, client) = start(&database, &socket);
    assert!(client.execute::<Value, _>(query).is_err());
    let refreshed: Option<HistoricalPartProjection> = client
        .execute_with_request_id(
            "same-query",
            HistoricalProjectionCommand::ReadHistoricalPart {
                owner,
                expected_conversation_revision: 2,
                maximum_output_bytes: 8192,
            },
        )
        .unwrap();
    assert_eq!(refreshed.unwrap().canonical_revision, 2);
    assert!(client.command_receipt("same-query").unwrap().is_none());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM command_receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(service);
}
