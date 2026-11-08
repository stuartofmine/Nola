// SPDX-License-Identifier: Apache-2.0
//! Application receipt and cold recovery across queued model-completion boundaries.
mod support;
use nola_application::{
    begin_assistant_turn, load_committed_host_requests, ApplicationCommand, BeginAssistantTurn,
    ExecuteReceivedCommand, ReduceAndAcceptHostResponse, ReduceAndCommitActiveSubmission,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use nola_store::NolaStore;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn runtime_capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type": "object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4_000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: id == "model.stream",
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}

fn runtime_config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert(
        "model.stream".to_string(),
        runtime_capability("model.stream", HostCapabilityRoute::Model),
    );
    support::goal_execution::with_execution_binding(AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string(
            "completion-conversation".to_string(),
        )),
        model: Some("test-model".to_string()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("recovery-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    })
}

fn response_for_request(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
        attempt_id: request.attempt_id.clone(),
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        request_id: request.request_id.clone(),
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

fn response_command(
    request: &HostRequestEnvelope,
    run: &str,
    digest: &str,
    payload: HostResponse,
) -> ApplicationCommand {
    ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "completion-conversation".into(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(
                request,
                request
                    .resume_after_sequence
                    .map_or(1, |sequence| sequence + 1),
                payload,
            ),
            received_at_ms: 2000,
        },
    }
}

fn canonical_rows(db: &rusqlite::Connection) -> Vec<Vec<String>> {
    ["SELECT checkpoint_json FROM core_checkpoint_bodies ORDER BY conversation_id",
     "SELECT id,role,status,coalesce(run_id,''),ordinal FROM messages ORDER BY id",
     "SELECT id,kind,state,coalesce(content_text,''),coalesce(content_json,'') FROM message_parts ORDER BY id",
     "SELECT effect_id,status,json_array(binding_json,payload_json) FROM effect_outbox JOIN effect_outbox_payloads USING(effect_id) ORDER BY effect_id"]
        .into_iter().map(|sql| {
            let mut statement = db.prepare(sql).unwrap();
            let count = statement.column_count();
            statement.query_map([], |row| {
                let contents = (0..count).map(|index| format!("{:?}", row.get_ref(index).unwrap())).collect::<Vec<_>>().join("|");
                Ok(format!("{:x}", Sha256::digest(contents.as_bytes())))
            }).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        }).collect()
}

fn user_input(id: &str, delivery: AgentInputDelivery) -> AgentSubmission {
    AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(ItemId::from_string(id.into()), id)],
        turn_context_items: vec![],
        turn_context_platform: Default::default(),
        turn_context_components: vec![],
        submission_id: Some(format!("submit-{id}")),
        submitted_at_epoch_ms: Some(1000),
        delivery,
    }
}

fn pending(database: &std::path::Path, run: &str) -> Vec<HostRequestEnvelope> {
    load_committed_host_requests(&mut NolaStore::open(database).unwrap(), run)
        .unwrap()
        .requests
}

