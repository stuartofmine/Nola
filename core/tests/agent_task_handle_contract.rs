#[path = "support/model_identity.rs"]
mod model_identity;
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use nola_core::agent::host::{
    CapabilityAvailability, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostId, HostLease,
    HostOutputPolicy, HostReplaySafety, HostRequest, HostRequestEnvelope, HostResponse,
    HostResponseEnvelope,
};
use nola_core::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentEvent, AgentInputDelivery, AgentItem, AgentSubmission,
    AgentThreadState, ContextPlatform, ItemId, SessionId, TaskKind, TaskState,
};
use serde_json::json;

#[path = "contracts/execution_evidence_contract.rs"]
mod execution_evidence_contract;

fn capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: None,
        input_schema: json!({"type": "object"}),
        input_normalization: nola_core::agent::host::HostInputNormalization {
            // These fixture tools operate only on in-memory test values.
            permission_contract: nola_domain::permission_resource_plan::PermissionOperationContract {
                risk: nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead,
                ..Default::default()
            },
            ..Default::default()
        },
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

fn config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    for (id, route) in [
        ("model.stream", HostCapabilityRoute::Model),
        ("context.compact", HostCapabilityRoute::Compaction),
        ("permission.ask", HostCapabilityRoute::Permission),
        ("host.cancel", HostCapabilityRoute::Tool),
        ("long_tool", HostCapabilityRoute::Tool),
        ("short_tool", HostCapabilityRoute::Tool),
    ] {
        capabilities.insert(id.to_string(), capability(id, route));
    }
    AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string("task-contract-session".to_string())),
        model: Some("test-model".to_string()),
        tools: ["long_tool", "short_tool"]
            .into_iter()
            .map(|name| {
                json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": name,
                        "parameters": {"type": "object"}
                    }
                })
            })
            .collect(),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("task-host".to_string()),
            revision: 3,
            availability_epoch: 2,
            lease: Some(HostLease {
                lease_id: "task-lease".to_string(),
                issued_at_ms: 1,
                last_heartbeat_ms: 1,
                expires_at_ms: u64::MAX,
            }),
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}

fn request_matching(
    requests: &[HostRequestEnvelope],
    predicate: impl Fn(&HostRequest) -> bool,
) -> HostRequestEnvelope {
    requests
        .iter()
        .find(|request| predicate(&request.payload))
        .cloned()
        .expect("expected Host request")
}

