// SPDX-License-Identifier: Apache-2.0
//! Real Core/Host reductions and SQLite evidence provenance; no provider calls.
use super::*;
use nola_domain::execution_evidence::*;
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::{AgentInputDelivery, AgentItem, ItemId};
use nola_store_port::execution_evidence::{ExecutionEvidenceQuery, ExecutionEvidenceRepository};

fn two_read_requests(
    path: &std::path::Path,
) -> (
    nola_store::NolaStore,
    String,
    String,
    Vec<HostRequestEnvelope>,
) {
    two_read_requests_with_config(path, crate::user_input_wait_tests::config())
}

#[test]
fn task_scope_declaration_survives_store_commit_and_rejects_cold_tampering() {
    use nola_store_port::run_decision::{RunDecisionQuery, RunDecisionRepository};
    for corruption in ["none", "source", "original_body"] {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("task-scope.sqlite");
        let (mut store,run,digest,requests)=two_read_requests(&path);
        for (index,request) in requests.iter().enumerate() {
            let payload=HostResponse::ToolCompleted {call_id:format!("read-{index}"),output:serde_json::json!({"text":"read"}),
                outcome:HostToolOutcome::Succeeded,activation:None,full_output_ref:None,artifacts:vec![]};
            let mut response=input(request,&run,&digest,2,payload);
            response.received_at_ms=1400+index as i64;
            reduce_and_accept_host_response(&mut store,response).unwrap();
        }
        for round in 0..2 {
            let model=claim_committed_host_requests(&mut store,ClaimCommittedHostRequests {run_id:run.clone(),
                worker_generation:format!("scope-{round}"),lease_owner:"scope".into(),claimed_at_ms:2000+round*100,
                lease_expires_at_ms:9000,limit:8}).unwrap().requests.remove(0);
            let calls=if round==0 { vec![serde_json::json!({"call_id":"declare","name":"task_scope_update","arguments":{
                "requirements":[{"key":"files","description":"Read both files","source_refs":["s1"],"dependencies":[],
                    "resource_scope":{"kind":"declared","required":[{"resource_uri":"file:///zero.txt","operation":"read","version":null,"predicate":null,"source_refs":["s1"]},
                        {"resource_uri":"file:///one.txt","operation":"read","source_refs":["s1"]}],"exclusions":[]}}],
                "dispositions":[{"kind":"replace","requirement_id":"r1","successors":[{"kind":"local_key","key":"files"}],"reason":"Record original file scope","user_input_refs":["s1"]}]}})] }
                else { vec![serde_json::json!({"call_id":"answer","name":"requirements_update","arguments":{"outcome":"answered","answer":"Both reads returned; no assessed completion claim."}})] };
            for (sequence,payload) in [(1,HostResponse::ExecutionStarted),(2,HostResponse::ModelAttemptStarted {provider:"fixture".into(),model:"fixture".into(),reason:"initial".into()}),
                (3,serde_json::from_value(serde_json::json!({"type":"model_tool_call_delta",
                    "part_id":format!("streamed-control-{round}"),"call_id":calls[0]["call_id"],
                    "capability_id":calls[0]["name"],"arguments_delta":calls[0]["arguments"].to_string()})).unwrap()),
                (4,HostResponse::ModelCompleted {items:vec![],tool_calls:calls,usage:serde_json::json!({"input_tokens":10,"output_tokens":2})})] {
                let mut response=input(&model,&run,&digest,sequence,payload);
                response.received_at_ms=2000+round*100+sequence as i64;
                reduce_and_accept_host_response(&mut store,response).unwrap();
            }
        }
        drop(store);
        let sql=rusqlite::Connection::open(&path).unwrap();
        for round in 0..2 {
            let count: i64 = sql.query_row("SELECT count(*) FROM canonical_tool_sources WHERE part_id=?1",
                [format!("streamed-control-{round}")], |row| row.get(0)).unwrap();
            assert_eq!(count, 2, "streamed internal control retains one exact Part for its call/result pair");
        }
        let scope:String=sql.query_row("SELECT payload_json FROM run_events WHERE run_id=?1 AND event_kind='requirement_scope_changed' ORDER BY sequence DESC LIMIT 1",[&run],|row|row.get(0)).unwrap();
        let scope:serde_json::Value=serde_json::from_str(&scope).unwrap();
        assert_eq!(scope["scope"]["requirements"][0]["resource_scope"]["required"].as_array().unwrap().len(),2);
        let raw:String=sql.query_row("SELECT payload_json FROM run_events WHERE run_id=?1 AND event_kind='run_result_decided' ORDER BY sequence DESC LIMIT 1",[&run],|row|row.get(0)).unwrap();
        let result:serde_json::Value=serde_json::from_str(&raw).unwrap();
        let query=RunDecisionQuery {conversation_id:"conversation".into(),run_id:run.clone(),turn_id:run.clone(),
            core_revision:result["core_envelope"]["revision"].as_u64().unwrap(),
            core_event_cursor:result["core_envelope"]["cursor"]["sequence"].as_u64().unwrap()};
        if corruption == "source" {
            let mut altered=scope;
            altered["scope"]["requirements"][0]["resource_scope"]["required"][0]["source_refs"]=serde_json::json!(["foreign-input"]);
            assert_eq!(sql.execute("UPDATE run_events SET payload_json=?1 WHERE run_id=?2 AND event_kind='requirement_scope_changed' AND json_extract(payload_json,'$.scope.revision')='2'",
                rusqlite::params![altered.to_string(),run]).unwrap(),1);
        } else if corruption == "original_body" {
            let (id,raw):(String,String)=sql.query_row("SELECT event_id,reduction_json FROM host_event_inbox WHERE json_extract(reduction_json,'$.normalized_response.payload.tool_calls[0].name')='task_scope_update'",
                [],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
            let mut original:serde_json::Value=serde_json::from_str(&raw).unwrap();
            original["normalized_response"]["payload"]["tool_calls"][0]["arguments"]["requirements"][0]["resource_scope"]["required"][0]["resource_uri"]=serde_json::json!("file:///substituted");
            assert_eq!(sql.execute("UPDATE host_event_inbox SET reduction_json=?1 WHERE event_id=?2",rusqlite::params![original.to_string(),id]).unwrap(),1);
        }
        drop(sql);
        let reopened=nola_store::NolaStore::open(&path).unwrap();
        if corruption != "none" { assert!(reopened.load_run_decision(&query).is_err(),"{corruption}"); }
        else { assert!(reopened.load_run_decision(&query).unwrap().is_some()); }
    }
}

#[test]
fn streamed_rejected_internal_controls_keep_the_original_tool_part_owner() {
    for tool in ["execution_constraints_update", "goal_update", "request_completion_review"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("streamed-control.sqlite");
        let (mut store, run, digest, requests) = two_read_requests(&path);
        for (index, request) in requests.iter().enumerate() {
            let mut response = input(request, &run, &digest, 2, HostResponse::ToolCompleted {
                call_id: format!("read-{index}"), output: serde_json::json!({"text":"read"}),
                outcome: HostToolOutcome::Succeeded, activation: None, full_output_ref: None, artifacts: vec![],
            });
            response.received_at_ms = 1400 + index as i64;
            reduce_and_accept_host_response(&mut store, response).unwrap();
        }
        let model = claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: "control".into(), lease_owner: "control".into(),
            claimed_at_ms: 2000, lease_expires_at_ms: 8000, limit: 8,
        }).unwrap().requests.remove(0);
        for (sequence, payload) in [
            (1, HostResponse::ExecutionStarted),
            (2, HostResponse::ModelAttemptStarted { provider: "fixture".into(), model: "fixture".into(), reason: "initial".into() }),
            (3, serde_json::from_value(serde_json::json!({"type":"model_tool_call_delta", "part_id":"streamed-original",
                "call_id":"control", "capability_id":tool, "arguments_delta":"{}"})).unwrap()),
            (4, HostResponse::ModelCompleted { items: vec![], tool_calls: vec![serde_json::json!({"call_id":"control",
                "name":tool,"arguments":{}})], usage: serde_json::json!({}) }),
        ] {
            let mut response = input(&model, &run, &digest, sequence, payload);
            response.received_at_ms = 2100 + sequence as i64;
            reduce_and_accept_host_response(&mut store, response).unwrap_or_else(|error| panic!("{tool}: {error}"));
        }
        drop(store);
        let sql = rusqlite::Connection::open(&path).unwrap();
        let parts: Vec<String> = sql.prepare("SELECT part_id FROM canonical_tool_sources WHERE json_extract(binding_json,'$.call_id')='control'")
            .unwrap().query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(parts, vec!["streamed-original", "streamed-original"], "{tool}");
    }
}

