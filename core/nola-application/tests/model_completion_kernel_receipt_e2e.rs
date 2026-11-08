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
use serde_json::{json, Value};
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
                request.resume_after_sequence.unwrap_or(0) + 1,
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
    support::admit_model_sampling(
        &mut NolaStore::open(database).unwrap(),
        run,
        digest,
        &mut admitted,
        2000,
    );
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
fn compact_requirement_assessment_commits_and_survives_cold_replay() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("compact-result.db");
    let mut store = NolaStore::open(&database).unwrap();
    let mut config = runtime_config();
    config.tools = vec![json!({"type":"function","function":{
        "name":"fixture.read","description":"Read the isolated fixture",
        "parameters":{"type":"object","properties":{}}}})];
    let mut read = runtime_capability("fixture.read", HostCapabilityRoute::Tool);
    read.exposure = HostCapabilityExposure::Visible;
    read.input_normalization.permission_contract.risk =
        nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead;
    config.host_catalog.as_mut().unwrap().capabilities.insert("fixture.read".into(), read);
    let begun = begin_assistant_turn(&mut store, BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "completion-conversation".into(),
        conversation_title: "Result protocol".into(),
        user_message_id: "initial".into(),
        user_part_id: "initial:body".into(),
        user_text: "Read the fixture and explain the result".into(),
        user_content_json: None,
        assistant_message_id: "submit-initial:assistant".into(),
        context_checkpoint_id: None,
        core_config: config,
        submission: user_input("initial", AgentInputDelivery::CurrentTurn),
        created_at_ms: 1000,
    }).unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    let mut request = pending(&database, &run).remove(0);
    support::admit_model_sampling(&mut store, &run, &digest, &mut request, 2000);
    drop(store);
    // Assessed completion requires a declared resource and a committed receipt.
    // Exercise the actual declaration/read/assessment protocol rather than
    // changing the fixture to an unassessed Answered result.
    let declaration = response_command(&request, &run, &digest, HostResponse::ModelCompleted {
        items: vec![],
        tool_calls: vec![json!({"call_id":"declare", "name":"task_scope_update", "arguments":{
            "requirements":[{"key":"explanation","description":"Explain the result",
                "source_refs":["s1"],"dependencies":[],
                "resource_scope":{"kind":"declared","required":[{
                    "resource_uri":"file:///fixture/result",
                    "operation":"read","source_refs":["s1"]}],"exclusions":[]}}],
            "dispositions":[{"kind":"replace","requirement_id":"r1",
                "successors":[{"kind":"local_key","key":"explanation"}],
                "reason":"Record the requested read scope","user_input_refs":["s1"]}]
        }})], usage: json!({"input_tokens":80,"output_tokens":20}),
    });
    let declaration_json = serde_json::to_string(&declaration).unwrap();
    support::open_runtime(&database).unwrap().execute_received_command(ExecuteReceivedCommand {
        client_id: "compact-client".into(), request_id: "declare-scope".into(),
        command_type: "reduce_and_accept_host_response".into(), command_json: declaration_json,
        expected_revision: None, received_at: "2000".into(),
    }, declaration).unwrap();
    let mut request = pending(&database, &run).remove(0);
    support::admit_model_sampling(&mut NolaStore::open(&database).unwrap(), &run, &digest, &mut request, 2000);
    support::open_runtime(&database).unwrap().execute(response_command(&request, &run, &digest,
        HostResponse::ModelCompleted { items:vec![], tool_calls:vec![json!({
            "call_id":"read-result","name":"fixture.read","arguments":{}
        })], usage:json!({}) })).unwrap();
    let mut tool = nola_application::claim_committed_host_requests(&mut NolaStore::open(&database).unwrap(),
        nola_application::ClaimCommittedHostRequests { run_id:run.clone(), worker_generation:"fixture".into(),
            lease_owner:"fixture".into(), claimed_at_ms:2000, lease_expires_at_ms:9000, limit:8 }).unwrap().requests.remove(0);
    assert!(matches!(tool.payload, HostRequest::ExecuteTool { .. }), "unexpected request: {tool:?}");
    support::open_runtime(&database).unwrap().execute(response_command(&tool, &run, &digest,
        HostResponse::ExecutionStarted)).unwrap();
    tool.resume_after_sequence = Some(tool.resume_after_sequence.unwrap_or(0) + 1);
    support::open_runtime(&database).unwrap().execute(response_command(&tool, &run, &digest,
        HostResponse::ToolCompleted { call_id:"read-result".into(), outcome:HostToolOutcome::Succeeded,
            output:json!({"contract":"nola.tool_result.v1","data":{"content":"fixture result"},
                "presentation":{"text":"fixture result","summary":"Read fixture"},
                "execution":{"state":"completed","local_stop":null,"effects":[{
                    "resource_uri":"file:///fixture/result","operation":"read",
                    "before_version":"fixture-v1","after_version":"fixture-v1"}],"checks":[]},
                "full_output_ref":null,"memory_usage":null}),
            activation:None, full_output_ref:None, artifacts:vec![] })).unwrap();
    let mut request = pending(&database, &run).remove(0);
    support::admit_model_sampling(&mut NolaStore::open(&database).unwrap(), &run, &digest, &mut request, 2000);
    let command = response_command(&request, &run, &digest, HostResponse::ModelCompleted {
        items: vec![],
        tool_calls: vec![json!({"call_id":"finish", "name":"requirements_update",
            "arguments":{"outcome":"completed", "answer":"The explanation is complete.",
                "assessments":[{"requirement":"r1", "verdict":"satisfied",
                    "rationale":"The read receipt and requested explanation are included.", "evidence":["e1"]}]}})],
        usage: json!({"input_tokens":80,"output_tokens":20}),
    });
    let raw = serde_json::to_string(&command).unwrap();
    let receipt = || ExecuteReceivedCommand {
        client_id: "compact-client".into(), request_id: "compact-result".into(),
        command_type: "reduce_and_accept_host_response".into(), command_json: raw.clone(),
        expected_revision: None, received_at: "2000".into(),
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    let accepted = runtime.execute_received_command(receipt(), command).unwrap();
    assert!(matches!(accepted.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }), "{accepted:?}");
    drop(runtime);
    let db = rusqlite::Connection::open(&database).unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    let before = canonical_rows(&db);
    let result: String = db.query_row("SELECT result_json FROM run_results WHERE run_id=?1",
        [&run], |row| row.get(0)).unwrap_or_else(|error| panic!("{error}; pending={:?}", pending(&database, &run)));
    assert_eq!(serde_json::from_str::<Value>(&result).unwrap()["value"]["kind"], "action_completed");
    runtime.execute_received_command(receipt(), serde_json::from_str(&raw).unwrap()).unwrap();
    assert_eq!(canonical_rows(&db), before);
    assert!(pending(&database, &run).is_empty());
}

