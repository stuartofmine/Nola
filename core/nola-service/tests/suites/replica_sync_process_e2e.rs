// SPDX-License-Identifier: Apache-2.0
//! Two isolated Service processes exercise the public replica protocol.
use nola_application::{ApplicationCommand, CreateConversation};
use nola_protocol::{
    ConversationTranscriptImportSource, ImportConversationTranscript,
    PendingSyncChange, RemoteSyncChange, RemoteSyncPage, RemoteSyncPageResult, ReplicaSyncCommand,
    SyncCursor, TranscriptCommand, TranscriptMessageInput, TranscriptPartInput,
};
use nola_service_client::ApplicationServiceClient;
use serde_json::Value;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
        assert!(
            service.0.try_wait().unwrap().is_none(),
            "service exited during startup"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("isolated replica service startup timed out");
}
fn append(client: &ApplicationServiceClient, ordinal: i64, text: &str) {
    let _: Value = client
        .execute(TranscriptCommand::ImportConversationTranscript {
            input: ImportConversationTranscript {
                import_id: format!("import-{ordinal}"),
                source: ConversationTranscriptImportSource::ExternalArchive,
                conversation_id: "c".into(),
                expected_message_count: ordinal as usize - 1,
                imported_at_ms: ordinal + 1,
                messages: vec![TranscriptMessageInput {
                    id: format!("m{ordinal}"),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal,
                    created_at_ms: 1,
                    completed_at_ms: Some(1),
                    parts: vec![TranscriptPartInput {
                        id: format!("p{ordinal}"),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: text.into(),
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    }],
                }],
            },
        })
        .unwrap();
}
fn profile(client: &ApplicationServiceClient, request: &str, expected_revision: u64, title: &str) {
    let changed: Value = client
        .execute_with_revision(
            request,
            expected_revision,
            ApplicationCommand::RenameConversation {
                title_source: None,
                conversation_id: "c".into(),
                title: title.into(),
                updated_at_ms: expected_revision as i64 + 1,
            },
        )
        .unwrap();
    assert_eq!(changed, true);
}
fn remote_page(changes: Vec<RemoteSyncChange>) -> ReplicaSyncCommand {
    ReplicaSyncCommand::ApplyRemoteSyncPage {
        input: RemoteSyncPage {
            replica_id: "cloud".into(),
            next_cursor: changes.last().unwrap().cursor,
            changes,
            applied_at_ms: 20,
        },
    }
}
fn window(client: &ApplicationServiceClient) -> Value {
    client
        .execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id: "c".into(),
            before_ordinal: None,
            limit: 10,
            content_byte_limit: 65_536,
        })
        .unwrap()
}

#[test]
fn two_services_restore_incremental_history_metadata_and_acknowledgements_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let sender_db = root.path().join("sender.db");
    let sender_socket = root.path().join("sender.sock");
    let receiver_db = root.path().join("receiver.db");
    let receiver_socket = root.path().join("receiver.sock");
    let (sender_service, sender) = start(&sender_db, &sender_socket);
    let (receiver_service, receiver) = start(&receiver_db, &receiver_socket);
    let _: Value = sender
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "c".into(),
                title: "Original".into(),
                settings_json: Some(r#"{"memoryEnabledOverride":true}"#.into()),
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    append(&sender, 1, "First original evidence");
    profile(&sender, "metadata-3", 2, "First title");
    append(&sender, 2, "Second original evidence");
    profile(&sender, "metadata-5", 4, "Final title");
    let _: Value = sender
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "other".into(),
                title: "Another conversation".into(),
                settings_json: None,
                created_at_ms: 6,
                updated_at_ms: 6,
            },
        })
        .unwrap();
    let pending: Vec<PendingSyncChange> = sender
        .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 20 })
        .unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|entry| entry.entity_type.as_str())
            .collect::<Vec<_>>(),
        [
            "conversation",
            "conversation_append",
            "conversation_metadata",
            "conversation_append",
            "conversation_metadata",
            "conversation"
        ]
    );
    assert!(!pending[3].payload_json.contains("First original evidence"));
    let changes: Vec<_> = pending
        .iter()
        .enumerate()
        .map(|(index, entry)| RemoteSyncChange {
            cursor: index as i64 + 1,
            operation_id: entry.operation_id.clone(),
            entity_type: entry.entity_type.clone(),
            entity_id: entry.entity_id.clone(),
            revision: entry.revision,
            deleted: entry.deleted,
            payload_json: entry.payload_json.clone(),
            created_at_ms: entry.created_at_ms,
        })
        .collect();

    // Missing dependencies must reject the page without acknowledging its cursor.
    let rejected: Result<RemoteSyncPageResult, _> =
        receiver.execute(remote_page(vec![changes[3].clone()]));
    assert!(rejected.is_err());
    let cursor: Option<SyncCursor> = receiver
        .execute(ReplicaSyncCommand::GetSyncCursor {
            replica_id: "cloud".into(),
        })
        .unwrap();
    assert!(cursor.is_none());
    let outcome: RemoteSyncPageResult = receiver.execute(remote_page(changes.clone())).unwrap();
    assert_eq!(outcome.applied, 6);
    assert_eq!(outcome.cursor, 6);
    let restored = window(&receiver);
    assert_eq!(restored["projection"]["title"], "Final title");
    assert_eq!(
        restored["event_sequence"], 1,
        "synchronized content must notify an already subscribed window"
    );
    let other: Value = receiver
        .execute(ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id: "other".into(),
            before_ordinal: None,
            limit: 10,
            content_byte_limit: 4096,
        })
        .unwrap();
    assert_eq!(
        other["event_sequence"], 1,
        "one page needs distinct event identities for each conversation"
    );
    assert_eq!(other["projection"]["title"], "Another conversation");
    assert!(restored.to_string().contains("First original evidence"));
    assert!(restored.to_string().contains("Second original evidence"));
    let settings: Option<String> = receiver
        .execute(ApplicationCommand::LoadConversationSettings {
            conversation_id: "c".into(),
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(settings.as_deref().unwrap()).unwrap()["memoryEnabledOverride"],
        true
    );
    drop(receiver_service);

    let (_receiver_service, receiver) = start(&receiver_db, &receiver_socket);
    assert_eq!(window(&receiver), restored);
    let mut replay = changes[3].clone();
    replay.cursor = 7;
    let outcome: RemoteSyncPageResult = receiver.execute(remote_page(vec![replay])).unwrap();
    assert_eq!(outcome.skipped, 1);
    assert_eq!(window(&receiver), restored);
    let echoes: Vec<PendingSyncChange> = receiver
        .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 20 })
        .unwrap();
    assert!(echoes.is_empty());

    for (index, entry) in pending.iter().enumerate() {
        let resolved: bool = sender
            .execute(ReplicaSyncCommand::ResolveSyncChange {
                operation_id: entry.operation_id.clone(),
                status: "acknowledged".into(),
                remote_cursor: Some(index as i64 + 1),
                resolved_at_ms: 30,
            })
            .unwrap();
        assert!(resolved);
    }
    drop(sender_service);
    let (_sender_service, sender) = start(&sender_db, &sender_socket);
    let pending: Vec<PendingSyncChange> = sender
        .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 20 })
        .unwrap();
    assert!(pending.is_empty());
}