fn two_read_requests_with_config(
    path: &std::path::Path,
    mut config: AgentCoreConfig,
) -> (nola_store::NolaStore, String, String, Vec<HostRequestEnvelope>) {
    let mut store = nola_store::NolaStore::open(path).unwrap();
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .get_mut("read_file")
        .unwrap()
        .parallelism = HostCapabilityParallelism::ConcurrentSafe;
    let accepted = crate::assistant_turn::admit_with_core(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "conversation".into(),
            conversation_title: "Parallel reads".into(),
            user_message_id: "origin".into(),
            user_part_id: "body".into(),
            user_text: "Read both".into(),
            user_content_json: None,
            assistant_message_id: "assistant".into(),
            context_checkpoint_id: None,
            core_config: config,
            created_at_ms: 1000,
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("origin".into()),
                    "Read both",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("origin".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
        },
        None,
    )
    .unwrap()
    .0;
    let run = accepted.begin.run_id;
    let digest = accepted.invocation_config_digest;
    let model = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: run.clone(),
            worker_generation: "model".into(),
            lease_owner: "model".into(),
            claimed_at_ms: 1001,
            lease_expires_at_ms: 8000,
            limit: 8,
        },
    )
    .unwrap()
    .requests
    .remove(0);
    for (sequence, payload) in [
        (1, HostResponse::ExecutionStarted),
        (
            2,
            HostResponse::ModelAttemptStarted {
                provider: "fixture".into(),
                model: "fixture".into(),
                reason: "initial".into(),
            },
        ),
        (
            3,
            HostResponse::ModelCompleted {
                items: vec![],
                tool_calls: vec![
                    serde_json::json!({"call_id":"read-0","name":"read_file","arguments":{"path":"zero.txt"}}),
                    serde_json::json!({"call_id":"read-1","name":"read_file","arguments":{"path":"one.txt"}}),
                ],
                usage: serde_json::json!({"input_tokens":10,"output_tokens":2}),
            },
        ),
    ] {
        reduce_and_accept_host_response(
            &mut store,
            input(&model, &run, &digest, sequence, payload),
        )
        .unwrap();
    }
    let mut requests = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: run.clone(),
            worker_generation: "parallel-tools".into(),
            lease_owner: "parallel-tools".into(),
            claimed_at_ms: 1200,
            lease_expires_at_ms: 9000,
            limit: 8,
        },
    )
    .unwrap()
    .requests;
    assert_eq!(
        requests.len(),
        2,
        "fixture must really dispatch two independent tools"
    );
    requests.sort_by_key(|request| match &request.payload {
        HostRequest::ExecuteTool { call_id, .. } => call_id.clone(),
        _ => panic!("expected tool"),
    });
    for request in &requests {
        let mut started = input(request, &run, &digest, 1, HostResponse::ExecutionStarted);
        started.received_at_ms = 1301;
        reduce_and_accept_host_response(&mut store, started).unwrap();
    }
    (store, run, digest, requests)
}

