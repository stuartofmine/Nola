// SPDX-License-Identifier: Apache-2.0
//! Consolidated Transcript Imports, Admissions, Frontiers & Uploads suite:
//! canonical run terminal query, user-only message admission & expert anchor,
//! submission frontier ownership, transcript import validation & run scope,
//! import projection journal atomicity, upload repository injection, and
//! 10MB uploaded transcript preparation + publication.

use crate::support;
use nola_application::agent_room::{AgentRoomCommand, JoinAgent, SubmitRoomMessage};
use nola_application::uploaded_transcript_preparation::prepare_uploaded_transcript;
use nola_application::uploaded_transcript_publication::publish_uploaded_transcript;
use nola_application::{
    execute_application_command, ApplicationCommand, ApplicationCommandReceiptOutcome,
    ApplicationProjectionScope, CreateConversation, EditConversationMessage,
    ExecuteReceivedCommand,
};
use nola_domain::remote_submission::{
    RemoteSubmissionIdentity, SubmissionFrontier, SubmissionFrontierRequest,
};
use nola_kernel::{ConversationProjectionReducer, ProjectionApplyError};
use nola_protocol::content_upload::{
    BeginContentUpload, ContentUploadOwner, ContentUploadRequest,
};
use nola_protocol::uploaded_transcript::UploadedTranscriptManifest;
use nola_protocol::{
    AdmitConversationUserMessage, CommandMetadata, ConversationTranscriptImportSource,
    ImportConversationTranscript, TranscriptMessageInput, TranscriptPartInput,
};
use nola_store::{
    canonical_text_store::CanonicalTextStore, content_upload::ContentUploadStore,
    transcript_preparation::TranscriptContentPreparationStore, CreateConversationInput, NolaStore,
};
use nola_store_port::content_upload::{ContentUploadRepository, UploadOwner, UploadProgress};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use support::ApplicationRuntime;

