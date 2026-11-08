// SPDX-License-Identifier: Apache-2.0
//! Application receipt and cold recovery across queued model-completion boundaries.
mod support;
use nola_application::{
    begin_assistant_turn, claim_committed_host_requests, load_committed_host_requests,
    ApplicationCommand, BeginAssistantTurn, ClaimCommittedHostRequests, ExecuteReceivedCommand,
    ReduceAndAcceptHostResponse, ReduceAndCommitActiveSubmission,
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
    AgentCoreConfig {
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
            received_at_ms: request.created_at_ms.saturating_add(300) as i64,
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

fn claim_and_start_effect(
    database: &std::path::Path,
    run: &str,
    digest: &str,
    request: &HostRequestEnvelope,
) -> HostRequestEnvelope {
    let claimed_at_ms = request.created_at_ms.saturating_add(100) as i64;
    let mut store = NolaStore::open(database).unwrap();
    let claimed = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: run.into(),
            worker_generation: format!("generation-{}", request.request_id),
            lease_owner: format!("runtime-{}", request.request_id),
            claimed_at_ms,
            lease_expires_at_ms: claimed_at_ms + 60_000,
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
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "completion-conversation".into(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(
                &claimed,
                start_sequence,
                HostResponse::ExecutionStarted,
            ),
            received_at_ms: claimed_at_ms + 100,
        },
    )
    .unwrap();
    claimed.resume_after_sequence = Some(start_sequence);
    claimed
}

fn admit_sampling(
    database: &std::path::Path,
    run: &str,
    digest: &str,
    request: &HostRequestEnvelope,
) -> HostRequestEnvelope {
    assert!(!request.attempt_started);
    let claimed_at_ms = request.created_at_ms.saturating_add(100) as i64;
    let mut store = NolaStore::open(database).unwrap();
    let claimed = claim_committed_host_requests(
        &mut store,
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
    let mut claimed = claimed.requests.into_iter().next().unwrap();
    assert_eq!(claimed.effect_id, request.effect_id);
    nola_application::reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "completion-conversation".into(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(&claimed, 1, HostResponse::ExecutionStarted),
            received_at_ms: claimed_at_ms + 100,
        },
    )
    .unwrap();
    claimed.resume_after_sequence = Some(1);
    drop(store);
    let mut runtime = support::open_runtime(database).unwrap();
    runtime
        .execute(response_command(
            &claimed,
            run,
            digest,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "test-model".into(),
                reason: "initial".into(),
            },
        ))
        .unwrap_or_else(|error| {
            panic!(
                "admit run={run} turn={:?} effect={} attempt={:?}: {error}",
                claimed.turn_id, claimed.effect_id.0, claimed.attempt_id
            )
        });
    drop(runtime);
    let mut admitted = pending(database, run)
        .into_iter()
        .find(|r| r.effect_id == request.effect_id)
        .unwrap();
    admitted.delivery = claimed.delivery;
    assert!(admitted.attempt_started);
    assert_eq!(admitted.attempt_id, request.attempt_id);
    assert_eq!(admitted.resume_after_sequence, Some(2));
    admitted
}

