// SPDX-License-Identifier: Apache-2.0
//! Real queued admissions span three Runs before receipt-bound archive replacement.
use super::*;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
};
use nola_protocol::{archive_restore::*, CommandMetadata};
use serde_json::Value;

fn submit_archive(
    runtime: &mut support::ApplicationRuntime,
    id: &str,
    command: ArchiveRestoreCommand,
) -> Value {
    let receipt = runtime
        .execute_received_command(
            ExecuteReceivedCommand {
                client_id: "three-run-archive".into(),
                request_id: id.into(),
                command_type: command.wire_command_name().into(),
                command_json: serde_json::to_string(&command).unwrap(),
                expected_revision: None,
                received_at: "2026-10-11T00:00:00Z".into(),
            },
            command.into(),
        )
        .unwrap();
    match receipt.outcome {
        ApplicationCommandReceiptOutcome::Committed { result, .. } => result,
        other => panic!("archive transaction did not commit: {other:?}"),
    }
}

#[test]
fn three_run_input_sources_survive_cold_promotion_then_archive_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("three-run-archive.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let first = begin(&mut store, "separate-core-session");
    for (index, id) in ["queued-a", "queued-b"].into_iter().enumerate() {
        let queued = reduce_and_commit_active_submission(
            &mut store,
            ReduceAndCommitActiveSubmission {
                conversation_id: CONVERSATION.into(),
                run_id: first.begin.run_id.clone(),
                transition_id: format!("queue-{id}"),
                invocation_config_digest: first.invocation_config_digest.clone(),
                submission: user_input(id, AgentInputDelivery::NextTurn, 1100 + index as i64),
                committed_at_ms: 1100 + index as i64,
            },
        )
        .unwrap();
        assert!(queued.commit.applied);
    }
    complete_begun(&mut store, &first, 1200);
    assert!(load_committed_host_requests(&store, &first.begin.run_id)
        .unwrap()
        .requests
        .is_empty());
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let second = nola_application::promote_queued_assistant_turn(
        &mut store,
        nola_application::PromoteQueuedAssistantTurn {
            conversation_id: CONVERSATION.into(),
            created_at_ms: 2000,
        },
    )
    .unwrap()
    .expect("first queued admission must survive restart");
    assert_ne!(first.begin.run_id, second.begin.run_id);
    complete_begun(&mut store, &second, 2100);
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let third = nola_application::promote_queued_assistant_turn(
        &mut store,
        nola_application::PromoteQueuedAssistantTurn {
            conversation_id: CONVERSATION.into(),
            created_at_ms: 3000,
        },
    )
    .unwrap()
    .expect("second original admission must survive two predecessors");
    assert_ne!(third.begin.run_id, first.begin.run_id);
    assert_ne!(third.begin.run_id, second.begin.run_id);
    complete_begun(&mut store, &third, 3100);
    assert!(nola_application::promote_queued_assistant_turn(
        &mut store,
        nola_application::PromoteQueuedAssistantTurn {
            conversation_id: CONVERSATION.into(),
            created_at_ms: 3200
        }
    )
    .unwrap()
    .is_none());
    drop(store);

    let db = rusqlite::Connection::open(&path).unwrap();
    let facts = db
        .prepare(
            "SELECT a.submission_id,a.source_run_id,p.source_run_id,s.previous_run_id,s.run_id
        FROM input_admissions a JOIN input_turn_applications p USING(core_session_id,admission_id)
        JOIN run_successor_admissions s USING(core_session_id,admission_id)
        ORDER BY a.accepted_sequence",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        facts,
        vec![
            (
                "submission-queued-a".into(),
                first.begin.run_id.clone(),
                second.begin.run_id.clone(),
                first.begin.run_id.clone(),
                second.begin.run_id.clone()
            ),
            (
                "submission-queued-b".into(),
                first.begin.run_id.clone(),
                third.begin.run_id.clone(),
                second.begin.run_id.clone(),
                third.begin.run_id.clone()
            ),
        ]
    );
    let original = NolaStore::open(&path)
        .unwrap()
        .load_conversation(CONVERSATION)
        .unwrap()
        .unwrap();
    assert_eq!(
        original
            .messages
            .iter()
            .filter(|m| m.role == "assistant")
            .count(),
        3
    );
    drop(db);

    let mut runtime = support::open_runtime(&path).unwrap();
    let revision = runtime
        .execute(ApplicationCommand::ArchiveDirectoryRevision {})
        .unwrap()
        .as_u64()
        .unwrap();
    submit_archive(
        &mut runtime,
        "begin-archive",
        ArchiveRestoreCommand::BeginArchiveRestore {
            input: BeginArchiveRestore {
                restore_id: "replace-three-runs".into(),
                expected_directory_revision: revision,
                created_at_ms: 4000,
            },
        },
    );
    submit_archive(
        &mut runtime,
        "stage-profile",
        ArchiveRestoreCommand::StageArchiveManifest {
            input: StageArchiveManifest {
                restore_id: "replace-three-runs".into(),
                sequence: 0,
                entry: ArchiveRestoreManifestEntry::Profile {
                    profile: ArchiveConversationProfile {
                        id: CONVERSATION.into(),
                        title: "Restored three-Run conversation".into(),
                        settings_json: None,
                        created_at_ms: 1000,
                        updated_at_ms: 4000,
                    },
                },
            },
        },
    );
    let staged: ArchiveRestoreProgress = serde_json::from_value(submit_archive(
        &mut runtime,
        "stage-message",
        ArchiveRestoreCommand::StageArchiveManifest {
            input: StageArchiveManifest {
                restore_id: "replace-three-runs".into(),
                sequence: 1,
                entry: ArchiveRestoreManifestEntry::Message {
                    conversation_id: CONVERSATION.into(),
                    uploads: vec![],
                    message: ArchiveMessage {
                        id: "restored-message".into(),
                        run_id: None,
                        role: "assistant".into(),
                        status: "completed".into(),
                        ordinal: 1,
                        created_at_ms: 4000,
                        completed_at_ms: Some(4000),
                        parts: vec![nola_domain::MessagePartInput {
                            id: "restored-part".into(),
                            kind: "text".into(),
                            ordinal: 1,
                            state: "completed".into(),
                            content_text: "Exact restored archive body".into(),
                            content_json: None,
                            content_digest: None,
                            created_at_ms: 4000,
                            updated_at_ms: 4000,
                        }],
                    },
                },
            },
        },
    ))
    .unwrap();
    drop(runtime);
    let before_commit = NolaStore::open(&path)
        .unwrap()
        .load_conversation(CONVERSATION)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(before_commit).unwrap(),
        serde_json::to_value(&original).unwrap(),
        "staging must not mutate the three completed Runs' visible history"
    );
    let mut runtime = support::open_runtime(&path).unwrap();
    let commit = ArchiveRestoreCommand::CommitArchiveRestore {
        input: CommitArchiveRestore {
            restore_id: "replace-three-runs".into(),
            expected_directory_revision: revision,
            expected_entry_count: staged.next_sequence,
            manifest_digest: staged.manifest_digest,
            committed_at_ms: 4100,
        },
    };
    let result = submit_archive(&mut runtime, "commit-archive", commit.clone());
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(
        submit_archive(&mut runtime, "commit-archive", commit),
        result
    );
    drop(runtime);
    let restored = NolaStore::open(&path).unwrap();
    assert!(restored.active_run(CONVERSATION).unwrap().is_none());
    let conversation = restored.load_conversation(CONVERSATION).unwrap().unwrap();
    assert_eq!(conversation.messages.len(), 1);
    assert_eq!(
        conversation.messages[0].parts[0].content_text,
        "Exact restored archive body"
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    for table in [
        "runs",
        "input_admissions",
        "input_turn_applications",
        "run_input_applications",
        "run_successor_admissions",
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "archive replacement left old {table}");
    }
    assert!(db
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}