#[test]
fn large_replica_pages_cross_ipc_in_order_without_exceeding_the_byte_budget() {
    let root = tempfile::tempdir().unwrap();
    let (_service, client) = start(
        &root.path().join("large.db"),
        &root.path().join("large.sock"),
    );
    let (_receiver_service, receiver) = start(
        &root.path().join("large-receiver.db"),
        &root.path().join("large-receiver.sock"),
    );
    let _: Value = client
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "c".into(),
                title: "Large replica page".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    let text = "x".repeat(7 * 1024 * 1024);
    append(&client, 1, &text);
    append(&client, 2, &text);
    drop(text);
    let first: Vec<PendingSyncChange> = client
        .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 100 })
        .unwrap();
    assert_eq!(
        first.len(),
        2,
        "initial base and first append fit, both appends do not"
    );
    assert_eq!(first[0].entity_type, "conversation");
    assert_eq!(first[1].entity_type, "conversation_append");
    assert_eq!(first[1].revision, 2);
    assert!(serde_json::to_vec(&first).unwrap().len() < 12 * 1024 * 1024);
    let transfer = |entries: &[PendingSyncChange], start_cursor: i64| {
        remote_page(
            entries
                .iter()
                .enumerate()
                .map(|(index, entry)| RemoteSyncChange {
                    cursor: start_cursor + index as i64,
                    operation_id: entry.operation_id.clone(),
                    entity_type: entry.entity_type.clone(),
                    entity_id: entry.entity_id.clone(),
                    revision: entry.revision,
                    deleted: entry.deleted,
                    payload_json: entry.payload_json.clone(),
                    created_at_ms: entry.created_at_ms,
                })
                .collect(),
        )
    };
    let received: RemoteSyncPageResult = receiver.execute(transfer(&first, 1)).unwrap();
    assert_eq!(received.applied, 2);
    assert_eq!(received.cursor, 2);
    for (index, entry) in first.iter().enumerate() {
        let resolved: bool = client
            .execute(ReplicaSyncCommand::ResolveSyncChange {
                operation_id: entry.operation_id.clone(),
                status: "acknowledged".into(),
                remote_cursor: Some(index as i64 + 1),
                resolved_at_ms: 10,
            })
            .unwrap();
        assert!(resolved);
    }
    drop(first);
    let second: Vec<PendingSyncChange> = client
        .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 100 })
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].entity_type, "conversation_append");
    assert_eq!(second[0].revision, 3);
    assert!(serde_json::to_vec(&second).unwrap().len() < 12 * 1024 * 1024);
    let received: RemoteSyncPageResult = receiver.execute(transfer(&second, 3)).unwrap();
    assert_eq!(received.applied, 1);
    assert_eq!(received.cursor, 3);
    let restored = window(&receiver);
    assert_eq!(restored["projection"], window(&client)["projection"]);
    assert!(
        restored.to_string().len() < 100_000,
        "replica history remains a bounded UI projection"
    );
    assert!(receiver.health().is_ok());
    assert!(client.health().is_ok());
}

