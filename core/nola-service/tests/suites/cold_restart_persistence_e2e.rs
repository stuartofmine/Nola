// SPDX-License-Identifier: Apache-2.0
//! Multi-domain cold-restart persistence suite consolidating 9 single-domain service restart E2Es
//! (`agent_command`, `context_usage`, `conversation_plan`, `document_source`, `model_runtime`,
//! `permission_request`, `platform_state`, `product_setting`, and `trace_journal`) onto one
//! shared database across 4 service process generations.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::support;
use nola_application::{
    AgentCommand, AgentCommandLease, ApplicationCommand, ConversationPlan, ConversationPlanCommand,
    ConversationPlanSopIdentity, ConversationPlanStepStatus, CreateRuntimeTask, DocumentSource,
    DocumentSourceCommand, DocumentSourceSummary, ModelCommand, ModelProfile, ModelRuntime,
    ModelRuntimePhase, PermissionIntervention, PermissionRequest, PermissionRequestCommand,
    PermissionRequestResolution, PermissionRequestStatus, ProductSetting, RuntimeTask,
    RuntimeTaskCommand, RuntimeTaskPriority, RuntimeTaskResourceBudget, RuntimeTaskScheduling,
    SessionRunLease, TraceRecord, TraceRun, WorkspaceCheckpoint,
};
use nola_core::agent_trace::{
    AgentTraceEvent, AgentTraceKind, AgentTracePayload, AgentTraceStatus, AgentTraceVisibility,
};
use nola_service_client::ApplicationServiceClient;
use serde_json::{json, Value};

struct OwnedService(Child);

