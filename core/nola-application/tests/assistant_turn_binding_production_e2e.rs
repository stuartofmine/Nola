// SPDX-License-Identifier: Apache-2.0
//! Explicit assistant ownership through public production Application use cases.
//! No SQL-created membership, trace reconstruction, or checkpoint owner scan.

mod support;
#[path = "support/three_run_archive.rs"]
mod three_run_archive;

use nola_application::{
    begin_assistant_turn, claim_committed_host_requests, load_committed_host_requests,
    load_conversation_projection_v1, reduce_and_accept_host_response,
    reduce_and_commit_active_submission, AcceptedAssistantTurn, AcceptedHostResponse,
    BeginAssistantTurn, ClaimCommittedHostRequests, ReduceAndAcceptHostResponse,
    ReduceAndCommitActiveSubmission,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use nola_agent_engine::context_engine::{ContextComponent, ContextComponentKind, ContextComponentSource};
use nola_store::NolaStore;
use nola_store_port::assistant_ownership::{AssistantJournalPlanningRepository, AssistantOwner};

const CONVERSATION: &str = "canonical-conversation";

fn core_config(core_session: &str) -> AgentCoreConfig {
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "Isolated model fixture".into(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type": "object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: true,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    support::goal_execution::with_execution_binding(AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string(core_session.into())),
        model: Some("fixture-model".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("isolated-host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: [("model.stream".into(), capability)].into_iter().collect(),
        }),
        ..Default::default()
    })
}

fn user_input(id: &str, delivery: AgentInputDelivery, at: i64) -> AgentSubmission {
    AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(ItemId::from_string(id.into()), id)],
        turn_context_items: Vec::new(),
        turn_context_platform: Default::default(),
        turn_context_components: Vec::new(),
        submission_id: Some(format!("submission-{id}")),
        submitted_at_epoch_ms: Some(at),
        delivery,
    }
}

fn begin(store: &mut NolaStore, core_session: &str) -> AcceptedAssistantTurn {
    begin_assistant_turn(store, begin_command(core_session)).unwrap()
}

fn begin_command(core_session: &str) -> BeginAssistantTurn {
    BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: CONVERSATION.into(),
        conversation_title: "Isolated owner binding acceptance".into(),
        user_message_id: "initial-user".into(),
        user_part_id: "initial-user:body".into(),
        user_text: "initial-user".into(),
        user_content_json: None,
        assistant_message_id: "host-admitted-assistant".into(),
        context_checkpoint_id: None,
        core_config: core_config(core_session),
        submission: user_input("initial-user", AgentInputDelivery::CurrentTurn, 1000),
        created_at_ms: 1000,
    }
}