fn complete_and_replay(
    database: &std::path::Path,
    run: &str,
    digest: &str,
    request: &HostRequestEnvelope,
    answer: Option<&str>,
) {
    let mut admitted = request.clone();
    let mut store = NolaStore::open(database).unwrap();
    support::admit_model_sampling(&mut store, run, digest, &mut admitted, 2000);
    drop(store);
    assert!(admitted.attempt_started);
    let request = &admitted;
    let command = response_command(
        request,
        run,
        digest,
        HostResponse::ModelCompleted {
            items: answer
                .into_iter()
                .map(|text| {
                    serde_json::to_value(AgentItem::assistant_message(
                        ItemId::from_string(format!("answer-{}", request.request_id)),
                        text,
                    ))
                    .unwrap()
                })
                .collect(),
            tool_calls: vec![],
            usage: json!({"input_tokens": 80, "output_tokens": 10}),
        },
    );
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = || ExecuteReceivedCommand {
        client_id: "completion-client".into(),
        request_id: request.request_id.clone(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "2000".into(),
    };
    let mut runtime = support::open_runtime(database).unwrap();
    let accepted = runtime
        .execute_received_command(receipt(), command)
        .unwrap();
    assert!(matches!(
        accepted.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    drop(runtime);
    let mut runtime = support::open_runtime(database).unwrap();
    let db = rusqlite::Connection::open(database).unwrap();
    let before = canonical_rows(&db);
    runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert_eq!(
        canonical_rows(&db),
        before,
        "Cold receipt replay cannot duplicate successor models or mutate canonical history"
    );
}

fn successor_fixture(database: &std::path::Path) -> (String, String, Vec<HostRequestEnvelope>) {
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "completion-conversation".into(),
            conversation_title: "Completion".into(),
            user_message_id: "initial".into(),
            user_part_id: "initial:body".into(),
            user_text: "initial".into(),
            user_content_json: None,
            assistant_message_id: "submit-initial:assistant".into(),
            context_checkpoint_id: None,
            core_config: runtime_config(),
            submission: user_input("initial", AgentInputDelivery::CurrentTurn),
            created_at_ms: 1000,
        },
    )
    .unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    let first = load_committed_host_requests(&mut store, &run)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    for (id, delivery) in [
        ("steering", AgentInputDelivery::CurrentTurn),
        ("next", AgentInputDelivery::NextTurn),
    ] {
        runtime
            .execute(ApplicationCommand::ReduceAndCommitActiveSubmission {
                input: ReduceAndCommitActiveSubmission {
                    conversation_id: "completion-conversation".into(),
                    run_id: run.clone(),
                    transition_id: format!("queue-{id}"),
                    invocation_config_digest: digest.clone(),
                    submission: user_input(id, delivery),
                    committed_at_ms: 1100,
                },
            })
            .unwrap();
    }
    drop(runtime);
    complete_and_replay(&database, &run, &digest, &first, None);
    let requests = pending(&database, &run);
    assert_eq!(requests.len(), 1);
    let second = &requests[0];
    assert!(matches!(second.payload, HostRequest::ModelStream { .. }));
    assert_eq!(
        second.turn_id, first.turn_id,
        "Current-turn steering must keep the original Turn alive"
    );
    assert_ne!(second.effect_id, first.effect_id);
    complete_and_replay(&database, &run, &digest, second, Some("Steering handled."));
    assert!(pending(&database, &run).is_empty(), "completion must not dispatch a successor under the predecessor Run");
    let mut store = NolaStore::open(&database).unwrap();
    let successor = nola_application::promote_queued_assistant_turn(
        &mut store,
        nola_application::PromoteQueuedAssistantTurn {
            conversation_id: "completion-conversation".into(),
            created_at_ms: 2000,
        },
    ).unwrap().expect("explicit successor admission");
    assert_ne!(successor.begin.run_id, run);
    let run = successor.begin.run_id;
    let digest = successor.invocation_config_digest;
    drop(store);
    let requests = pending(&database, &run);
    assert_eq!(requests.len(), 1);
    let third = &requests[0];
    assert!(matches!(third.payload, HostRequest::ModelStream { .. }));
    assert_ne!(
        third.turn_id, first.turn_id,
        "Next-turn input begins only after the original Turn completes"
    );
    let db = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
            .unwrap(),
        2,
        "Each explicitly admitted successor owns an independent canonical Run"
    );
    (run, digest, requests)
}

