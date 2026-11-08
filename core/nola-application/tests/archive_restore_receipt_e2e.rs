// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceipt, ApplicationCommandReceiptOutcome,
    ExecuteReceivedCommand,
};
use nola_domain::archive_restore::*;
use nola_protocol::{content_upload::*, CommandMetadata};
use nola_store::{
    AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput, NolaStore,
};
use nola_store_port::archive_restore::ArchiveRestoreRepository;
use support::ApplicationRuntime;

fn received(id: &str, command: &ApplicationCommand) -> ExecuteReceivedCommand {
    ExecuteReceivedCommand {
        client_id: "archive-client".into(),
        request_id: id.into(),
        command_type: command.wire_command_name().into(),
        command_json: serde_json::to_string(command).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    }
}
fn submit(
    runtime: &mut ApplicationRuntime,
    id: &str,
    command: ApplicationCommand,
) -> ApplicationCommandReceipt {
    runtime
        .execute_received_command(received(id, &command), command)
        .unwrap()
}
fn committed(receipt: ApplicationCommandReceipt) -> serde_json::Value {
    match receipt.outcome {
        ApplicationCommandReceiptOutcome::Committed { result, .. } => result,
        other => panic!("not committed: {other:?}"),
    }
}
fn profile(id: &str) -> ArchiveRestoreManifestEntry {
    ArchiveRestoreManifestEntry::Profile {
        profile: ArchiveConversationProfile {
            id: id.into(),
            title: format!("Restored {id}"),
            settings_json: Some("{}".into()),
            created_at_ms: 1,
            updated_at_ms: 3,
        },
    }
}
fn initial(path: &std::path::Path) {
    let mut store = NolaStore::open(path).unwrap();
    for id in ["c", "excluded"] {
        store
            .create_conversation(&CreateConversationInput {
                id: id.into(),
                title: "Original".into(),
                revision: 1,
                settings_json: Some("{}".into()),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
    }
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "c".into(),
            expected_message_count: 0,
            updated_at_ms: 2,
            messages: vec![MessageInput {
                id: "m".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "p".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "original evidence".into(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            }],
        })
        .unwrap();
}
fn originals(path: &std::path::Path, revision: u64) {
    let store = NolaStore::open(path).unwrap();
    assert_eq!(store.archive_directory_revision().unwrap(), revision);
    assert_eq!(
        store.load_conversation("c").unwrap().unwrap().messages[0].parts[0].content_text,
        "original evidence"
    );
    assert!(store.load_conversation("excluded").unwrap().is_some());
    assert!(store.load_conversation("new").unwrap().is_none());
}

#[test]
fn real_archive_receipts_keep_ten_million_byte_staging_invisible_and_commit_all_histories_with_bounded_events(
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("archive.db");
    initial(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    let revision = runtime
        .execute(ApplicationCommand::ArchiveDirectoryRevision {})
        .unwrap()
        .as_u64()
        .unwrap();
    committed(submit(
        &mut runtime,
        "begin",
        ApplicationCommand::BeginArchiveRestore {
            input: BeginArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: revision,
                created_at_ms: 3,
            },
        },
    ));
    for (sequence, id) in ["c", "new"].into_iter().enumerate() {
        committed(submit(
            &mut runtime,
            &format!("profile-{id}"),
            ApplicationCommand::StageArchiveManifest {
                input: StageArchiveManifest {
                    restore_id: "restore".into(),
                    sequence: sequence as u64,
                    entry: profile(id),
                },
            },
        ));
    }
    let owner = ContentUploadOwner {
        conversation_id: "c".into(),
        import_id: "restore".into(),
        part_id: "p".into(),
    };
    let total = 10_000_000usize;
    committed(submit(
        &mut runtime,
        "upload-begin",
        ApplicationCommand::BeginContentUpload {
            input: BeginContentUpload {
                upload_id: "body".into(),
                owner: owner.clone(),
                expected_utf8_bytes: total as u64,
            },
        },
    ));
    let mut offset = 0;
    while offset < total {
        let end = (offset + 32768).min(total);
        let mut text = "x".repeat(end - offset);
        if end == total {
            text.replace_range(text.len() - 9.., "TAIL-DONE");
        }
        committed(submit(
            &mut runtime,
            &format!("chunk-{offset}"),
            ApplicationCommand::AppendContentChunk {
                input: AppendContentChunk {
                    upload_id: "body".into(),
                    owner: owner.clone(),
                    offset_bytes: offset as u64,
                    text,
                },
            },
        ));
        offset = end;
        if offset % (32768 * 80) == 0 {
            drop(runtime);
            runtime = support::open_runtime(&path).unwrap();
            originals(&path, revision);
        }
    }
    committed(submit(
        &mut runtime,
        "seal",
        ApplicationCommand::SealContentUpload {
            input: ContentUploadRequest {
                upload_id: "body".into(),
                owner,
            },
        },
    ));
    let message:StageArchiveManifest=serde_json::from_value(serde_json::json!({"restore_id":"restore","sequence":2,"entry":{"kind":"message","conversation_id":"c","message":{"id":"m","run_id":null,"role":"user","status":"completed","ordinal":1,"created_at_ms":1,"completed_at_ms":2,"parts":[{"id":"p","kind":"text","ordinal":1,"state":"completed","content_text":"","content_json":null,"content_digest":null,"created_at_ms":1,"updated_at_ms":2}]},"uploads":[{"field":"content_text","upload_id":"body","message_id":"m","part_id":"p","expected_utf8_bytes":total}]}})).unwrap();
    let progress: ArchiveRestoreProgress = serde_json::from_value(committed(submit(
        &mut runtime,
        "message",
        ApplicationCommand::StageArchiveManifest { input: message },
    )))
    .unwrap();
    originals(&path, revision);
    drop(runtime);
    runtime = support::open_runtime(&path).unwrap();
    let input = CommitArchiveRestore {
        restore_id: "restore".into(),
        expected_directory_revision: revision,
        expected_entry_count: progress.next_sequence,
        manifest_digest: progress.manifest_digest,
        committed_at_ms: 4,
    };
    let fault = rusqlite::Connection::open(&path).unwrap();
    fault.execute_batch("CREATE TRIGGER fail_archive_sync BEFORE INSERT ON sync_outbox WHEN NEW.entity_type='archive_restore_commit' BEGIN SELECT RAISE(ABORT,'archive sync failure'); END").unwrap();
    let rejected = submit(
        &mut runtime,
        "final-sync-fail",
        ApplicationCommand::CommitArchiveRestore {
            input: input.clone(),
        },
    );
    assert!(matches!(
        rejected.outcome,
        ApplicationCommandReceiptOutcome::Rejected { .. }
    ));
    originals(&path, revision);
    fault.execute_batch("DROP TRIGGER fail_archive_sync; CREATE TRIGGER fail_archive_journal BEFORE INSERT ON projection_events WHEN NEW.scope_type='conversation' AND NEW.scope_id='new' BEGIN SELECT RAISE(ABORT,'archive journal failure'); END").unwrap();
    let command = ApplicationCommand::CommitArchiveRestore {
        input: input.clone(),
    };
    assert!(runtime
        .execute_received_command(received("final-journal-fail", &command), command)
        .is_err());
    originals(&path, revision);
    fault
        .execute_batch("DROP TRIGGER fail_archive_journal")
        .unwrap();
    let command = ApplicationCommand::CommitArchiveRestore {
        input: input.clone(),
    };
    let admission = received("final-success", &command);
    let result = runtime
        .execute_received_command(admission.clone(), command)
        .unwrap();
    assert!(
        serde_json::to_vec(&result.clone().into_protocol_receipt())
            .unwrap()
            .len()
            < 2048
    );
    committed(result);
    let before_replay: i64 = fault
        .query_row("SELECT COUNT(*) FROM projection_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    let projection_count: i64 = fault
        .query_row(
            "SELECT COUNT(*) FROM projection_events WHERE scope_type='conversation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(projection_count, 2);
    let list_count: i64 = fault
        .query_row(
            "SELECT COUNT(*) FROM projection_events WHERE scope_type='conversations'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(list_count, 3);
    let largest: i64 = fault
        .query_row(
            "SELECT MAX(length(CAST(payload_json AS BLOB))) FROM projection_events",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(largest < 4096);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let replay = runtime
        .execute_received_command(
            admission,
            ApplicationCommand::CommitArchiveRestore { input },
        )
        .unwrap();
    assert!(replay.replayed);
    committed(replay);
    assert_eq!(
        fault
            .query_row("SELECT COUNT(*) FROM projection_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        before_replay
    );
    let store = NolaStore::open(&path).unwrap();
    let window = store
        .load_conversation_message_window("c", None, 10, 1024)
        .unwrap()
        .unwrap();
    assert_eq!(window.conversation.revision, 3);
    assert_eq!(window.conversation.messages[0].id, "m");
    assert_eq!(window.conversation.messages[0].parts[0].id, "p");
    assert_eq!(window.deferred_text_parts[0].total_utf8_bytes, total as u64);
    assert_eq!(
        store
            .read_conversation_text_range("c", "m", "p", (total - 9) as u64, 32)
            .unwrap()
            .text,
        "TAIL-DONE"
    );
    assert!(store.load_conversation("excluded").unwrap().is_none());
    assert!(store.load_conversation("new").unwrap().is_some());
}

#[test]
fn two_conversations_with_two_hundred_uploaded_messages_restore_and_retired_sealed_reads_survive_completion(
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("many.db");
    initial(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    let revision = runtime
        .execute(ApplicationCommand::ArchiveDirectoryRevision {})
        .unwrap()
        .as_u64()
        .unwrap();
    committed(submit(
        &mut runtime,
        "begin-many",
        ApplicationCommand::BeginArchiveRestore {
            input: BeginArchiveRestore {
                restore_id: "many".into(),
                expected_directory_revision: revision,
                created_at_ms: 3,
            },
        },
    ));
    for (sequence, id) in ["c", "new"].into_iter().enumerate() {
        committed(submit(
            &mut runtime,
            &format!("profile-{id}"),
            ApplicationCommand::StageArchiveManifest {
                input: StageArchiveManifest {
                    restore_id: "many".into(),
                    sequence: sequence as u64,
                    entry: profile(id),
                },
            },
        ));
    }
    let body = "x".repeat(100_000);
    let mut last = None;
    for index in 0..200 {
        let conversation = if index < 100 { "c" } else { "new" };
        let upload_id = format!("many-{index}");
        let part_id = format!("part-{index}");
        let message_id = format!("message-{index}");
        let owner = ContentUploadOwner {
            conversation_id: conversation.into(),
            import_id: "many".into(),
            part_id: part_id.clone(),
        };
        committed(submit(
            &mut runtime,
            &format!("begin-{index}"),
            ApplicationCommand::BeginContentUpload {
                input: BeginContentUpload {
                    upload_id: upload_id.clone(),
                    owner: owner.clone(),
                    expected_utf8_bytes: body.len() as u64,
                },
            },
        ));
        for offset in (0..body.len()).step_by(32768) {
            committed(submit(
                &mut runtime,
                &format!("chunk-{index}-{offset}"),
                ApplicationCommand::AppendContentChunk {
                    input: AppendContentChunk {
                        upload_id: upload_id.clone(),
                        owner: owner.clone(),
                        offset_bytes: offset as u64,
                        text: body[offset..(offset + 32768).min(body.len())].into(),
                    },
                },
            ));
        }
        committed(submit(
            &mut runtime,
            &format!("seal-{index}"),
            ApplicationCommand::SealContentUpload {
                input: ContentUploadRequest {
                    upload_id: upload_id.clone(),
                    owner,
                },
            },
        ));
        let message = ArchiveMessage {
            id: message_id.clone(),
            run_id: None,
            role: "user".into(),
            status: "completed".into(),
            ordinal: index % 100 + 1,
            created_at_ms: 1,
            completed_at_ms: Some(2),
            parts: vec![MessagePartInput {
                id: part_id.clone(),
                kind: "text".into(),
                ordinal: 1,
                state: "completed".into(),
                content_text: String::new(),
                content_json: None,
                content_digest: None,
                created_at_ms: 1,
                updated_at_ms: 2,
            }],
        };
        let value = committed(submit(
            &mut runtime,
            &format!("stage-{index}"),
            ApplicationCommand::StageArchiveManifest {
                input: StageArchiveManifest {
                    restore_id: "many".into(),
                    sequence: index as u64 + 2,
                    entry: ArchiveRestoreManifestEntry::Message {
                        conversation_id: conversation.into(),
                        message,
                        uploads: vec![ArchivePartUpload {
                            field: nola_domain::PartBodyField::ContentText,
                            upload_id,
                            message_id,
                            part_id,
                            expected_utf8_bytes: body.len() as u64,
                        }],
                    },
                },
            },
        ));
        last = Some(serde_json::from_value::<ArchiveRestoreProgress>(value).unwrap());
        if index % 25 == 0 {
            drop(runtime);
            runtime = support::open_runtime(&path).unwrap();
            originals(&path, revision);
        }
    }
    let progress = last.unwrap();
    committed(submit(
        &mut runtime,
        "finish-many",
        ApplicationCommand::CommitArchiveRestore {
            input: CommitArchiveRestore {
                restore_id: "many".into(),
                expected_directory_revision: revision,
                expected_entry_count: progress.next_sequence,
                manifest_digest: progress.manifest_digest,
                committed_at_ms: 4,
            },
        },
    ));
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let owner = ContentUploadOwner {
        conversation_id: "c".into(),
        import_id: "many".into(),
        part_id: "part-0".into(),
    };
    let request = ContentUploadRequest {
        upload_id: "many-0".into(),
        owner: owner.clone(),
    };
    assert_eq!(
        runtime
            .execute(ApplicationCommand::SealContentUpload { input: request })
            .unwrap()["state"],
        "sealed"
    );
    let chunk = runtime
        .execute(ApplicationCommand::ReadContentUploadChunk {
            input: ReadContentUploadChunk {
                upload_id: "many-0".into(),
                owner,
                offset_bytes: 32768,
            },
        })
        .unwrap();
    assert_eq!(chunk["text"].as_str().unwrap(), &body[32768..65536]);
    let db = rusqlite::Connection::open(directory.path().join("many.db.uploads.sqlite")).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM upload_retirements", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        199
    );
    assert_eq!(
        db.query_row(
            "SELECT SUM(length(CAST(text AS BLOB))) FROM upload_chunks",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        100_000
    );
    let store = NolaStore::open(&path).unwrap();
    for id in ["c", "new"] {
        let conversation = store.load_conversation(id).unwrap().unwrap();
        assert_eq!(conversation.messages.len(), 100);
        assert!(conversation
            .messages
            .iter()
            .all(|message| message.parts[0].content_text == body));
    }
}
