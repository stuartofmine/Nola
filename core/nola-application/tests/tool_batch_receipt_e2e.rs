// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    begin_assistant_turn, claim_committed_host_requests, load_committed_host_requests,
    ApplicationCommand, BeginAssistantTurn, ClaimCommittedHostRequests, ExecuteReceivedCommand,
    ReduceAndAcceptHostResponse,
};
use nola_application::{
    PermissionIntervention, PermissionRequest, PermissionRequestCommand,
    PermissionRequestResolution, PermissionRequestStatus, RuntimeTask, RuntimeTaskPriority,
    RuntimeTaskResourceBudget, RuntimeTaskResourceUsage, RuntimeTaskScheduling,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use nola_store::NolaStore;
use serde_json::json;
use std::collections::BTreeMap;

fn assert_pending_logical_request(store: &NolaStore, run: &str, request: &HostRequestEnvelope) {
    let mut logical = request.clone();
    // Physical worker leases live in their own durable delivery ledger.
    logical.delivery = None;
    assert_eq!(serde_json::to_value(load_committed_host_requests(store, run).unwrap().requests).unwrap(), json!([logical]));
}

fn delivery_before(store: &NolaStore, request: &HostRequestEnvelope) -> nola_domain::EffectDeliveryAttempt {
    let lease = request.delivery.as_ref().expect("fixture has a real claimed delivery");
    let attempt = store.effect_delivery_attempt(&lease.attempt_id).unwrap().unwrap();
    assert_eq!(attempt.worker_generation.as_deref(), Some(lease.worker_generation.as_str()));
    assert_eq!(attempt.lease_owner.as_deref(), Some(lease.lease_owner.as_str()));
    assert_eq!(attempt.lease_token.as_deref(), Some(lease.lease_token.as_str()));
    assert_eq!(attempt.lease_expires_at_ms, Some(lease.lease_expires_at_ms as i64));
    attempt
}

fn assert_recovered_delivery(store: &NolaStore, before: &nola_domain::EffectDeliveryAttempt) {
    use nola_domain::{EffectDeliveryAttemptState as State, EffectDeliveryRecoveryPlan as Plan, EffectDeliveryReplaySafety as Safety};
    let mut expected = before.clone();
    if matches!(before.state, State::Leased | State::ExecutionStarted) {
        expected.state = State::Interrupted;
        expected.recovery_plan = Some(if before.state == State::Leased || before.replay_safety == Safety::Idempotent { Plan::Retry } else { Plan::Resume });
        assert_eq!(before.state, State::ExecutionStarted);
        expected.interruption_reason = Some(if before.replay_safety == Safety::Idempotent {
            "worker disappeared after idempotent execution started"
        } else { "worker disappeared after recoverable execution started" }.into());
        expected.finished_at_ms = Some(4000);
        expected.revision += 1;
    }
    assert_eq!(store.effect_delivery_attempt(&before.id).unwrap().unwrap(), expected);
}
fn runtime_capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type": "object"}),
        input_normalization: HostInputNormalization {
            permission_contract:nola_domain::permission_resource_plan::PermissionOperationContract {
                risk:if matches!(id,"test.read"|"test.serial") {nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead}
                    else {nola_domain::permission_resource_plan::PermissionOperationRisk::Unknown},..Default::default()
            },..Default::default()
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

fn runtime_config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert(
        "model.stream".to_string(),
        runtime_capability("model.stream", HostCapabilityRoute::Model),
    );
    AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string("recovery-conversation".to_string())),
        model: Some("test-model".to_string()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("recovery-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
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
fn admit_sampling(
    store: &mut NolaStore,
    run: &str,
    digest: &str,
    request: &HostRequestEnvelope,
) -> HostRequestEnvelope {
    admit_sampling_at(store, run, digest, request, 1_100)
}

fn admit_sampling_at(
    store: &mut NolaStore,
    run: &str,
    digest: &str,
    request: &HostRequestEnvelope,
    claimed_at_ms: i64,
) -> HostRequestEnvelope {
    assert!(!request.attempt_started);
    let claimed = claim_committed_host_requests(
        store,
        ClaimCommittedHostRequests {
            run_id: run.into(),
            worker_generation: format!("generation-{}", request.effect_id.0),
            lease_owner: format!("runtime-{}", request.effect_id.0),
            claimed_at_ms,
            lease_expires_at_ms: claimed_at_ms + 60_000,
            limit: 1,
        },
    )
    .unwrap();
    let mut request = claimed
        .requests
        .into_iter()
        .find(|r| r.effect_id == request.effect_id)
        .unwrap();
    nola_application::reduce_and_accept_host_response(
        store,
        ReduceAndAcceptHostResponse {
            conversation_id: request.session_id.clone(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(&request, 1, HostResponse::ExecutionStarted),
            received_at_ms: claimed_at_ms + 100,
        },
    )
    .unwrap();
    request.resume_after_sequence = Some(1);
    nola_application::reduce_and_accept_host_response(
        store,
        ReduceAndAcceptHostResponse {
            conversation_id: request.session_id.clone(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(
                &request,
                2,
                HostResponse::ModelAttemptStarted {
                    provider: "fixture-provider".into(),
                    model: "test-model".into(),
                    reason: "initial".into(),
                },
            ),
            received_at_ms: claimed_at_ms + 400,
        },
    )
    .unwrap();
    let mut admitted = load_committed_host_requests(store, run)
        .unwrap()
        .requests
        .into_iter()
        .find(|r| r.effect_id == request.effect_id)
        .unwrap();
    admitted.delivery = request.delivery;
    assert!(admitted.attempt_started);
    assert_eq!(admitted.attempt_id, request.attempt_id);
    assert_eq!(admitted.resume_after_sequence, Some(2));
    admitted
}

fn claim_and_start(
    store: &mut NolaStore,
    run: &str,
    digest: &str,
    request: &HostRequestEnvelope,
    now_ms: i64,
) -> HostRequestEnvelope {
    let claimed = claim_committed_host_requests(
        store,
        ClaimCommittedHostRequests {
            run_id: run.into(),
            worker_generation: format!("generation-{now_ms}-{}", request.request_id),
            lease_owner: format!("runtime-{now_ms}-{}", request.request_id),
            claimed_at_ms: now_ms,
            lease_expires_at_ms: now_ms + 60_000,
            limit: 1,
        },
    )
    .unwrap();
    let mut claimed = claimed.requests.into_iter().next().unwrap();
    assert_eq!(claimed.effect_id, request.effect_id);
    let start_sequence = claimed
        .resume_after_sequence
        .map_or(1, |sequence| sequence + 1);
    nola_application::reduce_and_accept_host_response(
        store,
        ReduceAndAcceptHostResponse {
            conversation_id: request.session_id.clone(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(
                &claimed,
                start_sequence,
                HostResponse::ExecutionStarted,
            ),
            received_at_ms: now_ms + 1,
        },
    )
    .unwrap();
    claimed.resume_after_sequence = Some(start_sequence);
    claimed
}

fn tools(store: &mut NolaStore, run: &str) -> Vec<HostRequestEnvelope> {
    load_committed_host_requests(store, run)
        .unwrap()
        .requests
        .into_iter()
        .filter(|request| {
            matches!(
                request.payload,
                HostRequest::ExecuteTool { .. } | HostRequest::RequestPermission { .. }
            )
        })
        .collect()
}
fn call_ids(requests: &[HostRequestEnvelope]) -> Vec<String> {
    requests
        .iter()
        .filter_map(|request| match &request.payload {
            HostRequest::ExecuteTool { call_id, .. } => Some(call_id.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn terminal_sqlite_checkpoint_keeps_invocation_but_no_tool_catalog_payload() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("terminal-tool-history.db");
    let mut store = NolaStore::open(&database).unwrap();
    let mut config = runtime_config();
    config.session_id = Some(SessionId::from_string("terminal-tool-history".into()));
    config.tools = [
        json!({"type":"function","function":{"name":"test.read","description":"CALLED_SCHEMA_SENTINEL","parameters":{"type":"object"}}}),
        json!({"type":"function","function":{"name":"test.unused","description":"UNUSED_SCHEMA_SENTINEL","parameters":{"type":"object"}}}),
    ]
    .into_iter()
    .collect();
    for capability_id in ["test.read", "test.unused"] {
        let mut descriptor = runtime_capability(capability_id, HostCapabilityRoute::Tool);
        descriptor.exposure = HostCapabilityExposure::Visible;
        descriptor.title = format!("DESCRIPTOR_{capability_id}");
        config
            .host_catalog
            .as_mut()
            .unwrap()
            .capabilities
            .insert(capability_id.into(), descriptor);
    }
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "terminal-tool-history".into(),
            conversation_title: "Tool history".into(),
            user_message_id: "history:user".into(),
            user_part_id: "history:user:body".into(),
            user_text: "Read one file".into(),
            user_content_json: None,
            assistant_message_id: "history:assistant".into(),
            context_checkpoint_id: None,
            core_config: config,
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("history:user".into()),
                    "Read one file",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("history:submission".into()),
                submitted_at_epoch_ms: Some(1_000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let model = load_committed_host_requests(&mut store, &begun.begin.run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    let model = admit_sampling(
        &mut store,
        &begun.begin.run_id,
        &begun.invocation_config_digest,
        &model,
    );
    nola_application::reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "terminal-tool-history".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response_for_request(
                &model,
                3,
                HostResponse::ModelCompleted {
                    items: vec![],
                    tool_calls: vec![json!({
                        "call_id":"history:call-1",
                        "name":"test.read",
                        "arguments":{"path":"README.md"}
                    })],
                    usage: json!({}),
                },
            ),
            received_at_ms: 2_000,
        },
    )
    .unwrap();
    let tool = tools(&mut store, &begun.begin.run_id).pop().unwrap();
    let tool = claim_and_start(
        &mut store,
        &begun.begin.run_id,
        &begun.invocation_config_digest,
        &tool,
        2_100,
    );
    nola_application::reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "terminal-tool-history".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response_for_request(
                &tool,
                2,
                HostResponse::ToolCompleted {
                    call_id: "history:call-1".into(),
                    output: json!({"content":"ACTUAL_TOOL_RESULT_SENTINEL"}),
                    outcome: HostToolOutcome::Succeeded,
                    activation: None,
                    full_output_ref: None,
                    artifacts: vec![],
                },
            ),
            received_at_ms: 2_200,
        },
    )
    .unwrap();
    let final_model = load_committed_host_requests(&mut store, &begun.begin.run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    let final_model = admit_sampling_at(
        &mut store,
        &begun.begin.run_id,
        &begun.invocation_config_digest,
        &final_model,
        2_300,
    );
    nola_application::reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "terminal-tool-history".into(),
            run_id: begun.begin.run_id,
            invocation_config_digest: begun.invocation_config_digest,
            response: response_for_request(
                &final_model,
                3,
                HostResponse::ModelCompleted {
                    items: vec![serde_json::to_value(AgentItem::assistant_message(
                        ItemId::from_string("history:answer".into()),
                        "Done",
                    ))
                    .unwrap()],
                    tool_calls: vec![],
                    usage: json!({}),
                },
            ),
            received_at_ms: 3_000,
        },
    )
    .unwrap();
    drop(store);

    let reopened = rusqlite::Connection::open(&database).unwrap();
    let checkpoint_json: String = reopened
        .query_row(
            "SELECT checkpoint_json FROM core_checkpoint_bodies WHERE conversation_id='terminal-tool-history'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!checkpoint_json.contains("CALLED_SCHEMA_SENTINEL"));
    assert!(!checkpoint_json.contains("UNUSED_SCHEMA_SENTINEL"));
    assert!(!checkpoint_json.contains("DESCRIPTOR_test.unused"));
    assert!(checkpoint_json.contains("history:call-1"));
    assert!(checkpoint_json.contains("test.read"));
    assert!(checkpoint_json.contains("ACTUAL_TOOL_RESULT_SENTINEL"));
    assert_eq!(
        reopened
            .query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM message_parts WHERE kind='tool' AND content_json LIKE '%history:call-1%'",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        1
    );
}

fn scenario(serial: bool, permission: bool) {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("tools.db");
    let mut store = NolaStore::open(&database).unwrap();
    let mut config = runtime_config();
    config.tools = ["test.read", "test.serial"].into_iter().map(|name|json!({"type":"function","function":{"name":name,"description":"fixture tool","parameters":{"type":"object"}}})).collect();
    let mut descriptor = runtime_capability("test.read", HostCapabilityRoute::Tool);
    descriptor.exposure = HostCapabilityExposure::Visible;
    descriptor.parallelism = HostCapabilityParallelism::ConcurrentSafe;
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .insert(descriptor.id.clone(), descriptor);
    let mut descriptor = runtime_capability("test.serial", HostCapabilityRoute::Tool);
    if permission {
        descriptor.permission_domain = Some("filesystem.write".into());
        descriptor.input_normalization.permission_contract.risk = nola_domain::permission_resource_plan::PermissionOperationRisk::Overwrite;
    }
    descriptor.exposure = HostCapabilityExposure::Visible;
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .insert(descriptor.id.clone(), descriptor);
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "recovery-conversation".into(),
            conversation_title: "Tools".into(),
            user_message_id: "user".into(),
            user_part_id: "user:body".into(),
            user_text: "Run tools".into(),
            user_content_json: None,
            assistant_message_id: "submission:assistant".into(),
            context_checkpoint_id: None,
            core_config: config,
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("user".into()),
                    "Run tools",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("submission".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        },
    )
    .unwrap();
    let request = load_committed_host_requests(&mut store, &begun.begin.run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|r| matches!(r.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    let request = admit_sampling(
        &mut store,
        &begun.begin.run_id,
        &begun.invocation_config_digest,
        &request,
    );
    let calls=(0..5).map(|index|json!({"call_id":format!("call-{index}"),"name":if serial&&index==2{"test.serial"}else{"test.read"},"arguments":{"path":format!("/workspace/output-{index}.txt")}})).collect();
    let response = response_for_request(
        &request,
        3,
        HostResponse::ModelCompleted {
            items: vec![],
            tool_calls: calls,
            usage: json!({}),
        },
    );
    let command = ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "recovery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response,
            received_at_ms: 2000,
        },
    };
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = || ExecuteReceivedCommand {
        client_id: "batch-client".into(),
        request_id: "model-complete".into(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "2000".into(),
    };
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let db = rusqlite::Connection::open(&database).unwrap();
    let effects = || {
        let mut query = db
            .prepare("SELECT effect_id,json_array(binding_json,payload_json),status FROM effect_outbox JOIN effect_outbox_payloads USING(effect_id) ORDER BY effect_id")
            .unwrap();
        query
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let baseline = effects();
    db.execute_batch("CREATE TRIGGER fail_batch_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'batch receipt failed'); END").unwrap();
    assert!(runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .is_err());
    assert_eq!(effects(), baseline);
    db.execute_batch("DROP TRIGGER fail_batch_receipt").unwrap();
    let first = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(
        matches!(
            &first.outcome,
            nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
        ),
        "{:?}",
        first.outcome
    );
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert_eq!(
        runtime
            .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
            .unwrap()
            .outcome,
        first.outcome
    );
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    let batch = tools(&mut store, &begun.begin.run_id);
    assert_eq!(
        call_ids(&batch),
        if serial {
            vec!["call-0", "call-1"]
        } else {
            vec!["call-0", "call-1", "call-2", "call-3"]
        }
    );
    let identities = serde_json::to_value(&batch).unwrap();
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    assert_eq!(
        serde_json::to_value(tools(&mut store, &begun.begin.run_id)).unwrap(),
        identities
    );
    let batch_count = batch.len();
    for (index, request) in batch.into_iter().enumerate() {
        let request = claim_and_start(
            &mut store,
            &begun.begin.run_id,
            &begun.invocation_config_digest,
            &request,
            2_100 + index as i64 * 10,
        );
        let HostRequest::ExecuteTool { call_id, .. } = &request.payload else {
            unreachable!()
        };
        nola_application::reduce_and_accept_host_response(
            &mut store,
            ReduceAndAcceptHostResponse {
                conversation_id: "recovery-conversation".into(),
                run_id: begun.begin.run_id.clone(),
                invocation_config_digest: begun.invocation_config_digest.clone(),
                response: response_for_request(
                    &request,
                    2,
                    HostResponse::ToolCompleted {
                        call_id: call_id.clone(),
                        output: json!({"ok":true}),
                        outcome: HostToolOutcome::Succeeded,
                        activation: None,
                        full_output_ref: None,
                        artifacts: vec![],
                    },
                ),
                received_at_ms: 3000,
            },
        )
        .unwrap();
        if index + 1 < batch_count {
            let pending = call_ids(&tools(&mut store, &begun.begin.run_id));
            assert!(
                !pending.contains(&if serial {
                    "call-2".into()
                } else {
                    "call-4".into()
                }),
                "running batch remains a barrier"
            );
        }
    }
    let next = tools(&mut store, &begun.begin.run_id);
    if permission {
        assert_eq!(next.len(), 1);
        assert!(matches!(
            next[0].payload,
            HostRequest::RequestPermission { .. }
        ));
        assert!(
            call_ids(&next).is_empty(),
            "unresolved permission cannot publish later tools"
        );
        let pinned = serde_json::to_value(&next).unwrap();
        drop(store);
        let mut store = NolaStore::open(&database).unwrap();
        assert_eq!(
            serde_json::to_value(tools(&mut store, &begun.begin.run_id)).unwrap(),
            pinned
        );
    } else {
        assert_eq!(
            call_ids(&next),
            if serial {
                vec!["call-2"]
            } else {
                vec!["call-4"]
            }
        );
    }
}
#[test]
fn concurrent_batch_is_four_then_one_across_receipt_and_restart() {
    scenario(false, false)
}
#[test]
fn serial_tool_is_a_committed_ordering_barrier() {
    scenario(true, false)
}

#[test]
fn unresolved_permission_blocks_later_tools_after_restart() {
    scenario(true, true)
}

fn permission_setup(
    database: &std::path::Path,
    conversation: &str,
) -> (String, String, HostRequestEnvelope) {
    permission_setup_with_context(database, conversation, None)
}

fn permission_setup_with_context(
    database: &std::path::Path,
    conversation: &str,
    context: Option<serde_json::Value>,
) -> (String, String, HostRequestEnvelope) {
    let mut store = NolaStore::open(database).unwrap();
    let mut config = runtime_config();
    config.session_id = Some(SessionId::from_string(conversation.into()));
    config.host_catalog.as_mut().unwrap().capabilities.insert(
        "permission.ask".into(),
        runtime_capability("permission.ask", HostCapabilityRoute::Permission),
    );
    config.tools = ["test.read", "test.serial"].into_iter().map(|name|json!({"type":"function","function":{"name":name,"description":"fixture tool","parameters":{"type":"object"}}})).collect();
    let mut descriptor = runtime_capability("test.read", HostCapabilityRoute::Tool);
    descriptor.exposure = HostCapabilityExposure::Visible;
    descriptor.parallelism = HostCapabilityParallelism::ConcurrentSafe;
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .insert(descriptor.id.clone(), descriptor);
    let mut descriptor = runtime_capability("test.serial", HostCapabilityRoute::Tool);
    if true {
        descriptor.permission_domain = Some("filesystem.write".into());
        descriptor.input_normalization.permission_contract.risk = nola_domain::permission_resource_plan::PermissionOperationRisk::Overwrite;
        descriptor.input_normalization.permission_resource =
            HostPermissionResourceDeclaration::FirstOf {
                keys: vec!["path".into()],
            };
    }
    descriptor.exposure = HostCapabilityExposure::Visible;
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .insert(descriptor.id.clone(), descriptor);
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: conversation.into(),
            conversation_title: "Tools".into(),
            user_message_id: format!("{conversation}:user"),
            user_part_id: format!("{conversation}:user:body"),
            user_text: "Run tools".into(),
            user_content_json: None,
            assistant_message_id: format!("{conversation}:submission:assistant"),
            context_checkpoint_id: None,
            core_config: config,
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string(format!("{conversation}:user")),
                    "Run tools",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some(format!("{conversation}:submission")),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        },
    )
    .unwrap();
    let request = load_committed_host_requests(&mut store, &begun.begin.run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|r| matches!(r.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    let request = admit_sampling(
        &mut store,
        &begun.begin.run_id,
        &begun.invocation_config_digest,
        &request,
    );

    let calls=(0..2).map(|index| {
        let mut arguments = json!({"path":"/workspace/same.txt","content":format!("version-{index}")});
        if let Some(context) = context.as_ref() { arguments["_nolaExecution"] = context.clone(); }
        json!({"call_id":format!("{conversation}:call-{index}"),"name":"test.serial","arguments":arguments})
    }).collect();
    nola_application::reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: conversation.into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response_for_request(
                &request,
                3,
                HostResponse::ModelCompleted {
                    items: vec![],
                    tool_calls: calls,
                    usage: json!({}),
                },
            ),
            received_at_ms: 2000,
        },
    )
    .unwrap();
    let pending = tools(&mut store, &begun.begin.run_id);
    assert_eq!(pending.len(), 1);
    assert!(matches!(
        pending[0].payload,
        HostRequest::RequestPermission { .. }
    ));
    let permission = claim_and_start(
        &mut store,
        &begun.begin.run_id,
        &begun.invocation_config_digest,
        &pending[0],
        2_100,
    );
    (
        begun.begin.run_id,
        begun.invocation_config_digest,
        permission,
    )
}
fn permission_command(
    conversation: &str,
    run: &str,
    digest: &str,
    request: &HostRequestEnvelope,
    resolution: HostPermissionResolution,
) -> ApplicationCommand {
    let HostRequest::RequestPermission {
        permission_request_id,
        ..
    } = &request.payload
    else {
        panic!("permission request expected")
    };
    ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: conversation.into(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(
                request,
                request
                    .resume_after_sequence
                    .map_or(1, |sequence| sequence + 1),
                HostResponse::PermissionResolved {
                    request_id: permission_request_id.clone(),
                    resolution,
                },
            ),
            received_at_ms: 3000,
        },
    }
}
fn permission_scenario(conversation_grant: bool) {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("permission.db");
    let (run, digest, request) = permission_setup(&database, "permission-conversation");
    let before_delivery = delivery_before(&NolaStore::open(&database).unwrap(), &request);
    let command = permission_command(
        "permission-conversation",
        &run,
        &digest,
        &request,
        if conversation_grant {
            HostPermissionResolution::allow_conversation(None)
        } else {
            HostPermissionResolution::allow_once()
        },
    );
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = || ExecuteReceivedCommand {
        client_id: "permission-test".into(),
        request_id: "allow".into(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "3000".into(),
    };
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute_batch("CREATE TRIGGER fail_permission_publication BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'permission receipt fault'); END").unwrap();
    let settlements = || {
        let mut statement = db.prepare("SELECT payload_json FROM run_events WHERE run_id=?1 AND event_kind='permission_settled' ORDER BY sequence").unwrap();
        statement.query_map([&run], |row| row.get::<_, String>(0)).unwrap()
            .map(|row| serde_json::from_str::<serde_json::Value>(&row.unwrap()).unwrap())
            .collect::<Vec<_>>()
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .is_err());
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    assert_pending_logical_request(&store, &run, &request);
    assert_eq!(store.effect_delivery_attempt(&before_delivery.id).unwrap().unwrap(), before_delivery);
    assert!(settlements().is_empty(), "failed command cannot publish settlement without its checkpoint");
    drop(store);
    db.execute_batch("DROP TRIGGER fail_permission_publication")
        .unwrap();
    let witness: String = db.query_row(
        "SELECT witness_json FROM core_delivery_witnesses WHERE conversation_id='permission-conversation'",
        [], |row| row.get(0)).unwrap();
    for field in ["call_id", "turn_id"] {
        let mut altered: serde_json::Value = serde_json::from_str(&witness).unwrap();
        altered["state"]["execution"]["permissions"]["pending"][0][field] = json!("wrong-owner");
        db.execute("UPDATE core_delivery_witnesses SET witness_json=?1 WHERE conversation_id='permission-conversation'",
            [altered.to_string()]).unwrap();
        let mut rejected_runtime = support::open_runtime(&database).unwrap();
        let mut rejected_command = receipt();
        rejected_command.request_id = format!("rejected-{field}");
        let rejected = rejected_runtime.execute_received_command(rejected_command, serde_json::from_str(&command_json).unwrap())
            .unwrap();
        assert!(matches!(&rejected.outcome, nola_application::ApplicationCommandReceiptOutcome::Rejected { message, .. }
            if message.contains("permission settlement lacks its exact original effect")), "{rejected:?}");
        drop(rejected_runtime);
        assert!(settlements().is_empty());
        let untouched = NolaStore::open(&database).unwrap();
        assert_pending_logical_request(&untouched, &run, &request);
        assert_eq!(untouched.effect_delivery_attempt(&before_delivery.id).unwrap().unwrap(), before_delivery);
        db.execute("UPDATE core_delivery_witnesses SET witness_json=?1 WHERE conversation_id='permission-conversation'",
            [&witness]).unwrap();
    }
    let mut runtime = support::open_runtime(&database).unwrap();
    let first = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    let committed = settlements();
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0]["receipt"]["outcome"]["type"], "authorization");
    assert_eq!(committed[0]["core_envelope"]["session_id"], "permission-conversation");
    assert_eq!(committed[0]["core_envelope"]["turn_id"], committed[0]["receipt"]["turn_id"]);
    assert!(committed[0]["core_envelope"]["cursor"]["sequence"].as_u64().unwrap() > 0);
    assert!(committed[0]["core_envelope"]["revision"].as_u64().unwrap() > 0);
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(first.outcome, replay.outcome);
    assert_eq!(settlements(), committed, "cold replay must not append another settlement");
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    let requests = tools(&mut store, &run);
    assert_eq!(call_ids(&requests), vec!["permission-conversation:call-0"]);
    let execute = claim_and_start(&mut store, &run, &digest, &requests[0], 3_100);
    let HostRequest::ExecuteTool { call_id, .. } = &execute.payload else {
        unreachable!()
    };
    nola_application::reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "permission-conversation".into(),
            run_id: run.clone(),
            invocation_config_digest: digest,
            response: response_for_request(
                &execute,
                2,
                HostResponse::ToolCompleted {
                    call_id: call_id.clone(),
                    output: json!({"ok":true}),
                    outcome: HostToolOutcome::Succeeded,
                    activation: None,
                    full_output_ref: None,
                    artifacts: vec![],
                },
            ),
            received_at_ms: 4000,
        },
    )
    .unwrap();
    let next = tools(&mut store, &run);
    assert_eq!(next.len(), 1);
    if conversation_grant {
        assert_eq!(
            call_ids(&next),
            vec!["permission-conversation:call-1"],
            "initial {:?} next {:?}",
            request.payload,
            next[0].payload
        );
    } else {
        assert!(matches!(
            next[0].payload,
            HostRequest::RequestPermission { .. }
        ));
    }
    drop(store);
    // A fresh conversation must still ask despite the existing conversation's grant.
    permission_setup(&database, "unrelated-conversation");
}
#[test]
fn permission_once_receipt_rollback_and_replay_never_authorizes_second_call() {
    permission_scenario(false)
}
#[test]
fn conversation_permission_survives_cold_restart_but_not_conversation_change() {
    permission_scenario(true)
}

#[test]
fn permission_instruction_preserves_call_until_explicit_approval_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("instruction.db");
    let (run, digest, request) = permission_setup(&database, "instruction-conversation");
    let HostRequest::RequestPermission {
        input: original_arguments,
        permission_request_id: original_permission_id,
        ..
    } = &request.payload
    else {
        panic!("initial permission required")
    };
    // Task permission rows are Host response transport, not Core tool-call state.
    let mut transport = support::open_runtime(&database).unwrap();
    task_permission_submit(
        &mut transport,
        ApplicationCommand::RuntimeTask {
            command: nola_application::RuntimeTaskCommand::Create {
                input: nola_application::CreateRuntimeTask::try_from(permission_parent_task())
                    .unwrap(),
            },
        },
        "create-task",
    );
    let transport_input =
        |id: &str, core_id: &str, history| nola_application::NewPermissionRequest {
            core_permission_request_id: Some(core_id.into()),
            permission_instructions: history,
            request_id: id.into(),
            task_id: "task-1".into(),
            child_session_id: "instruction-conversation".into(),
            capability: "test.serial".into(),
            input: original_arguments.clone(),
            scope_rule: "filesystem.write:/workspace/same.txt".into(),
            created_at_ms: 2000,
        };
    task_permission_submit(
        &mut transport,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Enqueue {
                input: transport_input("transport-original", original_permission_id, Vec::new()),
            },
        },
        "enqueue-original",
    );
    let transport_intervention = || ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::Intervene {
            request_id: "transport-original".into(),
            intervention: PermissionIntervention::ProvideInstruction {
                instruction: "Preserve the existing formatting when approved".into(),
            },
            resolved_at_ms: 3000,
        },
    };
    assert_eq!(
        task_permission_submit(
            &mut transport,
            transport_intervention(),
            "transport-instruction"
        )["type"],
        json!("resolved")
    );
    let delivered: PermissionRequest = serde_json::from_value(
        transport
            .execute(ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Get {
                    request_id: "transport-original".into(),
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(delivered.status, PermissionRequestStatus::Resolved);
    let Some(PermissionRequestResolution::Intervention {
        intervention:
            PermissionIntervention::ProvideInstruction {
                instruction: delivered_instruction,
            },
    }) = delivered.resolution
    else {
        panic!("Host poll must deliver typed custom response")
    };
    drop(transport);
    let db = rusqlite::Connection::open(&database).unwrap();
    let grants = || {
        db.query_row("SELECT COUNT(*) FROM permission_grants", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
    };
    let before_grants = grants();
    let mut store = NolaStore::open(&database).unwrap();
    let before_checkpoint = store
        .load_checkpoint("instruction-conversation")
        .unwrap()
        .unwrap();
    let before_outbox = serde_json::to_value(
        load_committed_host_requests(&mut store, &run)
            .unwrap()
            .requests,
    )
    .unwrap();
    drop(store);
    let command = permission_command(
        "instruction-conversation",
        &run,
        &digest,
        &request,
        HostPermissionResolution::provide_instruction(delivered_instruction),
    );
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = || ExecuteReceivedCommand {
        client_id: "permission-test".into(),
        request_id: "instruction".into(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "3000".into(),
    };
    db.execute_batch("CREATE TRIGGER fail_instruction_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'instruction receipt fault'); END").unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .is_err());
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    assert_eq!(
        store
            .load_checkpoint("instruction-conversation")
            .unwrap()
            .unwrap(),
        before_checkpoint
    );
    assert_eq!(
        serde_json::to_value(
            load_committed_host_requests(&mut store, &run)
                .unwrap()
                .requests
        )
        .unwrap(),
        before_outbox
    );
    assert_eq!(grants(), before_grants);
    drop(store);
    db.execute_batch("DROP TRIGGER fail_instruction_receipt")
        .unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    let first = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(matches!(
        &first.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    let waiting = load_committed_host_requests(&mut store, &run)
        .unwrap()
        .requests;
    assert_eq!(
        waiting.len(),
        1,
        "instruction only publishes the successor approval request"
    );
    let HostRequest::RequestPermission {
        permission_request_id,
        input,
        ..
    } = &waiting[0].payload
    else {
        panic!("instruction must stay suspended, never execute or request a model")
    };
    assert_ne!(permission_request_id, original_permission_id);
    let expected_instructions = json!([{"permission_request_id":original_permission_id,"instruction":"Preserve the existing formatting when approved"}]);
    assert_eq!(
        serde_json::to_value(&waiting[0].payload).unwrap()["permission_instructions"],
        expected_instructions
    );
    assert_ne!(waiting[0].effect_id, request.effect_id);
    assert_eq!(
        input, original_arguments,
        "custom instruction cannot rewrite executable arguments"
    );
    assert_eq!(grants(), before_grants);
    let mut transport = support::open_runtime(&database).unwrap();
    task_permission_submit(
        &mut transport,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Enqueue {
                input: transport_input(
                    "transport-successor",
                    permission_request_id,
                    serde_json::from_value(expected_instructions.clone()).unwrap(),
                ),
            },
        },
        "enqueue-successor",
    );
    // Replaying delivery of the old transport response must not resolve its successor.
    assert_eq!(
        task_permission_submit(
            &mut transport,
            transport_intervention(),
            "transport-instruction"
        )["type"],
        json!("resolved")
    );
    let successor: PermissionRequest = serde_json::from_value(
        transport
            .execute(ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::PendingForTask {
                    task_id: "task-1".into(),
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(successor.request_id, "transport-successor");
    assert_eq!(successor.status, PermissionRequestStatus::Pending);
    assert!(successor.resolution.is_none());
    assert_eq!(successor.input, *original_arguments);
    drop(transport);
    let checkpoint = store
        .load_checkpoint("instruction-conversation")
        .unwrap()
        .unwrap();
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert_eq!(first.outcome, replay.outcome);
    assert!(replay.replayed);
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    assert_eq!(
        serde_json::to_value(
            load_committed_host_requests(&mut store, &run)
                .unwrap()
                .requests
        )
        .unwrap(),
        serde_json::to_value(&waiting).unwrap()
    );
    assert_eq!(
        store
            .load_checkpoint("instruction-conversation")
            .unwrap()
            .unwrap(),
        checkpoint
    );
    drop(store);
    // Only explicit approval of the successor resumes the original call.
    let mut transport = support::open_runtime(&database).unwrap();
    task_permission_submit(
        &mut transport,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Authorize {
                request_id: "transport-successor".into(),
                decision: nola_application::PermissionAuthorizationDecision::AllowOnce,
                resolved_at_ms: 4000,
            },
        },
        "transport-approve",
    );
    let approved_transport: PermissionRequest = serde_json::from_value(
        transport
            .execute(ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Get {
                    request_id: "transport-successor".into(),
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        approved_transport.resolution,
        Some(PermissionRequestResolution::Authorization {
            decision: nola_application::PermissionAuthorizationDecision::AllowOnce
        })
    ));
    drop(transport);
    let mut store = NolaStore::open(&database).unwrap();
    let waiting_request = claim_and_start(&mut store, &run, &digest, &waiting[0], 3_100);
    drop(store);
    let mut approval = permission_command(
        "instruction-conversation",
        &run,
        &digest,
        &waiting_request,
        HostPermissionResolution::allow_once(),
    );
    if let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut approval {
        input.received_at_ms = 4_000;
    }
    let approval_json = serde_json::to_string(&approval).unwrap();
    let approval_receipt = || ExecuteReceivedCommand {
        client_id: "permission-test".into(),
        request_id: "approve-after-instruction".into(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: approval_json.clone(),
        expected_revision: None,
        received_at: "4000".into(),
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    let approved = runtime
        .execute_received_command(
            approval_receipt(),
            serde_json::from_str(&approval_json).unwrap(),
        )
        .unwrap();
    assert!(
        matches!(
            approved.outcome,
            nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
        ),
        "{:?}",
        approved.outcome
    );
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let approval_replay = runtime
        .execute_received_command(
            approval_receipt(),
            serde_json::from_str(&approval_json).unwrap(),
        )
        .unwrap();
    assert!(approval_replay.replayed);
    assert_eq!(approval_replay.outcome, approved.outcome);
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    let resumed = load_committed_host_requests(&mut store, &run)
        .unwrap()
        .requests;
    assert_eq!(resumed.len(), 1);
    let resumed = claim_and_start(&mut store, &run, &digest, &resumed[0], 4_100);
    let HostRequest::ExecuteTool { call_id, input, .. } = &resumed.payload else {
        panic!("explicit approval must resume original tool")
    };
    assert_eq!(call_id, "instruction-conversation:call-0");
    assert_eq!(
        serde_json::to_value(&resumed.payload).unwrap()["permission_instructions"],
        expected_instructions
    );
    assert_eq!(input, original_arguments);
    assert_eq!(
        grants(),
        before_grants,
        "allow once cannot create reusable grant"
    );
    nola_application::reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "instruction-conversation".into(),
            run_id: run.clone(),
            invocation_config_digest: digest,
            response: response_for_request(
                &resumed,
                2,
                HostResponse::ToolCompleted {
                    call_id: call_id.clone(),
                    output: json!({"ok":true}),
                    outcome: HostToolOutcome::Succeeded,
                    activation: None,
                    full_output_ref: None,
                    artifacts: vec![],
                },
            ),
            received_at_ms: 5000,
        },
    )
    .unwrap();
    let sibling = tools(&mut store, &run);
    assert_eq!(sibling.len(), 1);
    let HostRequest::RequestPermission { input, .. } = &sibling[0].payload else {
        panic!("sibling requires separate explicit approval")
    };
    assert_eq!(input["content"], "version-1");
    assert!(call_ids(&sibling).is_empty());
}

fn permission_parent_task() -> RuntimeTask {
    let mut task = RuntimeTask {
        id: "task-1".into(),
        kind: "agent".into(),
        status: "queued".into(),
        title: "Permission owner".into(),
        project_root: "/workspace".into(),
        owner_session_id: Some("parent-session".into()),
        parent_task_id: None,
        parent_tool_call_id: None,
        child_session_id: Some("instruction-conversation".into()),
        agent_type: Some("general".into()),
        prompt: Some("write a report".into()),
        progress_summary: None,
        last_activity: None,
        result_summary: None,
        error: None,
        output_path: None,
        unread: false,
        cancel_requested: false,
        cancel_requested_at_ms: None,
        worker_id: None,
        ownership_token: None,
        lease_until_ms: None,
        heartbeat_at_ms: None,
        retry_count: 0,
        retry_at_ms: None,
        last_error: None,
        input_watermark: None,
        completion_watermark: None,
        created_at_ms: 1,
        started_at_ms: None,
        ended_at_ms: None,
        updated_at_ms: 1,
        metadata_json: "{}".into(),
    };
    task.set_scheduling(RuntimeTaskScheduling {
        priority: RuntimeTaskPriority::Normal,
        queued_at_ms: task.created_at_ms,
        resource_budget: RuntimeTaskResourceBudget {
            max_wall_time_ms: Some(60_000),
            max_output_bytes: Some(1_048_576),
            max_cpu_time_ms: None,
            max_memory_bytes: None,
            max_processes: None,
        },
        output_bytes: 0,
        resource_usage: RuntimeTaskResourceUsage::default(),
        budget_exhaustion: None,
    })
    .expect("valid parent runtime task scheduling");
    task
}

fn task_permission_submit(
    runtime: &mut support::ApplicationRuntime,
    command: ApplicationCommand,
    id: &str,
) -> serde_json::Value {
    let receipt = runtime
        .execute_received_command(
            ExecuteReceivedCommand {
                client_id: "task-permission-test".into(),
                request_id: id.into(),
                command_type: if matches!(&command, ApplicationCommand::RuntimeTask { .. }) {
                    "runtime_task".into()
                } else {
                    "permission_request".into()
                },
                command_json: serde_json::to_string(&command).unwrap(),
                expected_revision: None,
                received_at: "3000".into(),
            },
            command,
        )
        .unwrap();
    match receipt.outcome {
        nola_application::ApplicationCommandReceiptOutcome::Committed { result, .. } => result,
        other => panic!("transport command rejected: {other:?}"),
    }
}

#[test]
fn interrupted_permission_receipt_rolls_back_then_cold_replays_same_annotated_call() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("interrupted-permission.db");
    let conversation = "interrupted-permission";
    let (run, digest, original) = permission_setup(&database, conversation);
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(permission_command(
            conversation,
            &run,
            &digest,
            &original,
            HostPermissionResolution::provide_instruction("Keep original formatting"),
        ))
        .unwrap();
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    let request = load_committed_host_requests(&store, &run)
        .unwrap()
        .requests
        .remove(0);
    let request = claim_and_start(&mut store, &run, &digest, &request, 3_100);
    let HostRequest::RequestPermission {
        permission_instructions,
        ..
    } = &request.payload
    else {
        panic!("ask expected")
    };
    assert_eq!(permission_instructions.len(), 1);
    let before_checkpoint = store.load_checkpoint(conversation).unwrap().unwrap();
    let before_delivery = delivery_before(&store, &request);
    drop(store);
    let command = ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: conversation.into(),
            run_id: run.clone(),
            invocation_config_digest: digest.clone(),
            response: response_for_request(
                &request,
                2,
                HostResponse::PermissionTransportInterrupted {
                    reason: nola_host_protocol::PermissionTransportInterruptionReason::Timeout,
                },
            ),
            received_at_ms: 4000,
        },
    };
    let envelope = ExecuteReceivedCommand {
        client_id: "interruption-test".into(),
        request_id: "timeout".into(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: serde_json::to_string(&command).unwrap(),
        expected_revision: None,
        received_at: "4000".into(),
    };
    let inspect = rusqlite::Connection::open(&database).unwrap();
    inspect.execute_batch("CREATE TRIGGER fail_interrupt_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'interrupt receipt fault'); END").unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap()
        )
        .is_err());
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    assert_eq!(
        store
            .load_checkpoint(conversation)
            .unwrap()
            .unwrap()
            .checkpoint_json,
        before_checkpoint.checkpoint_json
    );
    assert_pending_logical_request(&store, &run, &request);
    assert_eq!(store.effect_delivery_attempt(&before_delivery.id).unwrap().unwrap(), before_delivery);
    drop(store);
    inspect
        .execute_batch("DROP TRIGGER fail_interrupt_receipt")
        .unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    let committed = runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap(),
        )
        .unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap(),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(ApplicationCommand::ReduceAndCommitActiveSubmission {
            input: nola_application::ReduceAndCommitActiveSubmission {
                conversation_id: conversation.into(),
                run_id: run.clone(),
                transition_id: "resume-interrupted-permission".into(),
                invocation_config_digest: digest.clone(),
                submission: AgentSubmission::ResumeHostEffect {
                    effect_id: request.effect_id.0.clone(),
                },
                committed_at_ms: 4_100,
            },
        })
        .unwrap();
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    let resumed = load_committed_host_requests(&store, &run).unwrap().requests;
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].effect_id, request.effect_id);
    assert_eq!(resumed[0].payload, request.payload);
    assert_eq!(resumed[0].turn_id, request.turn_id);
    assert_ne!(resumed[0].request_id, request.request_id);
    assert_eq!(resumed[0].resume_after_sequence, Some(2));
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    let resumed = claim_and_start(&mut store, &run, &digest, &resumed[0], 4_200);
    drop(store);
    let mut approval = permission_command(
        conversation,
        &run,
        &digest,
        &resumed,
        HostPermissionResolution::allow_once(),
    );
    if let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut approval {
        input.received_at_ms = 5000;
    }
    let approval_envelope = ExecuteReceivedCommand {
        request_id: "approve".into(),
        command_json: serde_json::to_string(&approval).unwrap(),
        ..envelope
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    let accepted = runtime
        .execute_received_command(
            approval_envelope.clone(),
            serde_json::from_str(&approval_envelope.command_json).unwrap(),
        )
        .unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let repeated = runtime
        .execute_received_command(
            approval_envelope.clone(),
            serde_json::from_str(&approval_envelope.command_json).unwrap(),
        )
        .unwrap();
    assert!(repeated.replayed);
    assert_eq!(accepted.outcome, repeated.outcome);
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    let ready = load_committed_host_requests(&store, &run).unwrap().requests;
    assert_eq!(ready.len(), 1);
    let HostRequest::ExecuteTool {
        input,
        permission_instructions: history,
        call_id,
        ..
    } = &ready[0].payload
    else {
        panic!("original call must resume")
    };
    let HostRequest::RequestPermission {
        input: original_input,
        ..
    } = &request.payload
    else {
        unreachable!()
    };
    assert_eq!(input, original_input);
    assert_eq!(history, permission_instructions);
    assert_eq!(call_id, "interrupted-permission:call-0");
}

#[test]
fn server_startup_preserves_only_consistent_canonical_permission_and_its_trace() {
    for accepted in [false, true] {
        for corruption in [
            None,
            Some("request_id"),
            Some("session_id"),
            Some("acknowledged"),
            Some("checkpoint"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let database = directory.path().join("startup.db");
            let (run, digest, mut request) = permission_setup(&database, "startup-permission");
            let before_delivery = delivery_before(&NolaStore::open(&database).unwrap(), &request);
            if accepted {
                let mut store = NolaStore::open(&database).unwrap();
                nola_application::reduce_and_accept_host_response(
                    &mut store,
                    ReduceAndAcceptHostResponse {
                        conversation_id: "startup-permission".into(),
                        run_id: run.clone(),
                        invocation_config_digest: digest,
                        response: response_for_request(&request, 2, HostResponse::Accepted),
                        received_at_ms: 3000,
                    },
                )
                .unwrap();
                request = load_committed_host_requests(&store, &run)
                    .unwrap()
                    .requests
                    .remove(0);
            }
            let connection = rusqlite::Connection::open(&database).unwrap();
            connection.execute("INSERT INTO trace_journal_runs(trace_id,run_id,conversation_id,turn_id,status,started_at,updated_at) VALUES('trace-current','different-trace-run-id','startup-permission',?1,'running','before','before')", [&run]).unwrap();
            connection.execute("INSERT INTO trace_journal_runs(trace_id,run_id,conversation_id,turn_id,status,started_at,updated_at) VALUES('trace-old','old-trace-run','startup-permission','old-canonical-turn','running','before','before')", []).unwrap();
            match corruption {
                Some("request_id") | Some("session_id") => {
                    let mut changed = serde_json::to_value(nola_store_port::host_replay::CoreHostBindingOverlay::from_request(&request)).unwrap();
                    changed[corruption.unwrap()] = json!("wrong-owner");
                    connection
                        .execute(
                            "UPDATE effect_outbox SET binding_json=?1 WHERE effect_id=?2",
                            rusqlite::params![
                                serde_json::to_string(&changed).unwrap(),
                                request.effect_id.0
                            ],
                        )
                        .unwrap();
                }
                Some("acknowledged") => {
                    connection
                        .execute(
                            "UPDATE effect_outbox SET status='acknowledged' WHERE effect_id=?1",
                            [&request.effect_id.0],
                        )
                        .unwrap();
                }
                Some("checkpoint") => {
                    connection
                        .execute("UPDATE core_checkpoint_bodies SET checkpoint_json='{}'", [])
                        .unwrap();
                }
                _ => {}
            }
            let before: Vec<(String, String, String)> =
                {
                    let mut q = connection
                .prepare("SELECT id,state,COALESCE(content_text,'') FROM message_parts ORDER BY id")
                .unwrap();
                    q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                        .unwrap()
                        .collect::<Result<_, _>>()
                        .unwrap()
                };
            let mut store = NolaStore::open(&database).unwrap();
            let all_rows = || {
                [
                    "runs",
                    "messages",
                    "message_parts",
                    "core_checkpoints",
                    "effect_outbox",
                    "trace_journal_runs",
                ]
                .into_iter()
                .map(|table| {
                    let mut query = connection
                        .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                        .unwrap();
                    let columns = query.column_count();
                    query
                        .query_map([], |row| {
                            Ok((0..columns)
                                .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                                .collect::<Vec<_>>()
                                .join("|"))
                        })
                        .unwrap()
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap()
                })
                .collect::<Vec<_>>()
            };
            let unchanged = all_rows();
            let result = nola_application::execute_application_command(
                &mut store,
                ApplicationCommand::RecoverOrphanedRuns {
                    conversation_id: None,
                    reason: "server_restarted".into(),
                    interrupted_at: "after".into(),
                    interrupted_at_ms: 4000,
                },
            );
            if corruption.is_some() {
                assert!(
                    result.is_err(),
                    "Inconsistent canonical recovery facts must fail closed"
                );
                assert_eq!(
                    all_rows(),
                    unchanged,
                    "Rejected recovery cannot alter canonical records or either trace"
                );
                continue;
            }
            result.unwrap();
            let status: String = connection
                .query_row("SELECT status FROM runs WHERE id=?1", [&run], |r| r.get(0))
                .unwrap();
            let trace: String = connection
                .query_row(
                    "SELECT status FROM trace_journal_runs WHERE trace_id='trace-current'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let old_trace: String = connection
                .query_row(
                    "SELECT status FROM trace_journal_runs WHERE trace_id='trace-old'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(old_trace, "interrupted");
            if corruption.is_none() {
                assert_eq!(status, "running");
                assert_eq!(trace, "running");
                let mut q = connection
                    .prepare(
                        "SELECT id,state,COALESCE(content_text,'') FROM message_parts ORDER BY id",
                    )
                    .unwrap();
                let after: Vec<(String, String, String)> = q
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                assert_eq!(after, before);
                assert_pending_logical_request(&store, &run, &request);
                assert_recovered_delivery(&store, &before_delivery);
            }
        }
    }
}

#[test]
fn startup_retains_sop_permission_with_separate_invocation_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("sop-startup.db");
    let (run, _, request) = permission_setup_with_context(
        &database,
        "sop-startup",
        Some(json!({"sopRunId":"sop-run","sopStepId":"step"})),
    );
    let mut store = NolaStore::open(&database).unwrap();
    let before_delivery = delivery_before(&store, &request);
    assert_eq!(
        nola_application::execute_application_command(
            &mut store,
            ApplicationCommand::RecoverOrphanedRuns {
                conversation_id: None,
                reason: "startup".into(),
                interrupted_at: "4000".into(),
                interrupted_at_ms: 4000,
            }
        )
        .unwrap(),
        json!(1)
    );
    assert_pending_logical_request(&store, &run, &request);
    assert_recovered_delivery(&store, &before_delivery);
}

#[test]
fn cold_permission_host_failure_commits_without_running_the_guarded_tool() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("permission-host-failure.db");
    let conversation = "permission-host-failure";
    let (run, digest, request) = permission_setup(&database, conversation);
    // Reopen after the approval delivery's start ACK. The guarded tool has
    // never started, even though the physical approval request has.
    let mut store = NolaStore::open(&database).unwrap();
    let before = delivery_before(&store, &request);
    nola_application::reduce_and_accept_host_response(&mut store, ReduceAndAcceptHostResponse {
        conversation_id: conversation.into(), run_id: run.clone(), invocation_config_digest: digest,
        response: response_for_request(&request, request.resume_after_sequence.unwrap_or(0) + 1,
            HostResponse::Failed { error: HostError {
                code: "permission_ui_failed".into(), message: "fixture approval transport failure".into(),
                retryable: false, http_status: None,
            }}),
        received_at_ms: 3000,
    }).unwrap();
    drop(store);
    let store = NolaStore::open(&database).unwrap();
    assert_eq!(store.effect_delivery_attempt(&before.id).unwrap().unwrap().state,
        nola_domain::EffectDeliveryAttemptState::Settled);
    let pending = load_committed_host_requests(&store, &run).unwrap().requests;
    assert!(!pending.iter().any(|r| r.effect_id == request.effect_id));
    assert!(!pending.iter().any(|r| matches!(r.payload, HostRequest::ExecuteTool { .. })));
}
