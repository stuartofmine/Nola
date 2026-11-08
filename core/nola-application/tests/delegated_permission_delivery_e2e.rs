// SPDX-License-Identifier: Apache-2.0
//! Isolated SQLite Application proof of permission delivery admission; no external transport is run.
mod support;
use nola_application::{
    begin_assistant_turn, load_committed_host_requests, ApplicationCommand, BeginAssistantTurn,
    ClaimCommittedHostRequests, CommittedHostRequests, HeartbeatEffectDelivery,
    ReduceAndAcceptHostResponse,
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
    capabilities.insert(
        "agent.delegate".into(),
        runtime_capability("agent.delegate", HostCapabilityRoute::Model),
    );
    let mut config = AgentCoreConfig {
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
    };
    config.turn_identity.execution_backend =
        nola_domain::execution_backend::ExecutionBackendSelection {
            backend_id: "codex".into(),
            ..Default::default()
        };
    // This isolated protocol Host has a non-Git workspace. Like production
    // composition, admit its exact execution context before requesting a harness
    // effect; permission-delivery tests must not bypass workspace admission.
    use nola_host_protocol::delegation::{
        DelegatedModel, DelegatedRuntimeConfiguration, DelegationParentSnapshot,
        ValidatedDelegationParent,
    };
    config.turn_identity.delegation = Some(
        ValidatedDelegationParent::new(DelegationParentSnapshot {
            process_sandbox: None,
            bounded_file_tools: Default::default(),
            project_repository_identity: None,
            task_worktree: None,
            task_sandbox_requirement: None,
            authorized_workspace_roots: vec!["/fixture".into()],
            requested_native_sandbox: None,
            model: DelegatedModel {
                endpoint: String::new(),
                model: "test-model".into(),
                swe_mode: false,
            },
            runtime: DelegatedRuntimeConfiguration::default(),
            credential: None,
            fallback_credentials: vec![],
            profiles: nola_domain::delegation::effective_agent_profiles(&Default::default()),
            project_root: "/fixture".into(),
            read_only_tools: Default::default(),
            profile_credentials: Default::default(),
            execution_backend: "codex".into(),
            external_execution_model: None,
            external_execution_model_provider: None,
            max_iterations: config.max_model_iterations,
        })
        .unwrap(),
    );
    config.turn_identity.prompt.stable_prefix_digest = "a".repeat(64);
    config
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
            response: response_for_request(request, 1, payload),
            received_at_ms: 2000,
        },
    }
}

use nola_domain::{
    DelegatedInvocationRecord, DelegatedInvocationState, DelegatedPermissionDecision,
    DelegatedPermissionDeliveryRecord, DelegatedPermissionDeliveryState,
    EffectDeliveryAttemptState,
};
use nola_host_protocol::{
    DelegatedAgentEventV1, DelegatedPermissionOccurrence, HostPermissionResolution,
};
use std::path::Path;

struct Fixture {
    _directory: tempfile::TempDir,
    database: std::path::PathBuf,
    run: String,
    digest: String,
    delegated: HostRequestEnvelope,
    control: HostRequestEnvelope,
    delivery_id: String,
}

fn pending(database: &Path, run: &str) -> Vec<HostRequestEnvelope> {
    load_committed_host_requests(&mut NolaStore::open(database).unwrap(), run)
        .unwrap()
        .requests
}

fn command(
    request: &HostRequestEnvelope,
    run: &str,
    digest: &str,
    sequence: u64,
    payload: HostResponse,
) -> ApplicationCommand {
    let mut result = response_command(request, run, digest, payload);
    if let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut result {
        input.response.sequence = sequence;
    }
    result
}

fn claim_and_start(
    runtime: &mut support::ApplicationRuntime,
    run: &str,
    digest: &str,
    effect_id: &HostEffectId,
    now: i64,
) -> HostRequestEnvelope {
    claim_and_start_as(
        runtime,
        run,
        digest,
        effect_id,
        now,
        &format!("fixture-generation-{}", uuid::Uuid::new_v4()),
        &format!("fixture-owner-{}", uuid::Uuid::new_v4()),
    )
}

fn claim_and_start_as(
    runtime: &mut support::ApplicationRuntime,
    run: &str,
    digest: &str,
    effect_id: &HostEffectId,
    now: i64,
    worker_generation: &str,
    lease_owner: &str,
) -> HostRequestEnvelope {
    let claimed: CommittedHostRequests = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ClaimCommittedHostRequests {
                input: ClaimCommittedHostRequests {
                    run_id: run.into(),
                    worker_generation: worker_generation.into(),
                    lease_owner: lease_owner.into(),
                    claimed_at_ms: now,
                    lease_expires_at_ms: now + 60_000,
                    limit: 64,
                },
            })
            .unwrap(),
    )
    .unwrap();
    let mut request = claimed
        .requests
        .into_iter()
        .find(|request| &request.effect_id == effect_id)
        .expect("fixture must explicitly claim its pending Host effect");
    let sequence = request.resume_after_sequence.unwrap_or(0) + 1;
    let mut started = command(
        &request,
        run,
        digest,
        sequence,
        HostResponse::ExecutionStarted,
    );
    if let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut started {
        input.received_at_ms = now + 1;
    }
    runtime.execute(started).unwrap();
    request.resume_after_sequence = Some(sequence);
    request
}

fn fixture() -> Fixture {
    fixture_with_resolution(HostPermissionResolution::allow_once())
}