#[test]
fn canonical_terminal_query_user_admission_and_submission_frontier_contracts() {
    // 1. Canonical run terminal query is read-only and returns null for unknown runs.
    let mut mem = NolaStore::open(":memory:").unwrap();
    let terminal_cmd: ApplicationCommand = serde_json::from_value(json!({
        "type": "canonical_run_terminal", "conversation_id": "conversation", "run_id": "run"
    }))
    .unwrap();
    assert!(terminal_cmd.is_read_only_query());
    assert!(execute_application_command(&mut mem, terminal_cmd)
        .unwrap()
        .is_null());

    // 2. Canonical user-only message admission replays, conflicts, and anchors expert submissions without starting a Run.
    let conv_id = "11111111-1111-4111-8111-111111111111";
    let mut runtime = support::open_runtime(":memory:").unwrap();
    runtime
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: conv_id.into(),
                title: "Chat".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    let exec_rcpt = |rt: &mut ApplicationRuntime, req_id: &str, cmd: ApplicationCommand| -> Result<Value, String> {
        let received = ExecuteReceivedCommand {
            client_id: "chat-client".into(),
            request_id: req_id.into(),
            command_type: cmd.wire_command_name().into(),
            command_json: serde_json::to_string(&cmd).unwrap(),
            expected_revision: None,
            received_at: "2026-10-23T00:00:00Z".into(),
        };
        let receipt = rt
            .execute_received_command(received, cmd)
            .map_err(|e| e.to_string())?;
        match receipt.outcome {
            ApplicationCommandReceiptOutcome::Committed { result, .. } => Ok(result),
            other => Err(format!("unexpected admission outcome: {other:?}")),
        }
    };
    let admit = |msg_id: &str, text: &str| ApplicationCommand::AdmitConversationUserMessage {
        input: AdmitConversationUserMessage {
            conversation_id: conv_id.into(),
            message_id: msg_id.into(),
            text: text.into(),
            created_at_ms: 10,
        },
    };
    assert!(exec_rcpt(
        &mut runtime,
        "foreign-conversation",
        ApplicationCommand::AdmitConversationUserMessage {
            input: AdmitConversationUserMessage {
                conversation_id: "missing".into(),
                message_id: "message-x".into(),
                text: "hello".into(),
                created_at_ms: 11,
            },
        }
    )
    .is_err());

    let first = exec_rcpt(&mut runtime, "stable-request", admit("stable-message", "Ask experts")).unwrap();
    assert_eq!((&first["message_id"], &first["already_admitted"]), (&json!("stable-message"), &json!(false)));
    assert_eq!(
        exec_rcpt(&mut runtime, "stable-request", admit("stable-message", "Ask experts")).unwrap(),
        first
    );
    assert!(runtime
        .execute_received_command(
            ExecuteReceivedCommand {
                client_id: "chat-client".into(),
                request_id: "stable-request".into(),
                command_type: "admit_conversation_user_message".into(),
                command_json: serde_json::to_string(&admit("stable-message", "Changed text"))
                    .unwrap(),
                expected_revision: None,
                received_at: "2026-10-23T00:00:01Z".into(),
            },
            admit("stable-message", "Changed text"),
        )
        .is_err());
    let dup = exec_rcpt(
        &mut runtime,
        "new-transport-request",
        admit("stable-message", "Ask experts"),
    )
    .unwrap();
    assert_eq!(dup["already_admitted"], true);
    assert!(exec_rcpt(
        &mut runtime,
        "another-request",
        admit("stable-message", "Changed text")
    )
    .is_err());

    let loaded_conv = runtime
        .execute(ApplicationCommand::LoadConversation {
            conversation_id: conv_id.into(),
        })
        .unwrap();
    assert_eq!(loaded_conv["messages"].as_array().unwrap().len(), 1);
    assert_eq!(loaded_conv["messages"][0]["role"], "user");
    assert_eq!(loaded_conv["messages"][0]["run_id"], Value::Null);
    assert!(runtime
        .execute(ApplicationCommand::ActiveRun {
            conversation_id: conv_id.into()
        })
        .unwrap()
        .is_null());

    let assoc = exec_rcpt(
        &mut runtime,
        "association",
        ApplicationCommand::AgentRoom {
            command: AgentRoomCommand::EnsureForConversation {
                conversation_id: conv_id.into(),
                at_ms: 12,
            },
        },
    )
    .unwrap();
    let room = assoc["room"]["room"]["id"].as_str().unwrap();
    exec_rcpt(
        &mut runtime,
        "configure-expert",
        ApplicationCommand::AgentRoom {
            command: AgentRoomCommand::ConfigureAgent {
                input: nola_application::persistent_agent::ConfigureAgent {
                    id: "expert".to_string().try_into().unwrap(),
                    expected_directory_revision: None,
                    expected_agent_revision: None,
                    name: "Expert".into(),
                    description: "Inert test expert".into(),
                    defaults: nola_domain::persistent_agent::AgentDefaults::try_from(
                        json!({"language":"en"}),
                    )
                    .unwrap(),
                    availability: nola_domain::persistent_agent::AgentAvailability::Active,
                    updated_at_ms: 12,
                },
            },
        },
    )
    .unwrap();
    let joined = exec_rcpt(
        &mut runtime,
        "join-expert",
        ApplicationCommand::AgentRoom {
            command: AgentRoomCommand::Join {
                input: JoinAgent {
                    room_id: room.into(),
                    expected_room_revision: assoc["room"]["revision"].as_i64().unwrap(),
                    agent_id: "expert".to_string().try_into().unwrap(),
                    expected_agent_revision: 1,
                    at_ms: 12,
                },
            },
        },
    )
    .unwrap();
    let anchored = exec_rcpt(
        &mut runtime,
        "expert-submit",
        ApplicationCommand::AgentRoom {
            command: AgentRoomCommand::Submit {
                input: SubmitRoomMessage {
                    room_id: room.into(),
                    submission_id: uuid::Uuid::new_v4().to_string(),
                    expected_room_revision: joined["revision"].as_i64().unwrap(),
                    text: "Ask experts".into(),
                    participant_id: None,
                    at_ms: 13,
                    parent_message_id: Some("stable-message".into()),
                    conversation_id: Some(conv_id.into()),
                },
            },
        },
    )
    .unwrap();
    assert_eq!(anchored["input"]["parent_message_id"], "stable-message");

    // 3. Submission frontier read-only & exact source owner enforcement.
    let dir = tempfile::tempdir().unwrap();
    let frontier_path = dir.path().join("frontier.sqlite");
    let base_req = || SubmissionFrontierRequest {
        expected_host_id: "host".into(),
        client_id: "client".into(),
        conversation_id: "conversation".into(),
        expected_run_id: None,
        assistant_message_id: None,
    };
    let observe = |rt: &mut ApplicationRuntime, req: SubmissionFrontierRequest| -> Result<SubmissionFrontier, String> {
        serde_json::from_value(rt.execute(ApplicationCommand::ObserveSubmissionFrontier {
            source_host_id: "host".into(),
            request: req,
        })?)
        .map_err(|e| e.to_string())
    };
    let mut f_rt = support::open_runtime(&frontier_path).unwrap();
    let new_frontier = observe(&mut f_rt, base_req()).unwrap();
    assert_eq!(
        (new_frontier.configuration_revision, new_frontier.parent_message_id),
        (None, None)
    );
    let mut wrong_host = base_req();
    wrong_host.expected_host_id = "foreign".into();
    assert!(observe(&mut f_rt, wrong_host).is_err());
    let mut half_req = base_req();
    half_req.expected_run_id = Some("run".into());
    assert!(observe(&mut f_rt, half_req).is_err());
    drop(f_rt);

    let mut f_store = NolaStore::open(&frontier_path).unwrap();
    f_store
        .register_source_submission(
            &RemoteSubmissionIdentity {
                source_host_id: "host".into(),
                client_id: "client".into(),
                conversation_id: "conversation".into(),
                submission_id: "run".into(),
                user_message_id: "user".into(),
                assistant_message_id: "assistant".into(),
            },
            &json!({"input_items":[{"text":"actual source input"}]}),
        )
        .unwrap();
    f_store
        .begin_turn(&nola_store_port::run_creation::BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            conversation_id: "conversation".into(),
            conversation_title: "Remote".into(),
            user_message_id: "user".into(),
            user_part_id: "source-part".into(),
            user_text: "actual source input".into(),
            user_content_json: Some("{\"source\":true}".into()),
            additional_user_parts: vec![],
            assistant_message_id: "assistant".into(),
            assistant_part_id: "assistant:text".into(),
            run_id: "run".into(),
            context_checkpoint_id: None,
            created_at_ms: 100,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture".into(),
            outbox_effects: vec![],
            initial_assistant_part_mutations: vec![],
            assistant_turn_bindings: vec![],
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![],
        })
        .unwrap();
    drop(f_store);

    let mut f_rt = support::open_runtime(&frontier_path).unwrap();
    assert!(observe(&mut f_rt, base_req()).is_err());
    let mut owned = base_req();
    owned.expected_run_id = Some("run".into());
    owned.assistant_message_id = Some("assistant".into());
    let snap = observe(&mut f_rt, owned.clone()).unwrap();
    assert_eq!(snap.configuration_revision, Some(1));
    assert_eq!(snap.parent_message_id.as_deref(), Some("assistant"));
    let mut other_client = owned.clone();
    other_client.client_id = "another-client".into();
    assert!(observe(&mut f_rt, other_client).is_err());
    let mut other_assistant = owned.clone();
    other_assistant.assistant_message_id = Some("another-assistant".into());
    assert!(observe(&mut f_rt, other_assistant).is_err());
    assert_eq!(observe(&mut f_rt, owned).unwrap(), snap);
}

