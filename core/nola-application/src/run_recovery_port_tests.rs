// SPDX-License-Identifier: Apache-2.0
//! Real Core recovery facts exercised through an in-memory repository contract.
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use nola_agent_engine::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_agent_engine::agent::{AgentCore, AgentThreadState, PermissionUserDecision};
use nola_domain::{
    DelegatedInvocationRecord, DelegatedInvocationState, DelegatedPermissionDecision,
    DelegatedPermissionDeliveryRecord, DelegatedPermissionDeliveryState,
    EffectDeliveryAttemptState, EffectDeliveryRecoveryPlan, EffectDeliveryReplaySafety,
};
use nola_host_protocol::{
    DelegatedAgentEventV1, DelegatedPermissionDeliveryTarget, DelegatedPermissionOccurrence,
    EffectDeliveryLease,
};
use nola_store_port::run_effect::{StoredCheckpoint, StoredOutboxEffect};
use nola_store_port::store_maintenance::*;
use serde_json::json;
use sha2::{Digest, Sha256};

struct MemoryMaintenance {
    records: Vec<RunRecoveryRecord>,
    applied: Vec<RunRecoveryPlan>,
}
impl StoreMaintenanceRepository for MemoryMaintenance {
    type Error = String;
    fn with_run_recovery_plan<F>(
        &mut self,
        conversation: Option<&str>,
        reason: &str,
        at: &str,
        at_ms: i64,
        mut planner: F,
    ) -> Result<usize, String>
    where
        F: FnMut(&RunRecoveryRecord) -> Result<RunRecoveryPlan, String>,
    {
        assert_eq!(conversation, Some("recovery-port"));
        assert!(matches!(
            reason,
            "restart" | "observed_owner_process_terminated" | "recover_expired_delivery_lease"
        ));
        assert_eq!((at, at_ms), ("2000", 2000));
        // Stage every decision before applying any: a rejected record aborts the UoW.
        let staged = self
            .records
            .iter()
            .map(&mut planner)
            .collect::<Result<Vec<_>, _>>()?;
        self.applied = staged;
        Ok(0)
    }
}

pub(crate) fn recovery_host_binding(model: &str) -> nola_host_protocol::HostInvocationBinding {
    use nola_domain::generation::{resolve_output_limit, OutputTokenLimits, OutputTokenPreference};
    use nola_host_protocol::*;
    let route = HostModelRouteBinding::Http(nola_host_protocol::HostHttpModelRouteBinding {
        siwc_account: None,
        protocol: nola_domain::generation::ModelProtocol::ChatCompletions,
        output_token_field: nola_host_protocol::ModelOutputTokenField::MaxTokens,
        provider: Some("fixture".into()),
        endpoint: "https://fixture.invalid/v1".into(),
        endpoint_query_sha256: None,
        model: model.into(),
        credential_sha256: "a".repeat(64),
        credential_source: HostSecretSource::Ephemeral,
        endpoint_query_source: HostSecretSource::None,
    });
    let preference = OutputTokenPreference::Default;
    let resolved = resolve_output_limit(
        preference,
        OutputTokenLimits {
            known_output_cap: None,
        },
    )
    .unwrap();
    let binding = HostInvocationBinding {
        mac_launch: None,
        version: HOST_INVOCATION_BINDING_VERSION,
        primary: route.clone(),
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
            max_output_tokens: resolved.tokens,
            thinking_enabled: false, reasoning_summary_supported: None,
            prompt_cache: false,
            prompt_cache_key_sha256: None,
            prompt_cache_key_prefix_sha256: None,
            prompt_cache_retention: None,
        },
        output_admission: HostOutputAdmission {
            version: HOST_OUTPUT_ADMISSION_VERSION,
            configured_context_window: None,
            preference,
            resolved,
            routes: vec![HostRouteTokenLimits {
                route,
                max_input_tokens: None,
                max_output_tokens: None,
                max_total_tokens: None,
            }],
        },
        execution_policy_sha256: "b".repeat(64),
    };
    binding.validate().unwrap();
    binding
}

fn fixture(terminal: bool) -> RunRecoveryRecord {
    let mut config = AgentCoreConfig {
        turn_identity: crate::model_fixture::identity(),
        session_id: Some(SessionId::from_string("recovery-port".into())),
        model: Some("fixture".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: [(
                "model.stream".into(),
                HostCapabilityDescriptor {
                    id: "model.stream".into(),
                    title: "Model".into(),
                    route: HostCapabilityRoute::Model,
                    availability: CapabilityAvailability::Available,
                    exposure: HostCapabilityExposure::Internal,
                    permission_domain: None,
                    input_schema: json!({"type":"object"}),
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
                },
            )]
            .into_iter()
            .collect(),
        }),
        ..Default::default()
    };
    config.turn_identity.host_invocation = Some(recovery_host_binding("fixture"));
    let config_json = serde_json::to_string(&config).unwrap();
    let mut core = AgentCore::try_new(config).unwrap();
    let transition = core
        .submit_at(
            AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("input".into()),
                    "hello",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("initial-turn".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            1000,
        )
        .unwrap();
    let mut request = transition
        .host_requests
        .iter()
        .find(|r| matches!(r.payload, HostRequest::ModelStream { .. }))
        .unwrap()
        .clone();
    let HostRequest::ModelStream {
        text_part_id,
        reasoning_part_id,
        ..
    } = &request.payload
    else {
        unreachable!()
    };
    let attempt = nola_kernel::prepare_model_attempt(
        "store-run",
        &request.effect_id.0,
        &request.effect_id.0,
        "prepared-attempt",
        text_part_id,
        reasoning_part_id,
        1000,
    )
    .unwrap();
    request.attempt_id = Some(attempt.id.clone());
    request.attempt_started = false;
    if terminal {
        core.submit_at(
            AgentSubmission::HostResponse {
                response: HostResponseEnvelope {
                    request_id: request.request_id.clone(),
                    attempt_id: request.attempt_id.clone(),
                    effect_id: request.effect_id.clone(),
                    delegated_invocation_id: request.delegated_invocation_id.clone(),
                    host_id: request.host_id.clone(),
                    lease_id: request.lease_id.clone(),
                    session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision,
                    availability_epoch: request.availability_epoch,
                    delivery: request.delivery.clone(),
                    sequence: 1,
                    payload: HostResponse::ModelAttemptStarted {
                        provider: "fixture-provider".into(),
                        model: "fixture".into(),
                        reason: "initial".into(),
                    },
                },
            },
            1050,
        )
        .unwrap();
        core.submit_at(
            AgentSubmission::HostResponse {
                response: HostResponseEnvelope {
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
                    sequence: 2,
                    payload: HostResponse::ModelCompleted {
                        items: vec![serde_json::to_value(AgentItem::assistant_message(
                            ItemId::from_string("answer".into()),
                            "Hello.",
                        ))
                        .unwrap()],
                        tool_calls: vec![],
                        usage: json!({"input_tokens":10,"output_tokens":5}),
                    },
                },
            },
            1100,
        )
        .unwrap();
    }
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let current_turn = checkpoint.state.execution.active_turn.as_ref().unwrap();
    if terminal {
        assert!(current_turn.status.is_terminal());
    }
    let active_attempt_id = (!terminal).then(|| attempt.id.clone());
    RunRecoveryRecord {
        open_user_input_waits: Vec::new(),
        effect_delivery_attempts: Vec::new(),
        delegated_invocations: Vec::new(),
        delegated_permissions: Vec::new(),
        model_attempts: if terminal { vec![] } else { vec![attempt] },
        conversation_id: "recovery-port".into(),
        has_session_lease: false,
        session_lease: None,
        run: Some(RecoveryRunRecord {
            active_attempt_id,
            run_id: "store-run".into(),
            status: "running".into(),
            invocation_config_digest: Some(format!("{:x}", Sha256::digest(config_json.as_bytes()))),
            invocation_config_json: Some(config_json),
            latest_started_turn_id: Some(current_turn.turn_id.as_str().into()),
        }),
        checkpoint: Some(
            StoredCheckpoint {
                conversation_id: "recovery-port".into(),
                revision: checkpoint.revision as i64,
                event_cursor: checkpoint.last_event_cursor.sequence as i64,
                checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
                updated_at_ms: 1100,
            }
            .into(),
        ),
        effects: if terminal {
            vec![]
        } else {
            vec![StoredOutboxEffect {
                effect_id: request.effect_id.0.clone(),
                run_id: "store-run".into(),
                request_id: request.request_id.clone(),
                effect_kind: "model_stream".into(),
                payload_json: serde_json::to_string(&request).unwrap(),
                status: "pending".into(),
                created_at_ms: 1000,
                acknowledged_at_ms: None,
            }]
        },
        traces: vec![],
    }
}
fn recover(store: &mut MemoryMaintenance) -> Result<crate::run_recovery::RecoveryOutcome, String> {
    crate::run_recovery::recover(store, Some("recovery-port"), "restart", "2000", 2000)
}