fn complete_and_replay(
    database: &std::path::Path,
    run: &str,
    digest: &str,
    request: &HostRequestEnvelope,
    answer: Option<&str>,
) {
    let admitted = admit_sampling(database, run, digest, request);
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

#[test]
fn successor_turn_permission_interrupt_and_resume_keep_store_run_and_call_identity() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("completion.db");
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
            core_config: permission_config(),
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
    let admitted = admit_sampling(&database, &run, &digest, third);
    let third = &admitted;
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime.execute(response_command(third, &run, &digest, HostResponse::ModelCompleted {
        items: vec![], tool_calls: vec![json!({"call_id":"successor-call", "name":"test.write", "arguments":{"path":"/workspace/exact.txt", "content":"unchanged"}})], usage: json!({}),
    })).unwrap();
    drop(runtime);
    let ask = pending(&database, &run).remove(0);
    let ask = claim_and_start_effect(&database, &run, &digest, &ask);
    assert_eq!(ask.turn_id, third.turn_id);
    assert_eq!(ask.turn_id.as_ref().unwrap(), &run);
    let HostRequest::RequestPermission {
        permission_request_id,
        input: original_input,
        ..
    } = &ask.payload
    else {
        panic!("successor permission expected")
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(response_command(
            &ask,
            &run,
            &digest,
            HostResponse::PermissionTransportInterrupted {
                reason: nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
            },
        ))
        .unwrap();
    drop(runtime);
    let interrupted = pending(&database, &run);
    assert_eq!(interrupted.len(), 1);
    assert_eq!(
        interrupted[0].resume_after_sequence,
        Some(2),
        "Interrupted cursor must be durable before explicit resume"
    );
    db.execute("INSERT INTO trace_journal_runs(trace_id,run_id,conversation_id,turn_id,status,started_at,updated_at) VALUES('successor-trace','unrelated-diagnostic-run','completion-conversation',?1,'running','before','before')", [third.turn_id.as_ref().unwrap()]).unwrap();
    db.execute("INSERT INTO trace_journal_runs(trace_id,run_id,conversation_id,turn_id,status,started_at,updated_at) VALUES('stale-trace','another-diagnostic-run','completion-conversation','stale-turn','running','before','before')", []).unwrap();
    let canonical_before_maintenance = canonical_rows(&db);
    let mut maintenance = support::open_runtime(&database).unwrap();
    maintenance
        .execute(ApplicationCommand::RecoverOrphanedRuns {
            conversation_id: None,
            reason: "server_restarted".into(),
            interrupted_at: "after".into(),
            interrupted_at_ms: 4000,
        })
        .unwrap();
    drop(maintenance);
    assert_eq!(
        canonical_rows(&db),
        canonical_before_maintenance,
        "Startup must preserve successor permission wait and its canonical records"
    );
    let trace_state = |id: &str| {
        db.query_row::<String, _, _>(
            "SELECT status FROM trace_journal_runs WHERE trace_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(trace_state("successor-trace"), "running");
    assert_eq!(trace_state("stale-trace"), "interrupted");
    let committed_event_ids: Vec<String> = db.prepare(
        "SELECT event_id FROM run_events WHERE run_id=?1 ORDER BY sequence"
    ).unwrap().query_map([&run], |row| row.get(0)).unwrap()
        .collect::<Result<_, _>>().unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(ApplicationCommand::ReduceAndCommitActiveSubmission {
            input: ReduceAndCommitActiveSubmission {
                conversation_id: "completion-conversation".into(),
                run_id: run.clone(),
                transition_id: "resume-successor-ask".into(),
                invocation_config_digest: digest.clone(),
                submission: AgentSubmission::ResumeHostEffect {
                    effect_id: ask.effect_id.0.clone(),
                },
                committed_at_ms: 3000,
            },
        })
        .unwrap();
    drop(runtime);
    {
        use nola_store_port::run_effect::RunEffectRepository;
        let cold_store = NolaStore::open(&database).unwrap();
        let checkpoint = cold_store.load_recovery_checkpoint("completion-conversation")
            .unwrap().unwrap();
        assert!(checkpoint.replay_inputs.is_empty(),
            "recovery binding must materialize its accepted prefix for the next cold read");
        let base = checkpoint.replay_base.unwrap();
        let head: i64 = db.query_row("SELECT ordinal FROM host_event_replay_heads WHERE run_id=?1",
            [&run], |row| row.get(0)).unwrap();
        assert_eq!(base.through_ordinal, head);
        assert_eq!(base.checkpoint_revision, checkpoint.checkpoint.revision);
        let after: Vec<String> = db.prepare(
            "SELECT event_id FROM run_events WHERE run_id=?1 ORDER BY sequence"
        ).unwrap().query_map([&run], |row| row.get(0)).unwrap()
            .collect::<Result<_, _>>().unwrap();
        assert!(after.starts_with(&committed_event_ids), "materialization must retain all committed events");
    }
    let recovered = pending(&database, &run);
    assert_eq!(recovered.len(), 1);
    let recovered = &recovered[0];
    assert_eq!(recovered.effect_id, ask.effect_id);
    assert_eq!(recovered.turn_id, third.turn_id);
    assert_ne!(recovered.request_id, ask.request_id);
    assert_eq!(recovered.payload, ask.payload);
    assert_eq!(recovered.resume_after_sequence, Some(2));
    let stored_owner: String = db
        .query_row(
            "SELECT run_id FROM effect_outbox WHERE effect_id=?1",
            [&ask.effect_id.0],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored_owner, run,
        "Outbox ownership is the independently admitted successor Run"
    );
    let recovered = claim_and_start_effect(&database, &run, &digest, recovered);
    let approve = response_command(
        &recovered,
        &run,
        &digest,
        HostResponse::PermissionResolved {
            request_id: permission_request_id.clone(),
            resolution: HostPermissionResolution::allow_once(),
        },
    );
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime.execute(approve).unwrap();
    drop(runtime);
    let tools = pending(&database, &run);
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].turn_id, third.turn_id);
    let HostRequest::ExecuteTool { call_id, input, .. } = &tools[0].payload else {
        panic!("same successor tool must execute only after explicit approval")
    };
    assert_eq!(call_id, "successor-call");
    assert_eq!(input, original_input);
}

fn permission_config() -> AgentCoreConfig {
    let mut config = runtime_config();
    config.host_catalog.as_mut().unwrap().capabilities.insert(
        "permission.ask".into(),
        runtime_capability("permission.ask", HostCapabilityRoute::Permission),
    );
    config.tools = vec![
        json!({"type":"function", "function":{"name":"test.write", "description":"write fixture", "parameters":{"type":"object"}}}),
    ];
    let mut descriptor = runtime_capability("test.write", HostCapabilityRoute::Tool);
    descriptor.exposure = HostCapabilityExposure::Visible;
    descriptor.permission_domain = Some("filesystem.write".into());
    descriptor.input_normalization.permission_resource =
        HostPermissionResourceDeclaration::FirstOf {
            keys: vec!["path".into()],
        };
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .insert(descriptor.id.clone(), descriptor);
    config
}