#[test]
fn parallel_reverse_completion_retains_each_original_host_source_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("buffered-evidence.sqlite");
    let (mut store, run, digest, requests) = two_read_requests(&path);
    let mut receipts = [receipt(), receipt()];
    receipts[0].checks[0].version = "version-zero".into();
    receipts[1].checks[0].version = "version-one".into();
    let responses: Vec<_> = (0..2)
        .map(|index| {
            let mut payload = completed(typed_output(&receipts[index]));
            if let HostResponse::ToolCompleted { call_id, .. } = &mut payload {
                *call_id = format!("read-{index}");
            }
            let mut response = input(&requests[index], &run, &digest, 2, payload);
            response.received_at_ms = 1303 - index as i64;
            response
        })
        .collect();
    let buffered = reduce_and_accept_host_response(&mut store, responses[1].clone()).unwrap();
    assert!(
        !buffered
            .events
            .iter()
            .any(|event| matches!(event.event, AgentEvent::ExecutionEvidenceObserved { .. })),
        "second call must remain buffered behind first"
    );
    assert_eq!(evidence_count(&path), 0);
    drop(store);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let flushed = reduce_and_accept_host_response(&mut store, responses[0].clone()).unwrap();
    let evidence: Vec<_> = flushed
        .events
        .iter()
        .filter_map(|event| match &event.event {
            AgentEvent::ExecutionEvidenceObserved { evidence, .. } => Some(evidence),
            _ => None,
        })
        .collect();
    assert_eq!(evidence.len(), 2);
    let mut saved = Vec::new();
    for (index, observation) in evidence.into_iter().enumerate() {
        assert_eq!(observation.call_id, format!("read-{index}"));
        assert_eq!(observation.receipt.as_ref(), Some(&receipts[index]));
        assert_eq!(observation.source.request_id, requests[index].request_id);
        let query = ExecutionEvidenceQuery {
            conversation_id: "conversation".into(),
            run_id: run.clone(),
            evidence_id: observation.evidence_id.clone(),
        };
        let record = store.load_execution_evidence(&query).unwrap().unwrap();
        saved.push((query, record));
    }
    assert_ne!(saved[0].1.host_event_id, saved[1].1.host_event_id);
    assert_eq!(evidence_count(&path), 2);
    drop(store);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    for response in responses {
        reduce_and_accept_host_response(&mut store, response).unwrap();
    }
    for (query, record) in saved {
        assert_eq!(
            store.load_execution_evidence(&query).unwrap().unwrap(),
            record
        );
    }
    assert_eq!(evidence_count(&path), 2);
}

fn started(path: &std::path::Path) -> (nola_store::NolaStore, String, String, HostRequestEnvelope) {
    let (digest, wait) = crate::user_input_wait_tests::fixture_with_read(path, false, true);
    let mut store = nola_store::NolaStore::open(path).unwrap();
    let request = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: wait.run_id.clone(),
            worker_generation: "evidence-worker".into(),
            lease_owner: "evidence-worker".into(),
            claimed_at_ms: 1100,
            lease_expires_at_ms: 8000,
            limit: 8,
        },
    )
    .unwrap()
    .requests
    .remove(0);
    reduce_and_accept_host_response(
        &mut store,
        input(
            &request,
            &wait.run_id,
            &digest,
            1,
            HostResponse::ExecutionStarted,
        ),
    )
    .unwrap();
    (store, wait.run_id, digest, request)
}

fn input(
    request: &HostRequestEnvelope,
    run: &str,
    digest: &str,
    sequence: u64,
    payload: HostResponse,
) -> ReduceAndAcceptHostResponse {
    ReduceAndAcceptHostResponse {
        conversation_id: "conversation".into(),
        run_id: run.into(),
        invocation_config_digest: digest.into(),
        received_at_ms: 1100 + sequence as i64,
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
    }
}

fn completed(output: serde_json::Value) -> HostResponse {
    HostResponse::ToolCompleted {
        call_id: "independent-read".into(),
        output,
        outcome: HostToolOutcome::Succeeded,
        activation: None,
        full_output_ref: None,
        artifacts: vec![],
    }
}

fn receipt() -> ExecutionReceipt {
    ExecutionReceipt {
        local_stop: None,
        state: ExecutionState::Completed,
        effects: vec![],
        checks: vec![PostconditionCheck {
            kind: nola_domain::execution_evidence::ExecutionCheckKind::Postcondition,
            resource_uri: "file:///fixture/target".into(),
            version: "sha256-original-version".into(),
            predicate: format!("{}-LAST-EXACT-PREDICATE", "receipt-fact-".repeat(1200)),
            passed: true,
        }],
    }
}

fn typed_output(receipt: &ExecutionReceipt) -> serde_json::Value {
    serde_json::json!({"contract":"nola.tool_result.v1","data":{"text":"x".repeat(12000)},
        "presentation":{"text":"long original output","summary":"Read completed"},"execution":receipt,
        "full_output_ref":null,"memory_usage":null})
}