#[test]
fn startup_uses_current_checkpoint_instead_of_an_earlier_turn_terminal_event() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("completion.db");
    let (run, digest, requests) = successor_fixture(&database);
    let third = &requests[0];
    let db = rusqlite::Connection::open(&database).unwrap();
    let checkpoint_json =
        support::current_checkpoint(&database, "completion-conversation").checkpoint_json;
    let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState> =
        serde_json::from_str(&checkpoint_json).unwrap();
    let cold_core = nola_agent_engine::agent::AgentCore::restore(runtime_config(), checkpoint).unwrap();
    assert_eq!(
        cold_core
            .snapshot()
            .active_turn_id
            .as_ref()
            .map(|id| id.as_str()),
        third.turn_id.as_deref()
    );
    assert_eq!(third.turn_id.as_deref(), Some(run.as_str()));
    assert_eq!(
        cold_core.snapshot().turn_status,
        nola_agent_engine::agent::AgentTurnStatus::AwaitingModel
    );
    let before = canonical_rows(&db);
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(ApplicationCommand::RecoverOrphanedRuns {
            conversation_id: None,
            reason: "server_restarted".into(),
            interrupted_at: "after".into(),
            interrupted_at_ms: 4000,
        })
        .unwrap();
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    let active = store.active_run("completion-conversation").unwrap();
    assert_eq!(active.as_ref().map(|record| record.run_id.as_str()), Some(run.as_str()),
        "A pending recoverable successor Model effect cannot be completed by a prior Turn's terminal event");
    assert_eq!(
        canonical_rows(&db),
        before,
        "Startup must preserve canonical items, checkpoint and pending model for explicit recovery"
    );
    let recovered = load_committed_host_requests(&store, &run).unwrap().requests;
    assert_eq!(
        serde_json::to_value(&recovered).unwrap(),
        serde_json::to_value(&requests).unwrap()
    );
    drop(store);
    complete_and_replay(
        &database,
        &run,
        &digest,
        third,
        Some("Recovered successor completed."),
    );
    assert!(
        pending(&database, &run).is_empty(),
        "Recovery must actually accept the pending Host result, not merely relabel the Run"
    );
}

fn recovery(scoped: bool) -> ApplicationCommand {
    ApplicationCommand::RecoverOrphanedRuns {
        conversation_id: scoped.then(|| "completion-conversation".into()),
        reason: "server_restarted".into(),
        interrupted_at: "after".into(),
        interrupted_at_ms: 4000,
    }
}

fn follow_up_command(frontier: (Option<i64>, Option<String>)) -> BeginAssistantTurn {
    let mut core_config = runtime_config();
    core_config.model = Some("new-build-model".into());
    BeginAssistantTurn {
        expected_configuration_revision: frontier.0,
        expected_parent_message_id: frontier.1,
        conversation_id: "completion-conversation".into(),
        conversation_title: "Completion".into(),
        user_message_id: "follow-up".into(),
        user_part_id: "follow-up:body".into(),
        user_text: "follow-up".into(),
        user_content_json: None,
        assistant_message_id: "submit-follow-up:assistant".into(),
        context_checkpoint_id: None,
        core_config,
        submission: user_input("follow-up", AgentInputDelivery::CurrentTurn),
        created_at_ms: 4000,
    }
}

fn obsolete_host_bindings() -> Vec<serde_json::Value> {
    use nola_host_protocol::*;
    let route = HostModelRouteBinding::Native(HostNativeModelRouteBinding {
        adapter: HostNativeModelAdapter::LlamaCpp,
        adapter_contract_revision: 1,
        model_id: "old-build-model".into(),
        profile_ref: None,
    });
    let admission = HostOutputAdmission::admit(
        vec![HostRouteTokenLimits {
            route: route.clone(),
            max_input_tokens: None,
            max_output_tokens: None,
            max_total_tokens: None,
        }],
        nola_domain::generation::OutputTokenPreference::Default,
    )
    .unwrap();
    let binding = HostInvocationBinding {
        mac_launch: None,
        version: HOST_INVOCATION_BINDING_VERSION,
        primary: route,
        fallbacks: vec![],
        generation: HostGenerationBinding {
            max_retries: 2,
            parallel_tool_calls: None,
            reasoning_selection: None,
            reasoning_tier: Default::default(),
            sampling_override: false,
            generation_options: nola_domain::generation::GenerationOptions::default()
                .try_into()
                .unwrap(),
            temperature: serde_json::Number::from_f64(0.7).unwrap(),
            top_p: serde_json::Number::from_f64(0.9).unwrap(),
            max_output_tokens: admission.resolved.tokens,
            thinking_enabled: false, reasoning_summary_supported: None,
            prompt_cache: false,
            prompt_cache_key_sha256: None,
            prompt_cache_key_prefix_sha256: None,
            prompt_cache_retention: None,
        },
        output_admission: admission,
        execution_policy_sha256: "a".repeat(64),
    };
    let mut missing_tier = serde_json::to_value(binding).unwrap();
    missing_tier["generation"]
        .as_object_mut()
        .unwrap()
        .remove("reasoning_tier");
    assert!(
        serde_json::from_value::<HostInvocationBinding>(missing_tier.clone())
            .unwrap_err()
            .to_string()
            .contains("reasoning_tier")
    );
    vec![
        missing_tier,
        json!({"version":999,"obsolete_transport":{"unknown":true}}),
    ]
}

