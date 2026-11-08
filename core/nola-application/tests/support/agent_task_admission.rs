// SPDX-License-Identifier: Apache-2.0
//! Real source Host transaction -> narrow export -> trusted Task-owner import.
#[path = "model_identity.rs"]
mod fixture_model_identity;
use nola_agent_engine::agent::{
    host::*, AgentCoreConfig, AgentEvent, AgentInputDelivery, AgentItem, AgentSubmission, ItemId,
    SessionId, TaskKind, TaskLineage, TaskState,
};
use nola_application::{
    ApplicationCommand, BeginAssistantTurn, CreateRuntimeTask, RuntimeTaskCommand,
};
use serde_json::json;

fn capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.into(),
        title: id.into(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: if id == "model.stream" {
            HostCapabilityExposure::Internal
        } else {
            HostCapabilityExposure::Visible
        },
        permission_domain: None,
        input_schema: json!({"type":"object"}),
        input_normalization: HostInputNormalization {
            permission_contract:nola_domain::permission_resource_plan::PermissionOperationContract {
                risk:match id {
                    "fixture.task" => nola_domain::permission_resource_plan::PermissionOperationRisk::NewFile,
                    "wait_task" => nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead,
                    _ => nola_domain::permission_resource_plan::PermissionOperationRisk::Unknown,
                },..Default::default()
            },..Default::default()
        },
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000,
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

pub fn draft(
    target: &mut super::ApplicationRuntime,
    budget: Option<nola_application::RuntimeTaskResourceBudget>,
) {
    target
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: CreateRuntimeTask {
                    id: "task".into(),
                    kind: "agent".into(),
                    title: "Task".into(),
                    project_root: "/fixture".into(),
                    owner_session_id: Some("parent".into()),
                    parent_task_id: None,
                    parent_tool_call_id: Some("task-call".into()),
                    child_session_id: Some("child".into()),
                    agent_type: Some("explore".into()),
                    prompt: Some("Execute this task once".into()),
                    initial_progress_summary: None,
                    initial_activity: None,
                    output_path: None,
                    initially_unread: false,
                    input_watermark: Some(7),
                    created_at_ms: 100,
                    metadata: json!({}),
                    scheduling: nola_application::RuntimeTaskScheduling {
                        priority: nola_application::RuntimeTaskPriority::Normal,
                        queued_at_ms: 100,
                        resource_budget: budget.unwrap_or_default(),
                        resource_usage: Default::default(),
                        output_bytes: 0,
                        budget_exhaustion: None,
                    },
                },
            },
        })
        .unwrap();
}

pub fn source_fact(
    source: &mut super::ApplicationRuntime,
) -> nola_domain::agent_task_admission::AgentTaskAdmission {
    source_fact_for(source, "task", "parent", "child", "task-call", Some(7))
}

pub fn source_fact_for(
    source: &mut super::ApplicationRuntime,
    task_id: &str,
    parent_conversation_id: &str,
    child_conversation_id: &str,
    parent_call_id: &str,
    input_watermark: Option<i64>,
) -> nola_domain::agent_task_admission::AgentTaskAdmission {
    source_fact_for_with_turn_identity(
        source,
        task_id,
        parent_conversation_id,
        child_conversation_id,
        parent_call_id,
        input_watermark,
        fixture_model_identity::identity(),
    )
}

pub fn source_fact_for_with_turn_identity(
    source: &mut super::ApplicationRuntime,
    task_id: &str,
    parent_conversation_id: &str,
    child_conversation_id: &str,
    parent_call_id: &str,
    input_watermark: Option<i64>,
    turn_identity: nola_agent_engine::agent::AgentTurnIdentity,
) -> nola_domain::agent_task_admission::AgentTaskAdmission {
    source_fact_for_continuation(
        source,
        task_id,
        parent_conversation_id,
        child_conversation_id,
        parent_call_id,
        input_watermark,
        turn_identity,
        None,
    )
}