fn fixture_with_resolution(resolution: HostPermissionResolution) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("permission.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "completion-conversation".into(),
            conversation_title: "Permission delivery".into(),
            user_message_id: "initial".into(),
            user_part_id: "initial:body".into(),
            user_text: "initial".into(),
            user_content_json: None,
            assistant_message_id: "submit-initial:assistant".into(),
            context_checkpoint_id: None,
            core_config: runtime_config(),
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("initial".into()),
                    "initial",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("submit-initial".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        },
    )
    .unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    drop(store);
    let delegated_pending = pending(&database, &run)
        .into_iter()
        .find(|r| matches!(r.payload, HostRequest::DelegatedAgent { .. }))
        .unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    let delegated = claim_and_start(
        &mut runtime,
        &run,
        &digest,
        &delegated_pending.effect_id,
        1_100,
    );
    for (sequence, payload) in [
        (
            delegated.resume_after_sequence.unwrap() + 1,
            HostResponse::DelegatedAgentEvent {
                event: DelegatedAgentEventV1::SessionStarted {
                    backend_id: "codex".into(),
                    backend_session_id: "original-session".into(),
                },
            },
        ),
        (
            delegated.resume_after_sequence.unwrap() + 2,
            HostResponse::DelegatedAgentEvent {
                event: DelegatedAgentEventV1::PermissionRequested {
                    occurrence: DelegatedPermissionOccurrence {
                        occurrence_id: "original-occurrence".into(),
                        worker_generation: "original-generation".into(),
                    },
                    request_id: "original-rpc".into(),
                    call_id: "original-call".into(),
                    capability_id: "filesystem.write".into(),
                    resource: "/fixture/result.txt".into(),
                    reason: "Write the requested fixture".into(),
                    input: json!({"path":"/fixture/result.txt","content":"保留原始内容"}),
                },
            },
        ),
    ] {
        runtime
            .execute(command(&delegated, &run, &digest, sequence, payload))
            .unwrap();
    }
    let ask_pending = pending(&database, &run)
        .into_iter()
        .find(|r| matches!(r.payload, HostRequest::RequestPermission { .. }))
        .expect("explicit permission must be requested");
    let ask = claim_and_start(&mut runtime, &run, &digest, &ask_pending.effect_id, 1_400);
    let HostRequest::RequestPermission {
        permission_request_id,
        ..
    } = &ask.payload
    else {
        unreachable!()
    };
    runtime
        .execute(command(
            &ask,
            &run,
            &digest,
            ask.resume_after_sequence.unwrap() + 1,
            HostResponse::PermissionResolved {
                request_id: permission_request_id.clone(),
                resolution,
            },
        ))
        .unwrap();
    let control_pending = pending(&database, &run)
        .into_iter()
        .find(|r| matches!(r.payload, HostRequest::ResolveDelegatedPermission { .. }))
        .unwrap();
    let control = claim_and_start(
        &mut runtime,
        &run,
        &digest,
        &control_pending.effect_id,
        1_700,
    );
    drop(runtime);
    let HostRequest::ResolveDelegatedPermission {
        delivery: Some(delivery),
        ..
    } = &control.payload
    else {
        panic!("Application must bind a canonical delivery")
    };
    let delivery_id = delivery.delivery_id.clone();
    Fixture {
        _directory: directory,
        database,
        run,
        digest,
        delegated,
        control,
        delivery_id,
    }
}

impl Fixture {
    fn record(&self) -> DelegatedPermissionDeliveryRecord {
        NolaStore::open(&self.database)
            .unwrap()
            .delegated_permission(&self.delivery_id)
            .unwrap()
            .unwrap()
    }
    fn command(&self, sequence: u64, payload: HostResponse) -> ApplicationCommand {
        command(&self.control, &self.run, &self.digest, sequence, payload)
    }
    fn claim(&self) -> ApplicationCommand {
        self.command(
            self.control.resume_after_sequence.unwrap_or(0) + 1,
            HostResponse::DelegatedPermissionClaimed {
                delivery_id: self.delivery_id.clone(),
            },
        )
    }
    fn written(&self) -> ApplicationCommand {
        self.command(
            self.control.resume_after_sequence.unwrap_or(0) + 2,
            HostResponse::DelegatedPermissionTransportWritten {
                delivery_id: self.delivery_id.clone(),
            },
        )
    }
}