fn inflight_cancellation_fixture() -> RunRecoveryRecord {
    let mut record = fixture(false);
    let original: HostRequestEnvelope =
        serde_json::from_str(&record.effects[0].payload_json).unwrap();
    let original_effect_id = original.effect_id.0.clone();
    let cancel_effect_id = "initial-turn:host_effect:cancel".to_string();
    let cancel_request_id = "initial-turn:host_request:cancel".to_string();
    let mut cancel = original.clone();
    cancel.request_id = cancel_request_id.clone();
    cancel.effect_id = HostEffectId(cancel_effect_id.clone());
    cancel.attempt_id = None;
    cancel.attempt_started = false;
    cancel.delivery = None;
    cancel.resume_after_sequence = None;
    cancel.payload = HostRequest::Cancel {
        target_effect_id: HostEffectId(original_effect_id.clone()),
        target_not_started: None,
    };

    let mut checkpoint: serde_json::Value = serde_json::from_str(
        &record
            .checkpoint
            .as_ref()
            .unwrap()
            .checkpoint
            .checkpoint_json,
    )
    .unwrap();
    checkpoint["state"]["execution"]["active_turn"]["status"] = json!("cancelling");
    let effects = checkpoint["state"]["host"]["pending_host_effects"]["effects"]
        .as_object_mut()
        .unwrap();
    let original_entry = effects.get_mut(&original_effect_id).unwrap();
    original_entry["state"] = json!("cancelling");
    original_entry["cancel_effect_id"] = json!(cancel_effect_id.clone());
    let mut cancel_entry = original_entry.clone();
    cancel_entry["effect_id"] = json!(cancel_effect_id.clone());
    cancel_entry["request_id"] = json!(cancel_request_id.clone());
    cancel_entry["capability_id"] = json!("host.cancel");
    cancel_entry["replay_safety"] = json!("idempotent");
    cancel_entry["state"] = json!("dispatched");
    cancel_entry["target_effect_id"] = json!(original_effect_id.clone());
    cancel_entry["cancel_effect_id"] = serde_json::Value::Null;
    cancel_entry["last_response_sequence"] = serde_json::Value::Null;
    cancel_entry["request"] = serde_json::to_value(&cancel).unwrap();
    effects.insert(cancel_effect_id.clone(), cancel_entry);
    record
        .checkpoint
        .as_mut()
        .unwrap()
        .checkpoint
        .checkpoint_json = checkpoint.to_string();
    record.run.as_mut().unwrap().active_attempt_id = None;
    record.model_attempts.clear();
    record.effects = vec![StoredOutboxEffect {
        effect_id: cancel_effect_id,
        run_id: "store-run".into(),
        request_id: cancel_request_id,
        effect_kind: "cancel".into(),
        payload_json: serde_json::to_string(&cancel).unwrap(),
        status: "pending".into(),
        created_at_ms: 1200,
        acknowledged_at_ms: None,
    }];
    record
}