pub fn source_fact_for_continuation(
    source: &mut super::ApplicationRuntime,
    task_id: &str,
    parent_conversation_id: &str,
    child_conversation_id: &str,
    parent_call_id: &str,
    input_watermark: Option<i64>,
    turn_identity: nola_agent_engine::agent::AgentTurnIdentity,
    predecessor_task_id: Option<String>,
) -> nola_domain::agent_task_admission::AgentTaskAdmission {
    let config = AgentCoreConfig {
        turn_identity,
        session_id: Some(SessionId::from_string("actual-parent-core".into())),
        model: Some("fixture-model".into()),
        tools: vec![
            json!({"type":"function","function":{"name":"fixture.task","description":"Start the requested task","parameters":{"type":"object"}}}),
            json!({"type":"function","function":{"name":"wait_task","description":"Wait for the exact Task result","parameters":{"type":"object"}}}),
        ],
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("fixture-host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: [
                (
                    "model.stream".into(),
                    capability("model.stream", HostCapabilityRoute::Model),
                ),
                (
                    "fixture.task".into(),
                    capability("fixture.task", HostCapabilityRoute::Tool),
                ),
                (
                    "wait_task".into(),
                    capability("wait_task", HostCapabilityRoute::Tool),
                ),
            ]
            .into_iter()
            .collect(),
        }),
        ..Default::default()
    };
    let offset = if predecessor_task_id.is_some() {
        650
    } else {
        0
    };
    let (run, digest) = if predecessor_task_id.is_some() {
        let active: nola_application::ActiveRun = serde_json::from_value(
            source
                .execute(ApplicationCommand::ActiveRun {
                    conversation_id: parent_conversation_id.into(),
                })
                .unwrap(),
        )
        .unwrap();
        (
            active.run_id,
            active
                .invocation_config_digest
                .expect("admitted parent digest"),
        )
    } else {
        let frontier: Option<nola_application::ConversationTailSnapshot> = serde_json::from_value(
            source
                .execute(ApplicationCommand::LoadConversationTailSnapshotV1 {
                    conversation_id: parent_conversation_id.into(),
                    message_limit: 1,
                    part_text_limit: 1,
                    part_json_limit: 1,
                })
                .unwrap(),
        )
        .unwrap();
        let (configuration_revision, parent_message_id) = frontier
            .map(|tail| {
                (
                    Some(tail.conversation.configuration_revision),
                    tail.conversation
                        .messages
                        .last()
                        .map(|message| message.id.clone()),
                )
            })
            .unwrap_or((None, None));
        let begun: nola_application::AcceptedAssistantTurn = serde_json::from_value(
            source
                .execute(ApplicationCommand::BeginAssistantTurn {
                    input: BeginAssistantTurn {
                        expected_configuration_revision: configuration_revision,
                        expected_parent_message_id: parent_message_id,
                        conversation_id: parent_conversation_id.into(),
                        conversation_title: "Parent".into(),
                        user_message_id: "parent-user".into(),
                        user_part_id: "parent-user-body".into(),
                        user_text: "Delegate this explicitly durable task".into(),
                        user_content_json: None,
                        assistant_message_id: "parent-assistant".into(),
                        context_checkpoint_id: None,
                        core_config: config,
                        submission: AgentSubmission::UserInput {
                            items: vec![AgentItem::user_message(
                                ItemId::from_string("parent-user".into()),
                                "Delegate this explicitly durable task",
                            )],
                            turn_context_items: vec![],
                            turn_context_platform: Default::default(),
                            turn_context_components: vec![],
                            submission_id: Some("parent-run".into()),
                            submitted_at_epoch_ms: Some(500),
                            delivery: AgentInputDelivery::CurrentTurn,
                        },
                        created_at_ms: 500,
                    },
                })
                .unwrap(),
        )
        .unwrap();
        (begun.begin.run_id, begun.invocation_config_digest)
    };
    let claimed: nola_application::CommittedHostRequests = serde_json::from_value(
        source
            .execute(ApplicationCommand::ClaimCommittedHostRequests {
                input: nola_application::ClaimCommittedHostRequests {
                    run_id: run.clone(),
                    worker_generation: "source-model".into(),
                    lease_owner: "source".into(),
                    claimed_at_ms: 600 + offset,
                    lease_expires_at_ms: 10000,
                    limit: 16,
                },
            })
            .unwrap(),
    )
    .unwrap();
    let mut model = claimed
        .requests
        .into_iter()
        .find(|r| matches!(r.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    accept(
        source,
        &run,
        &digest,
        &model,
        parent_conversation_id,
        1,
        HostResponse::ExecutionStarted,
        601 + offset,
    );
    model.resume_after_sequence = Some(1);
    accept(
        source,
        &run,
        &digest,
        &model,
        parent_conversation_id,
        2,
        HostResponse::ModelAttemptStarted {
            provider: "fixture-provider".into(),
            model: "fixture-model".into(),
            reason: "initial".into(),
        },
        602 + offset,
    );
    model.resume_after_sequence = Some(2);
    accept(
        source,
        &run,
        &digest,
        &model,
        parent_conversation_id,
        3,
        HostResponse::ModelCompleted {
            items: vec![],
            tool_calls: vec![
                json!({"call_id":parent_call_id,"name":"fixture.task","arguments":{}}),
            ],
            usage: json!({}),
        },
        603 + offset,
    );
    let claimed: nola_application::CommittedHostRequests = serde_json::from_value(
        source
            .execute(ApplicationCommand::ClaimCommittedHostRequests {
                input: nola_application::ClaimCommittedHostRequests {
                    run_id: run.clone(),
                    worker_generation: "source-tool".into(),
                    lease_owner: "source".into(),
                    claimed_at_ms: 610 + offset,
                    lease_expires_at_ms: 10000,
                    limit: 16,
                },
            })
            .unwrap(),
    )
    .unwrap();
    let tool = claimed
        .requests
        .into_iter()
        .find(|r| matches!(r.payload, HostRequest::ExecuteTool { .. }))
        .expect("actual Core declares the promoted tool");
    accept(
        source,
        &run,
        &digest,
        &tool,
        parent_conversation_id,
        1,
        HostResponse::ExecutionStarted,
        611 + offset,
    );
    let promoted = accept(
        source,
        &run,
        &digest,
        &tool,
        parent_conversation_id,
        2,
        HostResponse::TaskPromoted {
            task_id: task_id.into(),
            input_watermark,
            kind: TaskKind::Agent,
            state: TaskState::Queued,
            progress: None,
            current_step: Some("Queued".into()),
            lineage: TaskLineage {
                resumed_from_task_id: predecessor_task_id,
                child_session_id: Some(SessionId::from_string(child_conversation_id.into())),
                parent_call_id: Some(nola_agent_engine::agent::CallId::from_string(
                    parent_call_id.into(),
                )),
                ..Default::default()
            },
        },
        612 + offset,
    );
    let cursor = promoted
        .events
        .iter()
        .find_map(|e| match &e.event {
            AgentEvent::TaskUpdated { task, .. } if task.id == task_id => Some(e.cursor.sequence),
            _ => None,
        })
        .unwrap();
    serde_json::from_value(
        source
            .execute(ApplicationCommand::ExportAgentTaskAdmission {
                conversation_id: parent_conversation_id.into(),
                run_id: run,
                task_id: task_id.into(),
                event_cursor: cursor,
            })
            .unwrap(),
    )
    .unwrap()
}

fn accept(
    source: &mut super::ApplicationRuntime,
    run: &str,
    digest: &str,
    request: &HostRequestEnvelope,
    conversation_id: &str,
    sequence: u64,
    payload: HostResponse,
    at: i64,
) -> nola_application::AcceptedHostResponse {
    serde_json::from_value(
        source
            .execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                input: nola_application::ReduceAndAcceptHostResponse {
                    conversation_id: conversation_id.into(),
                    run_id: run.into(),
                    invocation_config_digest: digest.into(),
                    received_at_ms: at,
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
                        sequence,
                        payload,
                    },
                },
            })
            .unwrap(),
    )
    .unwrap()
}