fn replace_checkpoint_binding(database: &std::path::Path, binding: &serde_json::Value) {
    let db = rusqlite::Connection::open(database).unwrap();
    let raw: String = db
        .query_row(
            "SELECT checkpoint_json FROM core_checkpoint_bodies",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut checkpoint: serde_json::Value = serde_json::from_str(&raw).unwrap();
    for parent in [
        "/state/configuration/turn_identity",
        "/state/execution/active_turn/context/configuration/turn_identity",
    ] {
        if let Some(identity) = checkpoint.pointer_mut(parent) {
            identity["host_invocation"] = binding.clone();
        }
    }
    assert!(serde_json::from_value::<
        nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState>,
    >(checkpoint.clone())
    .is_err());
    db.execute(
        "UPDATE core_checkpoint_bodies SET checkpoint_json=?1",
        [checkpoint.to_string()],
    )
    .unwrap();
}

#[test]
fn cold_new_turn_preserves_completed_history_and_goal_across_obsolete_host_schema() {
    use nola_agent_engine::agent::{
        GoalRecordStatus, GoalRequirement, GoalRuntimeCommand, GoalRuntimeCommandEnvelope,
    };
    for binding in obsolete_host_bindings() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("cross-build.db");
        let (old_run, digest, requests) = successor_fixture(&database);
        complete_and_replay(
            &database,
            &old_run,
            &digest,
            &requests[0],
            Some("Previous build completed."),
        );
        let mut store = NolaStore::open(&database).unwrap();
        assert!(store
            .active_run("completion-conversation")
            .unwrap()
            .is_none());
        support::goal_execution::admit_start(
            &mut store,
            nola_application::ReduceAndCommitIdleSubmission {
                conversation_id: "completion-conversation".into(),
                core_config: runtime_config(),
                submission: AgentSubmission::GoalRuntime {
                    envelope: GoalRuntimeCommandEnvelope {
                        operation_id: "durable-goal-start".into(),
                        command: GoalRuntimeCommand::Start {
                            id: "durable-goal".into(),
                            objective: "Preserve work across builds".into(),
                            constraints: vec!["Preserve canonical history".into()],
                            requirements: vec![GoalRequirement {
                                acceptance_policy: nola_agent_engine::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                                semantic_revision: 1, id: "cross-build-follow-up".into(),
                                description:
                                    "Resume the same goal after a build changes Host schema".into(),
                                dependencies: vec![],
                                status: GoalRecordStatus::Pending,
                                evidence_refs: vec![],
                            }],
                            now_epoch_ms: 3000,
                        },
                    },
                },
                committed_at_ms: 3000,
            },
        )
        .unwrap();
        drop(store);
        let before: serde_json::Value = serde_json::from_str(
            &support::current_checkpoint(&database, "completion-conversation").checkpoint_json,
        )
        .unwrap();
        assert!(before["state"]["conversation"]["transcript"]
            .to_string()
            .contains("Previous build completed."));
        assert_eq!(
            before["state"]["goal_projection"]["active"]["id"],
            "durable-goal"
        );
        replace_checkpoint_binding(&database, &binding);

        let mut store = NolaStore::open(&database).unwrap();
        // Desktop performs a configuration submission BEFORE beginning a Run.
        // Testing BeginAssistantTurn alone misses the actual bootstrap path.
        nola_application::reduce_and_commit_idle_submission(
            &mut store,
            nola_application::ReduceAndCommitIdleSubmission {
                conversation_id: "completion-conversation".into(),
                core_config: runtime_config(),
                submission: AgentSubmission::UpdateSessionConfiguration {
                    patch: nola_agent_engine::agent::AgentSessionConfigurationPatch {
                        max_model_iterations: Some(17),
                        ..Default::default()
                    },
                },
                committed_at_ms: 3100,
            },
        ).unwrap();
        let prepared: serde_json::Value = serde_json::from_str(
            &support::current_checkpoint(&database, "completion-conversation").checkpoint_json,
        ).unwrap();
        assert_eq!(prepared["state"]["conversation"]["transcript"],
            before["state"]["conversation"]["transcript"]);
        assert_eq!(prepared["state"]["goal_projection"], before["state"]["goal_projection"]);
        let frontier = support::admission_frontier::read(&store, "completion-conversation");
        let next = begin_assistant_turn(&mut store, follow_up_command(frontier)).unwrap();
        assert_ne!(next.begin.run_id, old_run);
        assert_eq!(
            store
                .active_run("completion-conversation")
                .unwrap()
                .unwrap()
                .run_id,
            next.begin.run_id
        );
        drop(store);
        let after: serde_json::Value = serde_json::from_str(
            &support::current_checkpoint(&database, "completion-conversation").checkpoint_json,
        )
        .unwrap();
        assert_eq!(
            after["state"]["goal_projection"],
            before["state"]["goal_projection"]
        );
        let old_items = before["state"]["conversation"]["transcript"]
            .as_array()
            .unwrap();
        let new_items = after["state"]["conversation"]["transcript"]
            .as_array()
            .unwrap();
        assert!(
            new_items.starts_with(old_items),
            "A new build must retain the complete canonical transcript"
        );
        let db = rusqlite::Connection::open(&database).unwrap();
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
                .unwrap(),
            3
        );
    }
}