fn canonical_rows(database: &Path) -> BTreeMap<String, Vec<String>> {
    let mut db = rusqlite::Connection::open(database).unwrap();
    let snapshot = db.transaction().unwrap();
    [
        "conversations",
        "conversation_settings",
        "runs",
        "run_invocation_payloads",
        "messages",
        "message_parts",
        "canonical_text_objects",
        "canonical_text_chunks",
        "message_part_body_references",
        "core_checkpoints",
        "core_checkpoint_bodies",
        "core_delivery_witnesses",
        "core_checkpoint_replay_bases",
        "committed_transitions",
        "effect_outbox",
        "effect_outbox_payloads",
        "effect_delivery_attempts",
        "host_event_inbox",
        "host_event_replay_heads",
        "host_event_replay_order",
        "run_events",
        "delegated_permission_deliveries",
        "delegated_invocations",
        "delegated_invocation_deliveries",
        "run_attempts",
        "run_attempt_deliveries",
        "command_receipts",
        "assistant_turn_message_bindings",
        "assistant_turn_part_bindings",
        "run_message_admission_records",
        "run_message_admission_owners",
        "run_message_admission_stages",
        "run_message_admission_chunks",
        "assistant_journal_heads",
        "assistant_journal_records",
        "assistant_journal_parts",
        "assistant_journal_part_versions",
        "assistant_journal_chunks",
        "assistant_journal_manifests",
        "assistant_journal_manifest_chunks",
        "assistant_journal_snapshots",
        "assistant_journal_order_counters",
        "assistant_journal_order_reservations",
        "assistant_journal_publications",
        "assistant_journal_publication_frames",
        "assistant_run_terminal_seals",
        "assistant_run_terminal_manifest",
        "trace_journal_runs",
        "trace_journal_events",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = snapshot
            .prepare(&format!("SELECT * FROM {table}"))
            .unwrap();
        let count = statement.column_count();
        let mut rows = statement
            .query_map([], |row| {
                Ok((0..count)
                    .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        rows.sort();
        (table.to_owned(), rows)
    })
    .collect()
}

fn permission_events(database: &Path, delivery_id: &str) -> Vec<DelegatedPermissionDeliveryRecord> {
    let db = rusqlite::Connection::open(database).unwrap();
    let mut statement = db
        .prepare(
            "SELECT payload_json FROM run_events
             WHERE event_kind='delegated_permission_changed' ORDER BY sequence",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|raw| {
            serde_json::from_str::<DelegatedPermissionDeliveryRecord>(&raw.unwrap()).unwrap()
        })
        .filter(|record| record.id == delivery_id)
        .collect()
}

fn invocation(fixture: &Fixture) -> DelegatedInvocationRecord {
    NolaStore::open(&fixture.database)
        .unwrap()
        .delegated_invocation(
            fixture
                .delegated
                .delegated_invocation_id
                .as_deref()
                .expect("delegated request has canonical invocation"),
        )
        .unwrap()
        .unwrap()
}

fn ownership_rows(database: &Path) -> Vec<Vec<String>> {
    let db = rusqlite::Connection::open(database).unwrap();
    [
        "SELECT id,run_id,effect_id,backend_id,text_part_id,reasoning_part_id FROM delegated_invocations ORDER BY id",
        "SELECT request_id,invocation_id FROM delegated_invocation_deliveries ORDER BY request_id",
        "SELECT id,run_id,invocation_id,effect_id,text_part_id,reasoning_part_id,predecessor_id FROM run_attempts ORDER BY id",
        "SELECT request_id,attempt_id FROM run_attempt_deliveries ORDER BY request_id",
    ]
    .into_iter()
    .map(|sql| {
        let mut statement = db.prepare(sql).unwrap();
        let count = statement.column_count();
        statement
            .query_map([], |row| {
                Ok((0..count)
                    .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
    .collect()
}

fn assert_owner_isolation(fixture: &Fixture) {
    let owner = invocation(fixture);
    let store = NolaStore::open(&fixture.database).unwrap();
    let delegated = store
        .delegated_text_part_owner("completion-conversation", &owner.text_part_id)
        .unwrap()
        .expect("delegated text Part keeps its invocation owner");
    assert_eq!(delegated.run_id, fixture.run);
    assert_eq!(delegated.invocation_id, owner.id);
    assert_eq!(delegated.request_id, fixture.delegated.request_id);
    assert_eq!(delegated.text_part_id, owner.text_part_id);
    assert_eq!(delegated.reasoning_part_id, owner.reasoning_part_id);
    assert_eq!(
        store
            .delegated_request_parts(&fixture.delegated.request_id)
            .unwrap(),
        Some(delegated)
    );
    assert_eq!(
        store
            .model_text_part_owner("completion-conversation", &owner.text_part_id)
            .unwrap(),
        None,
        "delegated output must never acquire a model Attempt owner"
    );
    assert_eq!(
        rusqlite::Connection::open(&fixture.database)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM run_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn long_delegated_permission_wait_renews_parent_atomically_past_original_expiry() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("long-permission-wait.db");
    let begun = begin_assistant_turn(
        &mut NolaStore::open(&database).unwrap(),
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "completion-conversation".into(),
            conversation_title: "Long permission wait".into(),
            user_message_id: "initial".into(),
            user_part_id: "initial:body".into(),
            user_text: "initial".into(),
            user_content_json: None,
            assistant_message_id: "submit-initial:assistant".into(),
            context_checkpoint_id: None,
            core_config: runtime_config(),
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("initial".into()),
                    "initial",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("submit-initial".into()),
                submitted_at_epoch_ms: Some(1_000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    let generation = "long-wait-generation";
    let owner = "long-wait-owner";
    let mut runtime = support::open_runtime(&database).unwrap();
    let delegated_pending = pending(&database, &run)
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::DelegatedAgent { .. }))
        .unwrap();
    let delegated = claim_and_start_as(
        &mut runtime,
        &run,
        &digest,
        &delegated_pending.effect_id,
        1_100,
        generation,
        owner,
    );
    for (sequence, payload) in [
        (
            delegated.resume_after_sequence.unwrap() + 1,
            HostResponse::DelegatedAgentEvent {
                event: DelegatedAgentEventV1::SessionStarted {
                    backend_id: "codex".into(),
                    backend_session_id: "long-wait-session".into(),
                },
            },
        ),
        (
            delegated.resume_after_sequence.unwrap() + 2,
            HostResponse::DelegatedAgentEvent {
                event: DelegatedAgentEventV1::PermissionRequested {
                    occurrence: DelegatedPermissionOccurrence {
                        occurrence_id: "long-wait-occurrence".into(),
                        worker_generation: "backend-generation".into(),
                    },
                    request_id: "long-wait-rpc".into(),
                    call_id: "long-wait-call".into(),
                    capability_id: "filesystem.write".into(),
                    resource: "/fixture/result.txt".into(),
                    reason: "Write the requested fixture".into(),
                    input: json!({"path":"/fixture/result.txt"}),
                },
            },
        ),
    ] {
        runtime
            .execute(command(&delegated, &run, &digest, sequence, payload))
            .unwrap();
    }
    let ask_pending = pending(&database, &run)
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::RequestPermission { .. }))
        .unwrap();
    let ask = claim_and_start_as(
        &mut runtime,
        &run,
        &digest,
        &ask_pending.effect_id,
        1_400,
        generation,
        owner,
    );
    let ask_lease = ask.delivery.clone().unwrap();
    let parent_lease = delegated.delivery.clone().unwrap();
    let heartbeat = |current_expiry, heartbeat_at, requested_expiry| {
        ApplicationCommand::HeartbeatEffectDelivery {
            input: HeartbeatEffectDelivery {
                run_id: run.clone(),
                attempt_id: ask_lease.attempt_id.clone(),
                worker_generation: generation.into(),
                lease_owner: owner.into(),
                lease_token: ask_lease.lease_token.clone(),
                current_lease_expires_at_ms: current_expiry,
                heartbeat_at_ms: heartbeat_at,
                requested_lease_expires_at_ms: requested_expiry,
            },
        }
    };

    runtime
        .execute(heartbeat(ask_lease.lease_expires_at_ms, 60_000, 120_000))
        .unwrap();
    let store = NolaStore::open(&database).unwrap();
    let renewed_ask = store
        .effect_delivery_attempt(&ask_lease.attempt_id)
        .unwrap()
        .unwrap();
    let renewed_parent = store
        .effect_delivery_attempt(&parent_lease.attempt_id)
        .unwrap()
        .unwrap();
    assert_eq!(renewed_ask.lease_expires_at_ms, Some(120_000));
    assert_eq!(renewed_parent.lease_expires_at_ms, Some(120_000));
    assert_eq!(
        renewed_parent.state,
        EffectDeliveryAttemptState::ExecutionStarted
    );

    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute_batch(&format!(
        "CREATE TRIGGER fail_linked_parent_heartbeat BEFORE UPDATE ON effect_delivery_attempts \
         WHEN OLD.id='{}' BEGIN SELECT RAISE(ABORT,'linked parent heartbeat fault'); END;",
        parent_lease.attempt_id
    ))
    .unwrap();
    assert!(runtime
        .execute(heartbeat(120_000, 80_000, 140_000))
        .is_err());
    let after_fault = NolaStore::open(&database).unwrap();
    assert_eq!(
        after_fault
            .effect_delivery_attempt(&ask_lease.attempt_id)
            .unwrap()
            .unwrap()
            .lease_expires_at_ms,
        Some(120_000),
        "the visible wait heartbeat must roll back with its parked parent"
    );
    db.execute_batch("DROP TRIGGER fail_linked_parent_heartbeat")
        .unwrap();

    runtime
        .execute(heartbeat(120_000, 100_000, 160_000))
        .unwrap();
    let final_store = NolaStore::open(&database).unwrap();
    assert_eq!(
        final_store
            .effect_delivery_attempt(&parent_lease.attempt_id)
            .unwrap()
            .unwrap()
            .lease_expires_at_ms,
        Some(160_000),
        "the parent remains live after its original 61.1s expiry"
    );
}

#[test]
fn canonical_delivery_binds_original_occurrence_and_requires_committed_claim() {
    let fixture = fixture();
    let prepared = fixture.record();
    assert_eq!(prepared.state, DelegatedPermissionDeliveryState::Prepared);
    assert_eq!(
        prepared.decision,
        Some(DelegatedPermissionDecision::AllowOnce)
    );
    assert!(uuid::Uuid::parse_str(&prepared.id).is_ok());
    assert_ne!(prepared.id, "original-rpc");
    assert_ne!(prepared.id, "original-occurrence");
    assert_eq!(prepared.run_id, fixture.run);
    assert_eq!(
        Some(&prepared.invocation_id),
        fixture.delegated.delegated_invocation_id.as_ref()
    );
    assert_eq!(prepared.target_effect_id, fixture.delegated.effect_id.0);
    assert_eq!(prepared.worker_generation, "original-generation");
    assert_eq!(prepared.occurrence_id, "original-occurrence");
    assert_eq!(prepared.backend_id, "codex");
    assert_eq!(prepared.backend_session_id, "original-session");
    assert_eq!(prepared.rpc_request_id, "original-rpc");
    assert_eq!(prepared.call_id, "original-call");
    assert_eq!(
        prepared.request_json,
        json!({"path":"/fixture/result.txt", "content":"保留原始内容"})
    );
    assert_eq!(
        prepared.request_digest,
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&prepared.request_json).unwrap())
        )
    );
    assert_eq!(
        prepared.control_effect_id.as_deref(),
        Some(fixture.control.effect_id.0.as_str())
    );
    assert_eq!(
        prepared.control_request_id.as_deref(),
        Some(fixture.control.request_id.as_str())
    );
    let prepared_events = permission_events(&fixture.database, &fixture.delivery_id);
    assert_eq!(prepared_events.len(), 2);
    assert_eq!(
        prepared_events
            .iter()
            .map(|record| (record.state, record.revision))
            .collect::<Vec<_>>(),
        vec![
            (DelegatedPermissionDeliveryState::AwaitingDecision, 1),
            (DelegatedPermissionDeliveryState::Prepared, 2),
        ]
    );
    assert_eq!(prepared_events.last(), Some(&prepared));
    assert_eq!(
        invocation(&fixture).state,
        DelegatedInvocationState::WaitingPermission
    );
    assert_owner_isolation(&fixture);
    let ownership = ownership_rows(&fixture.database);
    let before = canonical_rows(&fixture.database);
    let mut runtime = support::open_runtime(&fixture.database).unwrap();
    assert!(runtime
        .execute(fixture.command(
            fixture.control.resume_after_sequence.unwrap_or(0) + 1,
            HostResponse::DelegatedPermissionTransportWritten {
                delivery_id: fixture.delivery_id.clone()
            }
        ))
        .is_err());
    assert!(runtime
        .execute(fixture.command(
            fixture.control.resume_after_sequence.unwrap_or(0) + 1,
            HostResponse::DelegatedPermissionClaimed {
                delivery_id: "forged-delivery".into()
            }
        ))
        .is_err());
    assert_eq!(canonical_rows(&fixture.database), before);
    runtime.execute(fixture.claim()).unwrap();
    let claimed = fixture.record();
    assert_eq!(claimed.state, DelegatedPermissionDeliveryState::Claimed);
    assert_eq!(claimed.revision, prepared.revision + 1);
    assert!(claimed.claimed_at_ms.is_some());
    assert!(claimed.written_at_ms.is_none());
    assert_eq!(
        permission_events(&fixture.database, &fixture.delivery_id)
            .iter()
            .map(|record| (record.state, record.revision))
            .collect::<Vec<_>>(),
        vec![
            (DelegatedPermissionDeliveryState::AwaitingDecision, 1),
            (DelegatedPermissionDeliveryState::Prepared, 2),
            (DelegatedPermissionDeliveryState::Claimed, 3),
        ]
    );
    assert_eq!(ownership_rows(&fixture.database), ownership);
    runtime.execute(fixture.written()).unwrap();
    let written = fixture.record();
    assert_eq!(
        written.state,
        DelegatedPermissionDeliveryState::TransportWritten
    );
    assert!(written.written_at_ms.is_some());
    assert_eq!(
        permission_events(&fixture.database, &fixture.delivery_id),
        vec![
            prepared_events[0].clone(),
            prepared.clone(),
            claimed,
            written.clone(),
        ]
    );
    assert_eq!(ownership_rows(&fixture.database), ownership);
    assert_eq!(
        invocation(&fixture).state,
        DelegatedInvocationState::Running
    );
    assert_owner_isolation(&fixture);
    assert!(!pending(&fixture.database, &fixture.run)
        .iter()
        .any(|r| r.effect_id == fixture.control.effect_id));
    let store = NolaStore::open(&fixture.database).unwrap();
    assert_eq!(
        store
            .delegated_permission_for_occurrence(&prepared.invocation_id, "original-occurrence")
            .unwrap(),
        Some(written.clone())
    );
    assert_eq!(
        store
            .delegated_permission_for_control(&fixture.control.effect_id.0)
            .unwrap(),
        Some(written)
    );
}

#[test]
fn claim_final_receipt_failure_rolls_back_then_same_request_cold_replays_exactly() {
    let fixture = fixture();
    let prepared = fixture.record();
    let before = canonical_rows(&fixture.database);
    let command_json = serde_json::to_string(&fixture.claim()).unwrap();
    let receipt = || nola_application::ExecuteReceivedCommand {
        client_id: "permission-delivery-client".into(),
        request_id: "permission-claim".into(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "2000".into(),
    };
    let db = rusqlite::Connection::open(&fixture.database).unwrap();
    db.execute_batch("CREATE TRIGGER fail_permission_claim_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT CASE WHEN EXISTS(SELECT 1 FROM delegated_permission_deliveries WHERE state='claimed') THEN RAISE(ABORT,'permission claim final receipt fault') ELSE RAISE(ABORT,'claim missing before final receipt') END; END").unwrap();
    let mut runtime = support::open_runtime(&fixture.database).unwrap();
    let error = runtime
        .execute_received_command(receipt(), fixture.claim())
        .unwrap_err();
    assert!(
        error
            .message
            .contains("permission claim final receipt fault"),
        "{error}"
    );
    assert_eq!(fixture.record(), prepared);
    assert_eq!(canonical_rows(&fixture.database), before);
    db.execute_batch("DROP TRIGGER fail_permission_claim_receipt")
        .unwrap();
    let accepted = runtime
        .execute_received_command(receipt(), fixture.claim())
        .unwrap();
    assert!(matches!(
        accepted.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    assert_eq!(
        fixture.record().state,
        DelegatedPermissionDeliveryState::Claimed
    );
    let committed = canonical_rows(&fixture.database);
    drop(runtime);
    let mut runtime = support::open_runtime(&fixture.database).unwrap();
    let replay = runtime
        .execute_received_command(receipt(), fixture.claim())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, accepted.outcome);
    assert_eq!(canonical_rows(&fixture.database), committed);
    assert_eq!(fixture.record().revision, prepared.revision + 1);
    let receipt_row = rusqlite::Connection::open(&fixture.database)
        .unwrap()
        .query_row(
            "SELECT command_type,command_json,status,result_json,error_json,conflict_json
             FROM command_receipts WHERE client_id=?1 AND request_id=?2",
            ["permission-delivery-client", "permission-claim"],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(receipt_row.0, "reduce_and_accept_host_response");
    assert_eq!(receipt_row.1, command_json);
    assert_eq!(receipt_row.2, "committed");
    assert!(receipt_row.3.is_some());
    assert_eq!(receipt_row.4, None);
    assert_eq!(receipt_row.5, None);
    assert_owner_isolation(&fixture);
}

#[test]
fn cold_recovery_after_claim_preserves_uncertainty_and_rejects_new_delivery() {
    let fixture = fixture();
    let mut runtime = support::open_runtime(&fixture.database).unwrap();
    runtime.execute(fixture.claim()).unwrap();
    let claimed = fixture.record();
    let ownership = ownership_rows(&fixture.database);
    drop(runtime);
    let mut runtime = support::open_runtime(&fixture.database).unwrap();
    runtime
        .execute(ApplicationCommand::RecoverOrphanedRuns {
            conversation_id: Some("completion-conversation".into()),
            reason: "server_restarted".into(),
            interrupted_at: "after-claim".into(),
            interrupted_at_ms: 3000,
        })
        .unwrap();
    let unknown = fixture.record();
    assert_eq!(
        unknown.state,
        DelegatedPermissionDeliveryState::OutcomeUnknown
    );
    assert_eq!(unknown.claimed_at_ms, claimed.claimed_at_ms);
    assert_eq!(unknown.written_at_ms, None);
    assert_eq!(unknown.decision, claimed.decision);
    assert_eq!(unknown.revision, claimed.revision + 1);
    let events = permission_events(&fixture.database, &fixture.delivery_id);
    assert_eq!(events.len(), 4);
    assert_eq!(
        events
            .iter()
            .map(|record| (record.state, record.revision))
            .collect::<Vec<_>>(),
        vec![
            (DelegatedPermissionDeliveryState::AwaitingDecision, 1),
            (DelegatedPermissionDeliveryState::Prepared, 2),
            (DelegatedPermissionDeliveryState::Claimed, 3),
            (DelegatedPermissionDeliveryState::OutcomeUnknown, 4),
        ]
    );
    assert_eq!(events[2], claimed);
    assert_eq!(events[3], unknown);
    assert_eq!(ownership_rows(&fixture.database), ownership);
    assert_owner_isolation(&fixture);
    let before = canonical_rows(&fixture.database);
    assert!(runtime
        .execute(fixture.command(
            fixture.control.resume_after_sequence.unwrap_or(0) + 2,
            HostResponse::DelegatedPermissionClaimed {
                delivery_id: fixture.delivery_id.clone()
            }
        ))
        .is_err());
    assert!(runtime.execute(fixture.written()).is_err());
    assert_eq!(canonical_rows(&fixture.database), before);
    let requests = pending(&fixture.database, &fixture.run);
    assert!(
        requests
            .iter()
            .all(|r| !matches!(r.payload, HostRequest::ResolveDelegatedPermission { .. })),
        "OutcomeUnknown must not redeliver a control operation: {requests:?}"
    );
}

// Terminal coverage moved from the non-authoritative in-memory publication
// adapter. Every seal below is produced by real Core events and SQLite indexes.
#[test]
fn delegated_terminal_results_preserve_real_source_and_reopen_for_both_cancel_orders() {
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    for mode in ["answer", "control_first", "target_first", "preserve_unknown", "internal_failure"] {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("delegated-terminal.sqlite");
        let mut store = NolaStore::open(&database).unwrap();
        let begun = begin_assistant_turn(&mut store, BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "completion-conversation".into(), conversation_title: "Terminal".into(),
            user_message_id: "user".into(), user_part_id: "user:text".into(), user_text: "Hello".into(),
            user_content_json: None, assistant_message_id: "terminal:assistant".into(), context_checkpoint_id: None,
            core_config: runtime_config(), submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(ItemId::from_string("user".into()), "Hello")],
                turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                submission_id: Some("terminal".into()), submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            }, created_at_ms: 1000,
        }).unwrap();
        let run = begun.begin.run_id;
        let digest = begun.invocation_config_digest;
        drop(store);
        let initial = pending(&database, &run).into_iter().find(|request|
            matches!(request.payload, HostRequest::DelegatedAgent { .. })).unwrap();
        let mut runtime = support::open_runtime(&database).unwrap();
        let request = claim_and_start(&mut runtime, &run, &digest, &initial.effect_id, 1100);
        runtime.execute(command(&request, &run, &digest, 2, HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::SessionStarted {
                backend_id: "codex".into(), backend_session_id: "actual-session".into(),
            },
        })).unwrap();
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        let terminal_command = if mode == "answer" {
            runtime.execute(command(&request,&run,&digest,3,HostResponse::DelegatedAgentEvent {
                event:DelegatedAgentEventV1::ToolStarted {call_id:"external-call".into(),capability_id:"shell".into(),input:json!({"command":"fictional fixture command"})},
            })).unwrap();
            runtime.execute(command(&request,&run,&digest,4,HostResponse::DelegatedAgentEvent {
                event:DelegatedAgentEventV1::ToolCompleted {call_id:"external-call".into(),capability_id:"shell".into(),output:json!({"text":"fictional fixture output"}),
                    outcome:HostToolOutcome::Succeeded,full_output_ref:None,artifacts:vec![]},
            })).unwrap();
            let accounting=DelegatedAgentEventV1::Accounting {snapshot:nola_domain::delegated_invocation::ExternalUsageSnapshot {
                backend_id:"codex".into(),scope_id:"actual-session".into(),
                basis:nola_domain::delegated_invocation::ExternalUsageBasis::SessionCumulative,
                provider_id:None,model_id:Some("reported-model".into()),input_tokens:Some(1200),output_tokens:Some(50),
                cache_read_tokens:Some(800),cache_write_tokens:None,
            }};
            runtime.execute(command(&request,&run,&digest,5,HostResponse::DelegatedAgentEvent {event:accounting.clone()})).unwrap();
            // Exact replay and a later completion retain the original snapshot;
            // neither turns a cumulative total into another additive charge.
            runtime.execute(command(&request,&run,&digest,6,HostResponse::DelegatedAgentEvent {event:accounting})).unwrap();
            command(&request, &run, &digest, 7, HostResponse::DelegatedAgentEvent {
                event: DelegatedAgentEventV1::Completed {
                    items: vec![serde_json::to_value(AgentItem::assistant_message(
                        ItemId::from_string("answer".into()), "delegated answer")).unwrap()], usage: json!({}),
                },
            })
        } else {
            if mode == "internal_failure" {
                let HostRequest::DelegatedAgent {request: delegated} = &request.payload else { unreachable!() };
                runtime.execute(command(&request, &run, &digest, 3, HostResponse::DelegatedAgentEvent {
                    event: DelegatedAgentEventV1::TextDelta {part_id: delegated.text_part_id.clone(),
                        delta: "The same analysis is being reconsidered without new evidence or a changed result.\n".repeat(180)},
                })).unwrap();
                let store = NolaStore::open(&database).unwrap();
                assert!(store.active_run("completion-conversation").unwrap().is_some());
                assert!(store.local_assistant_run_terminal_seal("completion-conversation", &run).unwrap().is_none(),
                    "Internal failure must await the real stop receipt");
            } else {
            runtime.execute(ApplicationCommand::ReduceAndCommitActiveSubmission {
                input: nola_application::ReduceAndCommitActiveSubmission {
                    conversation_id: "completion-conversation".into(), run_id: run.clone(),
                    transition_id: "cancel-terminal".into(), invocation_config_digest: digest.clone(),
                    submission: AgentSubmission::Interrupt { goal_control: None, cancellation_id: "user-cancel".into(), reason: None },
                    committed_at_ms: 2100,
                },
            }).unwrap();
            }
            if mode == "preserve_unknown" {
                let inspected: nola_protocol::UnknownEffectRecoveryInspection = serde_json::from_value(
                    runtime.execute(ApplicationCommand::InspectUnknownEffectRecovery {
                        conversation_id:"completion-conversation".into(),expected_run_id:run.clone(),assistant_message_id:"terminal:assistant".into(),
                    }).unwrap()).unwrap();
                assert!(!inspected.retry_allowed);
                assert_eq!(inspected.independent_pending_effects,0);
                assert_eq!(inspected.targets.len(),1);
                let before_attempt=NolaStore::open(&database).unwrap().effect_delivery_attempt(&inspected.targets[0].attempt_id).unwrap().unwrap();
                let decision=ApplicationCommand::ResolveUnknownEffectRecovery {input:nola_protocol::ResolveUnknownEffectRecovery {
                    conversation_id:"completion-conversation".into(),run_id:run.clone(),invocation_config_digest:digest.clone(),
                    resolution_id:"preserve-stop-unknown".into(),targets:inspected.targets,
                    choice:nola_protocol::UnknownEffectRecoveryChoice::KeepUnknownBlocked {reason:"End Nola waiting; external execution may still continue. Do not retry or claim stop.".into()},
                    resolved_at_ms:2200,
                }};
                let decision_json=serde_json::to_string(&decision).unwrap();
                let reply=runtime.execute(serde_json::from_str(&decision_json).unwrap()).unwrap();
                assert_eq!(reply["outcome"],"unknown_preserved_blocked");
                assert_eq!(runtime.execute(decision).unwrap(),reply);
                drop(runtime);
                let store=NolaStore::open(&database).unwrap();
                let seal=store.local_assistant_run_terminal_seal("completion-conversation",&run).unwrap().unwrap();
                assert!(matches!(seal.result.value,nola_domain::run_result::RunResultValue::Blocked {..}));
                assert_eq!(store.effect_delivery_attempt(&before_attempt.id).unwrap().unwrap(),before_attempt);
                assert!(store.active_run("completion-conversation").unwrap().is_none());
                let ledger=store.delegated_invocation_for_effect(&request.effect_id.0).unwrap().unwrap();
                assert_eq!(ledger.state,nola_domain::DelegatedInvocationState::Cancelling);
                // A blocked wait is not a physical-stop receipt or permission to retry.
                assert_eq!(before_attempt.state,nola_domain::EffectDeliveryAttemptState::ExecutionStarted);
                continue;
            }
            if mode == "target_first" {
                // A target tail may be ACKed, but only the independent Cancel
                // control can commit stop confirmation or a terminal seal.
                drop(runtime);
                runtime = support::open_runtime(&database).unwrap();
                let checkpoint = support::current_checkpoint(&database, "completion-conversation");
                let store = NolaStore::open(&database).unwrap();
                let active = store.active_run("completion-conversation").unwrap().unwrap();
                assert_eq!(active.run_id, run);
                assert!(store.local_assistant_run_terminal_seal("completion-conversation", &run).unwrap().is_none());
                let before_pending = load_committed_host_requests(&store, &run).unwrap();
                assert!(before_pending.requests.iter().any(|r| matches!(&r.payload,
                    HostRequest::Cancel { target_effect_id, .. } if target_effect_id == &request.effect_id)));
                drop(store);
                let ApplicationCommand::ReduceAndAcceptHostResponse { mut input } = command(&request, &run, &digest, 3,
                    HostResponse::DelegatedAgentEvent { event: DelegatedAgentEventV1::CancellationConfirmed }) else { unreachable!() };
                input.received_at_ms = 2150;
                let target_done = input;
                let before = canonical_rows(&database);
                for _ in 0..2 {
                    let accepted: nola_application::AcceptedHostResponse = serde_json::from_value(
                        runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                            input: target_done.clone(),
                        }).unwrap()).unwrap();
                    assert!(!accepted.commit.applied);
                    assert!(accepted.commit.assistant_publications.is_empty());
                    assert_eq!(accepted.commit.checkpoint_revision, checkpoint.revision);
                    assert_eq!(accepted.commit.next_event_sequence, active.next_event_sequence);
                    assert!(accepted.events.is_empty());
                    assert!(accepted.traces.is_empty());
                    let control = accepted.control.unwrap();
                    assert_eq!(control.turn_status, nola_domain::AgentTurnStatus::Cancelling);
                    assert!(control.completion_outcome.is_none());
                    assert_eq!(canonical_rows(&database), before,
                        "target ACK must preserve checkpoint, events, receipts, ledgers and terminal source");
                }
                for damage in ["completed", "foreign_invocation", "foreign_request", "foreign_host",
                    "foreign_session", "foreign_turn", "foreign_catalog", "foreign_delivery", "model_attempt"] {
                    let mut input = target_done.clone();
                    match damage {
                        "completed" => input.response.payload = HostResponse::DelegatedAgentEvent {
                            event: DelegatedAgentEventV1::Completed {
                                items: vec![serde_json::to_value(AgentItem::assistant_message(
                                    ItemId::from_string("late-answer".into()), "MUST_NOT_PUBLISH")).unwrap()],
                                usage: json!({}),
                            },
                        },
                        "foreign_invocation" => input.response.delegated_invocation_id = Some("foreign".into()),
                        "foreign_request" => input.response.request_id = "foreign".into(),
                        "foreign_host" => input.response.host_id = HostId("foreign".into()),
                        "foreign_session" => input.response.session_id = "foreign".into(),
                        "foreign_turn" => input.response.turn_id = Some("foreign".into()),
                        "foreign_catalog" => input.response.catalog_revision += 1,
                        "foreign_delivery" => input.response.delivery.as_mut().unwrap().lease_token.push_str("-foreign"),
                        "model_attempt" => input.response.attempt_id = Some("foreign-model-attempt".into()),
                        _ => unreachable!(),
                    }
                    assert!(runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse { input }).is_err(), "{damage}");
                    assert_eq!(canonical_rows(&database), before, "{damage} changed canonical state");
                }
                let store = NolaStore::open(&database).unwrap();
                assert_eq!(store.active_run("completion-conversation").unwrap().unwrap(), active);
                assert!(store.local_assistant_run_terminal_seal("completion-conversation", &run).unwrap().is_none());
                assert_eq!(serde_json::to_value(load_committed_host_requests(&store, &run).unwrap()).unwrap(),
                    serde_json::to_value(before_pending).unwrap());
            }
            drop(runtime);
            runtime = support::open_runtime(&database).unwrap();
            let control = pending(&database, &run).into_iter()
                .find(|request| matches!(request.payload, HostRequest::Cancel { .. })).unwrap();
            let control = claim_and_start(&mut runtime, &run, &digest, &control.effect_id, 2200);
            let mut terminal = command(&control, &run, &digest,
                control.resume_after_sequence.unwrap() + 1,
                HostResponse::CancellationConfirmed { target_effect_id: request.effect_id.clone() });
            if let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut terminal {
                input.received_at_ms = 2300;
            }
            terminal
        };
        let ApplicationCommand::ReduceAndAcceptHostResponse { input: terminal_input } = terminal_command else {
            panic!("terminal fixture must preserve an exact Host response request");
        };
        if mode == "answer" {
            for damage in ["foreign_invocation", "foreign_request", "malformed_item"] {
                let before = canonical_rows(&database);
                let mut forged = terminal_input.clone();
                match damage {
                    "foreign_invocation" => forged.response.delegated_invocation_id = Some("foreign".into()),
                    "foreign_request" => forged.response.request_id = "foreign".into(),
                    _ => forged.response.payload = HostResponse::DelegatedAgentEvent {
                        event: DelegatedAgentEventV1::Completed {
                            items: vec![json!({"not":"an AgentItem"})], usage: json!({}),
                        },
                    },
                }
                assert!(runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse { input: forged }).is_err(), "{damage}");
                assert_eq!(canonical_rows(&database), before, "{damage} published partial completion");
            }
        }
        runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
            input: terminal_input.clone(),
        }).unwrap();
        drop(runtime);
        let store = NolaStore::open(&database).unwrap();
        let seal = store.local_assistant_run_terminal_seal("completion-conversation", &run).unwrap().unwrap();
        seal.validate().unwrap();
        assert_eq!(seal.result.computed_digest().unwrap(), seal.result_digest);
        if mode == "answer" {
            let record=store.delegated_invocation_for_effect(&request.effect_id.0).unwrap().unwrap();
            assert_eq!(record.usage["accounting"]["input_tokens"],1200);
            assert_eq!(record.usage["accounting"]["cache_read_tokens"],800);
            assert_eq!(record.usage["accounting"]["basis"],"session_cumulative");
            assert!(record.usage["accounting"]["provider_id"].is_null());
            use nola_store_port::run_decision::{RunDecisionQuery, RunDecisionRepository};
            // Normal open above runs outbox maintenance. Revalidate the real
            // source rather than accepting only the frozen seal JSON.
            assert!(store.load_run_decision(&RunDecisionQuery {
                conversation_id: "completion-conversation".into(), run_id: run.clone(),
                turn_id: request.turn_id.clone().unwrap(),
                core_revision: seal.result.decided_at_revision.0,
                core_event_cursor: seal.result.decision_event_cursor.0,
            }).unwrap().is_some());
        }
        match mode {
            "answer" => assert!(matches!(seal.result.value, nola_domain::run_result::RunResultValue::Answered { .. }), "{:?}", seal.result.value),
            "internal_failure" => assert!(matches!(seal.result.value, nola_domain::run_result::RunResultValue::Failed { .. })),
            _ => assert!(matches!(seal.result.value, nola_domain::run_result::RunResultValue::Cancelled { .. })),
        }
        assert!(store.pending_effects(&run).unwrap().is_empty());
        drop(store);
        let mut runtime = support::open_runtime(&database).unwrap();
        runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
            input: terminal_input,
        }).unwrap();
        if mode != "answer" {
            let before = canonical_rows(&database);
            let mut late_target = command(&request, &run, &digest, 3,
                HostResponse::DelegatedAgentEvent { event: DelegatedAgentEventV1::CancellationConfirmed });
            let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut late_target else { unreachable!() };
            input.received_at_ms = 2400;
            assert!(runtime.execute(late_target).is_err(),
                "a sealed Run no longer has a live Cancel relationship for a target no-op ACK");
            assert_eq!(canonical_rows(&database), before,
                "late target confirmation must not replace a committed Cancel receipt or terminal source");
        }
        drop(runtime);
        let store = NolaStore::open(&database).unwrap();
        assert_eq!(store.local_assistant_run_terminal_seal("completion-conversation", &run).unwrap().unwrap(), seal);
    }
}