#[test]
fn bare_runtime_begin_commit_failure_poison_requires_reopen_without_partial_rows() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bare-commit-failure.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let database = rusqlite::Connection::open(&path).unwrap();
    // This fails the outer COMMIT, not the canonical writer's savepoint. The
    // runtime must not adopt a successful inner write as a committed outcome.
    database.execute_batch(
        "CREATE TABLE commit_guard_parent (id TEXT PRIMARY KEY);
         CREATE TABLE commit_guard_child (parent_id TEXT REFERENCES commit_guard_parent(id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TRIGGER reject_run_outer_commit AFTER INSERT ON runs BEGIN
             INSERT INTO commit_guard_child(parent_id) VALUES ('missing-parent');
         END;",
    ).unwrap();
    let error = runtime
        .execute(nola_application::ApplicationCommand::BeginAssistantTurn {
            input: begin_command("independent-core-session"),
        })
        .unwrap_err();
    assert!(error.contains("commit outcome unknown"), "{error}");
    assert!(error.contains("reopen required"), "{error}");
    for table in [
        "runs",
        "messages",
        "core_checkpoints",
        "effect_outbox",
        "commit_guard_child",
    ] {
        let rows: i64 = database
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0, "partial canonical write remained in {table}");
    }
    let rejected = runtime
        .execute(nola_application::ApplicationCommand::ActiveRun {
            conversation_id: CONVERSATION.into(),
        })
        .unwrap_err();
    assert!(rejected.contains("reopen required"), "{rejected}");
    drop(runtime);
    database
        .execute_batch("DROP TRIGGER reject_run_outer_commit")
        .unwrap();
    let mut reopened = support::open_runtime(&path).unwrap();
    let accepted: AcceptedAssistantTurn = serde_json::from_value(
        reopened
            .execute(nola_application::ApplicationCommand::BeginAssistantTurn {
                input: begin_command("independent-core-session"),
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(accepted.execution.control.session_id.as_str(), "independent-core-session");
    assert_ne!(accepted.execution.control.session_id.as_str(), CONVERSATION);
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn bare_runtime_rejected_admission_rolls_back_and_can_retry_without_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bare-rejected-admission.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let mut invalid = begin_command("independent-core-session");
    invalid.assistant_message_id.clear();
    assert!(runtime
        .execute(nola_application::ApplicationCommand::BeginAssistantTurn { input: invalid })
        .is_err());
    let database = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    runtime
        .execute(nola_application::ApplicationCommand::BeginAssistantTurn {
            input: begin_command("independent-core-session"),
        })
        .unwrap();
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn bare_short_deltas_do_not_open_sqlite_writer_and_terminal_flush_commits_buffered_events() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bare-hot-delta.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let begun: AcceptedAssistantTurn = serde_json::from_value(
        runtime
            .execute(nola_application::ApplicationCommand::BeginAssistantTurn {
                input: begin_command("independent-core-session"),
            })
            .unwrap(),
    )
    .unwrap();
    let mut reader = NolaStore::open(&path).unwrap();
    let request = pending_model(&reader, &begun.begin.run_id);
    let request = admit_sampling(
        &mut reader,
        &begun.begin.run_id,
        &begun.invocation_config_digest,
        request,
        1100,
    );
    let before = reader.load_checkpoint(CONVERSATION).unwrap().unwrap();
    let sequence = request.resume_after_sequence.unwrap() + 1;
    let delta_command =
        |sequence, delta: &str| nola_application::ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: CONVERSATION.into(),
                run_id: begun.begin.run_id.clone(),
                invocation_config_digest: begun.invocation_config_digest.clone(),
                response: response(
                    &request,
                    sequence,
                    HostResponse::ModelDelta {
                        channel: ModelDeltaChannel::Text,
                        part_id: text_part_id(&request).into(),
                        delta: delta.into(),
                    },
                ),
                received_at_ms: 1200 + sequence as i64,
            },
        };
    let first: AcceptedHostResponse =
        serde_json::from_value(runtime.execute(delta_command(sequence, "first")).unwrap()).unwrap();
    assert!(!first.commit.applied);
    assert!(first.control.is_none());
    // Failure occurs after the supervisor takes ownership, with no SQL UoW.
    // Losing the Core here used to silently discard the accepted first delta.
    let mut invalid = delta_command(sequence + 1, "must-not-appear");
    if let nola_application::ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut invalid {
        input.invocation_config_digest = "foreign-digest".into();
    }
    assert!(runtime.execute(invalid).is_err());
    // A second connection owns the SQLite writer. A hot-only delta must still
    // succeed immediately; an unnecessary BEGIN IMMEDIATE would fail here.
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let second: AcceptedHostResponse = serde_json::from_value(
        runtime
            .execute(delta_command(sequence + 1, "second"))
            .unwrap(),
    )
    .unwrap();
    assert!(!second.commit.applied);
    assert!(second.control.is_none());
    let still_durable = reader.load_checkpoint(CONVERSATION).unwrap().unwrap();
    assert_eq!(still_durable.revision, before.revision);
    assert_eq!(still_durable.checkpoint_json, before.checkpoint_json);
    writer.execute_batch("ROLLBACK").unwrap();
    // Both mutation rollback and receipt-write rollback must preserve the hot
    // tail. Reuse the same runtime: reopening would conceal the regression.
    for receipt_fault in [false, true] {
        writer.execute_batch(if receipt_fault {
            "CREATE TRIGGER hot_tail_fault BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'hot receipt fault'); END"
        } else {
            "CREATE TRIGGER hot_tail_fault BEFORE INSERT ON host_event_inbox BEGIN SELECT RAISE(ABORT,'hot inbox fault'); END"
        }).unwrap();
        let command = delta_command(sequence + 2, "must-not-appear");
        let request = nola_application::ExecuteReceivedCommand {
            client_id: "hot-rollback".into(),
            request_id: format!("fault-{receipt_fault}"),
            command_type: "reduce_and_accept_host_response".into(),
            command_json: serde_json::to_string(&command).unwrap(),
            expected_revision: None, received_at: "1299".into(),
        };
        let result = runtime.execute_received_command(request, command);
        if receipt_fault {
            assert!(result.is_err());
        } else {
            assert!(matches!(result.unwrap().outcome,
                nola_application::ApplicationCommandReceiptOutcome::Rejected { .. }));
        }
        writer.execute_batch("DROP TRIGGER hot_tail_fault").unwrap();
        assert_eq!(reader.load_checkpoint(CONVERSATION).unwrap().unwrap().revision, before.revision);
    }
    let terminal: AcceptedHostResponse = serde_json::from_value(
        runtime
            .execute(
                nola_application::ApplicationCommand::ReduceAndAcceptHostResponse {
                    input: ReduceAndAcceptHostResponse {
                        conversation_id: CONVERSATION.into(),
                        run_id: begun.begin.run_id.clone(),
                        invocation_config_digest: begun.invocation_config_digest.clone(),
                        response: response(
                            &request,
                            sequence + 2,
                            HostResponse::ModelCompleted {
                                items: Vec::new(),
                                tool_calls: Vec::new(),
                                usage: serde_json::json!({}),
                            },
                        ),
                        received_at_ms: 1300,
                    },
                },
            )
            .unwrap(),
    )
    .unwrap();
    assert!(terminal.commit.applied);
    let after = reader.load_checkpoint(CONVERSATION).unwrap().unwrap();
    assert!(after.revision > before.revision);
    let projection =
        serde_json::to_value(load_conversation_projection_v1(&reader, CONVERSATION).unwrap())
            .unwrap();
    assert!(projection.to_string().contains("firstsecond"));
    assert!(!projection.to_string().contains("must-not-appear"));
}

const PRIVATE_MEMORY_BODY: &str = "PRIVATE_MEMORY_BODY_MUST_NOT_BECOME_PUBLIC_SOURCE";

fn memory_begin_command(id: &str, at: i64, internal: bool, frontier: (Option<i64>, Option<String>)) -> BeginAssistantTurn {
    let mut command = begin_command("different-core-session");
    command.expected_configuration_revision = frontier.0;
    command.expected_parent_message_id = frontier.1;
    command.user_message_id = format!("{id}-user");
    command.user_part_id = format!("{id}-user:body");
    command.user_text = format!("{id}-user");
    command.assistant_message_id = format!("{id}-assistant");
    command.created_at_ms = at;
    let component = ContextComponent {
        kind: ContextComponentKind::Memory,
        id: format!("{id}-memory-component"),
        // Untyped source-aware context is intentionally not a fabricated
        // persisted MemoryRecord. Core emits provenance, not this body.
        content: serde_json::json!({"content": PRIVATE_MEMORY_BODY}),
        source: Some(ContextComponentSource {
            title: Some(format!("Initial memory evidence {id}")),
            uri: Some("fixture-memory://record".into()),
            ..Default::default()
        }),
    };
    command.submission = if internal {
        AgentSubmission::AutomatedInput {
            origin: AgentInputOrigin::GoalContinuation,
            items: Vec::new(),
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: vec![component],
            submission_id: Some(format!("submission-{id}")),
            submitted_at_epoch_ms: Some(at),
        }
    } else {
        AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string(command.user_message_id.clone()),
                &command.user_text,
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: vec![component],
            submission_id: Some(format!("submission-{id}")),
            submitted_at_epoch_ms: Some(at),
            delivery: AgentInputDelivery::CurrentTurn,
        }
    };
    command
}

fn start_internal_goal(store: &mut NolaStore, at: i64) {
    use nola_agent_engine::agent::goal::*;
    support::goal_execution::admit_start(store,
        nola_application::ReduceAndCommitIdleSubmission {
            conversation_id: CONVERSATION.into(),
            core_config: core_config("different-core-session"),
            submission: AgentSubmission::GoalRuntime {
                envelope: GoalRuntimeCommandEnvelope {
                    operation_id: format!("start-goal-{at}"),
                    command: GoalRuntimeCommand::Start {
                        id: "source-goal".into(), objective: "Continue the accepted work".into(),
                        constraints: Vec::new(),
                        requirements: vec![GoalRequirement {
                            id: "accepted-work".into(), semantic_revision: 1,
                            description: "Preserve the actual source and owner".into(),
                            acceptance_policy: GoalAcceptancePolicy::ModelJudgment,
                            dependencies: Vec::new(), status: GoalRecordStatus::Pending,
                            evidence_refs: Vec::new(),
                        }],
                        now_epoch_ms: at,
                    },
                },
            },
            committed_at_ms: at,
        }).unwrap();
}

fn internal_memory_begin_command(store: &NolaStore, id: &str, at: i64) -> BeginAssistantTurn {
    let mut command = memory_begin_command(id, at, true,
        support::admission_frontier::read(store, CONVERSATION));
    let stored = store.load_checkpoint(CONVERSATION).unwrap().unwrap();
    let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
        nola_agent_engine::agent::AgentThreadState,
    > = serde_json::from_str(&stored.checkpoint_json).unwrap();
    let goal = checkpoint.state.goal_projection.active.as_ref().unwrap();
    let AgentSubmission::AutomatedInput { items, turn_context_items,
        turn_context_platform, turn_context_components, submission_id, origin, .. } = &mut command.submission
        else { unreachable!() };
    items.push(InternalTurnTrigger {
        trigger_id: format!("{}:internal-trigger", submission_id.as_ref().unwrap()),
        core_session_id: checkpoint.state.session_id.as_str().into(),
        conversation_id: Some(CONVERSATION.into()), origin: *origin,
        context_digest: internal_turn_context_digest(turn_context_items,
            turn_context_platform, turn_context_components),
        goal: Some(GoalTriggerBinding {
            goal_id: goal.id.clone(), goal_revision: checkpoint.state.goal_projection.revision,
        }),
    }.item().unwrap());
    command
}