fn delivery_fixture(
    state: EffectDeliveryAttemptState,
    safety: EffectDeliveryReplaySafety,
    cursor: Option<u64>,
    lease_expires_at_ms: i64,
) -> RunRecoveryRecord {
    let mut record = fixture(false);
    let effect = &mut record.effects[0];
    let effect_id = effect.effect_id.clone();
    let request_id = effect.request_id.clone();
    let wire_safety = match safety {
        EffectDeliveryReplaySafety::Idempotent => HostReplaySafety::Idempotent,
        EffectDeliveryReplaySafety::Recoverable => HostReplaySafety::Recoverable,
        EffectDeliveryReplaySafety::RequiresRecoveryDecision => {
            HostReplaySafety::RequiresRecoveryDecision
        }
    };
    let mut checkpoint: serde_json::Value = serde_json::from_str(
        &record
            .checkpoint
            .as_ref()
            .unwrap()
            .checkpoint
            .checkpoint_json,
    )
    .unwrap();
    let pending = checkpoint
        .pointer_mut(&format!(
            "/state/host/pending_host_effects/effects/{effect_id}"
        ))
        .and_then(serde_json::Value::as_object_mut)
        .expect("fixture checkpoint pending effect");
    pending.insert(
        "replay_safety".into(),
        serde_json::to_value(wire_safety).unwrap(),
    );
    pending.insert(
        "last_response_sequence".into(),
        cursor.map_or(serde_json::Value::Null, serde_json::Value::from),
    );
    if cursor.is_some() {
        pending.insert("state".into(), serde_json::Value::String("accepted".into()));
    }
    record
        .checkpoint
        .as_mut()
        .unwrap()
        .checkpoint
        .checkpoint_json = checkpoint.to_string();

    let prepared = nola_kernel::prepare_effect_delivery_attempt(
        "delivery-attempt",
        "store-run",
        &effect_id,
        &request_id,
        1,
        safety,
        None,
        1000,
    )
    .unwrap()
    .record;
    let claimed = nola_kernel::claim_effect_delivery_attempt(
        &prepared,
        "worker-generation",
        "runtime-owner",
        "lease-token",
        lease_expires_at_ms,
        1100,
    )
    .unwrap()
    .record;
    let started = || {
        nola_kernel::start_effect_delivery_execution(
            &claimed,
            "worker-generation",
            "runtime-owner",
            "lease-token",
            1200,
        )
        .unwrap()
        .record
    };
    let attempt = match state {
        EffectDeliveryAttemptState::Prepared => prepared,
        EffectDeliveryAttemptState::Leased => claimed.clone(),
        EffectDeliveryAttemptState::ExecutionStarted => started(),
        EffectDeliveryAttemptState::Interrupted => {
            nola_kernel::interrupt_effect_delivery_attempt(
                &claimed,
                "worker-generation",
                "runtime-owner",
                "lease-token",
                EffectDeliveryRecoveryPlan::Retry,
                "already interrupted",
                1600,
            )
            .unwrap()
            .record
        }
        EffectDeliveryAttemptState::OutcomeUnknown => {
            nola_kernel::mark_effect_delivery_outcome_unknown(
                &started(),
                "worker-generation",
                "runtime-owner",
                "lease-token",
                "already unknown",
                1600,
            )
            .unwrap()
            .record
        }
        EffectDeliveryAttemptState::Settled => {
            nola_kernel::settle_effect_delivery_attempt(
                &started(),
                "worker-generation",
                "runtime-owner",
                "lease-token",
                nola_domain::EffectDeliverySettlement::Succeeded,
                1400,
            )
            .unwrap()
            .record
        }
    };
    record.effect_delivery_attempts = vec![attempt];
    record
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

fn permission_fixture(state: DelegatedPermissionDeliveryState) -> RunRecoveryRecord {
    let mut config = AgentCoreConfig {
        turn_identity: crate::model_fixture::identity(),
        session_id: Some(SessionId::from_string("recovery-port".into())),
        model: Some("fixture".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: [(
                "agent.delegate".into(),
                HostCapabilityDescriptor {
                    id: "agent.delegate".into(),
                    title: "Delegate".into(),
                    route: HostCapabilityRoute::Model,
                    availability: CapabilityAvailability::Available,
                    exposure: HostCapabilityExposure::Internal,
                    permission_domain: None,
                    input_schema: json!({"type":"object"}),
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
                },
            )]
            .into_iter()
            .collect(),
        }),
        ..Default::default()
    };
    config.turn_identity.execution_backend =
        nola_domain::execution_backend::ExecutionBackendSelection {
            backend_id: "codex".into(),
            ..Default::default()
        };
    // This fixture has no repository and dispatches no physical file work.
    // External execution still needs the current admitted parent snapshot.
    config.turn_identity.delegation = Some(nola_host_protocol::delegation::ValidatedDelegationParent::new(
        nola_host_protocol::delegation::DelegationParentSnapshot {
            process_sandbox: None, bounded_file_tools: Default::default(),
            project_repository_identity: None, task_worktree: None, task_sandbox_requirement: None,
            authorized_workspace_roots: vec!["/fixture".into()], requested_native_sandbox: None,
            model: nola_host_protocol::delegation::DelegatedModel {
                endpoint: String::new(), model: "fixture".into(), swe_mode: false,
            },
            runtime: Default::default(), credential: None, fallback_credentials: vec![],
            profiles: nola_domain::delegation::effective_agent_profiles(&Default::default()),
            project_root: "/fixture".into(), read_only_tools: Default::default(), profile_credentials: Default::default(),
            execution_backend: "codex".into(), external_execution_model: None,
            external_execution_model_provider: None, max_iterations: 4,
        }
    ).unwrap());
    config.turn_identity.prompt.stable_prefix_digest = "a".repeat(64);
    let config_json = serde_json::to_string(&config).unwrap();
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit_at(
            AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("input".into()),
                    "hello",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("initial-turn".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            1000,
        )
        .unwrap();
    assert!(started.host_requests.iter().any(|request| matches!(request.payload, HostRequest::DelegatedAgent { .. })), "{started:?}");
    let mut delegated = started
        .host_requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::DelegatedAgent { .. }))
        .unwrap();
    let invocation_id = "delegated-invocation";
    delegated.delegated_invocation_id = Some(invocation_id.into());
    core.bind_committed_host_effect(&delegated).unwrap();
    core.submit_at(
        AgentSubmission::HostResponse {
            response: response(&delegated, 1, HostResponse::Accepted),
        },
        1010,
    )
    .unwrap();
    core.submit_at(
        AgentSubmission::HostResponse {
            response: response(
                &delegated,
                2,
                HostResponse::DelegatedAgentEvent {
                    event: DelegatedAgentEventV1::SessionStarted {
                        backend_id: "codex".into(),
                        backend_session_id: "backend-session".into(),
                    },
                },
            ),
        },
        1020,
    )
    .unwrap();
    let permission_input = json!({"path":"/workspace/result.txt","content":"fixture"});
    let asked = core
        .submit_at(
            AgentSubmission::HostResponse {
                response: response(
                    &delegated,
                    3,
                    HostResponse::DelegatedAgentEvent {
                        event: DelegatedAgentEventV1::PermissionRequested {
                            occurrence: DelegatedPermissionOccurrence {
                                occurrence_id: "permission-occurrence".into(),
                                worker_generation: "worker-generation".into(),
                            },
                            request_id: "backend-rpc".into(),
                            call_id: "backend-call".into(),
                            capability_id: "filesystem.write".into(),
                            resource: "/workspace/result.txt".into(),
                            reason: "write fixture".into(),
                            input: permission_input.clone(),
                        },
                    },
                ),
            },
            1030,
        )
        .unwrap();
    let ask = asked
        .host_requests
        .iter()
        .find(|request| matches!(request.payload, HostRequest::RequestPermission { .. }))
        .unwrap()
        .clone();

    let (text_part_id, reasoning_part_id) = match &delegated.payload {
        HostRequest::DelegatedAgent { request } => (
            request.text_part_id.clone(),
            request.reasoning_part_id.clone(),
        ),
        _ => unreachable!(),
    };
    let prepared_invocation = nola_kernel::prepare_delegated_invocation(
        "store-run",
        &delegated.effect_id.0,
        invocation_id,
        "codex",
        None,
        &text_part_id,
        &reasoning_part_id,
        1000,
    )
    .unwrap();
    let running = nola_kernel::transition_delegated_invocation(
        &prepared_invocation,
        DelegatedInvocationState::Running,
        None,
        None,
        None,
        1010,
    )
    .unwrap()
    .record;
    let running = nola_kernel::transition_delegated_invocation(
        &running,
        DelegatedInvocationState::Running,
        Some("backend-session"),
        None,
        None,
        1020,
    )
    .unwrap()
    .record;
    let invocation: DelegatedInvocationRecord = nola_kernel::transition_delegated_invocation(
        &running,
        DelegatedInvocationState::WaitingPermission,
        None,
        None,
        None,
        1030,
    )
    .unwrap()
    .record;

    let mut permission = DelegatedPermissionDeliveryRecord {
        id: "permission-delivery".into(),
        run_id: "store-run".into(),
        invocation_id: invocation_id.into(),
        target_effect_id: delegated.effect_id.0.clone(),
        worker_generation: "worker-generation".into(),
        occurrence_id: "permission-occurrence".into(),
        backend_id: "codex".into(),
        backend_session_id: "backend-session".into(),
        rpc_request_id: "backend-rpc".into(),
        call_id: "backend-call".into(),
        request_digest: format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&permission_input).unwrap())
        ),
        request_json: permission_input,
        control_effect_id: None,
        control_request_id: None,
        decision: None,
        state: DelegatedPermissionDeliveryState::AwaitingDecision,
        created_at_ms: 1030,
        claimed_at_ms: None,
        written_at_ms: None,
        revision: 1,
    };
    let mut effects = vec![delegated.clone(), ask.clone()];
    if state != DelegatedPermissionDeliveryState::AwaitingDecision {
        let pending_id = match &ask.payload {
            HostRequest::RequestPermission {
                permission_request_id,
                ..
            } => permission_request_id.clone(),
            _ => unreachable!(),
        };
        let resolved = core
            .submit_at(
                AgentSubmission::PermissionResponse {
                    request_id: PermissionRequestId::from_string(pending_id),
                    decision: PermissionUserDecision::AllowOnce,
                    grant: None,
                },
                1040,
            )
            .unwrap();
        let mut control = resolved
            .host_requests
            .into_iter()
            .find(|request| {
                matches!(
                    request.payload,
                    HostRequest::ResolveDelegatedPermission { .. }
                )
            })
            .unwrap();
        permission = nola_kernel::authorize_delegated_permission(
            &permission,
            control.effect_id.0.clone(),
            control.request_id.clone(),
            DelegatedPermissionDecision::AllowOnce,
            1040,
        )
        .unwrap()
        .record;
        let delivery = DelegatedPermissionDeliveryTarget {
            delivery_id: permission.id.clone(),
            invocation_id: permission.invocation_id.clone(),
            worker_generation: permission.worker_generation.clone(),
            occurrence_id: permission.occurrence_id.clone(),
            call_id: permission.call_id.clone(),
            request_digest: permission.request_digest.clone(),
        };
        let HostRequest::ResolveDelegatedPermission {
            delivery: request_delivery,
            ..
        } = &mut control.payload
        else {
            unreachable!()
        };
        *request_delivery = Some(delivery);
        // Resolving the ask completes that effect and replaces it with the
        // canonical worker-control delivery in Core's pending journal.
        effects.pop();
        effects.push(control);
        permission = match state {
            DelegatedPermissionDeliveryState::Prepared => permission,
            DelegatedPermissionDeliveryState::Claimed => {
                nola_kernel::claim_delegated_permission(&permission, 1050)
                    .unwrap()
                    .record
            }
            DelegatedPermissionDeliveryState::TransportWritten => {
                let claimed = nola_kernel::claim_delegated_permission(&permission, 1050)
                    .unwrap()
                    .record;
                nola_kernel::write_delegated_permission(&claimed, 1060)
                    .unwrap()
                    .record
            }
            DelegatedPermissionDeliveryState::OutcomeUnknown => {
                let claimed = nola_kernel::claim_delegated_permission(&permission, 1050)
                    .unwrap()
                    .record;
                nola_kernel::interrupt_delegated_permission(&claimed, 1060)
                    .unwrap()
                    .record
            }
            DelegatedPermissionDeliveryState::Interrupted => {
                nola_kernel::interrupt_delegated_permission(&permission, 1050)
                    .unwrap()
                    .record
            }
            DelegatedPermissionDeliveryState::AwaitingDecision => unreachable!(),
        };
    }
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let current_turn = checkpoint.state.execution.active_turn.as_ref().unwrap();
    RunRecoveryRecord {
        open_user_input_waits: Vec::new(),
        effect_delivery_attempts: Vec::new(),
        conversation_id: "recovery-port".into(),
        has_session_lease: false,
        session_lease: None,
        run: Some(RecoveryRunRecord {
            active_attempt_id: None,
            run_id: "store-run".into(),
            status: "running".into(),
            invocation_config_digest: Some(format!("{:x}", Sha256::digest(config_json.as_bytes()))),
            invocation_config_json: Some(config_json),
            latest_started_turn_id: Some(current_turn.turn_id.as_str().into()),
        }),
        checkpoint: Some(
            StoredCheckpoint {
                conversation_id: "recovery-port".into(),
                revision: checkpoint.revision as i64,
                event_cursor: checkpoint.last_event_cursor.sequence as i64,
                checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
                updated_at_ms: 1060,
            }
            .into(),
        ),
        effects: effects
            .into_iter()
            .map(|request| StoredOutboxEffect {
                effect_id: request.effect_id.0.clone(),
                run_id: "store-run".into(),
                request_id: request.request_id.clone(),
                effect_kind: match request.payload {
                    HostRequest::DelegatedAgent { .. } => "delegated_agent",
                    HostRequest::RequestPermission { .. } => "request_permission",
                    HostRequest::ResolveDelegatedPermission { .. } => {
                        "resolve_delegated_permission"
                    }
                    _ => unreachable!(),
                }
                .into(),
                payload_json: serde_json::to_string(&request).unwrap(),
                status: "pending".into(),
                created_at_ms: 1000,
                acknowledged_at_ms: None,
            })
            .collect(),
        model_attempts: vec![],
        delegated_invocations: vec![invocation],
        delegated_permissions: vec![permission],
        traces: vec![],
    }
}

