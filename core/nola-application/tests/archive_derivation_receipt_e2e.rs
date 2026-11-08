// SPDX-License-Identifier: Apache-2.0
//! Archive provenance publication shares the durable command's outer transaction.
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceipt, ApplicationCommandReceiptOutcome,
    ExecuteReceivedCommand,
};
use nola_domain::{
    archive_restore::*, CanonicalBranchIntent, CanonicalBranchTarget, CanonicalPartOwner,
    PartDerivation, PartDerivationProvenance,
};
use nola_protocol::CommandMetadata;
use nola_store::{
    AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput, NolaStore,
};
use nola_store_port::archive_restore::ArchiveRestoreRepository;
use serde_json::{json, Value};
use std::path::Path;
use support::ApplicationRuntime;

fn received(id: &str, command: &ApplicationCommand) -> ExecuteReceivedCommand {
    ExecuteReceivedCommand {
        client_id: "archive-proof-client".into(),
        request_id: id.into(),
        command_type: command.wire_command_name().into(),
        command_json: serde_json::to_string(command).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    }
}
fn submit(runtime: &mut ApplicationRuntime, id: &str, command: ApplicationCommand) -> Value {
    let receipt = runtime
        .execute_received_command(received(id, &command), command)
        .unwrap();
    match receipt.outcome {
        ApplicationCommandReceiptOutcome::Committed { result, .. } => result,
        other => panic!("fixture command did not commit: {other:?}"),
    }
}
fn seed(path: &Path, text: &str) -> (CanonicalPartOwner, PartDerivation, ArchiveMessage) {
    let mut store = NolaStore::open(path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "source".into(),
            title: "Source".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "source".into(),
            expected_message_count: 0,
            updated_at_ms: 2,
            messages: vec![MessageInput {
                id: "source-message".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "source-part".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: text.into(),
                    content_json: Some(format!(" {{ \"raw\": \"{text}\" }} ")),
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            }],
        })
        .unwrap();
    store
        .create_conversation_branch(&CanonicalBranchIntent {
            operation_id: "original-branch".into(),
            source_conversation_id: "source".into(),
            expected_source_revision: 2,
            through_message_id: "source-message".into(),
            conversation: CanonicalBranchTarget {
                id: "derived".into(),
                title: "Derived".into(),
                settings_json: None,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            committed_at_ms: 3,
        })
        .unwrap();
    let conversation = store.load_conversation("derived").unwrap().unwrap();
    let message = &conversation.messages[0];
    let part = &message.parts[0];
    let owner =
        CanonicalPartOwner::new("derived".into(), message.id.clone(), part.id.clone()).unwrap();
    let record = store.part_derivation_record(&owner).map(|record| record.map(|record| record.derivation)).unwrap().unwrap();
    let archived = ArchiveMessage {
        id: message.id.clone(),
        run_id: message.run_id.clone(),
        role: message.role.clone(),
        status: message.status.clone(),
        ordinal: message.ordinal,
        created_at_ms: message.created_at_ms,
        completed_at_ms: message.completed_at_ms,
        parts: vec![MessagePartInput {
            id: part.id.clone(),
            kind: part.kind.clone(),
            ordinal: part.ordinal,
            state: part.state.clone(),
            content_text: part.content_text.clone(),
            content_json: part.content_json.clone(),
            content_digest: part.content_digest.clone(),
            created_at_ms: part.created_at_ms,
            updated_at_ms: part.updated_at_ms,
        }],
    };
    (owner, record, archived)
}
fn rows(connection: &rusqlite::Connection, query: &str) -> Vec<String> {
    connection
        .prepare(query)
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}
fn stable_state(path: &Path) -> Value {
    let store = NolaStore::open(path).unwrap();
    let db = rusqlite::Connection::open(path).unwrap();
    json!({
        "directory_revision": store.archive_directory_revision().unwrap(),
        "source": store.load_conversation("source").unwrap(),
        "derived": store.load_conversation("derived").unwrap(),
        "proofs": rows(&db, "SELECT derivation_json || ':' || provenance FROM message_part_derivations ORDER BY target_part_id"),
        "outbox": rows(&db, "SELECT operation_id || payload_json || status FROM sync_outbox ORDER BY operation_id"),
        "events": rows(&db, "SELECT event_id || payload_json || sequence FROM projection_events ORDER BY event_id"),
        "receipts": rows(&db, "SELECT request_id || ':' || status || COALESCE(result_json,'') || COALESCE(error_json,'') FROM command_receipts WHERE request_id NOT LIKE 'final-%' ORDER BY request_id"),
    })
}