fn accepted_source(begun: &AcceptedAssistantTurn) -> (String, String) {
    let sources: Vec<_> = begun
        .events
        .iter()
        .filter_map(|envelope| match &envelope.event {
            AgentEvent::PartUpdated {
                turn_id,
                part: nola_agent_engine::agent::parts::AgentPart::Source(source),
                ..
            } => Some((source.id.clone(), turn_id.as_str().to_owned())),
            _ => None,
        })
        .collect();
    assert_eq!(
        sources.len(),
        1,
        "actual initial Core event must emit one Source"
    );
    sources.into_iter().next().unwrap()
}

fn assert_owned_initial_source(store: &NolaStore, begun: &AcceptedAssistantTurn) {
    let usage = store
        .conversation_committed_context_usage(CONVERSATION)
        .unwrap()
        .unwrap();
    assert_eq!(usage.identity.run_id, begun.begin.run_id);
    assert_eq!(
        usage.identity.assistant_message_id,
        begun.begin.assistant_message_id
    );
    assert_eq!(usage.admission.identity, usage.identity);
    assert!(usage.admission.sequence.0 > 0);
    let (part_id, turn) = accepted_source(begun);
    let conversation = store.load_conversation(CONVERSATION).unwrap().unwrap();
    let message = conversation
        .messages
        .iter()
        .find(|message| message.id == begun.begin.assistant_message_id)
        .unwrap();
    let part = message
        .parts
        .iter()
        .find(|part| part.id == part_id)
        .unwrap();
    assert_eq!(part.kind, "source");
    let persisted: nola_agent_engine::agent::parts::AgentPart =
        serde_json::from_str(part.content_json.as_deref().unwrap()).unwrap();
    assert_eq!(persisted.id(), part_id);
    assert_eq!(persisted.message_id(), begun.begin.assistant_message_id);
    assert_eq!(persisted.turn_id(), turn);
    let binding = AssistantJournalPlanningRepository::assistant_owner_for_part(
        store,
        CONVERSATION,
        &begun.begin.run_id,
        &part_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(binding.owner.assistant_message_id, message.id);
    assert_eq!(binding.turn_id.as_deref(), Some(turn.as_str()));
    let public = load_conversation_projection_v1(store, CONVERSATION).unwrap();
    let public_json = serde_json::to_string(&public).unwrap();
    assert!(public_json.contains(&part_id));
    assert!(!public_json.contains(PRIVATE_MEMORY_BODY));
}

fn complete_begun(store: &mut NolaStore, begun: &AcceptedAssistantTurn, at: i64) {
    let request = pending_model(store, &begun.begin.run_id);
    let request = admit_sampling(
        store,
        &begun.begin.run_id,
        &begun.invocation_config_digest,
        request,
        at,
    );
    accept(
        store,
        &begun.begin.run_id,
        &begun.invocation_config_digest,
        &request,
        request.resume_after_sequence.unwrap() + 1,
        HostResponse::ModelCompleted {
            items: vec![serde_json::to_value(AgentItem::assistant_message(
                ItemId::from_string(text_part_id(&request).to_owned()),
                "completed isolated answer",
            ))
            .unwrap()],
            tool_calls: Vec::new(),
            usage: serde_json::json!({}),
        },
        at + 3,
    );
    assert!(store.active_run(CONVERSATION).unwrap().is_none());
}

fn copy_and_replay_pending_sync(source: &mut NolaStore, receiver: &mut NolaStore) -> Vec<String> {
    use nola_store_port::replica_sync::{ApplyRemoteSyncPageInput, RemoteSyncChangeInput};
    let mut cursor = match receiver.sync_cursor("initial-admission-replica").unwrap() {
        Some(saved) => saved.cursor,
        None => {
            receiver
                .save_sync_cursor("initial-admission-replica", 0, 9000)
                .unwrap()
                .cursor
        }
    };
    let mut append_kinds = Vec::new();
    while let Some(change) = source.pending_sync_changes(1).unwrap().into_iter().next() {
        if change.entity_type == "conversation_append" {
            let payload: serde_json::Value = serde_json::from_str(&change.payload_json).unwrap();
            assert_eq!(payload["schema_version"], 1);
            append_kinds.push(
                payload["kind"]["kind"]
                    .as_str()
                    .expect("versioned AppendKind has an explicit typed kind tag")
                    .to_owned(),
            );
        }
        for replay in [false, true] {
            cursor += 1;
            let result = receiver
                .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                    replica_id: "initial-admission-replica".into(),
                    next_cursor: cursor,
                    applied_at_ms: 9000,
                    changes: vec![RemoteSyncChangeInput {
                        cursor,
                        operation_id: change.operation_id.clone(),
                        entity_type: change.entity_type.clone(),
                        entity_id: change.entity_id.clone(),
                        revision: change.revision,
                        deleted: change.deleted,
                        payload_json: change.payload_json.clone(),
                        created_at_ms: change.created_at_ms,
                    }],
                })
                .unwrap();
            assert_eq!(result.applied, usize::from(!replay));
            assert_eq!(result.skipped, usize::from(replay));
        }
        assert!(source
            .resolve_sync_change(&change.operation_id, "acknowledged", Some(cursor), 9000)
            .unwrap());
    }
    append_kinds
}

fn pending_model(store: &NolaStore, run: &str) -> HostRequestEnvelope {
    load_committed_host_requests(store, run)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .expect("production Core must have staged a model effect")
}

fn text_part_id(request: &HostRequestEnvelope) -> &str {
    let HostRequest::ModelStream { text_part_id, .. } = &request.payload else {
        panic!("model fixture")
    };
    text_part_id
}

fn response(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
        request_id: request.request_id.clone(),
        attempt_id: request.attempt_id.clone(),
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(),
        sequence,
        payload,
    }
}

