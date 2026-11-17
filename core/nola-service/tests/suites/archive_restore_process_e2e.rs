// SPDX-License-Identifier: Apache-2.0
//! Public archive commands through independent Service lifetimes.
use nola_application::{ApplicationCommand, CreateConversation};
use nola_protocol::{
    archive_restore::*, content_upload::*, CommandReceiptOutcome, ConversationRecallCommand,
    ConversationTextRange, ExportCommand, ExportPage,
};
use nola_service_client::ApplicationServiceClient;
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
    let client =
        ApplicationServiceClient::with_identity(database, socket, "archive-wire-test").unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if client.health().is_ok() {
            return (service, client);
        }
        assert!(service.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("archive service startup timed out");
}
fn profile(sequence: u64, id: &str) -> ArchiveRestoreCommand {
    ArchiveRestoreCommand::StageArchiveManifest {
        input: StageArchiveManifest {
            restore_id: "restore".into(),
            sequence,
            entry: ArchiveRestoreManifestEntry::Profile {
                profile: ArchiveConversationProfile {
                    id: id.into(),
                    title: format!("Archive {id}"),
                    settings_json: Some("{}".into()),
                    created_at_ms: 1,
                    updated_at_ms: 2,
                },
            },
        },
    }
}
fn page(client: &ApplicationServiceClient, id: &str) -> Option<ExportPage> {
    client
        .execute(ExportCommand::LoadConversationExportPage {
            conversation_id: id.into(),
            after_ordinal: None,
            expected_revision: None,
        })
        .unwrap()
}

#[test]
fn archive_queries_are_fresh_and_receipt_commit_replaces_multiple_conversations_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("archive.db");
    let socket = root.path().join("service.sock");
    let (service, client) = start(&database, &socket);
    for id in ["old-a", "old-b"] {
        let _: serde_json::Value = client
            .execute(ApplicationCommand::CreateConversation {
                input: CreateConversation {
                    id: id.into(),
                    title: id.into(),
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            })
            .unwrap();
    }
    let revision: u64 = client
        .execute(ArchiveRestoreCommand::ArchiveDirectoryRevision {})
        .unwrap();
    let _: ArchiveRestoreProgress = client
        .execute(ArchiveRestoreCommand::BeginArchiveRestore {
            input: BeginArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: revision,
                created_at_ms: 2,
            },
        })
        .unwrap();
    let _: ArchiveRestoreProgress = client.execute(profile(0, "new-a")).unwrap();
    let query = ArchiveRestoreCommand::ArchiveRestoreProgress {
        restore_id: "restore".into(),
    };
    let before: ArchiveRestoreProgress = client
        .execute_with_request_id("fresh-progress", query.clone())
        .unwrap();
    assert_eq!(before.next_sequence, 1);
    let owner = ContentUploadOwner {
        conversation_id: "new-a".into(),
        import_id: "restore".into(),
        part_id: "p".into(),
    };
    let body = "original 中😀\n\\\"".repeat(100);
    let _: serde_json::Value = client
        .execute(ApplicationCommand::BeginContentUpload {
            input: BeginContentUpload {
                upload_id: "upload".into(),
                owner: owner.clone(),
                expected_utf8_bytes: body.len() as u64,
            },
        })
        .unwrap();
    let _: serde_json::Value = client
        .execute(ApplicationCommand::AppendContentChunk {
            input: AppendContentChunk {
                upload_id: "upload".into(),
                owner: owner.clone(),
                offset_bytes: 0,
                text: body.clone(),
            },
        })
        .unwrap();
    // Receiving upload and staged profiles must survive a Service cold restart.
    drop(service);
    let (service, client) = start(&database, &socket);
    let upload: ContentUploadProgress = client
        .execute(ApplicationCommand::ContentUploadProgress {
            input: ContentUploadRequest {
                upload_id: "upload".into(),
                owner: owner.clone(),
            },
        })
        .unwrap();
    assert_eq!(upload.committed_utf8_bytes, body.len() as u64);
    let _: serde_json::Value = client
        .execute(ApplicationCommand::SealContentUpload {
            input: ContentUploadRequest {
                upload_id: "upload".into(),
                owner,
            },
        })
        .unwrap();
    let message = ArchiveRestoreCommand::StageArchiveManifest {
        input: StageArchiveManifest {
            restore_id: "restore".into(),
            sequence: 1,
            entry: ArchiveRestoreManifestEntry::Message {
                conversation_id: "new-a".into(),
                message: ArchiveMessage {
                    id: "m".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(2),
                    parts: vec![nola_domain::MessagePartInput {
                        id: "p".into(),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: String::new(),
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    }],
                },
                uploads: vec![ArchivePartUpload {
                    field: nola_domain::PartBodyField::ContentText,
                    upload_id: "upload".into(),
                    message_id: "m".into(),
                    part_id: "p".into(),
                    expected_utf8_bytes: body.len() as u64,
                }],
            },
        },
    };
    let _: ArchiveRestoreProgress = client
        .execute_with_request_id("message-stage", message.clone())
        .unwrap();
    let staged: ArchiveRestoreProgress = client.execute(profile(2, "new-b")).unwrap();
    assert!(page(&client, "new-a").is_none() && page(&client, "new-b").is_none());
    assert!(page(&client, "old-a").is_some() && page(&client, "old-b").is_some());
    let connection = rusqlite::Connection::open(&database).unwrap();
    let count = || {
        connection
            .query_row("SELECT COUNT(*) FROM command_receipts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    let before_queries = count();
    let after: ArchiveRestoreProgress = client
        .execute_with_request_id("fresh-progress", query.clone())
        .unwrap();
    assert_eq!(
        after.next_sequence, 3,
        "same query identity must read fresh durable staging"
    );
    let still: u64 = client
        .execute(ArchiveRestoreCommand::ArchiveDirectoryRevision {})
        .unwrap();
    assert_eq!(still, revision);
    assert_eq!(
        count(),
        before_queries,
        "queries must not retain receipts or replay stale progress"
    );
    drop(service);
    let (service, client) = start(&database, &socket);
    let replay: ArchiveRestoreProgress = client
        .execute_with_request_id("message-stage", message)
        .unwrap();
    assert_eq!(
        replay.next_sequence, 2,
        "write replay returns its original durable result"
    );
    let commit = ArchiveRestoreCommand::CommitArchiveRestore {
        input: CommitArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: revision,
            expected_entry_count: staged.next_sequence,
            manifest_digest: staged.manifest_digest,
            committed_at_ms: 3,
        },
    };
    let result: ArchiveRestoreResult = client
        .execute_with_request_id("final-commit", commit.clone())
        .unwrap();
    assert_eq!((result.conversation_count, result.message_count), (2, 1));
    drop(service);
    let (_service, client) = start(&database, &socket);
    let receipt = client.command_receipt("final-commit").unwrap().unwrap();
    assert!(matches!(
        receipt.outcome,
        CommandReceiptOutcome::Committed { .. }
    ));
    assert_eq!(
        client
            .execute_with_request_id::<ArchiveRestoreResult, _>("final-commit", commit.clone())
            .unwrap(),
        result
    );
    assert!(page(&client, "old-a").is_none() && page(&client, "old-b").is_none());
    assert!(
        page(&client, "new-a").is_some() && page(&client, "new-b").unwrap().messages.is_empty()
    );
    let range: ConversationTextRange = client
        .execute(ConversationRecallCommand::ReadConversationTextRange {
            conversation_id: "new-a".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            offset_bytes: 0,
            limit_bytes: 32768,
        })
        .unwrap();
    assert_eq!(range.text, body);
    assert!(range.end_of_content);
    let completed: ArchiveRestoreProgress = client.execute(query).unwrap();
    assert!(completed.completed);
    let mut changed = commit;
    if let ArchiveRestoreCommand::CommitArchiveRestore { input } = &mut changed {
        input.expected_entry_count += 1;
    }
    assert!(client
        .execute_with_request_id::<ArchiveRestoreResult, _>("final-commit", changed)
        .is_err());
}