#[test]
fn real_model_checkpoint_preserves_exact_current_turn_without_sqlite() {
    let mut store = MemoryMaintenance {
        records: vec![fixture(false)],
        applied: vec![],
    };
    let outcome = recover(&mut store).unwrap();
    assert_eq!(outcome.changed, 0);
    assert_eq!(
        store.applied[0].preserve_trace_turn_id.as_deref(),
        Some("initial-turn")
    );
    assert!(!store.applied[0].preserve_all_traces);
}

#[test]
fn inflight_cancellation_is_a_valid_recovery_state() {
    let mut store = MemoryMaintenance {
        records: vec![inflight_cancellation_fixture()],
        applied: vec![],
    };
    recover(&mut store).expect("nonterminal host.cancel handshake must remain recoverable");
    assert_eq!(store.applied.len(), 1);
}

#[test]
fn cancelling_physical_targets_are_retained_without_retry_and_require_exact_control_binding() {
    for started in [false, true] {
        let mut record = inflight_cancellation_fixture();
        let checkpoint: CoreCheckpoint<AgentThreadState> = serde_json::from_str(
            &record
                .checkpoint
                .as_ref()
                .unwrap()
                .checkpoint
                .checkpoint_json,
        )
        .unwrap();
        let target = checkpoint
            .state
            .host
            .pending_effects
            .iter()
            .find(|effect| {
                effect.state
                    == nola_agent_engine::agent::recovery::PendingHostEffectState::Cancelling
            })
            .unwrap();
        let prepared = nola_kernel::prepare_effect_delivery_attempt(
            "cancel-target-attempt",
            "store-run",
            &target.effect_id,
            &target.request_id,
            1,
            EffectDeliveryReplaySafety::Recoverable,
            None,
            1000,
        )
        .unwrap();
        let mut attempt = nola_kernel::claim_effect_delivery_attempt(
            &prepared.record,
            "generation",
            "owner",
            "token",
            1500,
            1100,
        )
        .unwrap()
        .record;
        if started {
            attempt = nola_kernel::start_effect_delivery_execution(
                &attempt,
                "generation",
                "owner",
                "token",
                1120,
            )
            .unwrap()
            .record;
        }
        record.effect_delivery_attempts.push(attempt);
        let mut store = MemoryMaintenance {
            records: vec![record.clone()],
            applied: vec![],
        };
        recover(&mut store).unwrap();
        assert_eq!(store.applied.len(), 1);
        assert!(
            store.applied[0]
                .effect_delivery_attempt_mutations
                .is_empty(),
            "expiry cannot retry the original cancelling business target"
        );
        for wrong_request in [false, true] {
            let mut corrupt = record.clone();
            if wrong_request {
                corrupt.effect_delivery_attempts[0].request_id = "foreign-request".into();
            } else {
                let mut payload: serde_json::Value =
                    serde_json::from_str(&corrupt.effects[0].payload_json).unwrap();
                payload["payload"]["target_effect_id"] = json!("foreign-effect");
                corrupt.effects[0].payload_json = payload.to_string();
            }
            let mut store = MemoryMaintenance {
                records: vec![corrupt],
                applied: vec![],
            };
            assert!(
                recover(&mut store).is_err(),
                "an unrelated control is not an exception for an orphaned physical attempt"
            );
            assert!(store.applied.is_empty());
        }
    }
}