fn observed(accepted: &AcceptedHostResponse) -> ExecutionEvidenceObservation {
    let observations: Vec<_> = accepted
        .events
        .iter()
        .filter_map(|event| match &event.event {
            AgentEvent::ExecutionEvidenceObserved { evidence, .. } => Some(evidence.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(observations.len(), 1);
    observations.into_iter().next().unwrap()
}

fn evidence_count(path: &std::path::Path) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM execution_evidence_sources",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn contains_bounded_projection(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            (map.get("truncated") == Some(&serde_json::Value::Bool(true))
                && map.contains_key("output_preview"))
                || map.values().any(contains_bounded_projection)
        }
        serde_json::Value::Array(items) => items.iter().any(contains_bounded_projection),
        _ => false,
    }
}

fn promoted(task_id: &str) -> HostResponse {
    HostResponse::TaskPromoted {
        task_id: task_id.into(), input_watermark: Some(1),
        kind: nola_agent_engine::agent::TaskKind::Tool,
        state: nola_agent_engine::agent::TaskState::Running,
        progress: Some(0.25), current_step: Some("Accepted by background worker".into()),
        lineage: Default::default(),
    }
}

#[test]
fn promoted_handoff_retains_deferred_no_receipt_source_across_reopen_and_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("promoted-evidence.sqlite");
    let (mut store, run, digest, request) = started(&path);
    let response = input(&request, &run, &digest, 2, promoted("promoted-tool"));
    let accepted = reduce_and_accept_host_response(&mut store, response.clone()).unwrap();
    let observation = observed(&accepted);
    assert_eq!(observation.call_id, "independent-read");
    assert_eq!(observation.tool_name, "read_file");
    assert_eq!(observation.outcome, HostToolOutcome::Deferred);
    assert!(observation.receipt.is_none());
    assert_eq!(observation.source.effect_id, request.effect_id.0);
    assert_eq!(observation.source.request_id, request.request_id);
    assert_eq!(observation.source.response_sequence.0, 2);
    let query = ExecutionEvidenceQuery {
        conversation_id: "conversation".into(), run_id: run.clone(), evidence_id: observation.evidence_id,
    };
    let saved = store.load_execution_evidence(&query).unwrap().unwrap();
    drop(store);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    assert_eq!(store.load_execution_evidence(&query).unwrap().unwrap(), saved);
    reduce_and_accept_host_response(&mut store, response).unwrap();
    assert_eq!(store.load_execution_evidence(&query).unwrap().unwrap(), saved);
    assert_eq!(evidence_count(&path), 1);
    let recovery = load_exact_run_checkpoint(&store, "conversation", &run).unwrap();
    let core = restore_run_recovery(&store, &run, &digest, &recovery).unwrap();
    let checkpoint = serde_json::to_value(core.checkpoint_view(CheckpointReason::Manual)).unwrap();
    let task = &checkpoint["state"]["task_projection"]["tasks"]["promoted-tool"];
    assert_eq!(task["state"], "running");
    assert!(task["durability"]["run_result"].is_null());
}

#[test]
fn promoted_evidence_rejects_forged_owner_call_task_outcome_and_receipt_on_cold_read() {
    for corruption in ["task_id", "owner", "call", "state", "revision", "host_task", "host_lineage", "outcome", "receipt"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt-promotion.sqlite");
        let (mut store, run, digest, request) = started(&path);
        let accepted = reduce_and_accept_host_response(&mut store,
            input(&request, &run, &digest, 2, promoted("promoted-tool"))).unwrap();
        let observation = observed(&accepted);
        let query = ExecutionEvidenceQuery { conversation_id:"conversation".into(), run_id:run.clone(),
            evidence_id:observation.evidence_id };
        let saved = store.load_execution_evidence(&query).unwrap().unwrap();
        drop(store);
        let sql = rusqlite::Connection::open(&path).unwrap();
        if corruption.starts_with("host_") {
            let raw: String = sql.query_row("SELECT reduction_json FROM host_event_inbox WHERE event_id=?1",
                [&saved.host_event_id], |row| row.get(0)).unwrap();
            let mut source: serde_json::Value = serde_json::from_str(&raw).unwrap();
            let payload = &mut source["normalized_response"]["payload"];
            if corruption == "host_task" { payload["task_id"] = serde_json::json!("foreign-task"); }
            else { payload["lineage"]["parent_call_id"] = serde_json::json!("foreign-call"); }
            assert_eq!(sql.execute("UPDATE host_event_inbox SET reduction_json=?1 WHERE event_id=?2",
                rusqlite::params![source.to_string(), saved.host_event_id]).unwrap(), 1);
        } else {
            let evidence = matches!(corruption, "outcome" | "receipt");
            let (id, raw): (String, String) = if evidence {
                sql.query_row("SELECT event_id,payload_json FROM run_events WHERE event_id=?1", [&saved.source_event_id],
                    |row| Ok((row.get(0)?,row.get(1)?))).unwrap()
            } else {
                sql.query_row("SELECT event_id,payload_json FROM run_events WHERE run_id=?1 AND event_kind='task_updated'
                    AND json_extract(payload_json,'$.task.id')='promoted-tool'", [&run],
                    |row| Ok((row.get(0)?,row.get(1)?))).unwrap()
            };
            let mut source: serde_json::Value = serde_json::from_str(&raw).unwrap();
            match corruption {
                "task_id" => source["task"]["id"] = serde_json::json!("foreign-task"),
                "owner" => source["task"]["owner_turn_id"] = serde_json::json!("foreign-turn"),
                "call" => source["task"]["lineage"]["parent_call_id"] = serde_json::json!("foreign-call"),
                "state" => source["task"]["state"] = serde_json::json!("completed"),
                "revision" => source["core_envelope"]["revision"] = serde_json::json!(saved.core_revision + 1),
                "outcome" => source["evidence"]["outcome"] = serde_json::json!("succeeded"),
                "receipt" => source["evidence"]["receipt"] = serde_json::to_value(receipt()).unwrap(),
                _ => unreachable!(),
            }
            assert_eq!(sql.execute("UPDATE run_events SET payload_json=?1 WHERE event_id=?2",
                rusqlite::params![source.to_string(), id]).unwrap(), 1);
        }
        drop(sql);
        let store = nola_store::NolaStore::open(&path).unwrap();
        assert!(store.load_execution_evidence(&query).is_err(), "{corruption}");
        assert_eq!(evidence_count(&path), 1, "failed cold read must preserve original index");
    }
}