/// Import only evidence exported by the actual source transaction; an already
/// prepared target draft supplies scheduling, never source authorization.
pub fn admit_existing(
    path: &std::path::Path,
    task_id: &str,
    parent_conversation_id: &str,
    child_conversation_id: &str,
    parent_call_id: &str,
    input_watermark: Option<i64>,
) -> nola_domain::agent_task_admission::AgentTaskAdmission {
    let source_path = tempfile::tempdir().unwrap();
    let mut source = super::open_runtime(source_path.path().join("source.sqlite")).unwrap();
    let fact = source_fact_for(
        &mut source,
        task_id,
        parent_conversation_id,
        child_conversation_id,
        parent_call_id,
        input_watermark,
    );
    let mut target = super::open_runtime(path).unwrap();
    let view: nola_domain::agent_task_admission::RuntimeAgentTaskAuthority =
        serde_json::from_value(
            target
                .execute(ApplicationCommand::ImportHostAgentTaskAdmission {
                    admission: fact.clone(),
                })
                .unwrap(),
        )
        .unwrap();
    view.validate().unwrap();
    assert_eq!(view.admission.as_ref(), Some(&fact));
    fact
}

pub fn admitted(
    path: &std::path::Path,
    budget: Option<nola_application::RuntimeTaskResourceBudget>,
) -> nola_domain::agent_task_admission::AgentTaskAdmission {
    let mut target = super::open_runtime(path).unwrap();
    draft(&mut target, budget);
    let source_path = tempfile::tempdir().unwrap();
    let mut source = super::open_runtime(source_path.path().join("source.sqlite")).unwrap();
    let fact = source_fact(&mut source);
    let view: nola_domain::agent_task_admission::RuntimeAgentTaskAuthority =
        serde_json::from_value(
            target
                .execute(ApplicationCommand::ImportHostAgentTaskAdmission {
                    admission: fact.clone(),
                })
                .unwrap(),
        )
        .unwrap();
    assert_eq!(view.admission.as_ref(), Some(&fact));
    fact
}
