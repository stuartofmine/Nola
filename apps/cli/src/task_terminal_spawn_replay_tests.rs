// SPDX-License-Identifier: Apache-2.0
//! Terminal spawn replay imports the durable receipt, never worker terminal state.
use super::*;

#[test]
fn failed_agent_spawn_replay_after_reopen_keeps_one_task_and_exact_receipt() {
    let _lock = crate::store::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("spawn-replay.sqlite");
    struct Environment(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for Environment {
        fn drop(&mut self) {
            for (key, value) in &self.0 {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
    let _environment = Environment(["NOLA_DB_PATH", "NOLA_HOME"].into_iter()
        .map(|key| (key, std::env::var_os(key))).collect());
    std::env::set_var("NOLA_DB_PATH", &path);
    std::env::set_var("NOLA_HOME", root.path());
    use nola_application::{ApplicationCommand, BeginAssistantTurn};
    use nola_core::agent::{host::*, AgentCoreConfig, AgentSubmission, SessionId};
    let mut config = crate::config::Config::from_args(vec![
        "nola".into(), "--project".into(), root.path().to_string_lossy().into_owned(),
        "--endpoint".into(), "https://fixture.invalid/v1".into(),
        "--model".into(), "fixture".into(),
    ]).unwrap().host_invocation_request().unwrap();
    config.tool_settings.project_root = root.path().into();
    config.session.session_id = Some("parent".into());
    let mut identity = source_admission_support::model_identity::identity();
    identity.delegation = crate::delegated_invocation::frozen_parent(&config).unwrap();
    let descriptor = HostCapabilityDescriptor {
        id: "spawn_agent".into(),
        title: "spawn".into(),
        route: HostCapabilityRoute::Tool,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: None,
        input_schema: json!({"type":"object"}),
        input_normalization: HostInputNormalization {
            permission_contract:
                nola_domain::permission_resource_plan::PermissionOperationContract {
                    risk:
                        nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead,
                    ..Default::default()
                },
            ..Default::default()
        },
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: false,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    let core_config = AgentCoreConfig {
        turn_identity: identity,
        session_id: Some(SessionId::from_string("parent".into())),
        model: Some("fixture".into()),
        tools: vec![
            json!({"type":"function","function":{"name":"spawn_agent","parameters":{"type":"object"}}}),
        ],
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: [("spawn_agent".into(), descriptor)].into_iter().collect(),
        }),
        ..Default::default()
    };
    let now = unix_epoch_millis();
    let mut app = crate::application_port::CliApplicationPort::persistent_default().unwrap();
    let accepted: nola_application::AcceptedAssistantTurn = app
        .execute(ApplicationCommand::BeginAssistantTurn {
            input: BeginAssistantTurn {
                conversation_id: "parent".into(),
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                conversation_title: "Replay".into(),
                user_message_id: "command:user".into(),
                user_part_id: "command:user:body".into(),
                user_text: "/agent general Original task".into(),
                user_content_json: None,
                assistant_message_id: "assistant".into(),
                context_checkpoint_id: None,
                core_config,
                submission: AgentSubmission::UserDelegation {
                    agent_type: "general".into(),
                    prompt: "Original task".into(),
                    submission_id: "command".into(),
                    submitted_at_epoch_ms: now,
                },
                created_at_ms: now,
            },
        })
        .unwrap();
    let worker = crate::process_observation::current_worker_identity()
        .unwrap()
        .lease_identity()
        .unwrap();
    let request = app
        .claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: accepted.begin.run_id.clone(),
            worker_generation: worker.worker_generation,
            lease_owner: worker.lease_owner,
            claimed_at_ms: now,
            lease_expires_at_ms: now + 60000,
            limit: 1,
        })
        .unwrap()
        .requests
        .remove(0);
    let HostRequest::ExecuteTool {
        call_id,
        input: arguments,
        ..
    } = &request.payload
    else {
        panic!("expected actual Core spawn request: {:?}", request.payload)
    };
    let call_id = call_id.clone();
    let args = arguments.as_object().unwrap().clone();
    let reduce = |app: &mut crate::application_port::CliApplicationPort, sequence, payload| {
        app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
            conversation_id: "parent".into(),
            run_id: accepted.begin.run_id.clone(),
            invocation_config_digest: accepted.invocation_config_digest.clone(),
            received_at_ms: unix_epoch_millis(),
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
                sequence,
                payload,
            },
        })
        .unwrap()
    };
    reduce(&mut app, 0, HostResponse::ExecutionStarted);
    let first = spawn_agent(&config, &request, "parent", &call_id, &args);
    assert!(!first.outcome.error, "{}", first.outcome.text);
    let promotion = first
        .task_promotion
        .expect("prepared task must await actual promotion admission");
    let task_id = promotion.task_id.clone();
    let watermark = promotion.input_watermark;
    let promoted = reduce(
        &mut app,
        1,
        HostResponse::TaskPromoted {
            task_id: promotion.task_id,
            input_watermark: watermark,
            kind: promotion.kind,
            state: promotion.state,
            progress: promotion.progress,
            current_step: promotion.current_step,
            lineage: promotion.lineage,
        },
    );
    let cursor = promoted
        .events
        .iter()
        .find_map(|event| match &event.event {
            nola_core::agent::AgentEvent::TaskUpdated { task, .. } if task.id == task_id => {
                Some(event.cursor.sequence)
            }
            _ => None,
        })
        .expect("committed task promotion source");
    let source = app
        .export_agent_task_admission("parent", &accepted.begin.run_id, &task_id, cursor)
        .unwrap();
    app.import_exported_agent_task_admission(&source).unwrap();
    drop(app);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let failed_at = unix_epoch_millis();
    let attempt = store
        .claim_runtime_task(
            &task_id,
            "worker",
            "generation",
            "token",
            failed_at,
            failed_at + 60000,
        )
        .unwrap()
        .unwrap();
    assert!(store
        .finish_runtime_task_detailed_with_event(
            &nola_application::RuntimeTaskLeaseWitness::from(&attempt),
            "failed",
            watermark,
            Some("startup failed"),
            None,
            Some("fixture startup failure"),
            None,
            true,
            "failed",
            "failed",
            None,
            "{}",
            None,
            failed_at + 1
        )
        .unwrap());
    drop(store);
    let before = crate::runtime_task_port::enqueue_terminal_reconciliation(&task_id).unwrap();
    for _ in 0..2 {
        assert!(
            replay_runtime_task_tool("spawn_agent", &call_id).is_none(),
            "terminal state must not be replayed as Host TaskUpdated"
        );
        let replay = spawn_agent(&config, &request, "parent", &call_id, &args);
        assert!(
            replay.outcome.error,
            "completed spawn effect cannot execute again"
        );
        assert!(replay.task_update.is_none());
        assert!(replay.task_promotion.is_none());
        let after = crate::runtime_task_port::enqueue_terminal_reconciliation(&task_id).unwrap();
        assert_eq!(after.receipt_id, before.receipt_id);
        assert_eq!(after.command_id, before.command_id);
        let reopened = NolaStore::open_default().unwrap();
        assert_eq!(reopened.list_all_runtime_tasks().unwrap().len(), 1);
        assert_eq!(
            reopened.runtime_task(&task_id).unwrap().unwrap().status,
            "failed"
        );
    }
}