fn response_for(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        attempt_id: None,
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

fn submit(
    core: &mut AgentCore,
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> nola_core::agent::AgentTransition {
    core.submit(AgentSubmission::HostResponse {
        response: response_for(request, sequence, payload),
    })
    .expect("correlated Host response")
}

fn checkpoint(core: &AgentCore) -> CoreCheckpoint<AgentThreadState> {
    core.checkpoint(CheckpointReason::Manual)
}

fn start_tool(core: &mut AgentCore, tool_name: &str) -> HostRequestEnvelope {
    let started = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string(format!("{tool_name}-user")),
                "perform the operation",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: ContextPlatform::Unknown,
            turn_context_components: Vec::new(),
            submission_id: Some(format!("{tool_name}-submission")),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("start Turn");
    let model = request_matching(&started.host_requests, |request| {
        matches!(request, HostRequest::ModelStream { .. })
    });
    let planned = submit(
        core,
        &model,
        1,
        HostResponse::ModelCompleted {
            items: Vec::new(),
            tool_calls: vec![json!({
                "call_id": format!("{tool_name}-call"),
                "name": tool_name,
                "arguments": {"value": 1}
            })],
            usage: json!({"input_tokens": 8, "output_tokens": 3, "total_tokens": 11}),
        },
    );
    request_matching(&planned.host_requests, |request| {
        matches!(request, HostRequest::ExecuteTool { .. })
    })
}

#[test]
fn deferred_tool_receipt_is_not_completed_mutation_or_validation_evidence() {
    use nola_core::agent::host::{
        HostCapabilityEffect, HostCapabilityEffectDeclaration, HostToolOutcome,
    };
    for outcome in [HostToolOutcome::Deferred, HostToolOutcome::Succeeded] {
        let mut configuration = config();
        let capability = configuration
            .host_catalog
            .as_mut()
            .unwrap()
            .capabilities
            .get_mut("long_tool")
            .unwrap();
        capability.input_normalization.effect =
            HostCapabilityEffectDeclaration::fixed(HostCapabilityEffect::Mutate);
        capability
            .input_normalization
            .requires_validation_after_mutation = true;
        let mut core = AgentCore::try_new(configuration).unwrap();
        let request = start_tool(&mut core, "long_tool");
        let completed = submit(
            &mut core,
            &request,
            1,
            HostResponse::ToolCompleted {
                call_id: "long_tool-call".into(),
                output: json!({"type":"deferred_tool_task", "task_id":"durable-task", "effects_completed":false}),
                outcome,
                activation: None,
                full_output_ref: None,
                artifacts: Vec::new(),
            },
        );
        let actually_completed = outcome == HostToolOutcome::Succeeded;
        assert_eq!(
            completed.snapshot.engineering.validation_required,
            actually_completed
        );
        assert_eq!(
            completed.snapshot.engineering.mutation_count,
            u32::from(actually_completed)
        );
        // The result's human/JSON text cannot override the typed outcome.
        assert!(!completed.snapshot.engineering.recovery_required);
    }
}

#[test]
fn explicit_task_promotion_retires_foreground_authority_and_allows_a_new_turn() {
    let mut core = AgentCore::try_new(config()).expect("create Core");
    let tool = start_tool(&mut core, "long_tool");
    let turn_id = tool.turn_id.clone().expect("tool Turn");
    let before_invalid = serde_json::to_value(checkpoint(&core)).unwrap();

    let invalid = core.submit(AgentSubmission::HostResponse {
        response: response_for(
            &tool,
            1,
            HostResponse::TaskPromoted {
                input_watermark: None,
                task_id: "task-invalid".to_string(),
                kind: TaskKind::Skill,
                state: TaskState::Completed,
                progress: Some(1.0),
                current_step: Some("done".to_string()),
                lineage: Default::default(),
            },
        ),
    });
    assert!(
        invalid.is_err(),
        "promotion cannot start in a terminal state"
    );
    assert_eq!(serde_json::to_value(checkpoint(&core)).unwrap(), before_invalid,
        "invalid promotion cannot publish Task creation evidence");
    let mut foreign = response_for(&tool, 1, HostResponse::TaskPromoted {
        input_watermark: None, task_id: "foreign-task".into(), kind: TaskKind::Skill,
        state: TaskState::Running, progress: None, current_step: None, lineage: Default::default(),
    });
    foreign.session_id = "foreign-session".into();
    assert!(core.submit(AgentSubmission::HostResponse {response:foreign}).is_err());
    assert_eq!(serde_json::to_value(checkpoint(&core)).unwrap(), before_invalid);

    let promoted = submit(
        &mut core,
        &tool,
        1,
        HostResponse::TaskPromoted {
            input_watermark: None,
            task_id: "task-1".to_string(),
            kind: TaskKind::Skill,
            state: TaskState::Running,
            progress: Some(0.25),
            current_step: Some("rendering document".to_string()),
            lineage: Default::default(),
        },
    );
    let task = promoted
        .snapshot
        .tasks
        .tasks
        .get("task-1")
        .expect("Task projection");
    assert_eq!(task.owner_session_id.as_str(), "task-contract-session");
    assert_eq!(task.owner_turn_id.as_str(), turn_id);
    assert_eq!(task.kind, TaskKind::Skill);
    assert_eq!(task.state, TaskState::Running);
    assert_eq!(promoted.snapshot.tasks.revision, 1);
    let observations = promoted.event_envelopes.iter().filter_map(|event| match &event.event {
        AgentEvent::ExecutionEvidenceObserved { evidence, .. } => Some(evidence), _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(observations.len(), 1, "one admitted handoff has one original observation");
    let observation = observations[0];
    observation.validate().unwrap();
    assert_eq!(observation.tool_name, "long_tool");
    assert_eq!(observation.source.effect_id, tool.effect_id.0);
    assert_eq!(observation.source.request_id, tool.request_id);
    assert_eq!(observation.source.host_id, tool.host_id.0);
    assert_eq!(observation.source.response_sequence.0, 1);
    assert_eq!(observation.outcome, nola_domain::host_effect::HostToolOutcome::Deferred);
    assert!(observation.receipt.is_none(), "promotion cannot attest child effects or verification");
    assert!(promoted.event_envelopes.iter().any(|event| {
        matches!(
            &event.event,
            AgentEvent::TaskUpdated { task, .. }
                if task.id == "task-1" && task.state == TaskState::Running
        )
    }));
    assert!(
        promoted
            .host_requests
            .iter()
            .any(|request| { matches!(request.payload, HostRequest::ModelStream { .. }) }),
        "foreground Turn must continue after promotion"
    );

    let promoted_checkpoint = checkpoint(&core);
    assert!(promoted_checkpoint.state.host.pending_effects.get(&tool.effect_id.0).is_none());
    let mut restored = AgentCore::restore(config(), promoted_checkpoint).expect("restore Core");
    let restored_observations = serde_json::to_value(checkpoint(&restored)).unwrap();
    let restored_observations = restored_observations.pointer("/state/execution/requirements/observations")
        .unwrap().as_object().unwrap();
    assert_eq!(restored_observations.len(), 1);
    assert_eq!(restored_observations[&observation.evidence_id]["source"], serde_json::to_value(&observation.source).unwrap());
    assert_eq!(restored_observations[&observation.evidence_id]["outcome"], "deferred");
    assert_eq!(restored_observations[&observation.evidence_id]["receipt"], serde_json::Value::Null);
    assert_eq!(restored.snapshot().tasks.tasks["task-1"].state, TaskState::Running);
    assert_eq!(restored.snapshot().tasks.tasks["task-1"].owner_turn_id.as_str(), turn_id);
    let before = checkpoint(&restored);
    assert!(restored.submit(AgentSubmission::ResumeHostEffect { effect_id: tool.effect_id.0.clone() }).is_err());
    assert_eq!(serde_json::to_value(checkpoint(&restored)).unwrap(), serde_json::to_value(&before).unwrap());
    for payload in [
        HostResponse::TaskPromoted { input_watermark: None, task_id: "task-1".into(),
            kind: TaskKind::Skill, state: TaskState::Running, progress: Some(0.25),
            current_step: Some("rendering document".into()), lineage: Default::default() },
        HostResponse::TaskUpdated { task_id: "task-1".into(), state: TaskState::Completed,
            progress: Some(1.0), current_step: Some("complete".into()), result_ref: Some("artifact://document.docx".into()),
            cancel_requested: false, ownership_token: None, completion_watermark: None },
    ] {
        for sequence in [1, 2] {
            assert!(restored.submit(AgentSubmission::HostResponse {
                response: response_for(&tool, sequence, payload.clone()),
            }).is_err(), "the retired foreground Host cannot regain Task authority");
            assert_eq!(serde_json::to_value(checkpoint(&restored)).unwrap(), serde_json::to_value(&before).unwrap());
        }
    }
    let model = request_matching(&promoted.host_requests, |request| matches!(request, HostRequest::ModelStream { .. }));
    let finished = submit(&mut restored, &model, 1, HostResponse::ModelCompleted {
        items: vec![serde_json::to_value(AgentItem::assistant_message(ItemId::from_string("parent-final".into()), "Task handed off.")).unwrap()],
        tool_calls: vec![], usage: json!({}),
    });
    assert!(finished.snapshot.turn_status.is_terminal());
    let completed = checkpoint(&restored);
    assert_eq!(completed.state.host.pending_effects.len(), 0);
    assert!(completed.state.permits_new_turn_execution_configuration());
    assert_eq!(completed.state.task_projection.tasks["task-1"].state, TaskState::Running);
    let mut cold = AgentCore::restore(config(), completed).unwrap();
    let next = cold.submit(AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(ItemId::from_string("next-user".into()), "Continue with a new request")],
        turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
        submission_id: Some("next-turn".into()), submitted_at_epoch_ms: Some(2000), delivery: AgentInputDelivery::CurrentTurn,
    }).unwrap();
    assert!(next.host_requests.iter().any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
    assert_eq!(next.snapshot.tasks.tasks["task-1"].state, TaskState::Running);
}

#[test]
fn ordinary_tool_completion_never_creates_a_task_handle() {
    let mut core = AgentCore::try_new(config()).expect("create Core");
    let tool = start_tool(&mut core, "short_tool");
    let completed = submit(
        &mut core,
        &tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "short_tool-call".to_string(),
            output: json!({"ok": true}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert!(completed.snapshot.tasks.is_empty());
    assert!(!completed
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TaskUpdated { .. })));
    let checkpoint = checkpoint(&core);
    assert!(checkpoint.state.task_projection.is_empty());
}

#[test]
fn cancellation_intent_survives_restart_and_converges_both_effects() {
    let mut core = AgentCore::try_new(config()).expect("create Core");
    let tool = start_tool(&mut core, "long_tool");
    let _promoted = submit(
        &mut core,
        &tool,
        1,
        HostResponse::TaskPromoted {
            input_watermark: None,
            task_id: "task-cancel".to_string(),
            kind: TaskKind::DataRefresh,
            state: TaskState::Waiting,
            progress: Some(0.4),
            current_step: Some("waiting for network".to_string()),
            lineage: Default::default(),
        },
    );

    let cancellation = core
        .submit(AgentSubmission::RequestTaskCancellation {
            task_id: "task-cancel".to_string(),
        })
        .expect("request Task cancellation");
    assert!(cancellation.snapshot.tasks.tasks["task-cancel"].cancel_requested);
    let cancel_request = request_matching(&cancellation.host_requests, |request| {
        matches!(
            request,
            HostRequest::CancelTask { task_id } if task_id == "task-cancel"
        )
    });
    let checkpoint = checkpoint(&core);
    let mut restored = AgentCore::restore(config(), checkpoint).expect("restore cancellation");
    assert!(restored.snapshot().tasks.tasks["task-cancel"].cancel_requested);
    let resumed = restored
        .submit(AgentSubmission::ResumeHostEffect {
            effect_id: cancel_request.effect_id.0.clone(),
        })
        .expect("explicitly resume cancellation effect");
    let restored_cancel = resumed
        .host_requests
        .into_iter()
        .find(|request| request.effect_id == cancel_request.effect_id)
        .expect("restore CancelTask effect");

    let cancelled = submit(
        &mut restored,
        &restored_cancel,
        1,
        HostResponse::TaskUpdated {
            task_id: "task-cancel".to_string(),
            state: TaskState::Cancelled,
            progress: Some(0.4),
            current_step: Some("cancelled".to_string()),
            result_ref: None,
            cancel_requested: true,
            ownership_token: None,
            completion_watermark: None,
        },
    );
    assert_eq!(
        cancelled.snapshot.tasks.tasks["task-cancel"].state,
        TaskState::Cancelled
    );
    let remaining = restored.pending_host_requests().expect("remaining effects");
    assert!(!remaining.iter().any(|request| {
        request.effect_id == tool.effect_id || request.effect_id == cancel_request.effect_id
    }));
    assert!(
        restored
            .submit(AgentSubmission::HostResponse {
                response: response_for(
                    &tool,
                    2,
                    HostResponse::TaskUpdated {
                        task_id: "task-cancel".to_string(),
                        state: TaskState::Completed,
                        progress: Some(1.0),
                        current_step: Some("late completion".to_string()),
                        result_ref: Some("result://late".to_string()),
                        cancel_requested: true,
                        ownership_token: None,
                        completion_watermark: None,
                    },
                ),
            })
            .is_err(),
        "late original-effect completion must not overwrite cancellation"
    );
}

fn owned_task(id: &str) -> nola_core::agent::task::TaskHandle {
    nola_core::agent::task::TaskHandle {
        id: id.into(),
        owner_session_id: SessionId::from_string("session".into()),
        owner_turn_id: nola_core::agent::TurnId::from_string("turn".into()),
        kind: TaskKind::Agent,
        state: TaskState::Queued,
        progress: None,
        current_step: None,
        result_ref: None,
        cancel_requested: false,
        cancel_requested_at_epoch_ms: None,
        lineage: nola_core::agent::TaskLineage {
            child_session_id: Some(SessionId::from_string("child".into())),
            parent_call_id: Some(nola_core::agent::CallId::from_string("task-call".into())),
            ..Default::default()
        },
        durability: Default::default(),
    }
}

fn reconcile_owned_task(task: nola_core::agent::task::TaskHandle, result_ref: &str)
    -> nola_core::agent::task::TaskHandle {
    // A pure reducer fixture, not a fabricated authenticated Store export.
    // Store/Application tests separately prove the receipt source transaction.
    let result: nola_domain::run_result::RunResult = serde_json::from_value(json!({
        "result_id":"child-result","run_id":"child-run","conversation_id":"child",
        "originating_submission_id":"child-input","input":{"watermark":"1","applications_digest":"00".repeat(32)},
        "decided_at_revision":"4","decision_event_cursor":"5","requirement_revision":"0","evidence_frontier":"5",
        "value":{"kind":"answered","answer_part_refs":["answer"]}
    })).unwrap();
    let fact = nola_core::agent::task::TaskTerminalReconciliation {
        protocol_version:1, authority_revision:2, ended_at_ms:21_600_000,
        result_digest:Some(result.computed_digest().unwrap()), run_result:Some(result),
        admission:nola_domain::agent_task_admission::AgentTaskAdmission {
            predecessor_task_id: None,
            protocol_version:nola_domain::agent_task_admission::AGENT_TASK_ADMISSION_VERSION, task_id:task.id.clone(), parent_conversation_id:"conversation".into(),
            parent_core_session_id:task.owner_session_id.as_str().into(), parent_run_id:"run".into(),
            parent_turn_id:task.owner_turn_id.as_str().into(), child_conversation_id:"child".into(),
            parent_tool_call_id:"task-call".into(), parent_effect_id:"effect".into(), input_watermark:Some(42),
            source_store_epoch:"epoch".into(), source_transition_id:"transition".into(), source_host_event_id:"host-event".into(),
            source_checkpoint_revision:1, core_revision:1, core_event_cursor:1, admission_digest:String::new(),
        }.seal().unwrap(), receipt_id:"receipt".into(), input_watermark:Some(42), completion_watermark:Some(42),
        guidance_results:vec![], status:"sealed".into(), result_ref:Some(result_ref.into()),
    };
    let owner = task.owner_session_id.clone();
    let mut projection = nola_core::agent::task::TaskProjection::default();
    projection.order.push(task.id.clone());
    projection.tasks.insert(task.id.clone(), task);
    projection.reconcile_terminal(&fact, &owner).unwrap()
}
fn owned_claim(
    task: nola_core::agent::task::TaskHandle,
    token: &str,
    expiry: i64,
) -> nola_core::agent::task::TaskHandle {
    nola_core::agent::task::reduce_task_authority(
        task,
        nola_core::agent::task::TaskAuthorityCommand::Claim {
            worker_id: format!("worker-{token}"),
            ownership_token: token.into(),
            now_epoch_ms: 10,
            lease_until_epoch_ms: expiry,
            input_watermark: Some(42),
        },
    )
    .unwrap()
}
#[test]
fn claimed_worker_commits_task_result_without_forging_original_host_response() {
    use nola_core::agent::task::{reduce_task_authority, TaskAuthorityCommand};
    let owned = owned_claim(owned_task("artifact-task"), "token-1", 1000);
    assert!(reduce_task_authority(
        owned.clone(),
        TaskAuthorityCommand::Complete {
            ownership_token: "foreign".into(),
            completion_watermark: Some(42),
            result_ref: Some("artifact://report.docx".into())
        }
    )
    .is_err());
    assert!(reduce_task_authority(
        owned.clone(),
        TaskAuthorityCommand::Complete {
            ownership_token: "token-1".into(),
            completion_watermark: Some(43),
            result_ref: Some("artifact://report.docx".into())
        }
    )
    .is_err());
    assert!(reduce_task_authority(
        owned.clone(),
        TaskAuthorityCommand::Complete {
            ownership_token: "token-1".into(),
            completion_watermark: Some(42),
            result_ref: Some("artifact://report.docx".into()),
        },
    ).is_err(), "even the current worker cannot certify an Agent Run result");
    let completed = reconcile_owned_task(owned, "artifact://report.docx");
    assert_eq!(completed.state, TaskState::Sealed);
    assert!(matches!(completed.durability.run_result.as_ref().unwrap().value,
        nola_domain::run_result::RunResultValue::Answered { .. }));
    assert_eq!(
        completed.result_ref.as_deref(),
        Some("artifact://report.docx")
    );
    assert_eq!(completed.durability.completion_watermark, Some(42));
    assert!(completed.durability.ownership_token.is_none());
}
#[test]
fn permission_handoff_is_checkpointed_and_replay_cannot_reclaim_old_authority() {
    use nola_core::agent::task::{reduce_task_authority, TaskAuthorityCommand};
    let mut owned = owned_claim(owned_task("permission"), "old", 100);
    owned.state = TaskState::Waiting;
    let owned = serde_json::from_value(serde_json::to_value(&owned).unwrap()).unwrap();
    let resumed = reduce_task_authority(
        owned,
        TaskAuthorityCommand::ResumeAfterPermission {
            previous_ownership_token: "old".into(),
            worker_id: "new-worker".into(),
            ownership_token: "new".into(),
            now_epoch_ms: 200,
            lease_until_epoch_ms: 300,
            input_watermark: Some(42),
        },
    )
    .unwrap();
    assert_eq!(resumed.state, TaskState::Running);
    assert_eq!(resumed.durability.ownership_token.as_deref(), Some("new"));
    let cold = serde_json::from_value(serde_json::to_value(&resumed).unwrap()).unwrap();
    assert!(reduce_task_authority(
        cold,
        TaskAuthorityCommand::Heartbeat {
            ownership_token: "old".into(),
            now_epoch_ms: 210,
            lease_until_epoch_ms: 310
        }
    )
    .is_err());
    assert!(reduce_task_authority(
        resumed,
        TaskAuthorityCommand::Claim {
            worker_id: "old-worker".into(),
            ownership_token: "old".into(),
            now_epoch_ms: 210,
            lease_until_epoch_ms: 310,
            input_watermark: Some(42)
        }
    )
    .is_err());
    // Idempotent/lost ACK replay belongs to the real Application receipt and
    // immutable continuation grant, covered by task_atomic_ownership_e2e.
}
#[test]
fn permission_recovery_atomically_hands_off_original_running_owner() {
    exercise_permission_recovery(false);
}
#[test]
fn permission_recovery_accepts_a_preceding_committed_ancestor_handoff() {
    exercise_permission_recovery(true);
}
fn exercise_permission_recovery(previous_handoff: bool) {
    use nola_core::agent::task::{reduce_task_authority, TaskAuthorityCommand};
    let mut owned = owned_claim(owned_task("recovery"), "original", 100);
    if previous_handoff {
        owned.state = TaskState::Waiting;
        owned = reduce_task_authority(
            owned,
            TaskAuthorityCommand::ResumeAfterPermission {
                previous_ownership_token: "original".into(),
                worker_id: "previous-worker".into(),
                ownership_token: "previous".into(),
                now_epoch_ms: 150,
                lease_until_epoch_ms: 200,
                input_watermark: Some(42),
            },
        )
        .unwrap();
    }
    let before = serde_json::to_value(&owned).unwrap();
    for tokens in [
        vec!["foreign".into()],
        vec![],
        vec!["original".into(), "original".into()],
    ] {
        assert!(reduce_task_authority(
            owned.clone(),
            TaskAuthorityCommand::RecoverAfterPermission {
                previous_ownership_tokens: tokens,
                worker_id: "successor-worker".into(),
                ownership_token: "successor".into(),
                now_epoch_ms: 300,
                lease_until_epoch_ms: 500,
                input_watermark: Some(42)
            }
        )
        .is_err());
        assert_eq!(serde_json::to_value(&owned).unwrap(), before);
    }
    assert!(reduce_task_authority(
        owned.clone(),
        TaskAuthorityCommand::RecoverAfterPermission {
            previous_ownership_tokens: vec!["original".into(), "previous".into()],
            worker_id: "successor-worker".into(),
            ownership_token: "successor".into(),
            now_epoch_ms: 300,
            lease_until_epoch_ms: 500,
            input_watermark: Some(43)
        }
    )
    .is_err());
    let resumed = reduce_task_authority(
        owned,
        TaskAuthorityCommand::RecoverAfterPermission {
            previous_ownership_tokens: vec!["original".into(), "previous".into()],
            worker_id: "successor-worker".into(),
            ownership_token: "successor".into(),
            now_epoch_ms: 300,
            lease_until_epoch_ms: 500,
            input_watermark: Some(42),
        },
    )
    .unwrap();
    assert_eq!(resumed.state, TaskState::Running);
    assert_eq!(
        resumed.durability.ownership_token.as_deref(),
        Some("successor")
    );
    let cold: nola_core::agent::task::TaskHandle =
        serde_json::from_value(serde_json::to_value(&resumed).unwrap()).unwrap();
    for token in ["original", "previous"] {
        assert!(reduce_task_authority(
            cold.clone(),
            TaskAuthorityCommand::Heartbeat {
                ownership_token: token.into(),
                now_epoch_ms: 310,
                lease_until_epoch_ms: 510
            }
        )
        .is_err());
    }
}
#[test]
fn virtual_six_hour_task_survives_hourly_restart_and_commits_artifact_once() {
    use nola_core::agent::task::{reduce_task_authority, TaskAuthorityCommand};
    let started = std::time::Instant::now();
    let mut task = owned_claim(owned_task("six-hour"), "six-hour-token", 120000);
    for minute in 1_i64..=360 {
        let at = minute * 60000;
        task = reduce_task_authority(
            task,
            TaskAuthorityCommand::Heartbeat {
                ownership_token: "six-hour-token".into(),
                now_epoch_ms: at,
                lease_until_epoch_ms: at + 120000,
            },
        )
        .unwrap();
        task.progress = Some(minute as f64 / 360.0);
        task.current_step = Some(format!("minute {minute} of 360"));
        if minute % 60 == 0 {
            task = serde_json::from_value(serde_json::to_value(&task).unwrap()).unwrap();
            assert_eq!(task.state, TaskState::Running);
            assert_eq!(
                task.durability.ownership_token.as_deref(),
                Some("six-hour-token")
            );
            assert_eq!(task.durability.input_watermark, Some(42));
        }
    }
    assert!(reduce_task_authority(
        task.clone(),
        TaskAuthorityCommand::Complete {
            ownership_token: "six-hour-token".into(),
            completion_watermark: Some(42),
            result_ref: Some("artifact://six-hour-result".into()),
        },
    ).is_err(), "a long-lived worker still requires the canonical Run receipt");
    task = reconcile_owned_task(task, "artifact://six-hour-result");
    assert_eq!(task.state, TaskState::Sealed);
    assert_eq!(task.durability.completion_watermark, Some(42));
    assert_eq!(
        task.result_ref.as_deref(),
        Some("artifact://six-hour-result")
    );
    assert!(task.durability.ownership_token.is_none());
    assert!(reduce_task_authority(
        task,
        TaskAuthorityCommand::Heartbeat {
            ownership_token: "six-hour-token".into(),
            now_epoch_ms: 21600001,
            lease_until_epoch_ms: 21720001
        }
    )
    .is_err());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "virtual clock must not sleep through six hours"
    );
}

#[test]
fn owned_live_wait_survives_repetition_and_restore_without_disabling_tool_loop_guard() {
    let mut configuration = config();
    configuration.host_catalog.as_mut().unwrap().capabilities.insert(
        "wait_task".into(), capability("wait_task", HostCapabilityRoute::Tool));
    configuration.tools.push(json!({"type":"function","function":{
        "name":"wait_task","description":"Wait for a promoted task","parameters":{"type":"object"}}}));
    let mut core = AgentCore::try_new(configuration.clone()).unwrap();
    let source = start_tool(&mut core, "long_tool");
    let promoted = submit(&mut core, &source, 1, HostResponse::TaskPromoted {
        input_watermark: None, task_id: "waiting-child".into(), kind: TaskKind::Agent,
        state: TaskState::Waiting, progress: None, current_step: Some("waiting for approval".into()),
        lineage: Default::default(),
    });
    let mut model = request_matching(&promoted.host_requests, |payload| matches!(payload, HostRequest::ModelStream { .. }));
    // These are explicit simulated transport responses through the production
    // reducer, not real-model/UI evidence. A cold restore retains ownership.
    for index in 0..8 {
        if index == 4 { core = AgentCore::restore(configuration.clone(), checkpoint(&core)).unwrap(); }
        let call_id = format!("wait-{index}");
        let planned = submit(&mut core, &model, 1, HostResponse::ModelCompleted {
            items: vec![], tool_calls: vec![json!({"call_id":call_id,"name":"wait_task",
                "arguments":{"task_id":"waiting-child","timeout_sec":1}})], usage: json!({}),
        });
        let wait = request_matching(&planned.host_requests, |payload| matches!(payload, HostRequest::ExecuteTool { .. }));
        let returned = submit(&mut core, &wait, 1, HostResponse::ToolCompleted {
            call_id, outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            output: json!({"task_id":"waiting-child","wait_timed_out":true,"action_required":"permission"}),
            activation: None, full_output_ref: None, artifacts: vec![],
        });
        model = request_matching(&returned.host_requests, |payload| matches!(payload, HostRequest::ModelStream { .. }));
        assert_eq!(returned.snapshot.tasks.tasks["waiting-child"].state, TaskState::Waiting);
    }
    // Ordinary repeated execution must still stop at its original threshold.
    for index in 0..3 {
        let call_id = format!("ordinary-{index}");
        let planned = submit(&mut core, &model, 1, HostResponse::ModelCompleted {
            items: vec![], tool_calls: vec![json!({"call_id":call_id,"name":"short_tool","arguments":{"value":1}})],
            usage: json!({}),
        });
        if index == 2 {
            assert!(!planned.host_requests.iter().any(|request| matches!(&request.payload, HostRequest::ExecuteTool { .. })));
            assert!(serde_json::to_string(&planned.snapshot.projection).unwrap().contains("repeated_tool_batch"));
            break;
        }
        let tool = request_matching(&planned.host_requests, |payload| matches!(payload, HostRequest::ExecuteTool { .. }));
        let returned = submit(&mut core, &tool, 1, HostResponse::ToolCompleted {
            call_id, outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            output: json!({"value":1}), activation: None, full_output_ref: None, artifacts: vec![],
        });
        model = request_matching(&returned.host_requests, |payload| matches!(payload, HostRequest::ModelStream { .. }));
    }
}