fn accept(
    store: &mut NolaStore,
    run: &str,
    digest: &str,
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
    at: i64,
) -> AcceptedHostResponse {
    let accepted = reduce_and_accept_host_response(
        store,
        ReduceAndAcceptHostResponse {
            // Deliberately not request.session_id: canonical conversation and
            // the frozen Core session are two different production identities.
            conversation_id: CONVERSATION.into(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response(request, sequence, payload),
            received_at_ms: at,
        },
    )
    .unwrap();
    assert!(accepted.commit.applied);
    accepted
}

fn admit_sampling(
    store: &mut NolaStore,
    run: &str,
    digest: &str,
    request: HostRequestEnvelope,
    at: i64,
) -> HostRequestEnvelope {
    let claimed = claim_committed_host_requests(
        store,
        ClaimCommittedHostRequests {
            run_id: run.into(),
            worker_generation: format!("generation-{}", uuid::Uuid::new_v4()),
            lease_owner: format!("owner-{}", uuid::Uuid::new_v4()),
            claimed_at_ms: at,
            lease_expires_at_ms: at + 60_000,
            limit: 64,
        },
    )
    .unwrap();
    let claimed = claimed
        .requests
        .into_iter()
        .find(|candidate| candidate.effect_id == request.effect_id)
        .expect("the exact pending effect must have been claimed");
    let start_sequence = claimed.resume_after_sequence.unwrap_or(0) + 1;
    accept(
        store,
        run,
        digest,
        &claimed,
        start_sequence,
        HostResponse::ExecutionStarted,
        at + 1,
    );
    accept(
        store,
        run,
        digest,
        &claimed,
        start_sequence + 1,
        HostResponse::ModelAttemptStarted {
            provider: "fixture-provider".into(),
            model: "fixture-model".into(),
            reason: "initial".into(),
        },
        at + 2,
    );
    let mut current = pending_model(store, run);
    assert_eq!(current.effect_id, claimed.effect_id);
    current.delivery = claimed.delivery;
    current
}

fn turn_owner(store: &NolaStore, run: &str, turn: &str) -> Option<AssistantOwner> {
    AssistantJournalPlanningRepository::assistant_owner_for_turn(store, CONVERSATION, run, turn)
        .unwrap()
}

#[test]
fn real_begin_active_queue_host_successor_bindings_survive_cold_reopen() {
    for core_session in [CONVERSATION, "different-core-session"] {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("owner-bindings.db");
        let mut store = NolaStore::open(&database).unwrap();
        let begun = begin(&mut store, core_session);
        let run = begun.begin.run_id.clone();
        let digest = begun.invocation_config_digest.clone();
        assert_eq!(begun.execution.control.session_id.as_str(), core_session);
        let initial_turn = begun
            .execution.control
            .active_turn_id
            .as_ref()
            .unwrap()
            .as_str()
            .to_owned();
        let initial_owner = turn_owner(&store, &run, &initial_turn).unwrap();
        assert_eq!(initial_owner.conversation_id, CONVERSATION);
        assert_eq!(initial_owner.run_id, run);
        assert_eq!(
            initial_owner.assistant_message_id,
            "host-admitted-assistant"
        );
        let first = pending_model(&store, &run);
        assert_eq!(first.session_id, core_session);
        let initial_part = text_part_id(&first).to_owned();
        let seeded = AssistantJournalPlanningRepository::assistant_owner_for_part(
            &store,
            CONVERSATION,
            &run,
            &initial_part,
        )
        .unwrap()
        .unwrap();
        assert_eq!(seeded.owner, initial_owner);
        assert_eq!(seeded.turn_id.as_deref(), Some(initial_turn.as_str()));
        let first = admit_sampling(&mut store, &run, &digest, first, 1100);
        let first_delta_sequence = first.resume_after_sequence.unwrap() + 1;
        accept(
            &mut store,
            &run,
            &digest,
            &first,
            first_delta_sequence,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: initial_part.clone(),
                delta: "initial answer".into(),
            },
            1200,
        );
        let queued = reduce_and_commit_active_submission(
            &mut store,
            ReduceAndCommitActiveSubmission {
                conversation_id: CONVERSATION.into(),
                run_id: run.clone(),
                transition_id: "queue-real-successor".into(),
                invocation_config_digest: digest.clone(),
                submission: user_input("successor-user", AgentInputDelivery::NextTurn, 1500),
                committed_at_ms: 1500,
            },
        )
        .unwrap();
        assert!(queued.commit.applied);
        assert_eq!(
            turn_owner(&store, &run, &initial_turn),
            Some(initial_owner.clone())
        );
        accept(
            &mut store,
            &run,
            &digest,
            &first,
            first_delta_sequence + 1,
            HostResponse::ModelCompleted {
                items: Vec::new(),
                tool_calls: Vec::new(),
                usage: serde_json::json!({}),
            },
            2000,
        );
        let initial_run = run.clone();
        assert!(load_committed_host_requests(&store, &initial_run).unwrap().requests.is_empty());
        let admitted = nola_application::promote_queued_assistant_turn(
            &mut store,
            nola_application::PromoteQueuedAssistantTurn {
                conversation_id: CONVERSATION.into(), created_at_ms: 2050,
            },
        ).unwrap().expect("explicit successor admission");
        let run = admitted.begin.run_id;
        let digest = admitted.invocation_config_digest;
        assert_ne!(run, initial_run);
        let successor = pending_model(&store, &run);
        let successor_turn = successor.turn_id.clone().unwrap();
        assert_ne!(successor_turn, initial_turn);
        assert!(turn_owner(&store, &initial_run, &successor_turn).is_none(),
            "a successor cannot acquire an owner under its predecessor Run");
        assert_eq!(successor.session_id, core_session);
        let successor_part = text_part_id(&successor).to_owned();
        // Independent admission establishes exact message and initial Part ownership.
        let birth_owner = turn_owner(&store, &run, &successor_turn).unwrap();
        assert_eq!(birth_owner.run_id, run);
        assert_eq!(birth_owner.conversation_id, CONVERSATION);
        assert_ne!(birth_owner.assistant_message_id, initial_owner.assistant_message_id);
        assert!(
            AssistantJournalPlanningRepository::assistant_owner_for_part(
                &store,
                CONVERSATION,
                &run,
                &successor_part,
            )
            .unwrap()
            .is_some()
        );
        let successor = admit_sampling(&mut store, &run, &digest, successor, 2100);
        let successor_delta_sequence = successor.resume_after_sequence.unwrap() + 1;
        accept(
            &mut store,
            &run,
            &digest,
            &successor,
            successor_delta_sequence,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: successor_part.clone(),
                delta: "successor answer".into(),
            },
            2200,
        );
        let successor_owner = turn_owner(&store, &run, &successor_turn).unwrap();
        assert_eq!(successor_owner, birth_owner, "first body cannot change Turn birth ownership");
        assert_eq!(
            successor_owner.run_id, run,
            "Successor ownership uses its independently admitted canonical Run"
        );
        assert_eq!(successor_owner.conversation_id, CONVERSATION);
        assert_ne!(
            successor_owner.assistant_message_id,
            initial_owner.assistant_message_id
        );
        let successor_part_owner = AssistantJournalPlanningRepository::assistant_owner_for_part(
            &store,
            CONVERSATION,
            &run,
            &successor_part,
        )
        .unwrap()
        .unwrap();
        assert_eq!(successor_part_owner.owner, successor_owner);
        assert_eq!(
            successor_part_owner.turn_id.as_deref(),
            Some(successor_turn.as_str())
        );
        accept(
            &mut store,
            &run,
            &digest,
            &successor,
            successor_delta_sequence + 1,
            HostResponse::ModelCompleted {
                items: Vec::new(),
                tool_calls: Vec::new(),
                usage: serde_json::json!({}),
            },
            2300,
        );
        drop(store);
        let cold = NolaStore::open(&database).unwrap();
        assert_eq!(turn_owner(&cold, &initial_run, &initial_turn), Some(initial_owner));
        assert_eq!(
            turn_owner(&cold, &run, &successor_turn),
            Some(successor_owner.clone())
        );
        assert_eq!(
            AssistantJournalPlanningRepository::assistant_owner_for_part(
                &cold,
                CONVERSATION,
                &run,
                &successor_part,
            )
            .unwrap()
            .unwrap(),
            successor_part_owner
        );
        assert!(cold.active_run(CONVERSATION).unwrap().is_none());
        let conversation = cold.load_conversation(CONVERSATION).unwrap().unwrap();
        let successor_message = conversation
            .messages
            .iter()
            .find(|message| message.id == successor_owner.assistant_message_id)
            .unwrap();
        assert_eq!(successor_message.run_id.as_deref(), Some(run.as_str()));
        assert_eq!(successor_message.status, "completed");
        assert_eq!(
            successor_message
                .parts
                .iter()
                .map(|part| part.content_text.as_str())
                .collect::<String>(),
            "successor answer"
        );
    }
}