#[test]
fn historical_code_wait_reads_cold_record_through_committed_effect_without_replay() {
    use nola_domain::code_mode::*;
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("historical-cell.db");
    let mut config = runtime_config();
    let mut wait = runtime_capability("code_mode_wait", HostCapabilityRoute::Tool);
    wait.exposure = HostCapabilityExposure::Visible;
    wait.input_schema = CodeCellWaitRequest::input_schema();
    wait.input_normalization.permission_contract.risk =
        nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead;
    config.tools = vec![json!({"type":"function","function":{
        "name":"code_mode_wait","description":"Observe a previously committed cell",
        "parameters":wait.input_schema.clone()}})];
    config.host_catalog.as_mut().unwrap().capabilities.insert("code_mode_wait".into(), wait);
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(&mut store, BeginAssistantTurn {
        expected_configuration_revision:None, expected_parent_message_id:None,
        conversation_id:"completion-conversation".into(), conversation_title:"History".into(),
        user_message_id:"initial".into(), user_part_id:"initial:body".into(),
        user_text:"initial".into(), user_content_json:None,
        assistant_message_id:"submit-initial:assistant".into(), context_checkpoint_id:None,
        core_config:config, submission:user_input("initial", AgentInputDelivery::CurrentTurn),
        created_at_ms:1000,
    }).unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    let mut model = pending(&database, &run).remove(0);
    support::admit_model_sampling(&mut store, &run, &digest, &mut model, 2000);
    drop(store);
    // Seed only historical facts: no old Core checkpoint, JS source or worker.
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute_batch("BEGIN;
        INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
            VALUES('old-run','completion-conversation','conversation','completed',1,1,1);
        INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms)
            VALUES('old-message','completion-conversation','old-run','assistant','completed',0,1);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,created_at_ms,updated_at_ms)
            VALUES('old-part','old-message','tool',1,'completed',1,1);
        COMMIT;").unwrap();
    let epoch:String = db.query_row("SELECT value FROM store_metadata WHERE key='assistant_source_epoch'",
        [], |row| row.get(0)).unwrap();
    let record = nola_store_port::code_cell::CodeCellRecord {
        store_epoch:epoch, conversation_id:"completion-conversation".into(), run_id:"old-run".into(),
        parent_part_id:"old-part".into(), core_revision:1,
        cell:CodeCellIdentity {host_id:"old-host".into(), conversation_id:"old-core-session".into(),
            turn_id:"old-turn".into(), parent_call_id:"old-call".into(), cell_id:"old-cell".into(), generation:"old-generation".into()},
        observation:CodeCellView {cell_id:"old-cell".into(),parent_call_id:"old-call".into(),
            phase:CodeCellPhase::Completed,terminal:true,settled:true,next_output_sequence:2,
            has_more_output:false,output:vec![CodeOutputChunk {sequence:1,text:"first".into()},
                CodeOutputChunk {sequence:2,text:"saved result".into()}],unsettled_calls:0,
            completion:CodeCellCompletionDetails::default()},
    };
    record.validate().unwrap();
    db.execute("INSERT INTO code_cell_observations(conversation_id,cell_id,run_id,parent_part_id,core_revision,record_json)
        VALUES(?1,'old-cell','old-run','old-part',1,?2)",
        rusqlite::params![record.conversation_id,serde_json::to_string(&record).unwrap()]).unwrap();
    drop(db);
    support::open_runtime(&database).unwrap().execute(response_command(&model,&run,&digest,
        HostResponse::ModelCompleted {items:vec![],tool_calls:vec![json!({"call_id":"observe-history",
            "name":"code_mode_wait","arguments":{"cell_id":"old-cell","after_output_sequence":1,
                "wait_ms":0,"terminate":false}})],usage:json!({})})).unwrap();
    let requests = pending(&database,&run);
    assert_eq!(requests.len(),1);
    let request = requests[0].clone();
    assert!(matches!(request.payload,HostRequest::ReadCodeCellHistory {..}), "{:?}",request.payload);
    let db = rusqlite::Connection::open(&database).unwrap();
    let before = canonical_rows(&db);
    let mut runtime = support::open_runtime(&database).unwrap();
    let read = |request| ApplicationCommand::ReadCodeCellHistoryForEffect {request};
    let view:CodeCellView = serde_json::from_value(runtime.execute(read(request.clone())).unwrap()).unwrap();
    assert_eq!(view.output,vec![CodeOutputChunk {sequence:2,text:"saved result".into()}]);
    assert!(view.settled);
    let mut forged = request.clone();
    forged.session_id = "foreign-session".into();
    assert!(runtime.execute(read(forged)).is_err());
    let mut forged = request.clone();
    if let HostRequest::ReadCodeCellHistory {after_output_sequence,..} = &mut forged.payload {
        *after_output_sequence = 0;
    }
    assert!(runtime.execute(read(forged)).is_err(), "cursor is part of the frozen request");
    assert_eq!(pending(&database,&run).len(),1,"history reads cannot dispatch a model or worker");
    assert_eq!(runtime.execute(read(request.clone())).unwrap(),serde_json::to_value(&view).unwrap());
    assert_eq!(canonical_rows(&db),before,"observation is read-only across cold Application calls");
    drop(runtime);
    let mut store = NolaStore::open(&database).unwrap();
    let mut claimed = nola_application::claim_committed_host_requests(&mut store,
        nola_application::ClaimCommittedHostRequests {run_id:run.clone(),worker_generation:"history-worker".into(),
            lease_owner:"history-owner".into(),claimed_at_ms:2000,lease_expires_at_ms:9000,limit:8})
        .unwrap().requests.into_iter().find(|value| value.effect_id == request.effect_id).unwrap();
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime.execute(response_command(&claimed,&run,&digest,HostResponse::ExecutionStarted)).unwrap();
    claimed.resume_after_sequence = Some(claimed.resume_after_sequence.unwrap_or(0)+1);
    let observed = runtime.execute(read(claimed.clone())).unwrap();
    assert_eq!(observed,serde_json::to_value(view).unwrap());
    runtime.execute(response_command(&claimed,&run,&digest,HostResponse::ToolCompleted {
        call_id:"observe-history".into(),outcome:HostToolOutcome::Succeeded,output:observed,
        activation:None,full_output_ref:None,artifacts:vec![],
    })).unwrap();
    assert!(runtime.execute(read(claimed)).is_err(),"settled effect cannot authorize another history read");
    let after = pending(&database,&run);
    assert_eq!(after.len(),1,"one observation result resumes exactly one model step");
    let HostRequest::ModelStream {messages,..} = &after[0].payload else {
        panic!("history completion must resume the model, never a JS worker");
    };
    assert!(messages.iter().any(|item| item["call_id"] == "observe-history"
        && item["type"] == "tool_result" && item["is_error"] == false
        && item["output"]["output"][0]["text"] == "saved result"));
}

#[test]
fn queued_input_boundaries_survive_receipts_and_cold_recovery_without_duplicate_models() {
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
    complete_and_replay(&database, &run, &digest, third, Some("Next turn handled."));
    assert!(pending(&database, &run).is_empty());
    let checkpoint: Value = serde_json::from_str(
        &db.query_row::<String, _, _>("SELECT checkpoint_json FROM core_checkpoint_bodies", [], |row| {
            row.get(0)
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        checkpoint
            .pointer("/state/conversation/transcript")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item.get("content").and_then(Value::as_str) == Some("steering"))
            .count(),
        1
    );
}