#[test]
fn buffered_promotion_recovery_stop_binds_original_host_not_the_flush_response() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("promotion-recovery.sqlite");
    let mut config = crate::user_input_wait_tests::config();
    config.max_strategy_recovery_attempts = 0;
    let (mut store, run, digest, requests) = two_read_requests_with_config(&path, config);
    // Seven real failed decisions establish the pending correction budget.
    // The eighth decision fails its first member and promotes its last member.
    // Respond to that promotion first, so its stop must retain B's Host source
    // when A later flushes the ordered batch.
    for (index, request) in requests.iter().enumerate() {
        let mut response = input(request, &run, &digest, 2, HostResponse::ToolCompleted {
            call_id: format!("read-{index}"), output: serde_json::json!({"error":"fixture failure"}),
            outcome: HostToolOutcome::Failed, activation:None, full_output_ref:None, artifacts:vec![],
        });
        response.received_at_ms = 1400 + index as i64;
        reduce_and_accept_host_response(&mut store, response).unwrap();
    }
    let mut original_promotion = None;
    for round in 1..8 {
        let now = 2000 + round * 100;
        let model = claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
            run_id:run.clone(), worker_generation:format!("model-{round}"), lease_owner:"model".into(),
            claimed_at_ms:now, lease_expires_at_ms:9000, limit:8,
        }).unwrap().requests.remove(0);
        let mut calls = vec![serde_json::json!({"call_id":format!("failure-{round}"),"name":"read_file",
            "arguments":{"path":format!("failure-{round}.txt")}})];
        if round == 7 { calls.push(serde_json::json!({"call_id":"last-promotion","name":"read_file",
            "arguments":{"path":"background-read.txt"}})); }
        for (sequence, payload) in [(1,HostResponse::ExecutionStarted),
            (2,HostResponse::ModelAttemptStarted {provider:"fixture".into(),model:"fixture".into(),reason:"initial".into()}),
            (3,HostResponse::ModelCompleted {items:vec![],tool_calls:calls,
                usage:serde_json::json!({"input_tokens":10,"output_tokens":2})})] {
            let mut response = input(&model, &run, &digest, sequence, payload);
            response.received_at_ms = now + sequence as i64;
            reduce_and_accept_host_response(&mut store, response).unwrap();
        }
        let mut tools = claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
            run_id:run.clone(),worker_generation:format!("tools-{round}"),lease_owner:"tools".into(),
            claimed_at_ms:now+4,lease_expires_at_ms:9000,limit:8,
        }).unwrap().requests;
        tools.sort_by_key(|request| match &request.payload {
            HostRequest::ExecuteTool {call_id,..} => call_id.clone(), _ => panic!("expected tool"),
        });
        assert_eq!(tools.len(), if round == 7 {2} else {1});
        for (index, request) in tools.iter().enumerate() {
            let mut response = input(request, &run, &digest, 1, HostResponse::ExecutionStarted);
            response.received_at_ms = now + 5 + index as i64;
            reduce_and_accept_host_response(&mut store, response).unwrap();
        }
        if round == 7 {
            let mut response = input(&tools[1], &run, &digest, 2, promoted("recovery-promoted-tool"));
            response.received_at_ms = now+8;
            let buffered = reduce_and_accept_host_response(&mut store, response).unwrap();
            assert!(!buffered.events.iter().any(|event| matches!(event.event,AgentEvent::ExecutionEvidenceObserved { .. })));
            original_promotion = Some(tools[1].clone());
        }
        let mut response = input(&tools[0], &run, &digest, 2, HostResponse::ToolCompleted {
            call_id:format!("failure-{round}"),output:serde_json::json!({"error":"fixture failure"}),
            outcome:HostToolOutcome::Failed,activation:None,full_output_ref:None,artifacts:vec![],
        });
        response.received_at_ms = now+9;
        let accepted = reduce_and_accept_host_response(&mut store, response).unwrap();
        if round == 7 {
            assert!(accepted.events.iter().any(|event| matches!(&event.event,
                AgentEvent::RuntimeDecisionFactObserved {fact,..} if matches!(&fact.kind,
                    nola_domain::core_requirements::CoreRuntimeDecisionFactKind::RecoveryStopped {
                        cause:nola_domain::core_requirements::RecoveryStopCause::BudgetExhausted {limit:0}, ..}))));
        }
    }
    let promotion = original_promotion.unwrap();
    drop(store);
    let sql = rusqlite::Connection::open(&path).unwrap();
    let (effect, request): (String,String) = sql.query_row("SELECT h.effect_id,h.request_id
        FROM run_requirement_sources s JOIN host_event_inbox h ON h.event_id=s.source_host_event_id
        JOIN run_events e ON e.event_id=s.source_event_id WHERE s.run_id=?1
        AND json_extract(e.payload_json,'$.fact.kind.kind')='recovery_stopped'", [&run],
        |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(effect,promotion.effect_id.0);
    assert_eq!(request,promotion.request_id);
    let raw: String = sql.query_row("SELECT payload_json FROM run_events
        WHERE run_id=?1 AND event_kind='run_result_decided'", [&run], |row| row.get(0)).unwrap();
    let event: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let query = nola_store_port::run_decision::RunDecisionQuery {
        conversation_id:"conversation".into(),run_id:run,
        turn_id:event["turn_id"].as_str().unwrap().into(),
        core_revision:event["core_envelope"]["revision"].as_u64().unwrap(),
        core_event_cursor:event["core_envelope"]["cursor"]["sequence"].as_u64().unwrap(),
    };
    drop(sql);
    let store = nola_store::NolaStore::open(&path).unwrap();
    let stored = nola_store_port::run_decision::RunDecisionRepository::load_run_decision(&store,&query).unwrap().unwrap();
    let original: serde_json::Value = serde_json::from_str(&stored.event_json).unwrap();
    assert_eq!(original["decision"]["value"]["kind"], "incomplete");
    assert_eq!(original["decision"]["value"]["reason"]["code"], "execution.no_progress");
}

#[test]
fn nullable_execution_receipt_survives_commit_reopen_and_replay() {
    for original in [None, Some(ExecutionReceipt::default())] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nullable-evidence.sqlite");
        let (mut store, run, digest, request) = started(&path);
        let mut output = typed_output(&receipt());
        output["execution"] = serde_json::to_value(&original).unwrap();
        // Business data must not fill in an absent authoritative receipt.
        output["data"] = serde_json::json!({"_nola_execution": receipt()});
        let response = input(&request, &run, &digest, 2, completed(output));
        let accepted = reduce_and_accept_host_response(&mut store, response.clone()).unwrap();
        let observation = observed(&accepted);
        assert_eq!(observation.receipt, original);
        assert_eq!(observation.outcome, HostToolOutcome::Succeeded);
        let query = ExecutionEvidenceQuery {
            conversation_id: "conversation".into(),
            run_id: run.clone(),
            evidence_id: observation.evidence_id.clone(),
        };
        let saved = store.load_execution_evidence(&query).unwrap().unwrap();
        assert_eq!(saved.observation, observation);
        drop(store);

        let mut store = nola_store::NolaStore::open(&path).unwrap();
        assert_eq!(
            store.load_execution_evidence(&query).unwrap().unwrap(),
            saved
        );
        reduce_and_accept_host_response(&mut store, response).unwrap();
        assert_eq!(
            store.load_execution_evidence(&query).unwrap().unwrap(),
            saved
        );
        assert_eq!(evidence_count(&path), 1);

        let recovery = load_exact_run_checkpoint(&store, "conversation", &run).unwrap();
        let core = restore_run_recovery(&store, &run, &digest, &recovery).unwrap();
        let checkpoint =
            serde_json::to_value(core.checkpoint_view(CheckpointReason::Manual)).unwrap();
        let projected = checkpoint["state"]["conversation"]["model_context"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["call_id"] == "independent-read" && item.get("output").is_some())
            .and_then(|item| item.get("output"))
            .expect("recovered context must retain the tool result");
        assert_eq!(
            projected.get("_nola_execution"),
            original
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .unwrap()
                .as_ref()
        );
    }
}