#[test]
fn startup_recovery_replays_tail_before_checking_current_effects() {
    let mut record = fixture(false);
    let request: HostRequestEnvelope =
        serde_json::from_str(&record.effects[0].payload_json).unwrap();
    let HostRequest::ModelStream { text_part_id, .. } = &request.payload else {
        panic!("model fixture");
    };
    let response = HostResponseEnvelope {
        request_id: request.request_id.clone(),
        attempt_id: request.attempt_id.clone(),
        effect_id: request.effect_id.clone(),
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(),
        sequence: request.resume_after_sequence.unwrap_or(0) + 1,
        payload: HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: text_part_id.clone(),
            delta: "durable tail".into(),
        },
    };
    let recovery = record.checkpoint.as_mut().unwrap();
    let config = serde_json::from_str(
        record
            .run
            .as_ref()
            .unwrap()
            .invocation_config_json
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let baseline = serde_json::from_str(&recovery.checkpoint.checkpoint_json).unwrap();
    let mut core = AgentCore::restore(config, baseline).unwrap();
    core.bind_committed_host_effect(&request).unwrap();
    assert_eq!(
        core.committed_host_request_resume_snapshot(&request.effect_id.0)
            .unwrap()
            .unwrap()
            .attempt_id,
        request.attempt_id
    );
    core.submit_at(
        AgentSubmission::HostResponse {
            response: response.clone(),
        },
        1200,
    )
    .unwrap();
    assert!(
        serde_json::to_string(&core.checkpoint(CheckpointReason::Manual))
            .unwrap()
            .contains("durable tail")
    );
    recovery.replay_base = Some(nola_store_port::run_effect::StoredCheckpointReplayBase {
        run_id: "store-run".into(),
        checkpoint_revision: recovery.checkpoint.revision,
        through_ordinal: 0,
    });
    recovery
        .replay_inputs
        .push(nola_store_port::run_effect::StoredCoreReplayInput {
            kind: nola_store_port::run_effect::ReplayInputKind::HostResponse,
            ordinal: 1,
            reduction_json: serde_json::to_string(
                &nola_store_port::host_replay::HostReplayReduction::Replayable {
                    protocol_version: 1,
                    decision_time_ms: 1200,
                    binding: Some(
                        nola_store_port::host_replay::CoreHostBindingOverlay::from_request(
                            &request,
                        ),
                    ),
                    normalized_response: serde_json::value::RawValue::from_string(
                        serde_json::to_string(&response).unwrap(),
                    )
                    .unwrap(),
                },
            )
            .unwrap(),
            received_at_ms: 1200,
        });
    recovery.checkpoint.event_cursor = core
        .checkpoint(CheckpointReason::Manual)
        .last_event_cursor
        .sequence as i64;
    // Store transaction revision is intentionally independent of Core revision.
    recovery.checkpoint.revision += 10;
    let mut wrong_owner = record.clone();
    wrong_owner
        .checkpoint
        .as_mut()
        .unwrap()
        .replay_base
        .as_mut()
        .unwrap()
        .run_id = "other-run".into();
    let mut invalid = MemoryMaintenance {
        records: vec![wrong_owner],
        applied: vec![],
    };
    assert!(recover(&mut invalid).is_err());
    assert!(invalid.applied.is_empty());
    let mut store = MemoryMaintenance {
        records: vec![record],
        applied: vec![],
    };
    recover(&mut store)
        .expect("startup must inspect replayed state, not reject the old baseline cursor");
    assert_eq!(store.applied.len(), 1);
}
#[test]
fn terminal_checkpoint_cannot_replace_seal_and_live_lease_skips_malformed_facts() {
    let mut missing_seal = MemoryMaintenance {
        records: vec![fixture(true)],
        applied: vec![],
    };
    assert!(
        matches!(recover(&mut missing_seal), Err(error) if error.contains("MissingTerminalCommit"))
    );
    assert!(missing_seal.applied.is_empty());
    let mut malformed = fixture(false);
    malformed.has_session_lease = true;
    malformed
        .checkpoint
        .as_mut()
        .unwrap()
        .checkpoint
        .checkpoint_json = "invalid".into();
    malformed.run.as_mut().unwrap().invocation_config_json = Some("invalid".into());
    let mut store = MemoryMaintenance {
        records: vec![malformed],
        applied: vec![],
    };
    let outcome = recover(&mut store).unwrap();
    assert_eq!(outcome.changed, 0);
    assert!(store.applied[0].preserve_all_traces);
}
#[test]
fn config_and_effect_conflicts_abort_all_staged_recovery() {
    for corruption in 0..4 {
        let mut invalid = fixture(false);
        match corruption {
            3 => invalid.run.as_mut().unwrap().latest_started_turn_id = Some(String::new()),
            0 => invalid.run.as_mut().unwrap().invocation_config_digest = Some("wrong".into()),
            1 => invalid.effects[0].request_id = "another-request".into(),
            _ => invalid.effects[0].run_id = "another-run".into(),
        }
        let mut store = MemoryMaintenance {
            records: vec![fixture(false), invalid],
            applied: vec![],
        };
        assert!(recover(&mut store).err().unwrap().contains("rejected"));
        assert!(
            store.applied.is_empty(),
            "no earlier valid preserve plan may escape a failed UoW"
        );
    }
}