impl Drop for OwnedService {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_ready_service(
    database: &Path,
    socket: &Path,
    client: &ApplicationServiceClient,
) -> OwnedService {
    let _ = std::fs::remove_file(socket);
    let mut service = OwnedService(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .args(["--database"])
            .arg(database)
            .args(["--socket"])
            .arg(socket)
            .args(["--parent-pid", &std::process::id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start real Application Service"),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return service;
        }
        assert!(
            service.0.try_wait().expect("inspect service").is_none(),
            "Application Service exited before readiness"
        );
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

fn sop() -> ConversationPlanSopIdentity {
    ConversationPlanSopIdentity {
        run_id: "service-run-1".to_string(),
        sop_id: "test.service.plan".to_string(),
        sop_version: 1,
    }
}

fn plan_steps(status: &str) -> Value {
    json!([{
        "id": "inspect",
        "content": "Inspect",
        "activeForm": "Inspecting",
        "status": status,
        "priority": "medium",
        "dependencyIds": [],
        "approvalStatus": "not_required",
        "capabilityRefs": ["builtin:read_file"],
        "expectedArtifactRefs": [],
        "successCriteria": ["Inspection completes"],
        "evidenceRefs": if status == "completed" { json!(["evidence:inspection"]) } else { json!([]) },
        "verificationStatus": if status == "completed" { "passed" } else { "pending" },
        "terminationReason": if status == "completed" { Value::String("success".to_string()) } else { Value::Null }
    }])
}

fn model_profile(id: &str, selected: bool, revision: i64, now: i64) -> ModelProfile {
    ModelProfile {
        id: id.to_string(),
        display_name: id.to_string(),
        provider: "local".to_string(),
        model_name: id.to_string(),
        runtime: "llama.cpp".to_string(),
        source_json: r#"{"repository":"publisher/model","revision":"immutable"}"#.to_string(),
        parameters_json: r#"{"contextTokens":65536,"temperature":0.7}"#.to_string(),
        selected,
        enabled: true,
        revision,
        created_at_ms: now,
        updated_at_ms: now,
        deleted_at_ms: None,
    }
}

fn permission_task() -> RuntimeTask {
    let mut task = RuntimeTask {
        id: "permission-task".into(),
        kind: "agent".into(),
        status: "queued".into(),
        title: "Create report".into(),
        project_root: "/tmp/project".into(),
        owner_session_id: Some("conversation-permission".into()),
        parent_task_id: None,
        parent_tool_call_id: Some("permission-parent-call".into()),
        child_session_id: Some("child-permission".into()),
        agent_type: Some("general".into()),
        prompt: None,
        progress_summary: Some("queued".into()),
        last_activity: None,
        result_summary: None,
        error: None,
        output_path: None,
        unread: true,
        cancel_requested: false,
        cancel_requested_at_ms: None,
        worker_id: None,
        ownership_token: None,
        lease_until_ms: None,
        heartbeat_at_ms: None,
        retry_count: 0,
        retry_at_ms: None,
        last_error: None,
        input_watermark: Some(7),
        completion_watermark: None,
        created_at_ms: 10,
        started_at_ms: None,
        ended_at_ms: None,
        updated_at_ms: 10,
        metadata_json: "{}".into(),
    };
    task.set_scheduling(RuntimeTaskScheduling {
        priority: RuntimeTaskPriority::Normal,
        queued_at_ms: task.created_at_ms,
        resource_budget: RuntimeTaskResourceBudget {
            max_wall_time_ms: Some(60_000),
            max_output_bytes: Some(1_048_576),
            ..Default::default()
        },
        output_bytes: 0,
        resource_usage: Default::default(),
        budget_exhaustion: None,
    })
    .expect("valid permission task scheduling");
    task
}

fn trace_event(sequence: u64, kind: AgentTraceKind, status: AgentTraceStatus) -> AgentTraceEvent {
    AgentTraceEvent {
        schema_version: 1,
        trace_id: "trace-cold".into(),
        run_id: "run-cold".into(),
        conversation_id: "conversation-cold".into(),
        message_id: Some("message-cold".into()),
        turn_id: "run-cold".into(),
        event_id: format!("event-{sequence}"),
        sequence,
        emitted_at: format!("2026-01-01T00:00:0{sequence}Z"),
        kind,
        span_id: None,
        parent_span_id: None,
        status,
        visibility: AgentTraceVisibility::Summary,
        payload: AgentTracePayload::default(),
    }
}

#[test]
fn all_service_domains_survive_multi_generation_cold_restarts() {
    let directory = tempfile::tempdir().expect("temporary state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");

    // Pre-service Store fixtures for context_usage ("usage") and trace_journal ("conversation-cold").
    let mut store = nola_store::NolaStore::open(&database).unwrap();
    support::committed_usage::seed(&mut store, 8192);
    let sql = rusqlite::Connection::open(&database).unwrap();
    sql.execute(
        "UPDATE core_checkpoint_bodies SET checkpoint_json=json_set(checkpoint_json,'$.unrelated_history',?1) WHERE conversation_id='usage'",
        ["x".repeat(10_000_000)],
    )
    .unwrap();
    drop(sql);
    let original_usage_checkpoint = store.load_checkpoint("usage").unwrap().unwrap();

    let cold_message = String::from("message-cold");
    let cold_part = String::from("assistant-part-cold");
    store
        .begin_turn(&nola_store::BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            conversation_id: "conversation-cold".into(),
            conversation_title: "Journal planning".into(),
            user_message_id: "user-cold".into(),
            user_part_id: "user-part-cold".into(),
            user_text: "继续 / continue".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: cold_message.clone(),
            assistant_part_id: cold_part.clone(),
            initial_assistant_part_mutations: vec![],
            assistant_turn_bindings: vec![
                nola_domain::AssistantTurnBinding::Admission {
                    turn_id: "run-cold".into(),
                    assistant_message_id: cold_message.clone(),
                },
                nola_domain::AssistantTurnBinding::Part {
                    turn_id: "run-cold".into(),
                    assistant_message_id: cold_message,
                    part_id: cold_part,
                },
            ],
            run_id: "run-cold".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture".into(),
            outbox_effects: vec![],
            model_attempt_mutations: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            memory_usage_entries: vec![],
            created_at_ms: 1,
        })
        .unwrap();
    drop(store);

    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let other_client = ApplicationServiceClient::with_socket(&database, &socket);
    let query_usage = || -> Value {
        client
            .execute(ApplicationCommand::LoadConversationContextUsageV1 {
                conversation_id: "usage".into(),
            })
            .unwrap()
    };
    let execute_model =
        |command| client.execute::<Value, _>(ApplicationCommand::Model { command });

    // ==================== GENERATION 1 ====================
    let first = start_ready_service(&database, &socket, &client);

    // 1. Context usage query before restart.
    let initial_usage = query_usage();
    assert_eq!(initial_usage["used_tokens"], 8_192);
    assert_eq!(initial_usage["context_window_tokens"], 65_536);
    assert_eq!(initial_usage["prompt_token_limit"], 57_344);
    assert_eq!(initial_usage["estimated"], false);
    assert!(initial_usage.to_string().len() < 1024);

    // 2. Product setting write (rev 1).
    let initial_setting: ProductSetting = client
        .execute(ApplicationCommand::PutProductSetting {
            key: "web.agent.settings".into(),
            value_json: r#"{"model":"first"}"#.into(),
            updated_at_ms: 10,
        })
        .unwrap();
    assert_eq!(initial_setting.revision, 1);

    // 3. Platform state: workspace checkpoint + owner-safe session run lease.
    let saved_cp: bool = client
        .execute(ApplicationCommand::SaveWorkspaceCheckpoint {
            input: WorkspaceCheckpoint {
                id: "cp-1".into(),
                root: "/workspace".into(),
                git_root: "/workspace".into(),
                revision: "abc123".into(),
                source: "snapshot".into(),
                label: "before edit".into(),
                created_at_ms: 10,
                status: "dirty".into(),
                untracked: vec!["notes.txt".into()],
            },
        })
        .unwrap();
    assert!(saved_cp);
    let acquired_lease: bool = client
        .execute(ApplicationCommand::AcquireSessionRunLease {
            input: SessionRunLease {
                session_id: "conversation-1".into(),
                owner: "owner-a".into(),
                pid: 42,
                process_start: "100".into(),
                acquired_at_ms: 20,
            },
        })
        .unwrap();
    assert!(acquired_lease);

    // 4. Conversation plan save & CAS update.
    let first_plan: ConversationPlan = client
        .execute(ApplicationCommand::ConversationPlan {
            command: ConversationPlanCommand::Save {
                expected_revision: None,
                conversation_id: "conversation-plan".to_string(),
                sop: sop(),
                todos: plan_steps("in_progress"),
                updated_at_ms: 100,
            },
        })
        .expect("save plan");
    assert_eq!(first_plan.revision, 1);
    let second_plan: ConversationPlan = client
        .execute(ApplicationCommand::ConversationPlan {
            command: ConversationPlanCommand::Save {
                expected_revision: Some(first_plan.revision),
                conversation_id: "conversation-plan".to_string(),
                sop: sop(),
                todos: plan_steps("completed"),
                updated_at_ms: 200,
            },
        })
        .expect("update plan");
    assert_eq!(second_plan.revision, 2);

    // 5. Document source replace.
    let doc_saved: usize = client
        .execute(ApplicationCommand::DocumentSource {
            command: DocumentSourceCommand::Replace {
                scope: "web:local".into(),
                kind: "uploaded_document".into(),
                inputs: vec![DocumentSource {
                    id: "web-doc-report".into(),
                    scope: "web:local".into(),
                    uri: "file:///report.txt".into(),
                    kind: "uploaded_document".into(),
                    mime_type: Some("text/plain".into()),
                    sha256: None,
                    byte_count: Some(25),
                    title: Some("Quarterly report".into()),
                    text_content: Some("revenue grew through cloud".into()),
                    metadata: json!({"id": "report", "name": "Quarterly report"}),
                    created_at_ms: 100,
                    updated_at_ms: 100,
                }],
            },
        })
        .expect("replace document sources");
    assert_eq!(doc_saved, 1);

    // 6. Trace journal started event.
    let trace_inserted: bool = client
        .execute(ApplicationCommand::RecordAgentTrace {
            event: trace_event(1, AgentTraceKind::RunStarted, AgentTraceStatus::Running),
            envelope: Some(json!({"type":"run.started","visible":true})),
        })
        .expect("record started trace");
    assert!(trace_inserted);

    // 7. Agent command enqueue, dedup, claim, and active-lease contention.
    let cmd_input = AgentCommand {
        session_id: "conversation-command".into(),
        command_id: "command-1".into(),
        payload_json: serde_json::to_string(&nola_core::agent::AgentSubmission::Interrupt {
            goal_control: None,
            cancellation_id: "cancel-command-1".into(),
            reason: Some("stop requested".into()),
        })
        .unwrap(),
        created_at_ms: 10,
    };
    let cmd_inserted: bool = client
        .execute(ApplicationCommand::EnqueueAgentCommand {
            input: cmd_input.clone(),
        })
        .expect("enqueue command");
    assert!(cmd_inserted);
    let cmd_duplicate: bool = client
        .execute(ApplicationCommand::EnqueueAgentCommand { input: cmd_input })
        .expect("deduplicate command");
    assert!(!cmd_duplicate);
    let claimed_cmd: Option<AgentCommandLease> = client
        .execute(ApplicationCommand::ClaimNextAgentCommand {
            session_id: "conversation-command".into(),
            lease_owner: "consumer-a".into(),
            now_ms: 20,
            lease_until_ms: 120,
        })
        .expect("claim command");
    let claimed_cmd = claimed_cmd.unwrap();
    assert_eq!(claimed_cmd.delivery_attempt, 1);
    let original_cmd_payload = claimed_cmd.payload_json;
    let original_boundary: nola_core::agent::host::AgentBoundaryInput =
        serde_json::from_str(&original_cmd_payload).unwrap();
    assert!(matches!(
        original_boundary,
        nola_core::agent::host::AgentBoundaryInput::Submission {
            submission: nola_core::agent::AgentSubmission::Interrupt {
                goal_control: None,
                cancellation_id,
                reason
            }
        } if cancellation_id == "cancel-command-1" && reason.as_deref() == Some("stop requested")
    ));
    let unavailable_cmd: Option<AgentCommandLease> = client
        .execute(ApplicationCommand::ClaimNextAgentCommand {
            session_id: "conversation-command".into(),
            lease_owner: "consumer-b".into(),
            now_ms: 30,
            lease_until_ms: 130,
        })
        .expect("honor active lease");
    assert!(unavailable_cmd.is_none());

    // 8. Model parameter preservation & rejection invariants.
    let stale_model = model_profile("model", false, 0, 100);
    execute_model(ModelCommand::UpsertProfile {
        input: stale_model.clone(),
    })
    .unwrap();
    let mut current_model = stale_model.clone();
    current_model.selected = true;
    current_model.display_name = "Updated by another client".into();
    current_model.source_json = r#"{"repository":"canonical/latest"}"#.into();
    current_model.updated_at_ms = 200;
    let _: Value = other_client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile {
                input: current_model.clone(),
            },
        })
        .unwrap();
    let mut edit_model = stale_model;
    edit_model.parameters_json = r#"{"contextTokens":131072,"temperature":0.42}"#.into();
    edit_model.updated_at_ms = 300;
    execute_model(ModelCommand::ConfigureParameters {
        input: edit_model.clone(),
    })
    .unwrap();
    let configured_model: ModelProfile = serde_json::from_value(
        execute_model(ModelCommand::GetProfile { id: "model".into() }).unwrap(),
    )
    .unwrap();
    assert!(configured_model.selected);
    assert_eq!(configured_model.source_json, current_model.source_json);
    assert_eq!(configured_model.display_name, current_model.display_name);
    assert_eq!(configured_model.created_at_ms, 100);
    assert_eq!(configured_model.parameters_json, edit_model.parameters_json);
    assert!(configured_model.revision > current_model.revision);
    let model_snapshot = execute_model(ModelCommand::ListProfiles).unwrap();
    assert!(execute_model(ModelCommand::ResetParameters {
        profile_ids: vec!["model".into(), "missing".into()],
        parameters_json: "{}".into(),
        updated_at_ms: 400,
    })
    .is_err());
    assert_eq!(execute_model(ModelCommand::ListProfiles).unwrap(), model_snapshot);
    let mut foreign_model = model_profile("model", true, 0, 500);
    foreign_model.provider = "other-provider".into();
    assert!(execute_model(ModelCommand::UpsertProfile {
        input: foreign_model
    })
    .is_err());
    assert_eq!(execute_model(ModelCommand::ListProfiles).unwrap(), model_snapshot);

    // 9. Permission task admission, claim, and atomic suspension.
    let _: Value = client
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: CreateRuntimeTask::try_from(permission_task()).unwrap(),
            },
        })
        .unwrap();
    let prepared_claim: Option<nola_application::RuntimeTaskAttempt> = client
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Claim {
                task_id: "permission-task".into(),
                worker_id: "permission-worker".into(),
                worker_generation: "permission-worker".into(),
                lease_token: "permission-lease".into(),
                now_ms: 11,
                lease_until_ms: 10_000,
            },
        })
        .unwrap();
    assert!(
        prepared_claim.is_none(),
        "a Prepared agent cannot claim before actual source admission"
    );
    let source_directory = tempfile::tempdir().unwrap();
    let mut source = support::open_runtime(source_directory.path().join("source.sqlite")).unwrap();
    let admission = support::agent_task_admission::source_fact_for(
        &mut source,
        "permission-task",
        "conversation-permission",
        "child-permission",
        "permission-parent-call",
        Some(7),
    );
    let authority = client
        .import_agent_task_admission_with_request_id("permission-task-admission", &admission)
        .unwrap();
    assert_eq!(authority.admission.as_ref(), Some(&admission));
    let attempt: nola_application::RuntimeTaskAttempt = client
        .execute::<Option<nola_application::RuntimeTaskAttempt>, _>(
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Claim {
                    task_id: "permission-task".into(),
                    worker_id: "permission-worker".into(),
                    worker_generation: "permission-worker".into(),
                    lease_token: "permission-lease".into(),
                    now_ms: 11,
                    lease_until_ms: 10_000,
                },
            },
        )
        .unwrap()
        .expect("permission claim grant");
    let suspension: nola_application::RuntimeTaskPermissionSuspension = client
        .execute::<Option<nola_application::RuntimeTaskPermissionSuspension>, _>(
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::SuspendForPermission {
                    input: nola_application::SuspendRuntimeTaskForPermission {
                        task_id: attempt.task_id.clone(),
                        attempt_id: attempt.id.clone(),
                        expected_attempt_revision: attempt.revision,
                        worker_generation: attempt.worker_generation.clone(),
                        lease_token: attempt.lease_token.clone(),
                        current_expiry_ms: attempt.lease_expires_at_ms,
                        permission_request: nola_store::StoredPermissionRequest {
                            closure: None,
                            request_id: "permission-1".into(),
                            task_id: "permission-task".into(),
                            child_session_id: "child-permission".into(),
                            capability: "file_write".into(),
                            input_json: json!({"path": "/tmp/project/report.md"}).to_string(),
                            scope_rule: "file_write:/tmp/project/report.md".into(),
                            status: "pending".into(),
                            decision: None,
                            custom_response: None,
                            created_at_ms: 20,
                            resolved_at_ms: None,
                            core_permission_request_id: Some("core-fixture-request".into()),
                            permission_instructions_json: "[]".into(),
                        },
                        continuation_id: "permission-continuation".into(),
                        continuation_json: json!({
                            "child_session_id": "child-permission",
                            "core_permission_request_id": "core-fixture-request",
                            "run_id": "permission-run"
                        })
                        .to_string(),
                        progress_summary: "Permission required".into(),
                        last_activity: Some("file_write".into()),
                        now_ms: 20,
                    },
                },
            },
        )
        .unwrap()
        .expect("suspension must atomically create the request and continuation");

    // ==================== COLD RESTART #1 -> GENERATION 2 ====================
    drop(first);
    let second = start_ready_service(&database, &socket, &client);

    // 1. Context usage survived restart unchanged.
    assert_eq!(query_usage(), initial_usage);

    // 2. Product setting restored, updated to rev 2, then deleted.
    let restored_setting: Option<ProductSetting> = client
        .execute(ApplicationCommand::GetProductSetting {
            key: "web.agent.settings".into(),
        })
        .unwrap();
    assert_eq!(
        restored_setting.unwrap().value_json,
        r#"{"model":"first"}"#
    );
    let revised_setting: ProductSetting = client
        .execute(ApplicationCommand::PutProductSetting {
            key: "web.agent.settings".into(),
            value_json: r#"{"model":"second"}"#.into(),
            updated_at_ms: 20,
        })
        .unwrap();
    assert_eq!(revised_setting.revision, 2);
    let deleted_setting: bool = client
        .execute(ApplicationCommand::DeleteProductSetting {
            key: "web.agent.settings".into(),
        })
        .unwrap();
    assert!(deleted_setting);

    // 3. Platform state restored; wrong-owner release rejected; owner release + checkpoint delete succeed.
    let checkpoints: Vec<WorkspaceCheckpoint> = client
        .execute(ApplicationCommand::ListWorkspaceCheckpoints {
            root: "/workspace".into(),
        })
        .unwrap();
    assert_eq!(checkpoints[0].revision, "abc123");
    let restored_lease: Option<SessionRunLease> = client
        .execute(ApplicationCommand::GetSessionRunLease {
            session_id: "conversation-1".into(),
        })
        .unwrap();
    assert_eq!(restored_lease.unwrap().owner, "owner-a");
    let wrong_release: bool = client
        .execute(ApplicationCommand::ReleaseSessionRunLease {
            session_id: "conversation-1".into(),
            owner: "owner-b".into(),
        })
        .unwrap();
    assert!(!wrong_release);
    let right_release: bool = client
        .execute(ApplicationCommand::ReleaseSessionRunLease {
            session_id: "conversation-1".into(),
            owner: "owner-a".into(),
        })
        .unwrap();
    assert!(right_release);
    let deleted_cp: bool = client
        .execute(ApplicationCommand::DeleteWorkspaceCheckpoint { id: "cp-1".into() })
        .unwrap();
    assert!(deleted_cp);

    // 4. Conversation plan restored at rev 2 Completed.
    let restored_plan: Option<ConversationPlan> = client
        .execute(ApplicationCommand::ConversationPlan {
            command: ConversationPlanCommand::Get {
                conversation_id: "conversation-plan".to_string(),
            },
        })
        .unwrap();
    let restored_plan = restored_plan.expect("stored plan");
    assert_eq!(restored_plan.revision, 2);
    assert_eq!(
        restored_plan.todos[0].status,
        ConversationPlanStepStatus::Completed
    );
    assert_eq!(restored_plan.sop, sop());

    // 5. Document source search survives restart.
    let doc_matches: Vec<DocumentSourceSummary> = client
        .execute(ApplicationCommand::DocumentSource {
            command: DocumentSourceCommand::Search {
                scope: "web:local".into(),
                query: Some("cloud".into()),
                kind: Some("uploaded_document".into()),
                limit: 20,
            },
        })
        .unwrap();
    assert_eq!(doc_matches.len(), 1);
    assert_eq!(doc_matches[0].title.as_deref(), Some("Quarterly report"));

    // 6. Trace started event restored; record terminal RunCompleted event.
    let restored_traces: Vec<TraceRecord> = client
        .execute(ApplicationCommand::TraceEventsAfter {
            trace_id: "trace-cold".into(),
            sequence: 0,
        })
        .unwrap();
    assert_eq!(restored_traces.len(), 1);
    assert!(restored_traces[0].envelope.is_some());
    let completed_trace: bool = client
        .execute(ApplicationCommand::RecordAgentTrace {
            event: trace_event(2, AgentTraceKind::RunCompleted, AgentTraceStatus::Succeeded),
            envelope: None,
        })
        .unwrap();
    assert!(completed_trace);

    // 7. Agent command expired lease replays at attempt 2, then acknowledges.
    let replayed_cmd: AgentCommandLease = client
        .execute::<Option<AgentCommandLease>, _>(ApplicationCommand::ClaimNextAgentCommand {
            session_id: "conversation-command".into(),
            lease_owner: "consumer-b".into(),
            now_ms: 121,
            lease_until_ms: 221,
        })
        .unwrap()
        .unwrap();
    assert_eq!(replayed_cmd.delivery_attempt, 2);
    assert_eq!(
        replayed_cmd.payload_json, original_cmd_payload,
        "restart must preserve the original cancellation identity and payload"
    );
    let acked_cmd: bool = client
        .execute(ApplicationCommand::AcknowledgeAgentCommand {
            session_id: "conversation-command".into(),
            command_id: "command-1".into(),
            lease_owner: "consumer-b".into(),
        })
        .unwrap();
    assert!(acked_cmd);

    // 8. Model profile snapshot survived restart; delete "model", then verify exclusive selection + runtime report.
    assert_eq!(execute_model(ModelCommand::ListProfiles).unwrap(), model_snapshot);
    execute_model(ModelCommand::DeleteProfile {
        id: "model".into(),
        deleted_at_ms: 600,
    })
    .unwrap();
    assert!(execute_model(ModelCommand::ConfigureParameters { input: edit_model }).is_err());
    assert_eq!(execute_model(ModelCommand::ListProfiles).unwrap(), json!([]));

    execute_model(ModelCommand::UpsertProfile {
        input: model_profile("model-small", true, 1, 100),
    })
    .unwrap();
    execute_model(ModelCommand::UpsertProfile {
        input: model_profile("model-large", true, 1, 200),
    })
    .unwrap();
    let profiles: Vec<ModelProfile> = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::ListProfiles,
        })
        .unwrap();
    assert_eq!(profiles.iter().filter(|p| p.selected).count(), 1);
    assert_eq!(profiles[0].id, "model-large");
    execute_model(ModelCommand::ReportRuntime {
        input: ModelRuntime {
            host_id: "host-a".to_string(),
            profile_id: "model-large".to_string(),
            phase: ModelRuntimePhase::Ready,
            endpoint_json: Some(r#"{"baseUrl":"http://127.0.0.1:43123/v1"}"#.to_string()),
            process_id: Some(1234),
            error: None,
            started_at_ms: Some(300),
            stopped_at_ms: None,
            updated_at_ms: 300,
        },
    })
    .unwrap();

    // 9. Pending permission restored; intervene with custom instruction.
    let pending_perm: Option<PermissionRequest> = client
        .execute(ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::PendingForTask {
                task_id: "permission-task".into(),
            },
        })
        .unwrap();
    assert_eq!(pending_perm.unwrap().request_id, "permission-1");
    let resolved_perm: nola_application::PermissionResolutionOutcome = client
        .execute(ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Intervene {
                request_id: "permission-1".into(),
                intervention: PermissionIntervention::ProvideInstruction {
                    instruction: "Write under /tmp/project/drafts instead".into(),
                },
                resolved_at_ms: 30,
            },
        })
        .unwrap();
    assert!(matches!(
        resolved_perm,
        nola_application::PermissionResolutionOutcome::Resolved { .. }
    ));

    // ==================== COLD RESTART #2 -> GENERATION 3 ====================
    drop(second);
    let third = start_ready_service(&database, &socket, &client);

    // Product setting deletion persisted.
    let missing_setting: Option<ProductSetting> = client
        .execute(ApplicationCommand::GetProductSetting {
            key: "web.agent.settings".into(),
        })
        .unwrap();
    assert!(missing_setting.is_none());

    // Platform checkpoint & lease deletions persisted.
    let empty_checkpoints: Vec<WorkspaceCheckpoint> = client
        .execute(ApplicationCommand::ListWorkspaceCheckpoints {
            root: "/workspace".into(),
        })
        .unwrap();
    let empty_lease: Option<SessionRunLease> = client
        .execute(ApplicationCommand::GetSessionRunLease {
            session_id: "conversation-1".into(),
        })
        .unwrap();
    assert!(empty_checkpoints.is_empty());
    assert!(empty_lease.is_none());

    // Trace terminal run persisted.
    let trace_runs: Vec<TraceRun> = client
        .execute(ApplicationCommand::TraceRunsForConversation {
            conversation_id: "conversation-cold".into(),
        })
        .unwrap();
    assert_eq!(trace_runs.len(), 1);
    assert_eq!(trace_runs[0].status, "succeeded");
    assert_eq!(trace_runs[0].last_sequence, 2);
    assert!(trace_runs[0].finished_at.is_some());

    // Agent command acknowledgement persisted.
    let gone_cmd: Option<AgentCommandLease> = client
        .execute(ApplicationCommand::ClaimNextAgentCommand {
            session_id: "conversation-command".into(),
            lease_owner: "consumer-c".into(),
            now_ms: 300,
            lease_until_ms: 400,
        })
        .unwrap();
    assert!(gone_cmd.is_none());

    // Model runtime recovery hint restored without claiming process liveness, then Stopped committed.
    let restored_runtime: ModelRuntime = client
        .execute::<Option<ModelRuntime>, _>(ApplicationCommand::Model {
            command: ModelCommand::GetRuntime {
                host_id: "host-a".to_string(),
            },
        })
        .unwrap()
        .expect("runtime record");
    assert_eq!(restored_runtime.phase, ModelRuntimePhase::Ready);
    assert_eq!(restored_runtime.process_id, Some(1234));
    execute_model(ModelCommand::ReportRuntime {
        input: ModelRuntime {
            host_id: "host-a".to_string(),
            profile_id: "model-large".to_string(),
            phase: ModelRuntimePhase::Stopped,
            endpoint_json: None,
            process_id: None,
            error: Some("cold-start probe did not find the prior process".to_string()),
            started_at_ms: Some(300),
            stopped_at_ms: Some(400),
            updated_at_ms: 400,
        },
    })
    .unwrap();
    let stopped_runtime: ModelRuntime = client
        .execute::<Option<ModelRuntime>, _>(ApplicationCommand::Model {
            command: ModelCommand::GetRuntime {
                host_id: "host-a".to_string(),
            },
        })
        .unwrap()
        .unwrap();
    assert_eq!(stopped_runtime.phase, ModelRuntimePhase::Stopped);

    // Resolved permission intervention survived restart; resume grants a fresh attempt.
    let restored_perm: PermissionRequest = client
        .execute::<Option<PermissionRequest>, _>(ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Get {
                request_id: "permission-1".into(),
            },
        })
        .unwrap()
        .expect("resolved permission retained");
    assert_eq!(restored_perm.status, PermissionRequestStatus::Resolved);
    assert_eq!(
        restored_perm.resolution,
        Some(PermissionRequestResolution::Intervention {
            intervention: PermissionIntervention::ProvideInstruction {
                instruction: "Write under /tmp/project/drafts instead".into(),
            },
        })
    );
    let grant: nola_application::RuntimeTaskAttempt = client
        .execute::<Option<nola_application::RuntimeTaskAttempt>, _>(
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::ResumeAfterPermissionFair {
                    input: nola_application::ResumeRuntimeTaskAfterPermissionFair {
                        task_id: "permission-task".into(),
                        suspension_id: suspension.id.clone(),
                        expected_suspension_revision: suspension.revision,
                        permission_request_id: suspension.permission_request_id.clone(),
                        continuation_id: suspension.continuation_id.clone(),
                        worker_id: "resumed-worker".into(),
                        worker_generation: "resumed-generation".into(),
                        lease_token: "resumed-token".into(),
                        now_ms: 40,
                        lease_until_ms: 10_000,
                        max_running_global: 2,
                        max_running_project: 2,
                    },
                },
            },
        )
        .unwrap()
        .expect("fresh resumed attempt");
    assert_ne!(grant.id, attempt.id);

    // ==================== COLD RESTART #3 -> GENERATION 4 ====================
    drop(third);
    let fourth = start_ready_service(&database, &socket, &client);

    let renewed: nola_application::RuntimeTaskAttempt = client
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::RenewLease {
                task_id: grant.task_id.clone(),
                attempt_id: grant.id.clone(),
                expected_attempt_revision: grant.revision,
                worker_generation: grant.worker_generation.clone(),
                lease_token: grant.lease_token.clone(),
                current_expiry_ms: grant.lease_expires_at_ms,
                now_ms: 50,
                requested_expiry_ms: 11_000,
            },
        })
        .expect("renew persisted exact grant");
    assert!(renewed.revision > grant.revision);
    let proof: nola_application::RuntimeTaskPermissionResume = client
        .execute::<Option<nola_application::RuntimeTaskPermissionResume>, _>(
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::PermissionResume {
                    task_id: grant.task_id.clone(),
                    suspension_id: suspension.id.clone(),
                    permission_request_id: suspension.permission_request_id.clone(),
                    continuation_id: suspension.continuation_id.clone(),
                },
            },
        )
        .unwrap()
        .expect("permission grant is durable");
    assert_eq!(
        proof.granted_attempt, grant,
        "receipt must not borrow the renewed lease"
    );
    assert_eq!(proof.suspension.state, "resumed");
    let old_renewal: Result<nola_application::RuntimeTaskAttempt, _> =
        client.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::RenewLease {
                task_id: attempt.task_id,
                attempt_id: attempt.id,
                expected_attempt_revision: attempt.revision,
                worker_generation: attempt.worker_generation,
                lease_token: attempt.lease_token,
                current_expiry_ms: attempt.lease_expires_at_ms,
                now_ms: 60,
                requested_expiry_ms: 12_000,
            },
        });
    assert!(
        old_renewal.is_err(),
        "pre-permission worker regained authority after restart"
    );

    drop(fourth);
    let final_store = nola_store::NolaStore::open(&database).unwrap();
    assert_eq!(
        final_store.load_checkpoint("usage").unwrap().unwrap(),
        original_usage_checkpoint
    );
}