#[test]
fn original_execution_receipt_survives_output_cropping_reopen_and_host_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("evidence.sqlite");
    let (mut store, run, digest, request) = started(&path);
    let original = receipt();
    let response = input(
        &request,
        &run,
        &digest,
        2,
        completed(typed_output(&original)),
    );
    let accepted = reduce_and_accept_host_response(&mut store, response.clone()).unwrap();
    let observation = observed(&accepted);
    assert_eq!(observation.receipt.as_ref(), Some(&original));
    let query = ExecutionEvidenceQuery {
        conversation_id: "conversation".into(),
        run_id: run.clone(),
        evidence_id: observation.evidence_id.clone(),
    };
    let saved = store.load_execution_evidence(&query).unwrap().unwrap();
    assert_eq!(saved.observation, observation);
    assert_eq!(saved.observation.source.response_sequence.0, 2);
    assert!(!saved.host_event_id.is_empty());
    assert!(!saved.source_event_id.is_empty());
    // Stored checkpoint JSON may be a baseline preceding this tool response.
    // Materialize its committed replay suffix exactly as runtime recovery does.
    let recovery = load_exact_run_checkpoint(&store, "conversation", &run).unwrap();
    let core = restore_run_recovery(&store, &run, &digest, &recovery).unwrap();
    let checkpoint = serde_json::to_value(core.checkpoint_view(CheckpointReason::Manual)).unwrap();
    assert!(
        contains_bounded_projection(&checkpoint["state"]["conversation"]["model_context"]),
        "fixture must actually crop a model/context projection"
    );
    assert_eq!(evidence_count(&path), 1);
    drop(store);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    assert_eq!(
        store.load_execution_evidence(&query).unwrap().unwrap(),
        saved
    );
    reduce_and_accept_host_response(&mut store, response).unwrap();
    assert_eq!(
        store.load_execution_evidence(&query).unwrap().unwrap(),
        saved
    );
    assert_eq!(evidence_count(&path), 1);
    let mut wrong = query.clone();
    wrong.conversation_id = "foreign".into();
    assert!(store.load_execution_evidence(&wrong).is_err());
}