#[test]
fn transcript_import_validation_foreign_run_scope_and_journal_atomicity() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("import-suite.sqlite");
    let mut runtime = support::open_runtime(&database).unwrap();
    for id in ["validation", "target", "other", "history"] {
        runtime
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
    let db = rusqlite::Connection::open(&database).unwrap();

    // 1. Invalid imports leave no messages, parts, or revision changes ("validation").
    let valid_msg = || TranscriptMessageInput {
        id: "message".into(),
        run_id: None,
        role: "user".into(),
        status: "completed".into(),
        ordinal: 1,
        created_at_ms: 1,
        completed_at_ms: Some(2),
        parts: vec![TranscriptPartInput {
            id: "part".into(),
            kind: "text".into(),
            ordinal: 1,
            state: "completed".into(),
            content_text: "retained evidence".into(),
            content_json: None,
            content_digest: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        }],
    };
    let mut bad_cases = vec![Vec::new()];
    for mutation in 0..7 {
        let mut item = valid_msg();
        match mutation {
            0 => item.id = " padded ".into(),
            1 => item.parts[0].id = "x".repeat(129),
            2 => item.completed_at_ms = Some(0),
            3 => item.parts[0].updated_at_ms = 0,
            4 => item.parts[0].ordinal = 2,
            5 => item.parts[0].state = "streaming".into(),
            _ => {
                item.parts = (0..513)
                    .map(|idx| {
                        let mut p = item.parts[0].clone();
                        p.id = format!("part-{idx}");
                        p.ordinal = idx + 1;
                        p
                    })
                    .collect();
            }
        }
        bad_cases.push(vec![item]);
    }
    for (idx, messages) in bad_cases.into_iter().enumerate() {
        assert!(runtime
            .execute(ApplicationCommand::ImportConversationTranscript {
                input: ImportConversationTranscript {
                    import_id: format!("invalid-{idx}"),
                    source: ConversationTranscriptImportSource::ExternalArchive,
                    conversation_id: "validation".into(),
                    expected_message_count: 0,
                    messages,
                    imported_at_ms: 3,
                },
            })
            .is_err());
        let state: (i64, i64, i64) = db
            .query_row(
                "SELECT (SELECT COUNT(*) FROM messages WHERE conversation_id='validation'),
                        (SELECT COUNT(*) FROM message_parts),
                        (SELECT revision FROM conversations WHERE id='validation')",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, (0, 0, 1), "case {idx}");
    }
    runtime
        .execute(ApplicationCommand::ImportConversationTranscript {
            input: ImportConversationTranscript {
                import_id: "valid".into(),
                source: ConversationTranscriptImportSource::ExternalArchive,
                conversation_id: "validation".into(),
                expected_message_count: 0,
                messages: vec![valid_msg()],
                imported_at_ms: 3,
            },
        })
        .unwrap();

    // 2. Foreign run import & edit cannot change canonical history ("target" vs "other").
    db.execute_batch(
        "INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms,completed_at_ms)
         VALUES('foreign-run','other','conversation','running',1,1,2,NULL);",
    )
    .unwrap();
    let run_msg = |id: &str, run: &str, ordinal: i64| TranscriptMessageInput {
        id: id.into(),
        run_id: Some(run.into()),
        role: "assistant".into(),
        status: "completed".into(),
        ordinal,
        created_at_ms: 1,
        completed_at_ms: Some(2),
        parts: vec![TranscriptPartInput {
            id: format!("{id}:text"),
            kind: "text".into(),
            ordinal: 1,
            state: "completed".into(),
            content_text: "evidence".into(),
            content_json: None,
            content_digest: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        }],
    };
    let import_run = |run: &str| ApplicationCommand::ImportConversationTranscript {
        input: ImportConversationTranscript {
            import_id: format!("import-{run}"),
            source: ConversationTranscriptImportSource::ExternalArchive,
            conversation_id: "target".into(),
            expected_message_count: 0,
            messages: vec![run_msg("original", run, 1)],
            imported_at_ms: 3,
        },
    };
    assert!(runtime
        .execute(import_run("foreign-run"))
        .unwrap_err()
        .contains("another conversation"));
    runtime.execute(import_run("own-run")).unwrap();
    assert!(runtime
        .execute(ApplicationCommand::EditConversationMessage {
            input: EditConversationMessage {
                operation_id: "foreign-edit".into(),
                conversation_id: "target".into(),
                expected_revision: 2,
                parent_message_id: "original".into(),
                replacement_message: run_msg("replacement", "foreign-run", 2),
                committed_at_ms: 4,
            },
        })
        .unwrap_err()
        .contains("another conversation"));

    // 3. Projection journal failure rolls back messages, revision, and receipt ("history"),
    //    then commit + cold replay emits bounded SnapshotRequired invalidation.
    let before_snap = runtime.conversation_projection_snapshot("history").unwrap();
    let mut reducer = ConversationProjectionReducer::restore(
        before_snap.projection.unwrap(),
        before_snap.current_sequence,
    );
    let hist_cmd = ApplicationCommand::ImportConversationTranscript {
        input: ImportConversationTranscript {
            import_id: "archive".into(),
            source: ConversationTranscriptImportSource::ExternalArchive,
            conversation_id: "history".into(),
            expected_message_count: 0,
            imported_at_ms: 2,
            messages: vec![TranscriptMessageInput {
                id: "hist-msg".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(1),
                parts: vec![TranscriptPartInput {
                    id: "hist-part".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "private-history-evidence".repeat(1000),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                }],
            }],
        },
    };
    let hist_env = ExecuteReceivedCommand {
        client_id: "importer".into(),
        request_id: "import-hist".into(),
        command_type: "import_conversation_transcript".into(),
        command_json: serde_json::to_string(&hist_cmd).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    db.execute_batch(
        "CREATE TRIGGER fail_import_journal BEFORE INSERT ON projection_events
         BEGIN SELECT RAISE(ABORT,'injected journal failure'); END;",
    )
    .unwrap();
    assert!(runtime
        .execute_received_command(
            hist_env.clone(),
            serde_json::from_str(&hist_env.command_json).unwrap()
        )
        .is_err());
    let hist_state: (i64, i64, i64) = db
        .query_row(
            "SELECT (SELECT COUNT(*) FROM messages WHERE conversation_id='history'),
                    (SELECT COUNT(*) FROM command_receipts WHERE request_id='import-hist'),
                    (SELECT revision FROM conversations WHERE id='history')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(hist_state, (0, 0, 1));
    db.execute_batch("DROP TRIGGER fail_import_journal;")
        .unwrap();

    let committed = runtime
        .execute_received_command(
            hist_env.clone(),
            serde_json::from_str(&hist_env.command_json).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        committed.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    let suffix = runtime
        .projection_event_suffix(
            ApplicationProjectionScope {
                scope_type: "conversation".into(),
                scope_id: "history".into(),
            },
            0,
            10,
        )
        .unwrap();
    assert_eq!(suffix.events.len(), 1);
    let event = suffix.events[0].event.clone();
    let encoded = serde_json::to_string(&event).unwrap();
    assert!(encoded.len() < 1024 && !encoded.contains("private-history-evidence"));
    let unchanged = reducer.projection().clone();
    assert_eq!(
        reducer.apply(event),
        Err(ProjectionApplyError::SnapshotRequired { revision: 2 })
    );
    assert_eq!(reducer.projection(), &unchanged);
    drop(runtime);

    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(runtime
        .execute_received_command(hist_env, hist_cmd)
        .unwrap()
        .replayed);
    assert_eq!(
        runtime
            .execute(ApplicationCommand::ReadConversationTextRange {
                conversation_id: "validation".into(),
                message_id: "message".into(),
                part_id: "part".into(),
                offset_bytes: 0,
                limit_bytes: 32,
            })
            .unwrap()["text"],
        "retained evidence"
    );
    assert_eq!(
        runtime
            .execute(ApplicationCommand::ReadConversationTextRange {
                conversation_id: "target".into(),
                message_id: "original".into(),
                part_id: "original:text".into(),
                offset_bytes: 0,
                limit_bytes: 32,
            })
            .unwrap()["text"],
        "evidence"
    );
    let restored = runtime.conversation_projection_snapshot("history").unwrap();
    assert_eq!(restored.projection.unwrap().messages[0].message_id.0, "hist-msg");
}

struct InjectedUploadFixture(Arc<AtomicUsize>);
impl ContentUploadRepository for InjectedUploadFixture {
    type Error = String;
    fn begin(&mut self, _: &str, _: &UploadOwner, _: u64) -> Result<UploadProgress, String> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err("fixture refuses mutation".into())
    }
    fn progress(&self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, String> {
        assert_eq!((id, owner.conversation_id.as_str()), ("u", "c"));
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(UploadProgress {
            expected_bytes: 12,
            committed_bytes: 4,
            sealed: false,
            cancelled: false,
        })
    }
    fn append(
        &mut self,
        _: &str,
        _: &UploadOwner,
        _: u64,
        _: &str,
    ) -> Result<UploadProgress, String> {
        panic!("unexpected append")
    }
    fn seal(&mut self, _: &str, _: &UploadOwner) -> Result<UploadProgress, String> {
        panic!("unexpected seal")
    }
    fn cancel(&mut self, _: &str, _: &UploadOwner) -> Result<UploadProgress, String> {
        panic!("unexpected cancel")
    }
    fn read_chunk(&self, _: &str, _: &UploadOwner, _: u64) -> Result<Option<String>, String> {
        panic!("unexpected content read")
    }
}

fn uploaded_manifest(bytes: u64) -> UploadedTranscriptManifest {
    serde_json::from_value(json!({
        "expected_conversation_revision": 1,
        "transcript": {
            "import_id": "i",
            "source": "backup_restore",
            "conversation_id": "c",
            "expected_message_count": 0,
            "imported_at_ms": 2,
            "messages": [{
                "id": "m", "run_id": null, "role": "user", "status": "completed",
                "ordinal": 1, "created_at_ms": 1, "completed_at_ms": 2,
                "parts": [{
                    "id": "p", "kind": "text", "ordinal": 1, "state": "completed",
                    "content_text": "", "content_json": null, "content_digest": null,
                    "created_at_ms": 1, "updated_at_ms": 2
                }]
            }]
        },
        "uploads": [{"upload_id": "u", "message_id": "m", "part_id": "p", "expected_utf8_bytes": bytes}]
    }))
    .unwrap()
}

#[test]
fn upload_repository_injection_and_ten_million_byte_transcript_preparation_and_publication() {
    // 1. Injected non-SQLite ContentUploadRepository is used without creating .uploads.sqlite.
    let inj_dir = tempfile::tempdir().unwrap();
    let inj_path = inj_dir.path().join("nola.db");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut inj_rt = ApplicationRuntime::from_store(
        NolaStore::open(&inj_path).unwrap(),
        Box::new(InjectedUploadFixture(calls.clone())),
    );
    let req_owner = || ContentUploadOwner {
        conversation_id: "c".into(),
        import_id: "i".into(),
        part_id: "p".into(),
    };
    let prog_cmd = || ApplicationCommand::ContentUploadProgress {
        input: ContentUploadRequest {
            upload_id: "u".into(),
            owner: req_owner(),
        },
    };
    assert_eq!(
        inj_rt.execute(prog_cmd()).unwrap_err(),
        "content upload owner has neither a live conversation nor an active archive profile"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    inj_rt
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "c".into(),
                title: "c".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    let prog = inj_rt.execute(prog_cmd()).unwrap();
    assert_eq!((&prog["committed_utf8_bytes"], &prog["expected_utf8_bytes"]), (&json!(4), &json!(12)));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let begin_up = |size| ApplicationCommand::BeginContentUpload {
        input: BeginContentUpload {
            upload_id: "u".into(),
            owner: req_owner(),
            expected_utf8_bytes: size,
        },
    };
    assert!(inj_rt.execute(begin_up(0)).is_err());
    assert!(inj_rt
        .execute(begin_up(nola_application::MAXIMUM_CONTENT_UPLOAD_BYTES + 1))
        .is_err());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        inj_rt.execute(begin_up(12)).unwrap_err(),
        "fixture refuses mutation"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert!(!inj_dir.path().join("nola.db.uploads.sqlite").exists());

    // 2. Single 10,000,000-byte multi-byte upload shared across BOTH preparation & publication!
    let directory = tempfile::tempdir().unwrap();
    let upload_path = directory.path().join("uploads.db");
    let canonical_path = directory.path().join("text.db");
    let main_path = directory.path().join("main.db");
    let owner = UploadOwner {
        conversation_id: "c".into(),
        import_id: "i".into(),
        part_id: "p".into(),
    };
    let bytes = 10_000_000;
    let mut uploads = ContentUploadStore::open(&upload_path, bytes).unwrap();
    uploads.begin("u", &owner, bytes).unwrap();
    let mut offset = 0;
    while offset < bytes {
        let length = (bytes - offset).min(32768) as usize;
        uploads
            .append("u", &owner, offset, &"\u{00e9}".repeat(length / 2))
            .unwrap();
        offset += length as u64;
    }
    let canonical = CanonicalTextStore::open(&canonical_path, bytes).unwrap();
    let mut prep_repo = TranscriptContentPreparationStore::new(uploads, canonical);
    assert!(prepare_uploaded_transcript(&mut prep_repo, &uploaded_manifest(bytes), bytes).is_err());
    drop(prep_repo);

    let mut uploads = ContentUploadStore::open_existing(&upload_path, bytes).unwrap();
    uploads.seal("u", &owner).unwrap();
    let canonical = CanonicalTextStore::open(&canonical_path, bytes).unwrap();
    let mut prep_repo = TranscriptContentPreparationStore::new(uploads, canonical);
    assert!(
        prepare_uploaded_transcript(&mut prep_repo, &uploaded_manifest(bytes - 1), bytes).is_err()
    );
    let references =
        prepare_uploaded_transcript(&mut prep_repo, &uploaded_manifest(bytes), bytes).unwrap();
    assert_eq!(
        prepare_uploaded_transcript(&mut prep_repo, &uploaded_manifest(bytes), bytes).unwrap(),
        references
    );
    let mut redirected = uploaded_manifest(bytes);
    redirected.transcript.messages[0].id = "other-message".into();
    redirected.uploads[0].message_id = "other-message".into();
    assert!(prepare_uploaded_transcript(&mut prep_repo, &redirected, bytes).is_err());
    let mut foreign = uploaded_manifest(bytes);
    foreign.transcript.import_id = "foreign".into();
    assert!(prepare_uploaded_transcript(&mut prep_repo, &foreign, bytes).is_err());
    drop(prep_repo);

    let canonical = CanonicalTextStore::open_existing(&canonical_path, bytes).unwrap();
    let tail = canonical
        .read_range(&references[0], bytes - 128, 128)
        .unwrap();
    assert_eq!(tail.text, "\u{00e9}".repeat(64));
    assert!(tail.end_of_content && tail.next_offset_bytes == bytes);
    assert!(serde_json::to_vec(&references).unwrap().len() < 1024);

    // Publish the exact same 10MB sealed upload into a NolaStore conversation and verify restart + replay.
    let uploads = ContentUploadStore::open_existing(&upload_path, bytes).unwrap();
    let mut store = NolaStore::open(&main_path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Large original".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    let pub_res =
        publish_uploaded_transcript(&mut store, &uploads, &uploaded_manifest(bytes), bytes)
            .unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(pub_res.message_count, 1);
    drop(store);

    let mut store = NolaStore::open(&main_path).unwrap();
    let pub_tail = store
        .read_conversation_text_range("c", "m", "p", bytes - 128, 128)
        .unwrap();
    assert_eq!(pub_tail.text, "\u{00e9}".repeat(64));
    assert_eq!(pub_tail.total_utf8_bytes, bytes);
    assert!(pub_tail.end_of_content);
    let window = store
        .load_conversation_message_window("c", None, 10, 1024)
        .unwrap()
        .unwrap();
    assert!(window.conversation.messages[0].parts[0].content_text.len() <= 1024);
    store.begin_write_unit_of_work().unwrap();
    assert_eq!(
        publish_uploaded_transcript(&mut store, &uploads, &uploaded_manifest(bytes), bytes)
            .unwrap(),
        pub_res
    );
    store.commit_write_unit_of_work().unwrap();
    let mut changed = uploaded_manifest(bytes);
    changed.transcript.messages[0].parts[0].created_at_ms = 0;
    assert!(publish_uploaded_transcript(&mut store, &uploads, &changed, bytes).is_err());
}
