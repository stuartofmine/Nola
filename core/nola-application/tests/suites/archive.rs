// SPDX-License-Identifier: Apache-2.0
//! Consolidated Archive & Restore integration suite: derivation provenance,
//! receipted large-payload restore, staging invariants, tool-source provenance,
//! replica sync fencing, and offline work-archive recovery.

mod derivation_receipt {
    use crate::support;
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
        let record = store
            .part_derivation_record(&owner)
            .map(|record| record.map(|record| record.derivation))
            .unwrap()
            .unwrap();
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
            rows(&fault, "SELECT derivation_json FROM message_part_derivations").len(),
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
}

mod restore_receipts_and_runtime {
    use crate::support;
    use nola_application::{
        ApplicationCommand, ApplicationCommandReceipt, ApplicationCommandReceiptOutcome,
        CreateConversation, ExecuteReceivedCommand,
    };
    use nola_domain::archive_restore::*;
    use nola_protocol::{archive_restore::ArchiveRestoreCommand, content_upload::*, CommandMetadata};
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
    fn submit_public(
        runtime: &mut ApplicationRuntime,
        id: &str,
        command: ArchiveRestoreCommand,
    ) -> serde_json::Value {
        let input = ExecuteReceivedCommand {
            client_id: "archive-client".into(),
            request_id: id.into(),
            command_type: command.wire_command_name().into(),
            command_json: serde_json::to_string(&command).unwrap(),
            expected_revision: None,
            received_at: "2026-09-08T00:00:00Z".into(),
        };
        committed(runtime.execute_received_command(input, command.into()).unwrap())
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
    fn staged_upload_is_invisible_until_public_commit_and_replays_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("archive.db");
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime
            .execute(ApplicationCommand::CreateConversation {
                input: CreateConversation {
                    id: "retained".into(),
                    title: "Existing".into(),
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            })
            .unwrap();
        let directory: u64 = serde_json::from_value(
            runtime
                .execute(ArchiveRestoreCommand::ArchiveDirectoryRevision {}.into())
                .unwrap(),
        )
        .unwrap();
        submit_public(
            &mut runtime,
            "begin",
            ArchiveRestoreCommand::BeginArchiveRestore {
                input: BeginArchiveRestore {
                    restore_id: "restore".into(),
                    expected_directory_revision: directory,
                    created_at_ms: 2,
                },
            },
        );
        submit_public(
            &mut runtime,
            "profile",
            ArchiveRestoreCommand::StageArchiveManifest {
                input: StageArchiveManifest {
                    restore_id: "restore".into(),
                    sequence: 0,
                    entry: profile("new"),
                },
            },
        );
        let owner = ContentUploadOwner {
            conversation_id: "new".into(),
            import_id: "restore".into(),
            part_id: "part".into(),
        };
        let body = "ab中😀tail";
        runtime
            .execute(ApplicationCommand::BeginContentUpload {
                input: BeginContentUpload {
                    upload_id: "body".into(),
                    owner: owner.clone(),
                    expected_utf8_bytes: body.len() as u64,
                },
            })
            .unwrap();
        runtime
            .execute(ApplicationCommand::AppendContentChunk {
                input: AppendContentChunk {
                    upload_id: "body".into(),
                    owner: owner.clone(),
                    offset_bytes: 0,
                    text: body.into(),
                },
            })
            .unwrap();
        let manifest = StageArchiveManifest {
            restore_id: "restore".into(),
            sequence: 1,
            entry: ArchiveRestoreManifestEntry::Message {
                conversation_id: "new".into(),
                message: ArchiveMessage {
                    id: "message".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(2),
                    parts: vec![nola_domain::MessagePartInput {
                        id: "part".into(),
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
                    upload_id: "body".into(),
                    message_id: "message".into(),
                    part_id: "part".into(),
                    expected_utf8_bytes: body.len() as u64,
                }],
            },
        };
        assert!(runtime
            .execute(
                ArchiveRestoreCommand::StageArchiveManifest {
                    input: manifest.clone()
                }
                .into()
            )
            .is_err());
        runtime
            .execute(ApplicationCommand::SealContentUpload {
                input: ContentUploadRequest {
                    upload_id: "body".into(),
                    owner,
                },
            })
            .unwrap();
        let staged: ArchiveRestoreProgress = serde_json::from_value(submit_public(
            &mut runtime,
            "message",
            ArchiveRestoreCommand::StageArchiveManifest {
                input: manifest.clone(),
            },
        ))
        .unwrap();
        assert_eq!(staged.next_sequence, 2);
        assert!(!staged.completed);
        assert!(runtime
            .execute(ApplicationCommand::LoadConversation {
                conversation_id: "new".into()
            })
            .unwrap()
            .is_null());
        assert!(!runtime
            .execute(ApplicationCommand::LoadConversation {
                conversation_id: "retained".into()
            })
            .unwrap()
            .is_null());
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        let progress: ArchiveRestoreProgress = serde_json::from_value(
            runtime
                .execute(
                    ArchiveRestoreCommand::ArchiveRestoreProgress {
                        restore_id: "restore".into(),
                    }
                    .into(),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(progress, staged);
        let replay = submit_public(
            &mut runtime,
            "message",
            ArchiveRestoreCommand::StageArchiveManifest { input: manifest },
        );
        assert_eq!(
            serde_json::from_value::<ArchiveRestoreProgress>(replay).unwrap(),
            staged
        );
        let commit = ArchiveRestoreCommand::CommitArchiveRestore {
            input: CommitArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: directory,
                expected_entry_count: staged.next_sequence,
                manifest_digest: staged.manifest_digest,
                committed_at_ms: 3,
            },
        };
        assert!(runtime
            .execute(commit.clone().into())
            .unwrap_err()
            .contains("command receipt write unit"));
        let committed_val = submit_public(&mut runtime, "commit", commit.clone());
        let result: ArchiveRestoreResult = serde_json::from_value(committed_val.clone()).unwrap();
        assert_eq!((result.conversation_count, result.message_count), (1, 1));
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        assert_eq!(submit_public(&mut runtime, "commit", commit), committed_val);
        assert!(runtime
            .execute(ApplicationCommand::LoadConversation {
                conversation_id: "retained".into()
            })
            .unwrap()
            .is_null());
        let range = runtime
            .execute(ApplicationCommand::ReadConversationTextRange {
                conversation_id: "new".into(),
                message_id: "message".into(),
                part_id: "part".into(),
                offset_bytes: 0,
                limit_bytes: 32,
            })
            .unwrap();
        assert_eq!(range["text"], body);
    }

    #[test]
    fn real_archive_receipts_keep_ten_million_byte_staging_invisible_and_commit_all_histories_with_bounded_events() {
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
        let message: StageArchiveManifest = serde_json::from_value(serde_json::json!({
            "restore_id":"restore","sequence":2,
            "entry":{"kind":"message","conversation_id":"c",
                "message":{"id":"m","run_id":null,"role":"user","status":"completed","ordinal":1,"created_at_ms":1,"completed_at_ms":2,
                    "parts":[{"id":"p","kind":"text","ordinal":1,"state":"completed","content_text":"","content_json":null,"content_digest":null,"created_at_ms":1,"updated_at_ms":2}]},
                "uploads":[{"field":"content_text","upload_id":"body","message_id":"m","part_id":"p","expected_utf8_bytes":total}]}
        }))
        .unwrap();
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
            .query_row("SELECT COUNT(*) FROM projection_events", [], |row| row.get(0))
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
    fn two_conversations_with_two_hundred_uploaded_messages_restore_and_retired_sealed_reads_survive_completion() {
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
        let db =
            rusqlite::Connection::open(directory.path().join("many.db.uploads.sqlite")).unwrap();
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
}

mod staging_invariants {
    use nola_application::stage_archive_manifest;
    use nola_domain::archive_restore::*;
    use nola_store::{content_upload::ContentUploadStore, NolaStore};
    use nola_store_port::{archive_restore::ArchiveRestoreRepository, content_upload::UploadOwner};

    fn profile(restore: &str, id: &str, sequence: u64) -> StageArchiveManifest {
        StageArchiveManifest {
            restore_id: restore.into(),
            sequence,
            entry: ArchiveRestoreManifestEntry::Profile {
                profile: ArchiveConversationProfile {
                    id: id.into(),
                    title: "Archive".into(),
                    settings_json: Some("{}".into()),
                    created_at_ms: 1,
                    updated_at_ms: 2,
                },
            },
        }
    }
    fn manifest(
        restore: &str,
        conversation: &str,
        sequence: u64,
        upload_id: &str,
        message_id: &str,
        part_id: &str,
        bytes: u64,
    ) -> StageArchiveManifest {
        serde_json::from_value(serde_json::json!({
            "restore_id": restore,
            "sequence": sequence,
            "entry": {
                "kind": "message",
                "conversation_id": conversation,
                "message": {
                    "id": message_id, "run_id": null, "role": "user", "status": "completed",
                    "ordinal": 1, "created_at_ms": 1, "completed_at_ms": 2,
                    "parts": [{
                        "id": part_id, "kind": "text", "ordinal": 1, "state": "completed",
                        "content_text": "", "content_json": null, "content_digest": null,
                        "created_at_ms": 1, "updated_at_ms": 2
                    }]
                },
                "uploads": [{
                    "field": "content_text", "upload_id": upload_id,
                    "message_id": message_id, "part_id": part_id,
                    "expected_utf8_bytes": bytes
                }]
            }
        }))
        .unwrap()
    }

    #[test]
    fn staged_uploads_enforce_wire_field_and_owner_invariants_through_reopen_and_commit() {
        // 1. Wire-level forbidden references and missing field rejection.
        let sample = manifest("restore", "c", 1, "body", "m", "p", 10_000_000);
        assert!(serde_json::to_vec(&sample).unwrap().len() < 1024);
        let mut wire_refs = serde_json::to_value(&sample).unwrap();
        wire_refs["entry"]["references"] = serde_json::json!([]);
        assert!(serde_json::from_value::<StageArchiveManifest>(wire_refs).is_err());
        let mut wire_no_field = serde_json::to_value(&sample).unwrap();
        wire_no_field["entry"]["uploads"][0]
            .as_object_mut()
            .unwrap()
            .remove("field");
        assert!(serde_json::from_value::<StageArchiveManifest>(wire_no_field).is_err());

        // 2. Shared SQLite store + upload store exercising malformed JSON, conflicting inline body,
        //    multi-chunk restore-owned invisibility, foreign restore rejection, dual-field binding,
        //    cold reopen, commit, and post-commit mutation fencing.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive-staging.db");
        let mut store = NolaStore::open(&path).unwrap();
        let mut uploads =
            ContentUploadStore::open(&dir.path().join("uploads.db"), 1_000_000).unwrap();
        store
            .begin_archive_restore(&BeginArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: 0,
                created_at_ms: 1,
            })
            .unwrap();
        stage_archive_manifest(&mut store, &uploads, &profile("restore", "c", 0), 1_000_000)
            .unwrap();
        let after_profiles = stage_archive_manifest(
            &mut store,
            &uploads,
            &profile("restore", "dual", 1),
            1_000_000,
        )
        .unwrap();
        assert!(store.load_conversation("c").unwrap().is_none());
        assert!(store
            .archive_restore_accepts_upload("restore", "c")
            .unwrap());

        // Conflicting inline source alongside upload binding is rejected without advancing sequence.
        let mut conflicting_inline = manifest("restore", "c", 2, "body", "m", "p", 10);
        if let ArchiveRestoreManifestEntry::Message { message, .. } = &mut conflicting_inline.entry
        {
            message.parts[0].content_text = "conflicting inline source".into();
        }
        assert!(
            stage_archive_manifest(&mut store, &uploads, &conflicting_inline, 1_000_000)
                .unwrap_err()
                .contains("field binding")
        );
        assert_eq!(store.archive_restore_progress("restore").unwrap(), after_profiles);

        // Malformed JSON upload never becomes staged history or prepared object.
        let bad_owner = UploadOwner {
            conversation_id: "c".into(),
            import_id: "restore".into(),
            part_id: "p-bad".into(),
        };
        uploads.begin("bad-json", &bad_owner, 7).unwrap();
        uploads.append("bad-json", &bad_owner, 0, "{broken").unwrap();
        uploads.seal("bad-json", &bad_owner).unwrap();
        let mut bad_json = manifest("restore", "c", 2, "bad-json", "m-bad", "p-bad", 7);
        if let ArchiveRestoreManifestEntry::Message { uploads, .. } = &mut bad_json.entry {
            uploads[0].field = nola_domain::PartBodyField::ContentJson;
        }
        assert!(stage_archive_manifest(&mut store, &uploads, &bad_json, 1_000_000).is_err());
        assert_eq!(store.archive_restore_progress("restore").unwrap(), after_profiles);
        assert!(store
            .prepared_archive_reference("restore", "bad-json")
            .unwrap()
            .is_none());

        // Multi-chunk UTF-8 text upload requires seal and rejects foreign restore theft.
        let owner_c = UploadOwner {
            conversation_id: "c".into(),
            import_id: "restore".into(),
            part_id: "p".into(),
        };
        let large_text = "中😀\"\\".repeat(10_000);
        assert!(large_text.len() > 32768);
        uploads
            .begin("body-c", &owner_c, large_text.len() as u64)
            .unwrap();
        let mut offset = 0;
        while offset < large_text.len() {
            let mut end = (offset + 32768).min(large_text.len());
            while !large_text.is_char_boundary(end) {
                end -= 1;
            }
            uploads
                .append("body-c", &owner_c, offset as u64, &large_text[offset..end])
                .unwrap();
            offset = end;
        }
        let msg_c = manifest("restore", "c", 2, "body-c", "m", "p", large_text.len() as u64);
        assert!(stage_archive_manifest(&mut store, &uploads, &msg_c, 1_000_000).is_err());
        uploads.seal("body-c", &owner_c).unwrap();
        let progress_c =
            stage_archive_manifest(&mut store, &uploads, &msg_c, 1_000_000).unwrap();
        assert_eq!(progress_c.next_sequence, 3);
        assert!(store.load_conversation("c").unwrap().is_none());
        assert_eq!(store.archive_directory_revision().unwrap(), 0);

        store
            .begin_archive_restore(&BeginArchiveRestore {
                restore_id: "foreign".into(),
                expected_directory_revision: 0,
                created_at_ms: 1,
            })
            .unwrap();
        stage_archive_manifest(&mut store, &uploads, &profile("foreign", "c", 0), 1_000_000)
            .unwrap();
        assert!(stage_archive_manifest(
            &mut store,
            &uploads,
            &manifest("foreign", "c", 1, "body-c", "m", "p", large_text.len() as u64),
            1_000_000
        )
        .is_err());

        // Dual-field uploads validate both bindings atomically and reject field reinterpretation.
        let owner_dual = UploadOwner {
            conversation_id: "dual".into(),
            import_id: "restore".into(),
            part_id: "p-dual".into(),
        };
        let dual_text = "原始 text";
        let dual_json = " {\"seed\":9007199254740993,\"text\":\"原始\"} ";
        for (id, raw) in [("dual-body", dual_text), ("dual-json", dual_json)] {
            uploads.begin(id, &owner_dual, raw.len() as u64).unwrap();
            uploads.append(id, &owner_dual, 0, raw).unwrap();
            uploads.seal(id, &owner_dual).unwrap();
        }
        let mut dual_input = manifest(
            "restore",
            "dual",
            3,
            "dual-body",
            "m-dual",
            "p-dual",
            dual_text.len() as u64,
        );
        if let ArchiveRestoreManifestEntry::Message {
            message, uploads, ..
        } = &mut dual_input.entry
        {
            message.parts[0].kind = "tool_result".into();
            uploads.push(ArchivePartUpload {
                upload_id: "dual-json".into(),
                message_id: "m-dual".into(),
                part_id: "p-dual".into(),
                field: nola_domain::PartBodyField::ContentJson,
                expected_utf8_bytes: dual_json.len() as u64,
            });
        }
        let mut invalid_id = dual_input.clone();
        if let ArchiveRestoreManifestEntry::Message { uploads, .. } = &mut invalid_id.entry {
            uploads[1].upload_id = " invalid-upload".into();
        }
        assert!(stage_archive_manifest(&mut store, &uploads, &invalid_id, 1_000_000).is_err());
        assert!(store
            .prepared_archive_reference("restore", "dual-body")
            .unwrap()
            .is_none());

        let mut duplicate_field = dual_input.clone();
        if let ArchiveRestoreManifestEntry::Message { uploads, .. } = &mut duplicate_field.entry {
            uploads[1].field = nola_domain::PartBodyField::ContentText;
        }
        assert!(stage_archive_manifest(&mut store, &uploads, &duplicate_field, 1_000_000).is_err());
        assert!(
            store
                .prepared_archive_reference("restore", "dual-body")
                .unwrap()
                .is_none(),
            "a bad second binding must be rejected before preparing the first body"
        );
        let final_progress =
            stage_archive_manifest(&mut store, &uploads, &dual_input, 1_000_000).unwrap();
        if let ArchiveRestoreManifestEntry::Message { uploads, .. } = &mut duplicate_field.entry {
            uploads.remove(0);
        }
        assert!(stage_archive_manifest(&mut store, &uploads, &duplicate_field, 1_000_000).is_err());
        assert_eq!(
            stage_archive_manifest(&mut store, &uploads, &dual_input, 1_000_000).unwrap(),
            final_progress
        );

        // Reopen before commit: staged replay returns exact progress, then commit installs both conversations.
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        assert_eq!(
            stage_archive_manifest(&mut store, &uploads, &dual_input, 1_000_000).unwrap(),
            final_progress
        );
        store.begin_write_unit_of_work().unwrap();
        store
            .commit_archive_restore(&CommitArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: 0,
                expected_entry_count: 4,
                manifest_digest: final_progress.manifest_digest,
                committed_at_ms: 3,
            })
            .unwrap();
        store.commit_write_unit_of_work().unwrap();
        assert!(!store
            .archive_restore_accepts_upload("restore", "c")
            .unwrap());
        assert!(
            stage_archive_manifest(&mut store, &uploads, &dual_input, 1_000_000)
                .unwrap()
                .completed
        );
        let mut post_commit_extra = msg_c;
        post_commit_extra.sequence = 4;
        assert!(
            stage_archive_manifest(&mut store, &uploads, &post_commit_extra, 1_000_000).is_err()
        );

        drop(store);
        let reopened = NolaStore::open(&path).unwrap();
        assert_eq!(
            reopened.load_conversation("c").unwrap().unwrap().messages[0].parts[0].content_text,
            large_text
        );
        let dual_part =
            &reopened.load_conversation("dual").unwrap().unwrap().messages[0].parts[0];
        assert_eq!(dual_part.content_text, dual_text);
        assert_eq!(dual_part.content_json.as_deref(), Some(dual_json));
    }
}

mod tool_source_lifecycle {
    use nola_application::stage_archive_manifest;
    use nola_domain::archive_restore::*;
    use nola_domain::tool_source::{ToolSourceClaim, ToolSourceKind, ToolSourceOriginalOwner};
    use nola_domain::{CanonicalBodyReference, CanonicalPartOwner, MessagePartInput, PartBodyField};
    use nola_store::{content_upload::ContentUploadStore, NolaStore};
    use nola_store_port::archive_restore::ArchiveRestoreRepository;
    use nola_store_port::content_upload::UploadOwner;
    use nola_store_port::replica_sync::{ApplyRemoteSyncPageInput, RemoteSyncChangeInput};
    use nola_store_port::tool_source::{ToolSourceProvenance, ToolSourceRepository};
    use sha2::{Digest, Sha256};

    fn owner() -> CanonicalPartOwner {
        CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap()
    }

    fn chunks(raw: &str) -> Vec<(u64, String)> {
        let mut chunks = Vec::new();
        let mut offset = 0;
        while offset < raw.len() {
            let mut end = (offset + 32768).min(raw.len());
            while !raw.is_char_boundary(end) {
                end -= 1;
            }
            chunks.push((offset as u64, raw[offset..end].into()));
            offset = end;
        }
        chunks
    }

    fn source(kind: ToolSourceKind) -> (ToolSourceClaim, String) {
        let id = match kind {
            ToolSourceKind::ToolCall => "original-call-item",
            ToolSourceKind::ToolResult => "original-result-item",
        };
        let value = match kind {
            ToolSourceKind::ToolCall => serde_json::json!({
                "type":"tool_call", "id":id, "call_id":"actual-call", "name":"read_file",
                "arguments":{"path":"isolated.txt"}
            }),
            ToolSourceKind::ToolResult => serde_json::json!({
                "type":"tool_result", "id":id, "call_id":"actual-call", "name":"read_file",
                "output":{"text":"原文🙂\n".repeat(8_000)}, "is_error":false,
                "full_output_ref":null, "artifacts":[]
            }),
        };
        let raw = format!(" \n{}\n", serde_json::to_string(&value).unwrap());
        let claim = ToolSourceClaim {
            schema_version: 1,
            source_item_id: id.into(),
            kind,
            call_id: "actual-call".into(),
            name: "read_file".into(),
            owner: owner(),
            original_owner: ToolSourceOriginalOwner {
                part_owner: owner(),
                run_id: "original-run".into(),
                turn_id: "original-turn".into(),
            },
            body: CanonicalBodyReference::for_field(
                format!("client-must-not-select-{id}"),
                "c".into(),
                "m".into(),
                "p".into(),
                PartBodyField::ContentJson,
                raw.len() as u64,
                chunks(&raw).len() as u32,
            )
            .unwrap(),
            body_sha256: format!("{:x}", Sha256::digest(raw.as_bytes())),
            derivation: None,
        };
        (claim, raw)
    }

    fn stage_base(store: &mut NolaStore, uploads: &ContentUploadStore) -> u64 {
        let directory = store.archive_directory_revision().unwrap();
        store
            .begin_archive_restore(&BeginArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: directory,
                created_at_ms: 1,
            })
            .unwrap();
        let entries = [
            ArchiveRestoreManifestEntry::Profile {
                profile: ArchiveConversationProfile {
                    id: "c".into(),
                    title: "Archive".into(),
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 2,
                },
            },
            ArchiveRestoreManifestEntry::Message {
                conversation_id: "c".into(),
                message: ArchiveMessage {
                    id: "m".into(),
                    run_id: Some("original-run".into()),
                    role: "assistant".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(2),
                    parts: vec![MessagePartInput {
                        id: "p".into(),
                        kind: "tool".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: "Visible presentation".into(),
                        content_json: Some("{}".into()),
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    }],
                },
                uploads: vec![],
            },
        ];
        for (sequence, entry) in entries.into_iter().enumerate() {
            stage_archive_manifest(
                store,
                uploads,
                &StageArchiveManifest {
                    restore_id: "restore".into(),
                    sequence: sequence as u64,
                    entry,
                },
                1_000_000,
            )
            .unwrap();
        }
        directory
    }

    fn upload(uploads: &mut ContentUploadStore, id: &str, raw: &str) {
        let owner = UploadOwner {
            conversation_id: "c".into(),
            import_id: "restore".into(),
            part_id: "p".into(),
        };
        uploads.begin(id, &owner, raw.len() as u64).unwrap();
        for (offset, text) in chunks(raw) {
            uploads.append(id, &owner, offset, &text).unwrap();
        }
        uploads.seal(id, &owner).unwrap();
    }

    fn stage_source(
        store: &mut NolaStore,
        uploads: &ContentUploadStore,
        claim: ToolSourceClaim,
        upload_id: &str,
        sequence: u64,
    ) -> Result<ArchiveRestoreProgress, String> {
        stage_archive_manifest(
            store,
            uploads,
            &StageArchiveManifest {
                restore_id: "restore".into(),
                sequence,
                entry: ArchiveRestoreManifestEntry::ToolSource {
                    claim,
                    upload_id: upload_id.into(),
                },
            },
            1_000_000,
        )
    }

    fn commit(
        store: &mut NolaStore,
        directory: u64,
        progress: &ArchiveRestoreProgress,
    ) -> CommitArchiveRestore {
        let input = CommitArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: directory,
            expected_entry_count: progress.next_sequence,
            manifest_digest: progress.manifest_digest.clone(),
            committed_at_ms: 3,
        };
        store.begin_write_unit_of_work().unwrap();
        store.commit_archive_restore(&input).unwrap();
        store.commit_write_unit_of_work().unwrap();
        input
    }

    #[test]
    fn archive_tool_source_digest_rollback_cold_open_and_replica_sync_lifecycle() {
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("source.sqlite");
        let destination_path = directory.path().join("destination.sqlite");
        let mut store = NolaStore::open(&source_path).unwrap();
        store
            .create_conversation(&nola_store::CreateConversationInput {
                id: "before".into(),
                title: "Must survive".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        let mut uploads =
            ContentUploadStore::open(&directory.path().join("uploads.sqlite"), 1_000_000).unwrap();
        let revision = stage_base(&mut store, &uploads);
        let call = source(ToolSourceKind::ToolCall);
        let result = source(ToolSourceKind::ToolResult);
        upload(&mut uploads, "call-upload", &call.1);
        upload(&mut uploads, "result-upload", &result.1);

        // 1. Bad upload digest is rejected before preparing any canonical text or archive reference.
        let before_progress = store.archive_restore_progress("restore").unwrap();
        let mut bad_claim = call.0.clone();
        bad_claim.body_sha256 = "0".repeat(64);
        assert!(stage_source(&mut store, &uploads, bad_claim, "call-upload", 2).is_err());
        assert_eq!(
            store.archive_restore_progress("restore").unwrap(),
            before_progress
        );
        let sql = rusqlite::Connection::open(&source_path).unwrap();
        let counts: (i64, i64) = sql
            .query_row(
                "SELECT (SELECT COUNT(*) FROM canonical_text_objects),
                    (SELECT COUNT(*) FROM archive_restore_prepared)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(counts, (0, 0));
        assert!(store.load_conversation("c").unwrap().is_none());

        // 2. Valid call + result tool sources stage cleanly while remaining invisible.
        stage_source(&mut store, &uploads, call.0.clone(), "call-upload", 2).unwrap();
        let progress =
            stage_source(&mut store, &uploads, result.0.clone(), "result-upload", 3).unwrap();
        assert!(store.load_conversation("c").unwrap().is_none());

        // 3. Install fault rolls back the entire visible replacement, keeping "before" intact.
        sql.execute_batch(
            "CREATE TRIGGER reject_archive_source BEFORE INSERT ON canonical_tool_sources
            BEGIN SELECT RAISE(ABORT,'fixture archive source fault'); END;",
        )
        .unwrap();
        let commit_input = CommitArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: revision,
            expected_entry_count: progress.next_sequence,
            manifest_digest: progress.manifest_digest.clone(),
            committed_at_ms: 3,
        };
        store.begin_write_unit_of_work().unwrap();
        assert!(store.commit_archive_restore(&commit_input).is_err());
        store.rollback_write_unit_of_work().unwrap();
        assert_eq!(
            store.load_conversation("before").unwrap().unwrap().title,
            "Must survive"
        );
        assert!(store.load_conversation("c").unwrap().is_none());
        assert_eq!(store.archive_directory_revision().unwrap(), revision);
        assert_eq!(store.archive_restore_progress("restore").unwrap(), progress);
        assert_eq!(
            sql.query_row("SELECT COUNT(*) FROM canonical_tool_sources", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        sql.execute_batch("DROP TRIGGER reject_archive_source")
            .unwrap();

        // 4. Commit + idempotent commit replay + cold reopen verify exact untrusted Archive provenance.
        let committed_input = commit(&mut store, revision, &progress);
        store.begin_write_unit_of_work().unwrap();
        store.commit_archive_restore(&committed_input).unwrap();
        store.commit_write_unit_of_work().unwrap();
        drop(store);

        let mut cold = NolaStore::open(&source_path).unwrap();
        assert!(cold.load_conversation("before").unwrap().is_none());
        let conversation = cold.load_conversation("c").unwrap().unwrap();
        assert_eq!(
            conversation.messages[0].parts[0].content_json.as_deref(),
            Some("{}")
        );
        let bindings = cold
            .tool_sources_for_part(&owner(), conversation.revision)
            .unwrap();
        assert_eq!(bindings.len(), 2);
        for (index, (binding, (claim, expected))) in
            bindings.iter().zip([&call, &result]).enumerate()
        {
            assert_eq!(binding.source_item_id, claim.source_item_id);
            assert_eq!(binding.body_sha256, claim.body_sha256);
            assert_ne!(binding.body.content_id(), claim.body.content_id());
            match &binding.provenance {
                ToolSourceProvenance::Archive {
                    claim: imported,
                    restore_id,
                    sequence,
                } => {
                    assert_eq!(restore_id, "restore");
                    assert_eq!(*sequence, index as u64 + 2);
                    assert_eq!(imported.original_owner, claim.original_owner);
                    assert_eq!(imported.body, binding.body);
                }
                other => panic!("archive cannot recreate local accepted authority: {other:?}"),
            }
            let mut raw = String::new();
            let mut offset = 0;
            loop {
                let page = cold
                    .read_tool_source_range(binding, conversation.revision, offset, 4099)
                    .unwrap();
                assert_eq!(page.offset_bytes, offset);
                assert_eq!(page.next_offset_bytes, offset + page.text.len() as u64);
                raw.push_str(&page.text);
                if page.end_of_content {
                    break;
                }
                assert!(page.next_offset_bytes > offset);
                offset = page.next_offset_bytes;
            }
            assert_eq!(&raw, expected);
        }

        // 5. Replica sync waits for body seals before exposing archive_restore_commit on destination.
        let mut destination = NolaStore::open(&destination_path).unwrap();
        let first = cold.pending_sync_changes(100).unwrap();
        assert!(first
            .iter()
            .any(|fact| fact.entity_type == "canonical_text_seal"));
        assert!(first
            .iter()
            .any(|fact| fact.entity_type == "archive_restore_entry"
                && serde_json::from_str::<serde_json::Value>(&fact.payload_json).unwrap()["entry"]
                    ["kind"]
                    == "tool_source"));
        assert!(
            !first
                .iter()
                .any(|fact| fact.entity_type == "archive_restore_commit"),
            "unacknowledged source seals must fence final visibility"
        );
        let mut cursor = 0;
        let mut sync_committed = false;
        for _ in 0..16 {
            let page = cold.pending_sync_changes(100).unwrap();
            if page.is_empty() {
                break;
            }
            let next_cursor = cursor + page.len() as i64;
            destination
                .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                    replica_id: "archive-replica".into(),
                    next_cursor,
                    applied_at_ms: 10,
                    changes: page
                        .iter()
                        .enumerate()
                        .map(|(index, fact)| RemoteSyncChangeInput {
                            cursor: cursor + index as i64 + 1,
                            operation_id: fact.operation_id.clone(),
                            entity_type: fact.entity_type.clone(),
                            entity_id: fact.entity_id.clone(),
                            revision: fact.revision,
                            deleted: fact.deleted,
                            payload_json: fact.payload_json.clone(),
                            created_at_ms: fact.created_at_ms,
                        })
                        .collect(),
                })
                .unwrap();
            sync_committed |= page
                .iter()
                .any(|fact| fact.entity_type == "archive_restore_commit");
            if !sync_committed {
                assert!(destination.load_conversation("c").unwrap().is_none());
            }
            for fact in page {
                assert!(cold
                    .resolve_sync_change(&fact.operation_id, "acknowledged", Some(next_cursor), 10)
                    .unwrap());
            }
            cursor = next_cursor;
        }
        assert!(sync_committed);
        assert!(cold.pending_sync_changes(100).unwrap().is_empty());
        drop(destination);
        let cold_destination = NolaStore::open(&destination_path).unwrap();
        let current = cold_destination
            .load_conversation("c")
            .unwrap()
            .unwrap()
            .revision;
        let binding = cold_destination
            .tool_source_by_id("c", &call.0.source_item_id, current)
            .unwrap()
            .unwrap();
        assert!(matches!(
            binding.provenance,
            ToolSourceProvenance::Archive { .. }
        ));
        assert_eq!(
            cold_destination
                .read_tool_source_range(&binding, current, 0, 32768)
                .unwrap()
                .text,
            call.1
        );
    }
}

mod work_archive_recovery {
    use crate::support::model_identity as fixture_model_identity;
    use nola_agent_engine::agent::recovery::CheckpointReason;
    use nola_agent_engine::agent::*;
    use nola_store::{
        work_archive::read_verified, CreateConversationInput, NolaStore, SaveCheckpointInput,
    };
    use nola_store_port::run_effect::*;

    fn submit(core: &mut AgentCore, id: &str, command: GoalRuntimeCommand) {
        core.submit(AgentSubmission::GoalRuntime {
            envelope: GoalRuntimeCommandEnvelope {
                operation_id: id.into(),
                command,
            },
        })
        .unwrap();
    }

    #[test]
    fn archived_goal_can_be_read_without_source_database_or_restoring_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("original.db");
        let config = AgentCoreConfig {
            turn_identity: fixture_model_identity::identity(),
            session_id: Some(SessionId::from_string("archive-session".into())),
            ..Default::default()
        };
        let mut core = AgentCore::try_new(config.clone()).unwrap();
        submit(
            &mut core,
            "start",
            GoalRuntimeCommand::Start {
                id: "g".into(),
                objective: "完成修复".into(),
                constraints: vec!["不得删除用户数据".into()],
                requirements: vec![GoalRequirement {
                    acceptance_policy:
                        nola_agent_engine::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                    semantic_revision: 1,
                    id: "r".into(),
                    description: "验证通过".into(),
                    dependencies: vec![],
                    status: GoalRecordStatus::Pending,
                    evidence_refs: vec![],
                }],
                now_epoch_ms: 1,
            },
        );
        let checkpoint = core.checkpoint(CheckpointReason::GoalUpdated);
        let mut store = NolaStore::open(&db).unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "archive-session".into(),
                title: "test".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        store
            .save_checkpoint(&SaveCheckpointInput {
                conversation_id: "archive-session".into(),
                expected_revision: 0,
                revision: 1,
                event_cursor: checkpoint.last_event_cursor.sequence as i64,
                checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
                updated_at_ms: 1,
            })
            .unwrap();
        let directory = store.work_archive_directory("archive-session").unwrap();
        store.flush_work_archives().unwrap();
        let publication = store.archive_publication_status().unwrap();
        assert_eq!(publication.published_generation, publication.target_generation);
        read_verified(&directory).unwrap();
        drop(store);
        std::fs::rename(&db, temp.path().join("offline.db")).unwrap();
        let archive = read_verified(&directory).unwrap();
        let restored =
            nola_application::inspect_archived_work(config.clone(), &archive.recovery).unwrap();
        assert_eq!(
            restored["goal"]["active"]["constraints"][0],
            "不得删除用户数据"
        );
        assert_eq!(restored["execution_resumed"], false);
        assert_eq!(restored["requires_permission_revalidation"], true);

        let pause = AgentSubmission::GoalRuntime {
            envelope: GoalRuntimeCommandEnvelope {
                operation_id: "pause".into(),
                command: GoalRuntimeCommand::Pause {
                    status: GoalStatus::Paused,
                    wake_at_epoch_ms: None,
                    now_epoch_ms: 2,
                },
            },
        };
        core.submit(pause.clone()).unwrap();
        let mut recovery = archive.recovery;
        recovery.checkpoint.revision = 2;
        recovery.checkpoint.event_cursor = core
            .checkpoint(CheckpointReason::GoalUpdated)
            .last_event_cursor
            .sequence as i64;
        recovery.replay_base = Some(StoredCheckpointReplayBase {
            run_id: "run".into(),
            checkpoint_revision: 1,
            through_ordinal: 0,
        });
        recovery.replay_inputs = vec![StoredCoreReplayInput {
            kind: ReplayInputKind::Submission,
            ordinal: 1,
            received_at_ms: 2,
            reduction_json: serde_json::to_string(&SubmissionReplayReduction {
                protocol_version: 1,
                session_id: "archive-session".into(),
                decision_time_ms: 2,
                submission_json: serde_json::to_string(&pause).unwrap(),
                bindings: vec![],
            })
            .unwrap(),
        }];
        let restored = nola_application::inspect_archived_work(config.clone(), &recovery).unwrap();
        assert_eq!(restored["goal"]["active"]["status"], "paused");
        recovery.replay_inputs[0].ordinal = 3;
        assert!(nola_application::inspect_archived_work(config, &recovery).is_err());
    }
}