#[test]
fn committed_evidence_rejects_tampered_index_core_event_and_host_receipt() {
    for corruption in ["index_call", "core_receipt", "host_receipt"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt-fixture.sqlite");
        let (mut store, run, digest, request) = started(&path);
        let accepted = reduce_and_accept_host_response(
            &mut store,
            input(
                &request,
                &run,
                &digest,
                2,
                completed(typed_output(&receipt())),
            ),
        )
        .unwrap();
        let observation = observed(&accepted);
        let query = ExecutionEvidenceQuery {
            conversation_id: "conversation".into(),
            run_id: run,
            evidence_id: observation.evidence_id,
        };
        let original = store.load_execution_evidence(&query).unwrap().unwrap();
        drop(store);
        // Deliberate corruption of an isolated fixture, not a production write
        // path and not a fabricated successful observation.
        {
            let sql = rusqlite::Connection::open(&path).unwrap();
            match corruption {
                "index_call" => {
                    assert_eq!(sql.execute("UPDATE execution_evidence_sources SET call_id='forged-call' WHERE evidence_id=?1",[&query.evidence_id]).unwrap(),1);
                }
                "core_receipt" => {
                    let source: String = sql
                        .query_row(
                            "SELECT payload_json FROM run_events WHERE event_id=?1",
                            [&original.source_event_id],
                            |row| row.get(0),
                        )
                        .unwrap();
                    let mut source: serde_json::Value = serde_json::from_str(&source).unwrap();
                    source["evidence"]["receipt"]["checks"][0]["version"] =
                        serde_json::json!("forged-version");
                    assert_eq!(
                        sql.execute(
                            "UPDATE run_events SET payload_json=?1 WHERE event_id=?2",
                            rusqlite::params![source.to_string(), original.source_event_id]
                        )
                        .unwrap(),
                        1
                    );
                }
                "host_receipt" => {
                    let source: String = sql
                        .query_row(
                            "SELECT reduction_json FROM host_event_inbox WHERE event_id=?1",
                            [&original.host_event_id],
                            |row| row.get(0),
                        )
                        .unwrap();
                    let mut source: serde_json::Value = serde_json::from_str(&source).unwrap();
                    source["normalized_response"]["payload"]["output"]["execution"]["checks"][0]
                        ["version"] = serde_json::json!("forged-version");
                    assert_eq!(
                        sql.execute(
                            "UPDATE host_event_inbox SET reduction_json=?1 WHERE event_id=?2",
                            rusqlite::params![source.to_string(), original.host_event_id]
                        )
                        .unwrap(),
                        1
                    );
                }
                _ => unreachable!(),
            }
        }
        let store = nola_store::NolaStore::open(&path).unwrap();
        assert!(
            store.load_execution_evidence(&query).is_err(),
            "{corruption}: corrupt evidence must not become None or verified"
        );
        assert_eq!(
            evidence_count(&path),
            1,
            "failed read must not erase corruption evidence"
        );
    }
}

#[test]
fn opaque_success_cannot_inject_a_typed_verified_receipt() {
    for contract in [
        None,
        Some(serde_json::json!("business.v1")),
        Some(serde_json::json!(42)),
        Some(serde_json::Value::Null),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opaque.sqlite");
        let (mut store, run, digest, request) = started(&path);
        let forged = ExecutionReceipt {
            local_stop: None,
            state: ExecutionState::Completed,
            effects: vec![],
            checks: vec![],
        };
        let mut output = serde_json::json!({"text":"Verified everything", "_nola_execution":forged, "execution":forged});
        if let Some(contract) = &contract {
            output["contract"] = contract.clone();
        }
        let response = input(&request, &run, &digest, 2, completed(output));
        let accepted = reduce_and_accept_host_response(&mut store, response).unwrap();
        let observation = observed(&accepted);
        assert!(
            observation.receipt.is_none(),
            "opaque contract {contract:?}"
        );
        assert_eq!(observation.outcome, HostToolOutcome::Succeeded);
        let recovery = load_exact_run_checkpoint(&store, "conversation", &run).unwrap();
        let core = restore_run_recovery(&store, &run, &digest, &recovery).unwrap();
        let checkpoint =
            serde_json::to_value(core.checkpoint_view(CheckpointReason::Manual)).unwrap();
        let tool_output = checkpoint["state"]["conversation"]["model_context"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["call_id"] == "independent-read" && item.get("output").is_some())
            .and_then(|item| item.get("output"))
            .expect("ordinary business output must survive into recovered model context");
        assert_eq!(
            tool_output.get("contract"),
            contract.as_ref(),
            "do not erase opaque business fields"
        );
        assert_eq!(
            tool_output["execution"],
            serde_json::to_value(forged).unwrap()
        );
        assert!(
            tool_output.get("_nola_execution").is_none(),
            "reserved forged slot must be removed"
        );
        let saved = store
            .load_execution_evidence(&ExecutionEvidenceQuery {
                conversation_id: "conversation".into(),
                run_id: run,
                evidence_id: observation.evidence_id,
            })
            .unwrap()
            .unwrap();
        assert!(saved.observation.receipt.is_none());
        assert_eq!(evidence_count(&path), 1);
    }
}

#[test]
fn wrong_host_source_cannot_commit_an_observation_or_advance_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wrong-source.sqlite");
    let (mut store, run, digest, request) = started(&path);
    let good = input(
        &request,
        &run,
        &digest,
        2,
        completed(typed_output(&receipt())),
    );
    let mut wrong = good.clone();
    wrong.response.host_id = nola_agent_engine::agent::host::HostId("foreign-host".into());
    let before = store.load_checkpoint("conversation").unwrap().unwrap();
    assert!(reduce_and_accept_host_response(&mut store, wrong).is_err());
    let after = store.load_checkpoint("conversation").unwrap().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.event_cursor, before.event_cursor);
    assert_eq!(after.checkpoint_json, before.checkpoint_json);
    assert_eq!(evidence_count(&path), 0);
    observed(&reduce_and_accept_host_response(&mut store, good).unwrap());
    assert_eq!(evidence_count(&path), 1);
}