#[test]
fn rejected_host_response_cannot_create_turn_or_part_bindings() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("rejected-owner-binding.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin(&mut store, "different-core-session");
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    let request = pending_model(&store, &run);
    let request = admit_sampling(&mut store, &run, &digest, request, 1100);
    let initial_turn = request.turn_id.as_deref().unwrap();
    let owner_before = turn_owner(&store, &run, initial_turn);
    let checkpoint_before = store.load_checkpoint(CONVERSATION).unwrap().unwrap();
    let mut rejected = response(
        &request,
        request.resume_after_sequence.unwrap() + 1,
        HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: "rejected-part".into(),
            delta: "not accepted".into(),
        },
    );
    rejected.request_id = "different-request".into();
    rejected.turn_id = Some("unaccepted-turn".into());
    assert!(reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: CONVERSATION.into(),
            run_id: run.clone(),
            invocation_config_digest: digest,
            response: rejected,
            received_at_ms: 1200,
        },
    )
    .is_err());
    assert_eq!(
        store.load_checkpoint(CONVERSATION).unwrap().unwrap(),
        checkpoint_before
    );
    assert_eq!(turn_owner(&store, &run, initial_turn), owner_before);
    drop(store);
    let cold = NolaStore::open(&database).unwrap();
    assert!(turn_owner(&cold, &run, "unaccepted-turn").is_none());
    assert!(
        AssistantJournalPlanningRepository::assistant_owner_for_part(
            &cold,
            CONVERSATION,
            &run,
            "rejected-part",
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(turn_owner(&cold, &run, initial_turn), owner_before);
}

#[test]
fn real_initial_source_normal_and_internal_admissions_survive_replica_replay_and_cold_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("initial-source.db");
    let receiver_database = directory.path().join("initial-source-replica.db");
    let mut store = NolaStore::open(&database).unwrap();
    let mut receiver = NolaStore::open(&receiver_database).unwrap();

    let first =
        begin_assistant_turn(&mut store, memory_begin_command("first", 1000, false, (None, None))).unwrap();
    assert_eq!(first.execution.control.session_id.as_str(), "different-core-session");
    assert_owned_initial_source(&store, &first);
    let first_source = accepted_source(&first).0;
    copy_and_replay_pending_sync(&mut store, &mut receiver);
    let replica = receiver.load_conversation(CONVERSATION).unwrap().unwrap();
    assert_eq!(replica.messages.len(), 2);
    assert!(replica.messages[1]
        .parts
        .iter()
        .any(|part| part.id == first_source));
    assert!(receiver.active_run(CONVERSATION).unwrap().is_none());
    complete_begun(&mut store, &first, 1100);
    copy_and_replay_pending_sync(&mut store, &mut receiver);

    // A later real user admission publishes the incremental TurnOpened wire
    // contract, rather than hiding the initial Source in a full snapshot.
    let second_input = memory_begin_command("second", 2000, false, support::admission_frontier::read(&store, CONVERSATION));
    let second = begin_assistant_turn(&mut store, second_input).unwrap();
    assert_owned_initial_source(&store, &second);
    let second_source = accepted_source(&second).0;
    assert!(copy_and_replay_pending_sync(&mut store, &mut receiver)
        .iter()
        .any(|kind| kind == "turn_opened"));
    let replica = receiver.load_conversation(CONVERSATION).unwrap().unwrap();
    assert_eq!(replica.messages.len(), 4);
    assert!(replica.messages[3]
        .parts
        .iter()
        .any(|part| part.id == second_source));
    complete_begun(&mut store, &second, 2100);
    copy_and_replay_pending_sync(&mut store, &mut receiver);

    start_internal_goal(&mut store, 2200);

    // Durable internal continuation must keep the visible two-message user
    // Turn contract, while adding the new accepted Source to its actual slot.
    let internal_input = internal_memory_begin_command(&store, "internal", 3000);
    let internal = begin_assistant_turn(&mut store, internal_input).unwrap();
    assert_eq!(
        internal.begin.assistant_message_id,
        second.begin.assistant_message_id
    );
    assert_ne!(internal.begin.run_id, second.begin.run_id);
    assert_owned_initial_source(&store, &internal);
    let visible = store.load_conversation(CONVERSATION).unwrap().unwrap();
    assert_eq!(
        visible.messages[2].run_id.as_deref(),
        Some(second.begin.run_id.as_str())
    );
    assert_eq!(
        visible.messages[3].run_id.as_deref(),
        Some(internal.begin.run_id.as_str())
    );
    let internal_source = accepted_source(&internal).0;
    assert_eq!(
        store
            .load_conversation(CONVERSATION)
            .unwrap()
            .unwrap()
            .messages
            .len(),
        4
    );
    assert!(copy_and_replay_pending_sync(&mut store, &mut receiver)
        .iter()
        .any(|kind| kind == "turn_resumed"));
    drop(store);
    drop(receiver);

    let cold = NolaStore::open(&database).unwrap();
    assert_owned_initial_source(&cold, &internal);
    let cold_replica = NolaStore::open(&receiver_database).unwrap();
    let local = cold.load_conversation(CONVERSATION).unwrap().unwrap();
    let replica = cold_replica
        .load_conversation(CONVERSATION)
        .unwrap()
        .unwrap();
    assert_eq!(replica.messages, local.messages);
    assert_eq!(replica.messages.len(), 4);
    assert!(replica.messages[3]
        .parts
        .iter()
        .any(|part| part.id == internal_source));
    assert!(cold_replica.active_run(CONVERSATION).unwrap().is_none());
    assert!(cold_replica.pending_sync_changes(20).unwrap().is_empty());
    assert!(!serde_json::to_string(&replica.messages)
        .unwrap()
        .contains(PRIVATE_MEMORY_BODY));
}