#[test]
fn orphaned_active_attempt_blocks_terminal_recovery_and_missing_active_owner_is_rejected() {
    let mut orphan = fixture(true);
    orphan.run.as_mut().unwrap().active_attempt_id = Some("unsettled-sampling".into());
    let mut unowned = fixture(false);
    unowned.run.as_mut().unwrap().active_attempt_id = None;
    for invalid in [orphan, unowned] {
        let mut store = MemoryMaintenance {
            records: vec![fixture(false), invalid],
            applied: vec![],
        };
        assert!(recover(&mut store).err().unwrap().contains("Attempt"));
        assert!(
            store.applied.is_empty(),
            "Attempt inconsistency must roll back all recovery plans"
        );
    }
}

#[test]
fn orphaned_permission_delivery_uses_transport_evidence_without_retrying_unknown_writes() {
    use DelegatedPermissionDeliveryState::*;
    let states = [
        AwaitingDecision,
        Prepared,
        Claimed,
        TransportWritten,
        OutcomeUnknown,
        Interrupted,
    ];
    let mut store = MemoryMaintenance {
        records: states.into_iter().map(permission_fixture).collect(),
        applied: vec![],
    };
    let outcome = recover(&mut store).unwrap();
    assert_eq!(outcome.changed, 0);
    assert_eq!(store.applied.len(), states.len());

    let planned = store
        .applied
        .iter()
        .map(|plan| {
            plan.delegated_permission_mutations.first().map(|mutation| {
                assert_eq!(
                    mutation.expected_revision,
                    Some(mutation.record.revision - 1)
                );
                mutation.record.state
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        planned,
        vec![
            Some(Interrupted),
            Some(Interrupted),
            Some(OutcomeUnknown),
            None,
            None,
            None,
        ]
    );
}

#[test]
fn permission_recovery_rejects_corrupt_owner_session_and_payload_before_any_plan_applies() {
    for corruption in 0..4 {
        let mut invalid = permission_fixture(DelegatedPermissionDeliveryState::AwaitingDecision);
        match corruption {
            0 => invalid.delegated_invocations.clear(),
            1 => invalid.delegated_permissions[0].backend_session_id = "other-session".into(),
            2 => invalid.delegated_permissions[0].request_digest = "wrong-digest".into(),
            _ => invalid.delegated_permissions[0].target_effect_id = "other-effect".into(),
        }
        let mut store = MemoryMaintenance {
            records: vec![fixture(false), invalid],
            applied: vec![],
        };
        let error = match recover(&mut store) {
            Err(error) => error,
            Ok(_) => panic!("corrupt permission recovery facts were accepted"),
        };
        assert!(
            error.contains("permission")
                || error.contains("delegated invocation")
                || error.contains("delegated recovery")
                || error.contains("delegated restore session lacks canonical invocation evidence"),
            "unexpected recovery error: {error}"
        );
        assert!(
            store.applied.is_empty(),
            "invalid permission facts must abort the complete recovery UoW"
        );
    }
}

#[test]
fn live_lease_skips_permission_validation_and_plans_no_delivery_mutation() {
    let mut live = permission_fixture(DelegatedPermissionDeliveryState::Claimed);
    live.has_session_lease = true;
    live.delegated_invocations.clear();
    live.delegated_permissions[0].request_digest = "malformed-live-owner-facts".into();
    live.checkpoint.as_mut().unwrap().checkpoint.checkpoint_json =
        "malformed-live-checkpoint".into();
    let mut store = MemoryMaintenance {
        records: vec![live],
        applied: vec![],
    };
    let outcome = recover(&mut store).unwrap();
    assert_eq!(outcome.changed, 0);
    assert_eq!(store.applied.len(), 1);
    assert!(store.applied[0].delegated_permission_mutations.is_empty());
    assert!(store.applied[0].preserve_all_traces);
}

#[test]
fn permission_recovery_rejects_pending_scope_eligibility_drift_atomically() {
    let mut invalid = permission_fixture(DelegatedPermissionDeliveryState::AwaitingDecision);
    let durable: HostRequestEnvelope = serde_json::from_str(&invalid.effects.iter()
        .find(|effect| effect.effect_kind == "request_permission").unwrap().payload_json).unwrap();
    assert!(matches!(&durable.payload,HostRequest::RequestPermission {allow_conversation:true,..}));
    let recovery = invalid.checkpoint.as_mut().unwrap();
    let mut checkpoint: CoreCheckpoint<AgentThreadState> = serde_json::from_str(&recovery.checkpoint.checkpoint_json).unwrap();
    assert_eq!(checkpoint.state.execution.permissions.pending.len(),1);
    let pending = &mut checkpoint.state.execution.permissions.pending[0];
    assert!(pending.allow_conversation);
    // Keep both copies of the original Host request byte-identical. Only the
    // canonical pending offer is corrupted, so request-copy equality alone
    // cannot detect this drift on a cold recovery.
    pending.allow_conversation = false;
    recovery.checkpoint.checkpoint_json = serde_json::to_string(&checkpoint).unwrap();
    let before_checkpoint = recovery.checkpoint.checkpoint_json.clone();
    let before_effects = invalid.effects.clone();
    let before_permissions = invalid.delegated_permissions.clone();
    let mut store = MemoryMaintenance {records:vec![fixture(false),invalid],applied:vec![]};
    let error = recover(&mut store).err().expect("scope drift must reject recovery");
    assert!(error.contains("permission checkpoint/request mismatch"),"unexpected recovery error: {error}");
    assert!(store.applied.is_empty(),"invalid pending eligibility must abort every staged recovery plan");
    assert_eq!(store.records[1].checkpoint.as_ref().unwrap().checkpoint.checkpoint_json,before_checkpoint);
    assert_eq!(store.records[1].effects,before_effects);
    assert_eq!(store.records[1].delegated_permissions,before_permissions);
}

#[test]
fn effect_delivery_recovery_plans_only_evidence_safe_transitions() {
    let leased = delivery_fixture(
        EffectDeliveryAttemptState::Leased,
        EffectDeliveryReplaySafety::Idempotent,
        None,
        1500,
    );
    let idempotent = delivery_fixture(
        EffectDeliveryAttemptState::ExecutionStarted,
        EffectDeliveryReplaySafety::Idempotent,
        None,
        1500,
    );
    let recoverable = delivery_fixture(
        EffectDeliveryAttemptState::ExecutionStarted,
        EffectDeliveryReplaySafety::Recoverable,
        Some(7),
        1500,
    );
    let recoverable_before_first_response = delivery_fixture(
        EffectDeliveryAttemptState::ExecutionStarted,
        EffectDeliveryReplaySafety::Recoverable,
        Some(0),
        1500,
    );
    let unsafe_effect = delivery_fixture(
        EffectDeliveryAttemptState::ExecutionStarted,
        EffectDeliveryReplaySafety::RequiresRecoveryDecision,
        None,
        1500,
    );
    let mut store = MemoryMaintenance {
        records: vec![
            leased,
            idempotent,
            recoverable,
            recoverable_before_first_response,
            unsafe_effect,
        ],
        applied: Vec::new(),
    };
    recover(&mut store).unwrap();
    let planned = store
        .applied
        .iter()
        .map(|plan| {
            let mutation = plan
                .effect_delivery_attempt_mutations
                .first()
                .expect("one recovery mutation");
            (
                mutation.record.state,
                mutation.record.recovery_plan,
                mutation.record.revision,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        planned,
        vec![
            (
                EffectDeliveryAttemptState::Interrupted,
                Some(EffectDeliveryRecoveryPlan::Retry),
                3,
            ),
            (
                EffectDeliveryAttemptState::Interrupted,
                Some(EffectDeliveryRecoveryPlan::Retry),
                4,
            ),
            (
                EffectDeliveryAttemptState::Interrupted,
                Some(EffectDeliveryRecoveryPlan::Resume),
                4,
            ),
            (
                EffectDeliveryAttemptState::Interrupted,
                Some(EffectDeliveryRecoveryPlan::Resume),
                4,
            ),
            (EffectDeliveryAttemptState::OutcomeUnknown, None, 4),
        ]
    );
}

#[test]
fn observed_orphan_recovery_is_fenced_against_replaced_or_live_owners() {
    for changed in 0..6 {
        let state = if changed == 5 {
            EffectDeliveryAttemptState::Leased
        } else {
            EffectDeliveryAttemptState::ExecutionStarted
        };
        let mut record = delivery_fixture(
            state,
            EffectDeliveryReplaySafety::Recoverable,
            Some(7),
            10_000,
        );
        let attempt = &record.effect_delivery_attempts[0];
        let observed = EffectDeliveryLease {
            attempt_id: attempt.id.clone(),
            worker_generation: attempt.worker_generation.clone().unwrap(),
            lease_owner: attempt.lease_owner.clone().unwrap(),
            lease_token: attempt.lease_token.clone().unwrap(),
            lease_expires_at_ms: attempt.lease_expires_at_ms.unwrap(),
        };
        match changed {
            1 => record.effect_delivery_attempts[0].worker_generation = Some("new-worker".into()),
            2 => record.effect_delivery_attempts[0].lease_token = Some("new-token".into()),
            3 => record.has_session_lease = true,
            4 => {
                let mut other = record.effect_delivery_attempts[0].clone();
                other.id = "unobserved-active-attempt".into();
                record.effect_delivery_attempts.push(other);
            }
            _ => {}
        }
        let mut store = MemoryMaintenance {
            records: vec![record],
            applied: vec![],
        };
        let expected_run_id = store.records[0].run.as_ref().unwrap().run_id.clone();
        let result = crate::run_recovery::recover_observed(
            &mut store,
            "recovery-port",
            &expected_run_id,
            &recovery_host_binding("fixture"),
            None,
            &[observed],
            "2000",
            2000,
        );
        if changed == 0 || changed == 5 {
            result.unwrap();
            assert_eq!(
                store.applied[0].effect_delivery_attempt_mutations[0]
                    .record
                    .recovery_plan,
                Some(if changed == 5 {
                    EffectDeliveryRecoveryPlan::Retry
                } else {
                    EffectDeliveryRecoveryPlan::Resume
                })
            );
        } else {
            assert!(result.err().unwrap().contains("ownership changed"));
            assert!(
                store.applied.is_empty(),
                "stale observations must not commit recovery"
            );
        }
    }
}

#[test]
fn claimed_delivery_lease_is_application_overlay_during_recovery_validation() {
    let mut record = delivery_fixture(
        EffectDeliveryAttemptState::Leased,
        EffectDeliveryReplaySafety::Idempotent,
        None,
        1500,
    );
    let attempt = &record.effect_delivery_attempts[0];
    let mut request: serde_json::Value =
        serde_json::from_str(&record.effects[0].payload_json).unwrap();
    request["delivery"] = serde_json::to_value(EffectDeliveryLease {
        attempt_id: attempt.id.clone(),
        worker_generation: attempt.worker_generation.clone().unwrap(),
        lease_owner: attempt.lease_owner.clone().unwrap(),
        lease_token: attempt.lease_token.clone().unwrap(),
        lease_expires_at_ms: attempt.lease_expires_at_ms.unwrap(),
    })
    .unwrap();
    record.effects[0].payload_json = serde_json::to_string(&request).unwrap();

    let mut store = MemoryMaintenance {
        records: vec![record],
        applied: Vec::new(),
    };
    recover(&mut store).expect("a claimed delivery lease must not change Core payload identity");
    assert_eq!(store.applied.len(), 1);
    assert_eq!(
        store.applied[0].effect_delivery_attempt_mutations[0]
            .record
            .recovery_plan,
        Some(EffectDeliveryRecoveryPlan::Retry)
    );
}

#[test]
fn rebound_transport_request_requires_exact_physical_delivery_ownership() {
    for corrupt_lease in [false, true] {
        let mut record = delivery_fixture(
            EffectDeliveryAttemptState::ExecutionStarted,
            EffectDeliveryReplaySafety::Recoverable, Some(0), 1500,
        );
        let attempt = &mut record.effect_delivery_attempts[0];
        attempt.request_id = "previous-transport-request".into();
        let mut request: serde_json::Value = serde_json::from_str(&record.effects[0].payload_json).unwrap();
        request["delivery"] = serde_json::to_value(EffectDeliveryLease {
            attempt_id: attempt.id.clone(),
            worker_generation: attempt.worker_generation.clone().unwrap(),
            lease_owner: attempt.lease_owner.clone().unwrap(),
            lease_token: if corrupt_lease { "foreign-token".into() } else { attempt.lease_token.clone().unwrap() },
            lease_expires_at_ms: attempt.lease_expires_at_ms.unwrap(),
        }).unwrap();
        record.effects[0].payload_json = serde_json::to_string(&request).unwrap();
        let mut store = MemoryMaintenance { records: vec![record], applied: vec![] };
        if corrupt_lease {
            assert!(recover(&mut store).is_err());
            assert!(store.applied.is_empty());
        } else {
            recover(&mut store).unwrap();
            assert_eq!(store.applied[0].effect_delivery_attempt_mutations[0].record.recovery_plan,
                Some(EffectDeliveryRecoveryPlan::Resume));
        }
    }
}

#[test]
fn recoverable_started_delivery_requires_matching_canonical_cursor() {
    for corruption in 0..4 {
        let mut invalid = delivery_fixture(
            EffectDeliveryAttemptState::ExecutionStarted,
            EffectDeliveryReplaySafety::Recoverable,
            Some(7),
            1500,
        );
        match corruption {
            0 => {
                let mut checkpoint: serde_json::Value = serde_json::from_str(
                    &invalid
                        .checkpoint
                        .as_ref()
                        .unwrap()
                        .checkpoint
                        .checkpoint_json,
                )
                .unwrap();
                let effect_id = invalid.effects[0].effect_id.clone();
                *checkpoint
                    .pointer_mut(&format!(
                        "/state/host/pending_host_effects/effects/{effect_id}/last_response_sequence"
                    ))
                    .unwrap() = serde_json::Value::Null;
                invalid
                    .checkpoint
                    .as_mut()
                    .unwrap()
                    .checkpoint
                    .checkpoint_json = checkpoint.to_string();
            }
            1 => {
                let mut checkpoint: serde_json::Value = serde_json::from_str(
                    &invalid
                        .checkpoint
                        .as_ref()
                        .unwrap()
                        .checkpoint
                        .checkpoint_json,
                )
                .unwrap();
                let effect_id = invalid.effects[0].effect_id.clone();
                *checkpoint
                    .pointer_mut(&format!(
                        "/state/host/pending_host_effects/effects/{effect_id}/request/request_id"
                    ))
                    .unwrap() = serde_json::Value::String("foreign-request".into());
                invalid
                    .checkpoint
                    .as_mut()
                    .unwrap()
                    .checkpoint
                    .checkpoint_json = checkpoint.to_string();
            }
            2 => invalid.effect_delivery_attempts[0].request_id = "foreign-request".into(),
            _ => invalid.effect_delivery_attempts[0].effect_id = "foreign-effect".into(),
        }
        let mut store = MemoryMaintenance {
            records: vec![fixture(false), invalid],
            applied: Vec::new(),
        };
        let error = match recover(&mut store) {
            Err(error) => error,
            Ok(_) => panic!("corrupt effect delivery recovery facts were accepted"),
        };
        assert!(
            error.contains("effect delivery") || error.contains("pending Host request"),
            "unexpected recovery rejection: {error}"
        );
        assert!(
            store.applied.is_empty(),
            "cursor corruption must abort the whole recovery unit of work"
        );
    }
}

#[test]
fn live_lease_and_terminal_or_unexpired_delivery_attempts_are_unchanged() {
    let mut live = delivery_fixture(
        EffectDeliveryAttemptState::ExecutionStarted,
        EffectDeliveryReplaySafety::Recoverable,
        None,
        1500,
    );
    live.has_session_lease = true;
    live.checkpoint.as_mut().unwrap().checkpoint.checkpoint_json =
        "malformed-live-checkpoint".into();
    live.effect_delivery_attempts[0].lease_token = None;

    let unexpired = delivery_fixture(
        EffectDeliveryAttemptState::Leased,
        EffectDeliveryReplaySafety::Idempotent,
        None,
        2500,
    );
    let interrupted = delivery_fixture(
        EffectDeliveryAttemptState::Interrupted,
        EffectDeliveryReplaySafety::Idempotent,
        None,
        1500,
    );
    let unknown = delivery_fixture(
        EffectDeliveryAttemptState::OutcomeUnknown,
        EffectDeliveryReplaySafety::RequiresRecoveryDecision,
        None,
        1500,
    );
    let settled = delivery_fixture(
        EffectDeliveryAttemptState::Settled,
        EffectDeliveryReplaySafety::Idempotent,
        None,
        2500,
    );
    let mut store = MemoryMaintenance {
        records: vec![live, unexpired, interrupted, unknown, settled],
        applied: Vec::new(),
    };
    recover(&mut store).unwrap();
    assert_eq!(store.applied.len(), 5);
    assert!(store
        .applied
        .iter()
        .all(|plan| plan.effect_delivery_attempt_mutations.is_empty()));
}

#[test]
fn exact_expired_recovery_checks_current_expiry_and_frozen_run_before_writes() {
    for scenario in 0..6 {
        let mut record = delivery_fixture(
            EffectDeliveryAttemptState::ExecutionStarted,
            EffectDeliveryReplaySafety::Recoverable,
            Some(1),
            1500,
        );
        let run = record.run.as_ref().unwrap().run_id.clone();
        let mut binding = recovery_host_binding("fixture");
        let mut expected = run.clone();
        match scenario {
            1 => record.effect_delivery_attempts[0].lease_expires_at_ms = Some(2500), // same token, renewed
            2 => expected = "replacement-run".into(),
            3 => binding = recovery_host_binding("unauthorized-model"),
            4 => record.has_session_lease = true,
            5 => {
                record.effect_delivery_attempts[0].state = EffectDeliveryAttemptState::Leased;
                record.effect_delivery_attempts[0].execution_started_at_ms = None;
            }
            _ => {}
        }
        let before = record.effect_delivery_attempts[0].clone();
        let mut store = MemoryMaintenance {
            records: vec![record],
            applied: vec![],
        };
        let result = crate::run_recovery::recover_expired(
            &mut store,
            "recovery-port",
            &expected,
            &binding,
            "2000",
            2000,
        );
        match scenario {
            0 | 5 => {
                result.unwrap();
                let mutations = &store.applied[0].effect_delivery_attempt_mutations;
                assert_eq!(mutations.len(), 1);
                assert_eq!(
                    mutations[0].record.state,
                    EffectDeliveryAttemptState::Interrupted
                );
                assert_eq!(
                    mutations[0].record.recovery_plan,
                    Some(if scenario == 5 {
                        EffectDeliveryRecoveryPlan::Retry
                    } else {
                        EffectDeliveryRecoveryPlan::Resume
                    })
                );
                assert!(store.applied[0].delegated_permission_mutations.is_empty());
            }
            1 => {
                result.unwrap();
                assert!(store.applied[0]
                    .effect_delivery_attempt_mutations
                    .is_empty());
                assert!(store.applied[0].preserve_all_traces);
            }
            _ => {
                assert!(result.is_err());
                assert!(store.applied.is_empty());
            }
        }
        assert_eq!(store.records[0].effect_delivery_attempts[0], before);
    }
}

#[test]
fn expired_recovery_mixed_deliveries_never_treats_expiry_as_whole_run_death() {
    for corrupt_expiry in [false, true] {
        let mut record = delivery_fixture(
            EffectDeliveryAttemptState::ExecutionStarted,
            EffectDeliveryReplaySafety::Recoverable,
            Some(1),
            1500,
        );
        let expired_id = record.effect_delivery_attempts[0].id.clone();
        let mut other = record.effect_delivery_attempts[0].clone();
        other.id = "other-current-attempt".into();
        other.effect_id = "other-current-effect".into();
        other.request_id = "other-current-request".into();
        other.lease_expires_at_ms = if corrupt_expiry { None } else { Some(2500) };
        record.effect_delivery_attempts.push(other);
        let run = record.run.as_ref().unwrap().run_id.clone();
        let mut store = MemoryMaintenance {
            records: vec![record],
            applied: vec![],
        };
        let result = crate::run_recovery::recover_expired(
            &mut store,
            "recovery-port",
            &run,
            &recovery_host_binding("fixture"),
            "2000",
            2000,
        );
        if corrupt_expiry {
            assert!(
                result.is_err(),
                "A leased attempt with missing expiry is corrupt, not owner-death evidence"
            );
            assert!(
                store.applied.is_empty(),
                "One malformed lease must roll back every planned interruption"
            );
        } else {
            result.unwrap();
            let plan = &store.applied[0];
            assert_eq!(plan.effect_delivery_attempt_mutations.len(), 1);
            assert_eq!(
                plan.effect_delivery_attempt_mutations[0].record.id,
                expired_id
            );
            assert!(plan.release_session_lease.is_none());
            assert!(plan.delegated_permission_mutations.is_empty());
            assert!(plan.preserve_all_traces);
        }
    }
}
