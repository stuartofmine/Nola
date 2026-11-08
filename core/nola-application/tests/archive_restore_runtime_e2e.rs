// SPDX-License-Identifier: Apache-2.0
//! Public archive command dispatch, upload ownership and durable final receipt.
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, CreateConversation,
    ExecuteReceivedCommand,
};
use nola_protocol::{archive_restore::*, content_upload::*, CommandMetadata};
use serde_json::Value;
use support::ApplicationRuntime;

fn submit(runtime: &mut ApplicationRuntime, id: &str, command: ArchiveRestoreCommand) -> Value {
    let input = ExecuteReceivedCommand {
        client_id: "archive-client".into(),
        request_id: id.into(),
        command_type: command.wire_command_name().into(),
        command_json: serde_json::to_string(&command).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    let receipt = runtime
        .execute_received_command(input, command.into())
        .unwrap();
    match receipt.outcome {
        ApplicationCommandReceiptOutcome::Committed { result, .. } => result,
        other => panic!("archive command failed: {other:?}"),
    }
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
    submit(
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
    let stage = ArchiveRestoreCommand::StageArchiveManifest {
        input: StageArchiveManifest {
            restore_id: "restore".into(),
            sequence: 0,
            entry: ArchiveRestoreManifestEntry::Profile {
                profile: ArchiveConversationProfile {
                    id: "new".into(),
                    title: "Restored".into(),
                    settings_json: Some("{}".into()),
                    created_at_ms: 1,
                    updated_at_ms: 2,
                },
            },
        },
    };
    submit(&mut runtime, "profile", stage);
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
    let staged: ArchiveRestoreProgress = serde_json::from_value(submit(
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
    let replay = submit(
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
    let committed = submit(&mut runtime, "commit", commit.clone());
    let result: ArchiveRestoreResult = serde_json::from_value(committed.clone()).unwrap();
    assert_eq!((result.conversation_count, result.message_count), (1, 1));
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(submit(&mut runtime, "commit", commit), committed);
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