#[test]
fn obsolete_active_run_cannot_be_replaced_by_new_build_configuration() {
    for binding in obsolete_host_bindings() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("active-cross-build.db");
        let (run, _, _) = successor_fixture(&database);
        replace_checkpoint_binding(&database, &binding);
        let db = rusqlite::Connection::open(&database).unwrap();
        let before = canonical_rows(&db);
        let mut store = NolaStore::open(&database).unwrap();
        let idle_error = nola_application::reduce_and_commit_idle_submission(
            &mut store,
            nola_application::ReduceAndCommitIdleSubmission {
                conversation_id: "completion-conversation".into(),
                core_config: runtime_config(),
                submission: AgentSubmission::UpdateSessionConfiguration {
                    patch: Default::default(),
                },
                committed_at_ms: 3100,
            },
        ).unwrap_err();
        assert!(idle_error.contains("cannot overlap an active Run"), "{idle_error}");
        assert_eq!(canonical_rows(&db), before);
        let frontier = support::admission_frontier::read(&store, "completion-conversation");
        let error = begin_assistant_turn(&mut store, follow_up_command(frontier)).unwrap_err();
        assert!(error.contains("cannot overlap an active Run"), "{error}");
        assert_eq!(
            store
                .active_run("completion-conversation")
                .unwrap()
                .unwrap()
                .run_id,
            run
        );
        assert_eq!(
            canonical_rows(&db),
            before,
            "Rejected admission must not rewrite history, output or pending effects"
        );
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
                .unwrap(),
            2
        );
    }
}