#[test]
fn escaped_oversized_head_returns_a_bounded_error_and_stays_pending() {
    let root = tempfile::tempdir().unwrap();
    let (_service, client) = start(
        &root.path().join("escaped.db"),
        &root.path().join("escaped.sock"),
    );
    let _: Value = client
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "c".into(),
                title: "Escaped replica page".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    // The import fits the frame, but its JSON payload string needs another
    // layer of escaping when it becomes a replica command result.
    append(&client, 1, &"\n".repeat(4 * 1024 * 1024));
    let first: Vec<PendingSyncChange> = client
        .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 100 })
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].entity_type, "conversation");
    let resolved: bool = client
        .execute(ReplicaSyncCommand::ResolveSyncChange {
            operation_id: first[0].operation_id.clone(),
            status: "acknowledged".into(),
            remote_cursor: Some(1),
            resolved_at_ms: 10,
        })
        .unwrap();
    assert!(resolved);
    for _ in 0..2 {
        let result: Result<Vec<PendingSyncChange>, _> =
            client.execute(ReplicaSyncCommand::PendingSyncChanges { limit: 100 });
        let error = result.unwrap_err();
        assert!(error.contains("chunked content transfer required"));
        assert!(error.contains("entry remains pending"));
        assert!(error.len() < 2048);
        assert!(client.health().is_ok());
    }
}

#[test]
fn client_cannot_advance_replica_cursor_past_unreceived_changes() {
    let root = tempfile::tempdir().unwrap();
    let (_service, client) = start(
        &root.path().join("cursor.db"),
        &root.path().join("cursor.sock"),
    );
    let result: Result<RemoteSyncPageResult, _> =
        client.execute(ReplicaSyncCommand::ApplyRemoteSyncPage {
            input: RemoteSyncPage {
                replica_id: "cloud".into(),
                changes: vec![],
                next_cursor: 100,
                applied_at_ms: 1,
            },
        });
    assert!(result.unwrap_err().contains("last change"));
    let cursor: Option<SyncCursor> = client
        .execute(ReplicaSyncCommand::GetSyncCursor {
            replica_id: "cloud".into(),
        })
        .unwrap();
    assert!(cursor.is_none());
    assert!(client.health().is_ok());
}

#[test]
fn typed_sdk_reads_unicode_originals_with_bounded_progress_and_owner_checks() {
    let root = tempfile::tempdir().unwrap();
    let (_service, client) = start(
        &root.path().join("reader.db"),
        &root.path().join("reader.sock"),
    );
    let _: Value = client
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "c".into(),
                title: "Reader".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    let original = "ab界😀tail";
    append(&client, 1, original);
    let mut offset = 0;
    let mut restored = String::new();
    loop {
        let range = client
            .read_conversation_text_range("c", "m1", "p1", offset, 4)
            .unwrap();
        assert_eq!(range.total_utf8_bytes, original.len() as u64);
        assert!(range.text.len() <= 4);
        restored.push_str(&range.text);
        offset = range.next_offset_bytes;
        if range.end_of_content {
            break;
        }
    }
    assert_eq!(restored, original);
    let end = client
        .read_conversation_text_range("c", "m1", "p1", offset, 4)
        .unwrap();
    assert!(end.text.is_empty() && end.end_of_content);
    assert!(client
        .read_conversation_text_range("other", "m1", "p1", 0, 4)
        .is_err());
    assert!(client
        .read_conversation_text_range("c", "m1", "p1", 3, 4)
        .is_err());
    // Generic RPC clients cannot bypass the shared contract by omitting the
    // SDK convenience method. Application validates before querying storage.
    let rejected: Result<Value, _> = client.execute(
        nola_protocol::ConversationRecallCommand::ReadConversationTextRange {
            conversation_id: "c".into(),
            message_id: "m1".into(),
            part_id: "p1".into(),
            offset_bytes: 0,
            limit_bytes: 100_000,
        },
    );
    assert!(rejected
        .unwrap_err()
        .contains("invalid canonical transcript byte range"));
    let rejected: Result<Value, _> = client.execute(
        nola_protocol::ConversationRecallCommand::SearchConversationText {
            conversation_id: "c".into(),
            query: "tail".into(),
            limit: 21,
        },
    );
    assert!(rejected
        .unwrap_err()
        .contains("invalid canonical transcript search bounds"));
    let hits: Vec<nola_protocol::ConversationRecallHit> = client
        .execute(
            nola_protocol::ConversationRecallCommand::SearchConversationText {
                conversation_id: "c".into(),
                query: "tail".into(),
                limit: 1,
            },
        )
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message_id, "m1");
    assert_eq!(hits[0].part_id, "p1");
    assert!(hits[0].snippet.contains("tail"));
}