#[test]
fn tool_sources_after_current_turn_guidance_keep_the_accepted_message_segment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guided-tools.sqlite");
    let (mut store, run, digest, requests) = two_read_requests(&path);
    crate::reduce_and_commit_active_submission(&mut store, crate::ReduceAndCommitActiveSubmission {
        conversation_id: "conversation".into(), run_id: run.clone(), transition_id: "guidance".into(),
        invocation_config_digest: digest.clone(), committed_at_ms: 1500,
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(ItemId::from_string("guidance".into()), "Keep artifacts portable")],
            turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
            submission_id: Some("guidance".into()), submitted_at_epoch_ms: Some(1500),
            delivery: AgentInputDelivery::CurrentTurn,
        },
    }).unwrap();
    for (index, request) in requests.iter().enumerate() {
        let mut response = input(request, &run, &digest, 2, HostResponse::ToolCompleted {
            call_id: format!("read-{index}"), output: serde_json::json!({"text":"read"}),
            outcome: HostToolOutcome::Succeeded, activation: None, full_output_ref: None, artifacts: vec![],
        });
        response.received_at_ms = 1400 + index as i64;
        reduce_and_accept_host_response(&mut store, response).unwrap();
    }
    let model = claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
        run_id: run.clone(), worker_generation: "guided".into(), lease_owner: "guided".into(),
        claimed_at_ms: 2000, lease_expires_at_ms: 8000, limit: 8,
    }).unwrap().requests.remove(0);
    for (sequence, payload) in [
        (1, HostResponse::ExecutionStarted),
        (2, HostResponse::ModelAttemptStarted { provider: "fixture".into(), model: "fixture".into(), reason: "initial".into() }),
        (3, HostResponse::ModelCompleted { items: vec![], tool_calls: vec![serde_json::json!({
            "call_id":"guided-read", "name":"read_file", "arguments":{"path":"zero.txt"}
        })], usage: serde_json::json!({}) }),
    ] {
        let mut response = input(&model, &run, &digest, sequence, payload);
        response.received_at_ms = 2100 + sequence as i64;
        reduce_and_accept_host_response(&mut store, response).unwrap();
    }
    drop(store);
    let sql = rusqlite::Connection::open(&path).unwrap();
    let message: String = sql.query_row("SELECT message_id FROM canonical_tool_sources WHERE json_extract(binding_json,'$.call_id')='guided-read'", [], |row| row.get(0)).unwrap();
    assert_ne!(message, "assistant");
    let accepted: bool = sql.query_row("SELECT EXISTS(SELECT 1 FROM assistant_message_segments WHERE message_id=?1)", [message], |row| row.get(0)).unwrap();
    assert!(accepted);
}

#[test]
fn physical_model_usage_survives_rejected_completion() {
    let dir=tempfile::tempdir().unwrap();
    let (mut store,run,digest,requests)=two_read_requests(&dir.path().join("usage.sqlite"));
    for (index, request) in requests.iter().enumerate() {
        reduce_and_accept_host_response(&mut store, input(request,&run,&digest,2,
            HostResponse::ToolCompleted { call_id:format!("read-{index}"), output:serde_json::json!({"text":"read"}),
                outcome:HostToolOutcome::Succeeded,activation:None,full_output_ref:None,artifacts:vec![] })).unwrap();
    }
    let model=claim_committed_host_requests(&mut store,ClaimCommittedHostRequests {
        run_id:run.clone(),worker_generation:"usage".into(),lease_owner:"usage".into(),
        claimed_at_ms:2000,lease_expires_at_ms:8000,limit:8 }).unwrap().requests.remove(0);
    let usage=serde_json::json!({"input_tokens":77437,"output_tokens":373,"cached_input_tokens":64229});
    for (sequence,payload) in [(1,HostResponse::ExecutionStarted),
        (2,HostResponse::ModelAttemptStarted {provider:"fixture".into(),model:"fixture".into(),reason:"initial".into()}),
        (3,HostResponse::ModelUsageReported {usage:usage.clone()})] {
        let mut response=input(&model,&run,&digest,sequence,payload);response.received_at_ms=2100+sequence as i64;
        reduce_and_accept_host_response(&mut store,response).unwrap();
    }
    let mut invalid=input(&model,&run,&digest,4,HostResponse::ModelCompleted {
        items:vec![serde_json::json!("invalid-item")],tool_calls:vec![],usage:usage.clone() });
    invalid.received_at_ms=2200;
    assert!(reduce_and_accept_host_response(&mut store,invalid).is_err());
    let attempt=store.active_model_attempt_for_effect(&model.effect_id.0).unwrap().unwrap();
    assert_eq!(attempt.usage,usage);
    let mut valid=input(&model,&run,&digest,4,HostResponse::ModelCompleted {
        items:vec![],tool_calls:vec![serde_json::json!({
            "call_id":"after-usage", "name":"read_file", "arguments":{"path":"zero.txt"}
        })],usage:serde_json::json!({"input_tokens":154874,"output_tokens":746,"request_count":2}) });
    valid.received_at_ms=2300;
    reduce_and_accept_host_response(&mut store,valid).unwrap();
    let completed=store.model_attempt(&attempt.id).unwrap().unwrap();
    assert_eq!(completed.usage,usage, "logical recovery totals must not double-charge the physical attempt");
}