fn recovery_rows(db: &rusqlite::Connection) -> Vec<Vec<String>> {
    let mut rows = canonical_rows(db);
    for table in ["runs", "trace_journal_runs", "session_run_leases"] {
        let mut query = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
            .unwrap();
        let columns = query.column_count();
        rows.push(
            query
                .query_map([], |row| {
                    Ok((0..columns)
                        .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        );
    }
    rows
}

fn traces(db: &rusqlite::Connection, turn: &str) {
    db.execute("INSERT INTO trace_journal_runs(trace_id,run_id,conversation_id,turn_id,status,started_at,updated_at) VALUES('current','diagnostic-only','completion-conversation',?1,'running','before','before')", [turn]).unwrap();
    db.execute("INSERT INTO trace_journal_runs(trace_id,run_id,conversation_id,turn_id,status,started_at,updated_at) VALUES('stale','other-diagnostic','completion-conversation','old-turn','running','before','before')", []).unwrap();
}

#[test]
fn scoped_recovery_respects_a_live_session_lease_without_touching_trace_or_history() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("lease.db");
    let (_, _, requests) = successor_fixture(&database);
    let db = rusqlite::Connection::open(&database).unwrap();
    traces(&db, requests[0].turn_id.as_deref().unwrap());
    db.execute("INSERT INTO session_run_leases(session_id,owner,pid,process_start,acquired_at_ms) VALUES('completion-conversation','live-owner',123,'process-identity',1000)", []).unwrap();
    let before = recovery_rows(&db);
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime.execute(recovery(true)).unwrap();
    drop(runtime);
    assert_eq!(
        recovery_rows(&db),
        before,
        "An explicit conversation scope cannot override a live ownership lease"
    );
}

#[test]
fn corrupt_checkpoint_recovery_fails_without_mutating_canonical_or_trace_records() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("corrupt.db");
    let (_, _, requests) = successor_fixture(&database);
    let db = rusqlite::Connection::open(&database).unwrap();
    traces(&db, requests[0].turn_id.as_deref().unwrap());
    db.execute("UPDATE core_checkpoint_bodies SET checkpoint_json='{}'", [])
        .unwrap();
    let before = recovery_rows(&db);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(
        runtime.execute(recovery(false)).is_err(),
        "Unknown checkpoint identity must fail closed"
    );
    drop(runtime);
    assert_eq!(recovery_rows(&db), before);
}

#[test]
fn recovery_trace_and_receipt_faults_roll_back_then_cold_replay_is_exact() {
    for trace_fault in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("rollback.db");
        let (run, _, requests) = successor_fixture(&database);
        let db = rusqlite::Connection::open(&database).unwrap();
        traces(&db, requests[0].turn_id.as_deref().unwrap());
        let before = recovery_rows(&db);
        let command_json = serde_json::to_string(&recovery(false)).unwrap();
        let receipt = |id: &str| ExecuteReceivedCommand {
            client_id: "recovery-client".into(),
            request_id: id.into(),
            command_type: "recover_orphaned_runs".into(),
            command_json: command_json.clone(),
            expected_revision: None,
            received_at: "4000".into(),
        };
        db.execute_batch(if trace_fault {
            "CREATE TRIGGER fail_recovery BEFORE UPDATE ON trace_journal_runs BEGIN SELECT RAISE(ABORT,'trace recovery fault'); END"
        } else {
            "CREATE TRIGGER fail_recovery BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'receipt recovery fault'); END"
        }).unwrap();
        let mut runtime = support::open_runtime(&database).unwrap();
        let failed = runtime.execute_received_command(receipt("fault"), recovery(false));
        if trace_fault {
            let outcome = failed.unwrap().outcome;
            assert!(
                matches!(outcome, nola_application::ApplicationCommandReceiptOutcome::Rejected { ref message, .. }
                if message.contains("trace recovery fault")),
                "Unexpected receipt: {outcome:?}"
            );
        } else {
            let error = failed.unwrap_err();
            assert!(
                error.message.contains("receipt recovery fault"),
                "Unexpected rejection: {error}"
            );
        }
        drop(runtime);
        assert_eq!(
            recovery_rows(&db),
            before,
            "Failure cannot publish partial startup changes"
        );
        db.execute_batch("DROP TRIGGER fail_recovery").unwrap();
        let mut runtime = support::open_runtime(&database).unwrap();
        let committed = runtime
            .execute_received_command(receipt("success"), recovery(false))
            .unwrap();
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        let after = recovery_rows(&db);
        let replay = runtime
            .execute_received_command(receipt("success"), recovery(false))
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, committed.outcome);
        assert_eq!(recovery_rows(&db), after);
        drop(runtime);
        let store = NolaStore::open(&database).unwrap();
        assert_eq!(
            store
                .active_run("completion-conversation")
                .unwrap()
                .unwrap()
                .run_id,
            run
        );
        assert_eq!(
            serde_json::to_value(load_committed_host_requests(&store, &run).unwrap().requests)
                .unwrap(),
            serde_json::to_value(&requests).unwrap()
        );
        let state = |trace: &str| {
            db.query_row::<String, _, _>(
                "SELECT status FROM trace_journal_runs WHERE trace_id=?1",
                [trace],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(state("current"), "running");
        assert_eq!(state("stale"), "interrupted");
    }
}