#[test]
fn archive_proof_receipt_rolls_back_each_final_boundary_and_replays_after_cold_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("target.db");
    let (owner, old_proof, _) = seed(&path, "old evidence");
    let (incoming_owner, claim, message) = seed(
        &directory.path().join("archive-source.db"),
        "replacement evidence",
    );
    assert_eq!(owner, incoming_owner);
    assert_ne!(old_proof, claim);
    let mut runtime = support::open_runtime(&path).unwrap();
    let revision = runtime
        .execute(ApplicationCommand::ArchiveDirectoryRevision {})
        .unwrap()
        .as_u64()
        .unwrap();
    submit(
        &mut runtime,
        "begin",
        ApplicationCommand::BeginArchiveRestore {
            input: BeginArchiveRestore {
                restore_id: "restore-proof".into(),
                expected_directory_revision: revision,
                created_at_ms: 4,
            },
        },
    );
    let mut progress = None;
    for (sequence, entry) in [
        ArchiveRestoreManifestEntry::Profile {
            profile: ArchiveConversationProfile {
                id: "derived".into(),
                title: "Restored".into(),
                settings_json: None,
                created_at_ms: 3,
                updated_at_ms: 4,
            },
        },
        ArchiveRestoreManifestEntry::Message {
            conversation_id: "derived".into(),
            message,
            uploads: vec![],
        },
        ArchiveRestoreManifestEntry::PartDerivation {
            record: claim.clone(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        progress = Some(
            serde_json::from_value::<ArchiveRestoreProgress>(submit(
                &mut runtime,
                &format!("stage-{sequence}"),
                ApplicationCommand::StageArchiveManifest {
                    input: StageArchiveManifest {
                        restore_id: "restore-proof".into(),
                        sequence: sequence as u64,
                        entry,
                    },
                },
            ))
            .unwrap(),
        );
    }
    let progress = progress.unwrap();
    let input = CommitArchiveRestore {
        restore_id: "restore-proof".into(),
        expected_directory_revision: revision,
        expected_entry_count: progress.next_sequence,
        manifest_digest: progress.manifest_digest,
        committed_at_ms: 5,
    };
    let before = stable_state(&path);
    let fault = rusqlite::Connection::open(&path).unwrap();
    for (stage, trigger) in [
        ("proof", "CREATE TRIGGER injected_failure BEFORE INSERT ON message_part_derivations BEGIN SELECT RAISE(ABORT,'proof insert failure'); END"),
        ("outbox", "CREATE TRIGGER injected_failure BEFORE INSERT ON sync_outbox WHEN NEW.entity_type='archive_restore_commit' BEGIN SELECT RAISE(ABORT,'archive outbox failure'); END"),
        ("journal", "CREATE TRIGGER injected_failure BEFORE INSERT ON projection_events BEGIN SELECT RAISE(ABORT,'archive journal failure'); END"),
        ("receipt", "CREATE TRIGGER injected_failure BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'archive receipt failure'); END"),
    ] {
        fault.execute_batch(trigger).unwrap();
        let id = format!("final-{stage}");
        let command = ApplicationCommand::CommitArchiveRestore { input: input.clone() };
        let result = runtime.execute_received_command(received(&id, &command), command);
        if let Ok(receipt) = result {
            assert!(matches!(receipt.outcome, ApplicationCommandReceiptOutcome::Rejected { .. }), "{stage}: {receipt:?}");
        }
        assert_eq!(stable_state(&path), before, "partial final write at {stage}");
        let receipt = runtime.command_receipt("archive-proof-client", &id).unwrap();
        if let Some(receipt) = receipt {
            assert!(matches!(receipt.outcome, ApplicationCommandReceiptOutcome::Rejected { .. }));
        }
        let progress = runtime.execute(ApplicationCommand::ArchiveRestoreProgress { restore_id: "restore-proof".into() }).unwrap();
        assert_eq!(progress["completed"], false);
        fault.execute_batch("DROP TRIGGER injected_failure").unwrap();
        drop(runtime);
        runtime = support::open_runtime(&path).unwrap();
        assert_eq!(stable_state(&path), before);
    }
    let command = ApplicationCommand::CommitArchiveRestore {
        input: input.clone(),
    };
    let admission = received("final-success", &command);
    let receipt: ApplicationCommandReceipt = runtime
        .execute_received_command(admission.clone(), command)
        .unwrap();
    assert!(matches!(
        receipt.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    let protocol_receipt = receipt.into_protocol_receipt();
    let successful = stable_state(&path);
    let store = NolaStore::open(&path).unwrap();
    let imported = store.part_derivation_record(&owner).unwrap().unwrap();
    assert_eq!(imported.derivation, claim);
    assert_eq!(imported.provenance, PartDerivationProvenance::Archive);
    assert!(store.load_conversation("source").unwrap().is_none());
    assert_eq!(
        store
            .load_conversation("derived")
            .unwrap()
            .unwrap()
            .messages[0]
            .parts[0]
            .content_text,
        "replacement evidence"
    );
    assert_eq!(
        rows(
            &fault,
            "SELECT derivation_json FROM message_part_derivations"
        )
        .len(),
        1
    );
    drop(store);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let replay = runtime
        .execute_received_command(
            admission,
            ApplicationCommand::CommitArchiveRestore { input },
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.into_protocol_receipt(), protocol_receipt);
    assert_eq!(stable_state(&path), successful);
    assert_eq!(
        rows(
            &fault,
            "SELECT request_id FROM command_receipts WHERE request_id='final-success'"
        )
        .len(),
        1
    );
}