#[test]
fn real_goal_continuation_seals_its_own_input_scope_without_inventing_a_user_turn() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("goal-continuation-scope.db");
    let mut store = NolaStore::open(&database).unwrap();
    let previous = begin_assistant_turn(&mut store,
        memory_begin_command("goal-user", 1000, false, (None, None))).unwrap();
    complete_begun(&mut store, &previous, 1100);
    start_internal_goal(&mut store, 1200);
    let input = internal_memory_begin_command(&store, "resume", 2000);
    let AgentSubmission::AutomatedInput { items, .. } = &input.submission else { unreachable!() };
    assert!(matches!(&items[0], AgentItem::Message { role: AgentRole::Developer, .. }));
    let trigger = InternalTurnTrigger::from_item(&items[0]).unwrap();
    assert_eq!(trigger.origin, AgentInputOrigin::GoalContinuation);
    assert!(!trigger.origin.carries_live_user_authority());
    let mut foreign = input.clone();
    let AgentSubmission::AutomatedInput { items, .. } = &mut foreign.submission else { unreachable!() };
    let mut forged = trigger.clone();
    forged.conversation_id = Some("foreign-conversation".into());
    items[0] = forged.item().unwrap();
    assert!(begin_assistant_turn(&mut store, foreign).is_err());
    assert!(store.active_run(CONVERSATION).unwrap().is_none());

    let mut changed_model = input.clone();
    let binding = changed_model.core_config.turn_identity.host_invocation.as_mut().unwrap();
    if let nola_host_protocol::HostModelRouteBinding::Http(route) = &mut binding.primary {
        route.model = "unselected-model".into();
    }
    binding.output_admission.routes[0].route = binding.primary.clone();
    assert!(begin_assistant_turn(&mut store, changed_model).unwrap_err().contains("model authority changed"));
    assert!(store.active_run(CONVERSATION).unwrap().is_none());

    let resumed = begin_assistant_turn(&mut store, input).unwrap();
    assert_ne!(resumed.begin.run_id, previous.begin.run_id);
    assert_eq!(resumed.begin.assistant_message_id, previous.begin.assistant_message_id);
    let events = serde_json::to_value(&resumed.events).unwrap();
    let events = events.as_array().unwrap();
    assert!(events.iter().any(|event| event["event"]["type"] == "input_applied"));
    assert!(events.iter().any(|event| event["event"]["type"] == "requirement_scope_changed"));
    complete_begun(&mut store, &resumed, 2100);
    drop(store);

    let cold = NolaStore::open(&database).unwrap();
    assert!(cold.active_run(CONVERSATION).unwrap().is_none());
    let visible = cold.load_conversation(CONVERSATION).unwrap().unwrap();
    assert_eq!(visible.messages.len(), 2, "an internal trigger is not a second user turn");
    assert_eq!(visible.messages.iter().filter(|message| message.role == "user").count(), 1);
    let inspect = rusqlite::Connection::open(&database).unwrap();
    let raw: String = inspect.query_row("SELECT result_json FROM run_results WHERE run_id=?1",
        [&resumed.begin.run_id], |row| row.get(0)).unwrap();
    let result: nola_domain::run_result::RunResult = serde_json::from_str(&raw).unwrap();
    result.validate().unwrap();
    assert_eq!(result.run_id, resumed.begin.run_id);
    assert_eq!(result.originating_submission_id, "submission-resume");
    assert_eq!(result.input.watermark.0, 1);
    assert!(matches!(result.value, nola_domain::run_result::RunResultValue::Answered { .. }));
    for kind in ["requirement_scope_changed", "run_result_decided"] {
        let count: i64 = inspect.query_row(
            "SELECT COUNT(*) FROM run_requirement_sources WHERE run_id=?1 AND kind=?2",
            rusqlite::params![resumed.begin.run_id, kind], |row| row.get(0)).unwrap();
        assert_eq!(count, 1, "current Run must own its own {kind} source");
    }
    let permission_count: i64 = inspect.query_row("SELECT COUNT(*) FROM permission_requests", [],
        |row| row.get(0)).unwrap();
    assert_eq!(permission_count, 0, "internal input provenance grants no tool authority");

    use nola_store_port::run_decision::{RunDecisionQuery, RunDecisionRepository};
    // Both immutable decisions remain verifiable after the same visible
    // assistant message has handed off to the successor Run and cold reopened.
    let mut previous_query = None;
    for run in [&previous.begin.run_id, &resumed.begin.run_id] {
        let raw: String = inspect.query_row("SELECT result_json FROM run_results WHERE run_id=?1",
            [run], |row| row.get(0)).unwrap();
        let result: nola_domain::run_result::RunResult = serde_json::from_str(&raw).unwrap();
        let query = RunDecisionQuery {
            conversation_id: CONVERSATION.into(), run_id: run.clone(), turn_id: run.clone(),
            core_revision: result.decided_at_revision.0,
            core_event_cursor: result.decision_event_cursor.0,
        };
        assert!(cold.load_run_decision(&query).unwrap().is_some());
        let nola_domain::run_result::RunResultValue::Answered { answer_part_refs } = result.value
            else { panic!("fixture must produce an Answered decision") };
        for part in answer_part_refs {
            let owner = AssistantJournalPlanningRepository::assistant_owner_for_part(
                &cold, CONVERSATION, run, &part).unwrap().unwrap();
            assert_eq!(owner.owner.run_id, *run);
            assert_eq!(owner.owner.assistant_message_id, resumed.begin.assistant_message_id);
            let turn_owner = AssistantJournalPlanningRepository::assistant_owner_for_turn(
                &cold, CONVERSATION, run, owner.turn_id.as_deref().unwrap()).unwrap().unwrap();
            assert_eq!(turn_owner, owner.owner);
            if run == &previous.begin.run_id {
                assert!(AssistantJournalPlanningRepository::assistant_owner_for_part(
                    &cold, CONVERSATION, &resumed.begin.run_id, &part).is_err(),
                    "the successor cannot claim an old Part through the reused message");
            }
        }
        if run == &previous.begin.run_id { previous_query = Some(query); }
    }
    inspect.execute("DELETE FROM assistant_turn_part_bindings WHERE run_id=?1",
        [&previous.begin.run_id]).unwrap();
    assert!(cold.load_run_decision(&previous_query.unwrap()).is_err(),
        "a reusable message alone cannot substitute for the historical Part binding");
}