#[test]
fn branch_export_restores_claims_into_fresh_service_as_archive_and_replays_after_restart() {
    use nola_domain::{
        CanonicalBranchIntent, CanonicalBranchTarget, CanonicalPartOwner, PartDerivationProvenance,
        StoredPartDerivation,
    };
    use nola_store::{
        AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput,
    };
    let root = tempfile::tempdir().unwrap();
    let source_path = root.path().join("source.db");
    let target_path = root.path().join("target.db");
    let source_socket = root.path().join("source.sock");
    let target_socket = root.path().join("target.sock");
    let body = "original 中😀\\\"\n".repeat(100);
    let raw_json = " {\"original_owner\":\"original-part\",\"escaped\":\"\\u4e2d\"} ";
    // Offline fixture only: the remainder uses public SDK queries and commands.
    let mut store = nola_store::NolaStore::open(&source_path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "source".into(),
            title: "Original".into(),
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
                id: "original-message".into(),
                run_id: Some("original-run".into()),
                role: "assistant".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "original-part".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: body.clone(),
                    content_json: Some(raw_json.into()),
                    content_digest: Some("business-digest".into()),
                    created_at_ms: 17,
                    updated_at_ms: 29,
                }],
            }],
        })
        .unwrap();
    store
        .create_conversation_branch(&CanonicalBranchIntent {
            operation_id: "export-branch".into(),
            source_conversation_id: "source".into(),
            expected_source_revision: 2,
            through_message_id: "original-message".into(),
            conversation: CanonicalBranchTarget {
                id: "branch".into(),
                title: "Branch".into(),
                settings_json: None,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            committed_at_ms: 3,
        })
        .unwrap();
    assert!(store.delete_conversation("source").unwrap());
    drop(store);
    let (source_service, source) = start(&source_path, &source_socket);
    let exported = page(&source, "branch").unwrap();
    let message = &exported.messages[0];
    let part = &message.parts[0];
    assert!(part.has_derivation);
    let owner =
        CanonicalPartOwner::new("branch".into(), message.id.clone(), part.id.clone()).unwrap();
    let query = ExportCommand::ReadConversationPartDerivation {
        owner: owner.clone(),
        expected_conversation_revision: exported.conversation.revision,
    };
    let original: StoredPartDerivation = source
        .execute::<Option<StoredPartDerivation>, _>(query)
        .unwrap()
        .unwrap();
    assert_eq!(original.provenance, PartDerivationProvenance::LocalBranch);
    assert_eq!(
        original.derivation.immediate_source.owner.conversation_id(),
        "source"
    );
    let read = |field| -> String {
        let range: nola_protocol::ExportRange = source
            .execute(ExportCommand::ReadConversationExportRange {
                conversation_id: "branch".into(),
                message_id: message.id.clone(),
                part_id: part.id.clone(),
                field,
                expected_revision: exported.conversation.revision,
                offset_bytes: 0,
                limit_bytes: 32768,
            })
            .unwrap();
        assert!(range.end_of_content);
        range.text
    };
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
            content_text: read(nola_protocol::ExportBodyField::ContentText),
            content_json: Some(read(nola_protocol::ExportBodyField::ContentJson)),
            content_digest: part.content_digest.clone(),
            created_at_ms: part.created_at_ms,
            updated_at_ms: part.updated_at_ms,
        }],
    };
    drop(source_service);
    let (target_service, target) = start(&target_path, &target_socket);
    let revision: u64 = target
        .execute(ArchiveRestoreCommand::ArchiveDirectoryRevision {})
        .unwrap();
    let _: ArchiveRestoreProgress = target
        .execute(ArchiveRestoreCommand::BeginArchiveRestore {
            input: BeginArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: revision,
                created_at_ms: 4,
            },
        })
        .unwrap();
    let _: ArchiveRestoreProgress = target.execute(profile(0, "branch")).unwrap();
    let _: ArchiveRestoreProgress = target
        .execute(ArchiveRestoreCommand::StageArchiveManifest {
            input: StageArchiveManifest {
                restore_id: "restore".into(),
                sequence: 1,
                entry: ArchiveRestoreManifestEntry::Message {
                    conversation_id: "branch".into(),
                    message: archived,
                    uploads: vec![],
                },
            },
        })
        .unwrap();
    let proof = ArchiveRestoreCommand::StageArchiveManifest {
        input: StageArchiveManifest {
            restore_id: "restore".into(),
            sequence: 2,
            entry: ArchiveRestoreManifestEntry::PartDerivation {
                record: original.derivation.clone(),
            },
        },
    };
    let progress: ArchiveRestoreProgress = target
        .execute_with_request_id("proof-stage", proof.clone())
        .unwrap();
    assert_eq!(progress.next_sequence, 3);
    assert!(page(&target, "branch").is_none());
    drop(target_service);
    let (target_service, target) = start(&target_path, &target_socket);
    assert_eq!(
        target
            .execute_with_request_id::<ArchiveRestoreProgress, _>("proof-stage", proof)
            .unwrap(),
        progress
    );
    let commit = ArchiveRestoreCommand::CommitArchiveRestore {
        input: CommitArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: revision,
            expected_entry_count: progress.next_sequence,
            manifest_digest: progress.manifest_digest,
            committed_at_ms: 5,
        },
    };
    let result: ArchiveRestoreResult = target
        .execute_with_request_id("proof-final", commit.clone())
        .unwrap();
    drop(target_service);
    let (_target_service, target) = start(&target_path, &target_socket);
    assert_eq!(
        target
            .execute_with_request_id::<ArchiveRestoreResult, _>("proof-final", commit)
            .unwrap(),
        result
    );
    let restored_page = page(&target, "branch").unwrap();
    assert!(restored_page.messages[0].parts[0].has_derivation);
    let restored: StoredPartDerivation = target
        .execute::<Option<StoredPartDerivation>, _>(ExportCommand::ReadConversationPartDerivation {
            owner,
            expected_conversation_revision: restored_page.conversation.revision,
        })
        .unwrap()
        .unwrap();
    assert_eq!(restored.provenance, PartDerivationProvenance::Archive);
    assert_eq!(restored.derivation, original.derivation);
    assert!(serde_json::to_vec(&restored).unwrap().len() <= 24576);
    assert!(page(&target, "source").is_none());
    let restored_part = &restored_page.messages[0].parts[0];
    assert_eq!(
        restored_part.content_digest.as_deref(),
        Some("business-digest")
    );
    assert_eq!(
        (restored_part.created_at_ms, restored_part.updated_at_ms),
        (17, 29)
    );
}