#[test]
fn conversation_grant_creates_a_distinct_delivery_for_a_reused_backend_rpc_id() {
    let fixture = fixture_with_resolution(HostPermissionResolution::allow_conversation(None));
    let first = fixture.record();
    assert_eq!(
        first.decision,
        Some(DelegatedPermissionDecision::AllowConversation)
    );
    let mut runtime = support::open_runtime(&fixture.database).unwrap();
    runtime.execute(fixture.claim()).unwrap();
    runtime.execute(fixture.written()).unwrap();
    let written = fixture.record();
    let ownership = ownership_rows(&fixture.database);
    runtime
        .execute(command(
            &fixture.delegated,
            &fixture.run,
            &fixture.digest,
            4,
            HostResponse::DelegatedAgentEvent {
                event: DelegatedAgentEventV1::PermissionRequested {
                    occurrence: DelegatedPermissionOccurrence {
                        occurrence_id: "second-occurrence".into(),
                        worker_generation: "original-generation".into(),
                    },
                    request_id: "original-rpc".into(),
                    call_id: "second-call".into(),
                    capability_id: "filesystem.write".into(),
                    resource: "/fixture/result.txt".into(),
                    reason: "Second operation under the conversation grant".into(),
                    input: json!({"path":"/fixture/result.txt", "content":"第二次调用"}),
                },
            },
        ))
        .unwrap();
    drop(runtime);

    let requests = pending(&fixture.database, &fixture.run);
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request.payload, HostRequest::RequestPermission { .. })),
        "the conversation grant resolves the next occurrence without another user prompt"
    );
    let second_control = requests
        .into_iter()
        .find(|request| {
            matches!(
                request.payload,
                HostRequest::ResolveDelegatedPermission { .. }
            )
        })
        .expect("conversation grant creates a second control delivery");
    let HostRequest::ResolveDelegatedPermission {
        occurrence,
        delivery: Some(target),
        target_effect_id,
        backend_id,
        backend_session_id,
        permission_request_id,
        resolution,
        ..
    } = &second_control.payload
    else {
        panic!("second control must carry its canonical delivery target")
    };
    assert_eq!(occurrence.occurrence_id, "second-occurrence");
    assert_eq!(occurrence.worker_generation, first.worker_generation);
    assert_ne!(target.delivery_id, first.id);
    assert_ne!(second_control.effect_id, fixture.control.effect_id);
    assert_eq!(target.invocation_id, first.invocation_id);
    assert_eq!(target.occurrence_id, "second-occurrence");
    assert_eq!(target.worker_generation, first.worker_generation);
    assert_eq!(target.call_id, "second-call");
    assert_eq!(*target_effect_id, fixture.delegated.effect_id);
    assert_eq!(backend_id, &first.backend_id);
    assert_eq!(
        backend_session_id.as_deref(),
        Some(first.backend_session_id.as_str())
    );
    assert_eq!(permission_request_id, &first.rpc_request_id);
    assert_eq!(
        resolution,
        &HostPermissionResolution::allow_once(),
        "a stored conversation grant authorizes this concrete occurrence once"
    );

    let store = NolaStore::open(&fixture.database).unwrap();
    let second = store
        .delegated_permission(&target.delivery_id)
        .unwrap()
        .unwrap();
    assert_eq!(second.revision, 2);
    assert_eq!(second.state, DelegatedPermissionDeliveryState::Prepared);
    assert_eq!(
        second.decision,
        Some(DelegatedPermissionDecision::AllowOnce)
    );
    assert_eq!(second.invocation_id, first.invocation_id);
    assert_eq!(second.target_effect_id, first.target_effect_id);
    assert_eq!(second.occurrence_id, "second-occurrence");
    assert_eq!(second.worker_generation, first.worker_generation);
    assert_eq!(second.rpc_request_id, first.rpc_request_id);
    assert_eq!(second.call_id, "second-call");
    assert_eq!(
        second.request_json,
        json!({"path":"/fixture/result.txt", "content":"第二次调用"})
    );
    assert_eq!(
        store
            .delegated_permission_for_occurrence(&second.invocation_id, "second-occurrence")
            .unwrap(),
        Some(second.clone())
    );
    assert_eq!(
        store
            .delegated_permission_for_control(&second_control.effect_id.0)
            .unwrap(),
        Some(second.clone())
    );
    assert_eq!(
        store.delegated_permission(&first.id).unwrap(),
        Some(written)
    );
    drop(store);
    assert_eq!(
        permission_events(&fixture.database, &second.id)
            .iter()
            .map(|record| (record.state, record.revision))
            .collect::<Vec<_>>(),
        vec![
            (DelegatedPermissionDeliveryState::AwaitingDecision, 1),
            (DelegatedPermissionDeliveryState::Prepared, 2),
        ]
    );
    assert_eq!(ownership_rows(&fixture.database), ownership);
    assert_owner_isolation(&fixture);

    let mut runtime = support::open_runtime(&fixture.database).unwrap();
    let second_control = claim_and_start(
        &mut runtime,
        &fixture.run,
        &fixture.digest,
        &second_control.effect_id,
        2_000,
    );
    let before = canonical_rows(&fixture.database);
    assert!(runtime
        .execute(command(
            &second_control,
            &fixture.run,
            &fixture.digest,
            second_control.resume_after_sequence.unwrap_or(0) + 1,
            HostResponse::DelegatedPermissionClaimed {
                delivery_id: first.id,
            },
        ))
        .is_err());
    assert_eq!(canonical_rows(&fixture.database), before);
    runtime
        .execute(command(
            &second_control,
            &fixture.run,
            &fixture.digest,
            second_control.resume_after_sequence.unwrap_or(0) + 1,
            HostResponse::DelegatedPermissionClaimed {
                delivery_id: second.id.clone(),
            },
        ))
        .unwrap();
    assert_eq!(
        NolaStore::open(&fixture.database)
            .unwrap()
            .delegated_permission(&second.id)
            .unwrap()
            .unwrap()
            .state,
        DelegatedPermissionDeliveryState::Claimed
    );
    runtime
        .execute(command(
            &second_control,
            &fixture.run,
            &fixture.digest,
            second_control.resume_after_sequence.unwrap_or(0) + 2,
            HostResponse::DelegatedPermissionTransportWritten {
                delivery_id: second.id.clone(),
            },
        ))
        .unwrap();
    assert_eq!(
        permission_events(&fixture.database, &second.id)
            .iter()
            .map(|record| (record.state, record.revision))
            .collect::<Vec<_>>(),
        vec![
            (DelegatedPermissionDeliveryState::AwaitingDecision, 1),
            (DelegatedPermissionDeliveryState::Prepared, 2),
            (DelegatedPermissionDeliveryState::Claimed, 3),
            (DelegatedPermissionDeliveryState::TransportWritten, 4),
        ]
    );
    assert_eq!(ownership_rows(&fixture.database), ownership);
    assert_owner_isolation(&fixture);
}