#[test]
fn internal_admission_publication_failure_rolls_back_all_execution_and_message_facts() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("internal-admission-rollback.db");
    let mut store = NolaStore::open(&database).unwrap();
    let previous =
        begin_assistant_turn(&mut store, memory_begin_command("previous", 1000, false, (None, None))).unwrap();
    complete_begun(&mut store, &previous, 1100);
    start_internal_goal(&mut store, 1200);
    let inspect = rusqlite::Connection::open(&database).unwrap();
    let facts = || {
        let tables = inspect
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        tables
            .into_iter()
            .map(|table| {
                // The background publisher may acknowledge already committed
                // work while admission is rejected. Its receipt/timestamps are
                // independently owned; the dirty source generation must still
                // roll back exactly with every other command-owned fact.
                let columns_to_read = if table == "work_archive_publication" {
                    "singleton,target_generation"
                } else {
                    "*"
                };
                let mut query = inspect
                    .prepare(&format!("SELECT {columns_to_read} FROM \"{}\"", table.replace('"', "\"\"")))
                    .unwrap();
                let columns = query.column_count();
                let mut rows = query
                    .query_map([], |row| {
                        Ok((0..columns)
                            .map(|column| format!("{:?}", row.get_ref(column).unwrap()))
                            .collect::<Vec<_>>())
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                rows.sort();
                (table, rows)
            })
            .collect::<Vec<_>>()
    };
    let baseline = facts();
    // The real internal admission has already written its handoff, checkpoint,
    // initial events, Parts and outbox before this final publication fault.
    inspect.execute_batch("CREATE TRIGGER fail_internal_admission BEFORE INSERT ON run_message_admission_records BEGIN SELECT RAISE(ABORT,'internal admission publication fault'); END;").unwrap();
    let internal_input = internal_memory_begin_command(&store, "internal", 2000);
    let error = begin_assistant_turn(&mut store, internal_input.clone()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("internal admission publication fault"),
        "{error}"
    );
    let after = facts();
    assert_eq!(after.len(), baseline.len());
    for ((table, rows), (expected_table, expected_rows)) in after.iter().zip(&baseline) {
        assert_eq!(table, expected_table);
        assert_eq!(rows, expected_rows, "admission rollback changed table {table}");
    }
    inspect
        .execute_batch("DROP TRIGGER fail_internal_admission;")
        .unwrap();
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    let internal = begin_assistant_turn(&mut store, internal_input).unwrap();
    assert_owned_initial_source(&store, &internal);
    let started: String = inspect.query_row(
        "SELECT payload_json FROM run_events WHERE run_id=?1 AND event_kind='turn_started' ORDER BY sequence LIMIT 1",
        [&internal.begin.run_id], |row| row.get(0)).unwrap();
    let started: serde_json::Value = serde_json::from_str(&started).unwrap();
    assert_eq!(started["turn_id"], internal.begin.run_id);
    assert_eq!(started["core_envelope"]["turn_id"], internal.begin.run_id);
    assert_eq!(started["session_id"], internal.execution.control.session_id.as_str());
    assert_eq!(
        started["core_envelope"]["session_id"],
        internal.execution.control.session_id.as_str()
    );
    let visible = store.load_conversation(CONVERSATION).unwrap().unwrap();
    assert_eq!(visible.messages.len(), 2);
    assert_eq!(
        visible.messages[0].run_id.as_deref(),
        Some(previous.begin.run_id.as_str())
    );
    assert_eq!(
        visible.messages[1].run_id.as_deref(),
        Some(internal.begin.run_id.as_str())
    );
    drop(store);
    let cold = NolaStore::open(&database).unwrap();
    assert_owned_initial_source(&cold, &internal);
}

#[test]
fn real_initial_source_receipt_rollback_retry_and_cold_replay_are_atomic() {
    use nola_application::{
        ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
    };
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("initial-source-receipt.db");
    let command = ApplicationCommand::BeginAssistantTurn {
        input: memory_begin_command("receipt", 1000, false, (None, None)),
    };
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt_input = || ExecuteReceivedCommand {
        client_id: "initial-source-client".into(),
        request_id: "initial-source-admission".into(),
        command_type: "begin_assistant_turn".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "1000".into(),
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    let inspect = rusqlite::Connection::open(&database).unwrap();
    let facts = || {
        [
            "conversations",
            "messages",
            "message_parts",
            "runs",
            "core_checkpoints",
            "assistant_turn_message_bindings",
            "assistant_turn_part_bindings",
            "effect_outbox",
            "sync_outbox",
            "command_receipts",
        ]
        .into_iter()
        .map(|table| {
            let mut query = inspect
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let column_count = query.column_count();
            query
                .query_map([], |row| {
                    Ok((0..column_count)
                        .map(|column| format!("{:?}", row.get_ref(column).unwrap()))
                        .collect::<Vec<_>>())
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
        .collect::<Vec<_>>()
    };
    let baseline = facts();
    // Fault injection is confined to this owned fixture DB. It proves the
    // actual receipt UoW rolls back after canonical initial Parts were written.
    inspect.execute_batch("CREATE TRIGGER fail_initial_source_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'initial source receipt fault'); END;").unwrap();
    let error = runtime
        .execute_received_command(
            receipt_input(),
            serde_json::from_str(&command_json).unwrap(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("initial source receipt fault"));
    assert_eq!(facts(), baseline);
    inspect
        .execute_batch("DROP TRIGGER fail_initial_source_receipt;")
        .unwrap();
    let committed = runtime
        .execute_received_command(
            receipt_input(),
            serde_json::from_str(&command_json).unwrap(),
        )
        .unwrap();
    let ApplicationCommandReceiptOutcome::Committed { result, .. } = &committed.outcome else {
        panic!("production source admission must commit")
    };
    let begun: AcceptedAssistantTurn = serde_json::from_value(result.clone()).unwrap();
    let committed_facts = facts();
    drop(runtime);
    let mut reopened = support::open_runtime(&database).unwrap();
    let replay = reopened
        .execute_received_command(
            receipt_input(),
            serde_json::from_str(&command_json).unwrap(),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    assert_eq!(facts(), committed_facts);
    drop(reopened);
    let cold = NolaStore::open(&database).unwrap();
    assert_owned_initial_source(&cold, &begun);
    assert_eq!(
        cold.load_conversation(CONVERSATION)
            .unwrap()
            .unwrap()
            .messages
            .len(),
        2
    );
}

#[test]
fn goal_step_admits_once_recovers_exact_round_and_consumes_its_seal() {
    use nola_application::{execute_application_command, ApplicationCommand};
    for (recorded_before_restart,supply_configuration) in [(false,false),(true,false),(false,true),(true,true)] {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("embedded-goal.db");
    let mut store = NolaStore::open(&database).unwrap();
    let previous = begin_assistant_turn(&mut store,
        memory_begin_command("goal-owner", 1000, false, (None,None))).unwrap();
    complete_begun(&mut store,&previous,1100);
    let frozen=execute_application_command(&mut store,ApplicationCommand::LoadHostExecutionState {conversation_id:CONVERSATION.into()}).unwrap()["configuration"].clone();
    let command = |control:serde_json::Value| serde_json::from_value::<ApplicationCommand>(serde_json::json!({
        "type":"step_goal","conversation_id":CONVERSATION,"source_host_id":"isolated-host","start_configuration":if supply_configuration && !control.is_null() {frozen.clone()}else{serde_json::Value::Null},"control":if control.is_null(){serde_json::Value::Null}else{serde_json::to_value(nola_application::goal_control_intent::prepare(serde_json::from_value(control).unwrap(), chrono::Utc::now().timestamp_millis()).unwrap()).unwrap()}})).unwrap();
    let first = execute_application_command(&mut store,command(serde_json::json!({
        "conversation_id":CONVERSATION,"operation_id":"start-embedded","action":"start",
        "expected_goal_id":null,"expected_control_token":null,"objective":"Review the retained fixture",
        "constraints":[],"wake_at_epoch_ms":null}))).unwrap();
    let run = first["instruction"]["run_id"].as_str().unwrap_or_else(||panic!("Goal admission failed: {first}")).to_owned();
    assert_eq!(first["goal"]["contracts"].as_array().unwrap().len(),1);
    drop(store);
    let mut store=NolaStore::open(&database).unwrap();
    let recovered=execute_application_command(&mut store,command(serde_json::Value::Null)).unwrap();
    assert_eq!(recovered["instruction"]["run_id"],run);
    assert_eq!(recovered["goal"]["contracts"].as_array().unwrap().len(),1,"handoff recovery must retain the original contract");
    let active=store.active_run(CONVERSATION).unwrap().unwrap();
    let digest=active.invocation_config_digest.unwrap();
    let request=pending_model(&store,&run);
    let at=chrono::Utc::now().timestamp_millis();
    let request=admit_sampling(&mut store,&run,&digest,request,at);
    accept(&mut store,&run,&digest,&request,request.resume_after_sequence.unwrap()+1,
        HostResponse::ModelCompleted{items:vec![serde_json::to_value(AgentItem::assistant_message(
            ItemId::from_string(text_part_id(&request).into()),"Completed this bounded round")).unwrap()],
            tool_calls:vec![],usage:serde_json::json!({})},at+3);
    assert!(store.active_run(CONVERSATION).unwrap().is_none());
    if recorded_before_restart {
        let bound=nola_application::goal_execution::load_goal_execution_binding(&store,CONVERSATION,"goal-start-embedded").unwrap().unwrap();
        let command=serde_json::from_value::<ApplicationCommand>(serde_json::json!({"type":"reduce_and_commit_idle_submission","input":{
            "conversation_id":CONVERSATION,"core_config":bound.template.core_config,"committed_at_ms":at+4,
            "submission":{"type":"goal_runtime","envelope":{"operation_id":"goal-start-embedded:round:1:execution",
                "command":{"type":"record_execution","report":{"contract_id":"goal-start-embedded:round:1",
                    "summary":"Execution ended without a summary","claimed_record_ids":[],"artifact_refs":[],"blockers":[],
                    "reported_at_epoch_ms":at+4}}}}}})).unwrap();
        execute_application_command(&mut store,command).unwrap();
    }
    drop(store);
    let mut store=NolaStore::open(&database).unwrap();
    let result=execute_application_command(&mut store,command(serde_json::Value::Null)).unwrap();
    assert_ne!(result["instruction"]["run_id"],run,"a sealed Run must never be reopened");
    let reports=result["goal"]["execution_reports"].as_array().unwrap();
    assert_eq!(reports.len(),1);
    assert!(!reports[0]["summary"].as_str().unwrap().contains("Host restarted"),"sealed work is an actual result, not a synthetic interrupted round");
    assert_eq!(result["goal"]["contracts"][0]["execution_run_id"],run);
    assert_eq!(result["goal"]["contracts"][0]["status"],"audited");
    }
}

#[test]
fn goal_step_pause_persists_interrupt_before_native_executor_attaches() {
    assert_goal_pause_interrupt(false);
}

#[test]
fn queued_goal_pause_uses_the_same_interrupt_before_model_dispatch() {
    assert_goal_pause_interrupt(true);
}

fn assert_goal_pause_interrupt(queued_boundary: bool) {
    use nola_application::{execute_application_command,ApplicationCommand};
    let directory=tempfile::tempdir().unwrap();
    let mut store=NolaStore::open(directory.path().join("goal-stop.db")).unwrap();
    let previous=begin_assistant_turn(&mut store,memory_begin_command("goal-stop-owner",1000,false,(None,None))).unwrap();
    complete_begun(&mut store,&previous,1100);
    let step=|control:serde_json::Value|serde_json::from_value::<ApplicationCommand>(serde_json::json!({
        "type":"step_goal","conversation_id":CONVERSATION,"source_host_id":"isolated-host","control":if control.is_null(){serde_json::Value::Null}else{serde_json::to_value(nola_application::goal_control_intent::prepare(serde_json::from_value(control).unwrap(), chrono::Utc::now().timestamp_millis()).unwrap()).unwrap()}})).unwrap();
    let first=execute_application_command(&mut store,step(serde_json::json!({
        "conversation_id":CONVERSATION,"operation_id":"goal-stop-start","action":"start","expected_goal_id":null,
        "expected_control_token":null,"objective":"Inspect the fixture","constraints":[],"wake_at_epoch_ms":null}))).unwrap();
    let run=first["instruction"]["run_id"].as_str().unwrap().to_owned();
    let observation=execute_application_command(&mut store,ApplicationCommand::LoadGoalControlObservation{conversation_id:CONVERSATION.into()}).unwrap();
    let pause=step(serde_json::json!({
        "conversation_id":CONVERSATION,"operation_id":"goal-stop-pause","action":"pause","expected_goal_id":observation["goal_id"],
        "expected_control_token":observation["control_token"],"objective":"","constraints":[],"wake_at_epoch_ms":null}));
    let stopped=if queued_boundary {
        let ApplicationCommand::StepGoal {control:Some(envelope),..}=pause else {unreachable!()};
        let active=store.active_run(CONVERSATION).unwrap().unwrap();
        let accepted=execute_application_command(&mut store,ApplicationCommand::ReduceAndCommitActiveSubmission {
            input:ReduceAndCommitActiveSubmission {conversation_id:CONVERSATION.into(),run_id:run.clone(),
                invocation_config_digest:active.invocation_config_digest.unwrap(),transition_id:"queued-goal-stop".into(),
                submission:AgentSubmission::GoalRuntime {envelope},committed_at_ms:chrono::Utc::now().timestamp_millis()}
        }).unwrap();
        assert_eq!(accepted["control"]["turn_status"],"cancelling",
            "queued Goal pause must interrupt before another model dispatch, not only change Goal status");
        execute_application_command(&mut store,step(serde_json::Value::Null)).unwrap()
    } else {
        let stopped=execute_application_command(&mut store,pause).unwrap();
        assert_eq!(stopped["interrupted_run_id"],run);
        stopped
    };
    assert_eq!(stopped["goal"]["status"],"paused");
    assert_eq!(stopped["instruction"]["type"],"execute_run");
    assert_eq!(stopped["instruction"]["run_id"],run);
    // A durable stop request is not physical-stop proof. Core preserves the
    // exact Run until its existing cancellation acknowledgement completes.
    assert_eq!(store.active_run(CONVERSATION).unwrap().unwrap().run_id,run);
    let execution=execute_application_command(&mut store,ApplicationCommand::LoadHostExecutionState{conversation_id:CONVERSATION.into()}).unwrap();
    assert_eq!(execution["control"]["turn_status"],"cancelling");
    let seal=execute_application_command(&mut store,ApplicationCommand::CanonicalRunTerminal{conversation_id:CONVERSATION.into(),run_id:run.clone()}).unwrap();
    assert!(seal.is_null());
    let checkpoint=execute_application_command(&mut store,ApplicationCommand::LoadCheckpoint{conversation_id:CONVERSATION.into()}).unwrap();
    let checkpoint:serde_json::Value=serde_json::from_str(checkpoint["checkpoint_json"].as_str().unwrap()).unwrap();
    assert_eq!(checkpoint["state"]["execution"]["requirements"]["cancellation_id"],"goal-control:goal-stop-pause");
    let still_paused=execute_application_command(&mut store,step(serde_json::Value::Null)).unwrap();
    assert_eq!(still_paused["instruction"]["run_id"],run,"pending cancellation must recover the original Run until Host ACK");
    assert_eq!(still_paused["goal"]["contracts"].as_array().unwrap().len(),1);
}
