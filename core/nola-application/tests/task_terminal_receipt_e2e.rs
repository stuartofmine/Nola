// SPDX-License-Identifier: Apache-2.0
//! Real Store terminal writers, receipt-only inbox admission and Core reduction.
mod support;
use nola_application::{ApplicationCommand,RuntimeTaskCommand,RuntimeTaskLeaseWitness};
use nola_agent_engine::agent::{AgentSubmission,TaskState};
use serde_json::json;

fn seed(path:&std::path::Path,claim:bool)->Option<RuntimeTaskLeaseWitness> {
    let mut runtime=support::open_runtime(path).unwrap();
    support::agent_task_admission::draft(&mut runtime,None);
    let admission=support::agent_task_admission::source_fact(&mut runtime);
    runtime.execute(ApplicationCommand::ImportHostAgentTaskAdmission {admission}).unwrap();
    let mut store=nola_store::NolaStore::open(path).unwrap();
    claim.then(||RuntimeTaskLeaseWitness::from(&store.claim_runtime_task(
        "task","worker","generation","token",200,1000).unwrap().unwrap()))
}
fn finish(store:&mut nola_store::NolaStore,lease:&RuntimeTaskLeaseWitness)->bool {
    store.finish_runtime_task_detailed_with_event(lease,"failed",Some(7),Some("done"),Some("result"),None,None,true,
        "failed","failed",Some("done"),"{}",None,300).unwrap()
}

#[test]
fn application_final_observation_commits_with_shell_receipt_or_not_at_all() {
    for mode in ["valid", "cpu_backwards", "output_backwards", "reserved_key"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("final-observation.sqlite");
        let mut runtime = support::open_runtime(&path).unwrap();
        let input = serde_json::from_value(json!({
            "id":"shell", "kind":"shell_process", "title":"Shell fixture", "project_root":"/fixture",
            "initially_unread":false, "metadata":{}, "created_at_ms":100,
            "scheduling":{
                "priority":"normal", "queued_at_ms":100,
                "resource_budget":{"max_wall_time_ms":1000,"max_output_bytes":1000},
                "output_bytes":10,
                "resource_usage":{"cpu_time_ms":5,"peak_memory_bytes":20,"peak_processes":1},
                "budget_exhaustion":null
            }
        })).unwrap();
        runtime.execute(ApplicationCommand::RuntimeTask { command:RuntimeTaskCommand::Create {input} }).unwrap();
        let mut store = nola_store::NolaStore::open(&path).unwrap();
        let attempt = store.claim_runtime_task("shell","worker","generation","token",200,1000).unwrap().unwrap();
        let before = store.runtime_task("shell").unwrap().unwrap();
        let observation = nola_application::RuntimeTaskFinalObservation {
            output_bytes:Some(if mode == "output_backwards" {9} else {25}),
            resource_usage:Some(nola_application::RuntimeTaskResourceUsage {
                cpu_time_ms:if mode == "cpu_backwards" {4} else {7},
                peak_memory_bytes:30, peak_processes:2,
            }),
            metadata_patch:std::collections::BTreeMap::from([(
                if mode == "reserved_key" {"_nola_scheduler"} else {"shell_execution"}.into(),
                Some(json!({"exit_code":0,"signal":null})),
            )]),
        };
        let result = runtime.execute(ApplicationCommand::RuntimeTask {
            command:RuntimeTaskCommand::FinishDetailedWithEvent {
                lease:RuntimeTaskLeaseWitness::from(&attempt), status:"completed".into(),
                completion_watermark:None, progress_summary:None, result_summary:None, error:None,
                output_path:None, unread:true, event_kind:"finished".into(), event_status:"completed".into(),
                event_summary:None, event_detail_json:"{}".into(), final_observation:Some(observation), now_ms:300,
            },
        });
        if mode == "valid" {
            assert_eq!(result.unwrap(),json!(true));
            drop(store);
            let reopened = nola_store::NolaStore::open(&path).unwrap();
            let task = reopened.runtime_task("shell").unwrap().unwrap();
            let metadata:serde_json::Value = serde_json::from_str(&task.metadata_json).unwrap();
            assert_eq!(task.status,"completed");
            assert_eq!(metadata["_nola_scheduler"]["output_bytes"],25);
            assert_eq!(metadata["_nola_scheduler"]["resource_usage"]["cpu_time_ms"],7);
            assert_eq!(metadata["_nola_scheduler"]["resource_usage"]["peak_memory_bytes"],30);
            assert_eq!(metadata["shell_execution"]["exit_code"],0);
            let receipt = reopened.runtime_task_terminal_receipt("shell").unwrap().unwrap();
            assert_eq!(receipt.status,"completed");
            assert!(matches!(receipt.source,nola_application::RuntimeTaskTerminalSource::WorkerFinish { lease }
                if lease.attempt_id == attempt.id));
        } else {
            let error = result.unwrap_err();
            assert!(error.contains(if mode == "reserved_key" {"reserved key"} else {"moved backwards"}),"{mode}: {error}");
            assert_eq!(store.runtime_task("shell").unwrap().unwrap(),before);
            assert_eq!(store.runtime_task_current_attempt("shell").unwrap().unwrap(),attempt);
            assert!(store.runtime_task_terminal_receipt("shell").unwrap().is_none());
        }
    }
}

#[test]
fn unbound_agent_cannot_manufacture_completion_or_sealed_result() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no-inferred-completion.sqlite");
    let lease = seed(&path, true).unwrap();
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let before = store.runtime_task("task").unwrap();
    let attempt = store.runtime_task_attempt(&lease.attempt_id).unwrap();
    for status in ["completed", "sealed"] {
        assert!(store.finish_runtime_task_detailed_with_event(&lease, status, Some(7),
            None, Some("worker says done"), None, None, true, "worker_done", status,
            None, "{}", None, 300).is_err());
        assert_eq!(store.runtime_task("task").unwrap(), before);
        assert_eq!(store.runtime_task_attempt(&lease.attempt_id).unwrap(), attempt);
        assert!(store.runtime_task_terminal_receipt("task").unwrap().is_none());
    }
}

#[test]
fn every_terminal_writer_records_an_immutable_receipt_and_reopens() {
    for writer in ["finish","expire","expire_event","queued_cancel","expired_cancel","retry_cancel"] {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("terminal.sqlite");
        let lease=seed(&path,writer!="queued_cancel");let mut store=nola_store::NolaStore::open(&path).unwrap();
        match writer {
            "finish"=>assert!(finish(&mut store,lease.as_ref().unwrap())),
            "expire"=>assert!(store.fail_expired_runtime_task("task","expired",1100).unwrap()),
            "expire_event"=>assert!(store.fail_expired_runtime_task_with_event("task","expired","expired","failed",None,"{}",1100).unwrap()),
            "queued_cancel"=>assert!(store.request_runtime_task_cancel("task",300).unwrap()),
            "expired_cancel"=>{
                assert!(store.request_runtime_task_cancel("task",300).unwrap());
                assert!(store.runtime_task_terminal_receipt("task").unwrap().is_none());
                assert!(store.recover_expired_runtime_task("task","cancelled",1200,1100).unwrap());
            }
            "retry_cancel"=>{
                assert!(store.retry_runtime_task(lease.as_ref().unwrap(),400,"retry",300).unwrap());
                assert!(store.request_runtime_task_cancel("task",350).unwrap());
            }
            _=>unreachable!(),
        }
        let receipt=store.runtime_task_terminal_receipt("task").unwrap().unwrap();
        assert_eq!(receipt.owner_session_id,"parent");assert_eq!(receipt.input_watermark,Some(7));
        assert_eq!(matches!(receipt.source,nola_store_port::runtime_task::RuntimeTaskTerminalSource::Cancellation),matches!(writer,"queued_cancel"|"expired_cancel"|"retry_cancel"));
        assert_eq!(receipt.attempts.len(),usize::from(writer!="queued_cancel"));
        drop(store);
        assert_eq!(nola_store::NolaStore::open(&path).unwrap().runtime_task_terminal_receipt("task").unwrap(),Some(receipt));
        let db=rusqlite::Connection::open(&path).unwrap();
        assert!(db.execute("UPDATE runtime_task_terminal_receipts SET receipt_id='foreign'",[]).is_err());
        assert!(db.execute("DELETE FROM runtime_task_terminal_receipts",[]).is_err());
        assert!(db.execute("UPDATE durable_tasks SET input_watermark=8 WHERE id='task'",[]).is_err());
    }
}

#[test]
fn bound_task_all_terminal_writers_require_canonical_run_evidence() {
    for writer in ["finish", "expire", "expire_event", "expired_cancel"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bound-terminal.sqlite");
        let lease = seed(&path, writer != "queued_cancel");
        let mut store = nola_store::NolaStore::open(&path).unwrap();
        if writer == "retry_cancel" {
            assert!(store.retry_runtime_task(lease.as_ref().unwrap(), 400, "retry", 300).unwrap());
        }
        if writer == "expired_cancel" {
            assert!(store.request_runtime_task_cancel("task", 300).unwrap());
        }
        // Retained admission is itself enough to forbid a second terminal
        // authority, even when the bound Run is missing/cannot be read.
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("INSERT INTO runtime_task_run_bindings
            (task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,input_watermark,bound_at_ms)
            VALUES('task','parent','child','unavailable-run','original-attempt',7,200)", []).unwrap();
        let before = store.runtime_task("task").unwrap();
        let attempt = lease.as_ref().map(|lease| store.runtime_task_attempt(&lease.attempt_id).unwrap());
        let events: i64 = db.query_row("SELECT COUNT(*) FROM durable_task_events WHERE task_id='task'",
                                       [], |row| row.get(0)).unwrap();
        let result = match writer {
            "finish" => store.finish_runtime_task_detailed_with_event(lease.as_ref().unwrap(),
                "failed", Some(7), None, None, None, None, true, "failed", "failed", None, "{}", None, 350),
            "expire" => store.fail_expired_runtime_task("task", "expired", 1100),
            "expire_event" => store.fail_expired_runtime_task_with_event("task", "expired",
                "expired", "failed", None, "{}", 1100),
            "expired_cancel" => store.recover_expired_runtime_task("task", "cancelled", 1200, 1100),
            _ => store.request_runtime_task_cancel("task", 350),
        };
        assert!(result.unwrap_err().to_string().contains("canonical Run seal"), "{writer}");
        assert_eq!(store.runtime_task("task").unwrap(), before, "{writer}");
        if let Some(lease) = &lease {
            assert_eq!(store.runtime_task_attempt(&lease.attempt_id).unwrap(), attempt.unwrap(), "{writer}");
        }
        assert!(store.runtime_task_terminal_receipt("task").unwrap().is_none());
        assert_eq!(db.query_row("SELECT COUNT(*) FROM durable_task_events WHERE task_id='task'",
                               [], |row| row.get::<_, i64>(0)).unwrap(), events, "{writer}");
        drop(store);
        assert_eq!(nola_store::NolaStore::open(&path).unwrap().runtime_task("task").unwrap(), before);
    }
}

#[test]
fn bound_task_cancel_is_durable_intent_not_terminal_or_new_execution() {
    for status in ["running", "queued", "retry_waiting", "waiting_permission", "waiting_input", "suspended"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancel-intent.sqlite");
        let lease = seed(&path, true).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("INSERT INTO runtime_task_run_bindings
            (task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,input_watermark,bound_at_ms)
            VALUES('task','parent','child','original-run',?1,7,200)", [&lease.attempt_id]).unwrap();
        db.execute("UPDATE durable_tasks SET status=?1 WHERE id='task'", [status]).unwrap();
        let mut store = nola_store::NolaStore::open(&path).unwrap();
        let before = store.runtime_task("task").unwrap().unwrap();
        let attempt = store.runtime_task_attempt(&lease.attempt_id).unwrap();
        assert!(store.request_runtime_task_cancel("task", 350).unwrap(), "{status}");
        let requested = store.runtime_task("task").unwrap().unwrap();
        assert!(requested.cancel_requested);
        assert_eq!(requested.status, status);
        assert_eq!(requested.ended_at_ms, before.ended_at_ms);
        assert_eq!(requested.ownership_token, before.ownership_token);
        assert_eq!(requested.retry_count, before.retry_count);
        assert_eq!(requested.progress_summary.as_deref(), Some("cancellation_requested"));
        assert_eq!(store.runtime_task_attempt(&lease.attempt_id).unwrap(), attempt);
        assert!(store.runtime_task_terminal_receipt("task").unwrap().is_none());
        // Polling or an ACK retry cannot replace the original intent timestamp.
        assert!(!store.request_runtime_task_cancel("task", 400).unwrap());
        assert_eq!(store.runtime_task("task").unwrap().unwrap(), requested);
        drop(store);
        let reopened = nola_store::NolaStore::open(&path).unwrap();
        assert_eq!(reopened.runtime_task("task").unwrap().unwrap(), requested);
        assert!(reopened.runtime_task_terminal_receipt("task").unwrap().is_none());
    }
}

#[test]
fn receipt_write_failure_rolls_back_worker_finish_and_attempt_revocation() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("rollback.sqlite");
    let lease=seed(&path,true).unwrap();let mut store=nola_store::NolaStore::open(&path).unwrap();
    let before=store.runtime_task_attempt(&lease.attempt_id).unwrap();
    let db=rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON runtime_task_terminal_receipts BEGIN SELECT RAISE(ABORT,'receipt fault'); END;").unwrap();
    assert!(store.finish_runtime_task_detailed_with_event(&lease,"failed",Some(7),None,None,None,None,true,"failed","failed",None,"{}",None,300).is_err());
    assert_eq!(store.runtime_task("task").unwrap().unwrap().status,"running");
    assert_eq!(store.runtime_task_attempt(&lease.attempt_id).unwrap(),before);
    assert!(store.runtime_task_terminal_receipt("task").unwrap().is_none());
    let events:i64=db.query_row("SELECT COUNT(*) FROM durable_task_events WHERE kind='completed'",[],|r|r.get(0)).unwrap();assert_eq!(events,0);
}

fn projection(runtime:&mut support::ApplicationRuntime,receipt:&str)->nola_domain::agent_task_admission::AgentTaskTerminalProjection {
    serde_json::from_value(runtime.execute(ApplicationCommand::ExportAgentTaskTerminalProjection {
        task_id:"task".into(),expected_receipt_id:receipt.into(),guidance_message_ids:vec![],
    }).unwrap()).unwrap()
}
fn assert_projected(runtime:&mut support::ApplicationRuntime,receipt:&str) {
    assert_projected_state(runtime, receipt, TaskState::Failed);
}
fn assert_projected_state(runtime:&mut support::ApplicationRuntime,receipt:&str,expected:TaskState) {
    let value=runtime.execute(ApplicationCommand::LoadCheckpoint {conversation_id:"parent".into()}).unwrap();
    let cp:nola_application::ConversationCheckpoint=serde_json::from_value(value).unwrap();
    let state:nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState>=serde_json::from_str(&cp.checkpoint_json).unwrap();
    let task=&state.state.task_projection.tasks["task"];
    assert_eq!(task.state,expected);
    assert_eq!(task.current_step, None, "a terminal Task must retire its prior queued/live step");
    assert_eq!(task.progress, None, "a terminal receipt must not invent full delivery progress");
    assert_eq!(task.durability.terminal_receipt_id.as_deref(),Some(receipt));
}
#[test]
fn local_terminal_notice_commits_projection_and_ack_together_and_survives_reopen() {
    let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("local-notice.sqlite");
    let lease=seed(&path,true).unwrap(); let mut store=nola_store::NolaStore::open(&path).unwrap();
    assert!(finish(&mut store,&lease));
    let receipt=store.runtime_task_terminal_receipt("task").unwrap().unwrap(); drop(store);
    let mut app=support::open_runtime(&path).unwrap();
    app.execute(ApplicationCommand::EnqueueTaskTerminalReconciliation {task_id:"task".into(),expected_receipt_id:receipt.receipt_id.clone()}).unwrap();
    let apply=||ApplicationCommand::ApplyPendingTaskNotice {conversation_id:"parent".into(),consumer_id:"native-host".into(),now_ms:400};
    let db=rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_notice_ack BEFORE DELETE ON agent_commands BEGIN SELECT RAISE(ABORT,'ack fault'); END;").unwrap();
    assert!(app.execute(apply()).is_err());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_commands",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    db.execute_batch("DROP TRIGGER fail_notice_ack").unwrap();
    let accepted=app.execute(apply()).unwrap();
    assert!(!accepted["events"].as_array().unwrap().is_empty());
    assert_projected(&mut app,&receipt.receipt_id);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_commands",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(app); let mut app=support::open_runtime(&path).unwrap();
    assert_eq!(app.execute(apply()).unwrap(),serde_json::Value::Null);
    assert_projected(&mut app,&receipt.receipt_id);
}
#[test]
fn receipt_delivery_is_stable_and_reducer_rejects_forged_or_legacy_terminal_authority() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("delivery.sqlite");
    let lease=seed(&path,true).unwrap();let mut store=nola_store::NolaStore::open(&path).unwrap();
    assert!(finish(&mut store,&lease));let receipt=store.runtime_task_terminal_receipt("task").unwrap().unwrap();
    let mut runtime=support::open_runtime(&path).unwrap();
    let enqueue=||ApplicationCommand::EnqueueTaskTerminalReconciliation {task_id:"task".into(),expected_receipt_id:receipt.receipt_id.clone()};
    let ack=runtime.execute(enqueue()).unwrap();assert_eq!(runtime.execute(enqueue()).unwrap(),ack);
    assert!(runtime.execute(ApplicationCommand::EnqueueTaskTerminalReconciliation {task_id:"task".into(),expected_receipt_id:"foreign".into()}).is_err());
    let db=rusqlite::Connection::open(&path).unwrap();
    let count:i64=db.query_row("SELECT COUNT(*) FROM agent_commands",[],|r|r.get(0)).unwrap();assert_eq!(count,1);
    let payload:String=db.query_row("SELECT payload_json FROM agent_commands",[],|r|r.get(0)).unwrap();
    let notification:serde_json::Value=serde_json::from_str(&payload).unwrap();
    assert_eq!(notification,json!({"type":"task_terminal_available","task_id":"task","receipt_id":receipt.receipt_id}));
    assert!(serde_json::from_str::<AgentSubmission>(&payload).is_err(),"wake-up is not executable authority");
    assert!(runtime.execute(ApplicationCommand::EnqueueAgentCommand {input:nola_application::AgentCommand {
        session_id:"parent".into(),command_id:"forged".into(),payload_json:payload,created_at_ms:400,
    }}).is_err());
    let fact=projection(&mut runtime,&receipt.receipt_id);
    let before: nola_application::ConversationCheckpoint = serde_json::from_value(runtime.execute(
        ApplicationCommand::LoadCheckpoint {conversation_id:"parent".into()}).unwrap()).unwrap();
    let before: nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState> =
        serde_json::from_str(&before.checkpoint_json).unwrap();
    assert_eq!(before.state.task_projection.tasks["task"].current_step.as_deref(), Some("Queued"));
    let ordinary=AgentSubmission::ReconcileTaskTerminal {operation_id:"terminal:receipt".into(),fact:fact.clone()};
    assert!(runtime.execute(ApplicationCommand::ReduceAndCommitIdleSubmission {input:nola_application::ReduceAndCommitIdleSubmission {
        conversation_id:"parent".into(),core_config:support::model_identity::config(),submission:ordinary,committed_at_ms:700,
    }}).is_err(),"even exact fact cannot enter ordinary JSON reducer");
    let command=||ApplicationCommand::ImportHostAgentTaskTerminalProjection {projection:fact.clone()};
    let first=runtime.execute(command()).unwrap();
    assert_eq!(runtime.execute(command()).unwrap(),first);
    assert_projected(&mut runtime,&receipt.receipt_id);
    drop(runtime);
    assert_projected(&mut support::open_runtime(&path).unwrap(),&receipt.receipt_id);
}

#[test]
fn original_saved_and_ephemeral_sources_import_global_terminal_without_global_rows() {
    for ephemeral in [false,true] {
        let dir=tempfile::tempdir().unwrap();
        let source_path=dir.path().join("source.sqlite");
        let mut source=if ephemeral {support::open_runtime(":memory:").unwrap()}else{support::open_runtime(&source_path).unwrap()};
        let admission=support::agent_task_admission::source_fact(&mut source);
        let global_path=dir.path().join("global.sqlite");
        let mut global=support::open_runtime(&global_path).unwrap();
        support::agent_task_admission::draft(&mut global,None);
        global.execute(ApplicationCommand::ImportHostAgentTaskAdmission {admission:admission.clone()}).unwrap();
        // The original source owns its TaskQueued declaration but no global Task row.
        assert!(source.execute(ApplicationCommand::RuntimeTask {command:RuntimeTaskCommand::Get {id:"task".into()}}).unwrap().is_null());
        let mut store=nola_store::NolaStore::open(&global_path).unwrap();
        let lease=RuntimeTaskLeaseWitness::from(&store.claim_runtime_task("task","worker","generation","token",200,1000).unwrap().unwrap());
        assert!(finish(&mut store,&lease));
        let receipt=store.runtime_task_terminal_receipt("task").unwrap().unwrap();
        let fact=projection(&mut global,&receipt.receipt_id);
        let legacy=AgentSubmission::UpdateTask {operation_id:"forged".into(),task_id:"task".into(),state:TaskState::Completed,
            progress:Some(1.0),current_step:None,result_ref:None,cancel_requested:false,ownership_token:"token".into(),completion_watermark:Some(7)};
        let error=source.execute(ApplicationCommand::ReduceAndCommitIdleSubmission {input:nola_application::ReduceAndCommitIdleSubmission {
            conversation_id:"parent".into(),core_config:support::model_identity::config(),submission:legacy,committed_at_ms:700,
        }}).unwrap_err();
        assert!(error.contains("terminal receipt"),"{error}");
        let response=nola_agent_engine::agent::host::HostResponseEnvelope {
            attempt_id:None,delegated_invocation_id:None,request_id:"forged-request".into(),
            effect_id:nola_agent_engine::agent::host::HostEffectId(admission.parent_effect_id.clone()),
            host_id:nola_agent_engine::agent::host::HostId("fixture-host".into()),lease_id:None,
            session_id:admission.parent_core_session_id.clone(),turn_id:Some(admission.parent_turn_id.clone()),
            catalog_revision:1,availability_epoch:1,delivery:None,sequence:3,
            payload:nola_agent_engine::agent::host::HostResponse::TaskUpdated {task_id:"task".into(),state:TaskState::Completed,
                progress:Some(1.0),current_step:None,result_ref:None,cancel_requested:false,
                ownership_token:Some("token".into()),completion_watermark:Some(7)},
        };
        let error=source.execute(ApplicationCommand::ReduceAndAcceptHostResponse {input:nola_application::ReduceAndAcceptHostResponse {
            conversation_id:"parent".into(),run_id:admission.parent_run_id.clone(),invocation_config_digest:"not-authority".into(),
            response,received_at_ms:700,
        }}).unwrap_err();
        assert!(error.contains("terminal Host response requires a terminal receipt"),"{error}");
        let mut foreign=support::open_runtime(":memory:").unwrap();
        let different=support::agent_task_admission::source_fact(&mut foreign);
        assert_ne!(different.source_store_epoch,admission.source_store_epoch);
        assert!(foreign.execute(ApplicationCommand::ImportHostAgentTaskTerminalProjection {projection:fact.clone()}).is_err());
        for changed in ["source","child","call","watermark"] {
            let mut wrong=fact.clone();
            match changed {
                "source"=>wrong.admission.source_store_epoch="foreign".into(),
                "child"=>wrong.admission.child_conversation_id="foreign".into(),
                "call"=>wrong.admission.parent_tool_call_id="foreign".into(),
                _=>{wrong.input_watermark=Some(6);wrong.completion_watermark=Some(6);}
            }
            wrong.admission=wrong.admission.seal().unwrap();
            assert!(source.execute(ApplicationCommand::ImportHostAgentTaskTerminalProjection {projection:wrong}).is_err(),"{changed}");
        }
        let command=||ApplicationCommand::ImportHostAgentTaskTerminalProjection {projection:fact.clone()};
        let ack=source.execute(command()).unwrap();
        assert_eq!(source.execute(command()).unwrap(),ack);
        assert_projected(&mut source,&receipt.receipt_id);
        if !ephemeral {drop(source);assert_projected(&mut support::open_runtime(&source_path).unwrap(),&receipt.receipt_id);}
    }
}

#[test]
fn task_terminal_host_responses_are_rejected_at_all_application_reducer_entries() {
    use nola_agent_engine::agent::host::{HostResponse,HostResponseEnvelope,HostId,HostEffectId};
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("host-response.sqlite");
    seed(&path,true);let mut runtime=support::open_runtime(&path).unwrap();
    let response=HostResponseEnvelope {
        attempt_id:None,delegated_invocation_id:None,request_id:"request".into(),effect_id:HostEffectId("effect".into()),
        host_id:HostId("host".into()),lease_id:None,session_id:"actual-parent-core".into(),turn_id:Some("parent-turn".into()),
        catalog_revision:1,availability_epoch:1,delivery:None,sequence:1,
        payload:HostResponse::TaskUpdated {task_id:"task".into(),state:TaskState::Completed,progress:Some(1.0),current_step:None,
            result_ref:None,cancel_requested:false,ownership_token:Some("token".into()),completion_watermark:Some(7)},
    };
    let commands=[
        ApplicationCommand::ReduceAndAcceptHostResponse {input:nola_application::ReduceAndAcceptHostResponse {
            conversation_id:"parent".into(),run_id:"parent-run".into(),invocation_config_digest:"digest".into(),response:response.clone(),received_at_ms:300}},
        ApplicationCommand::ReduceAndCommitActiveSubmission {input:nola_application::ReduceAndCommitActiveSubmission {
            conversation_id:"parent".into(),run_id:"parent-run".into(),transition_id:"transition".into(),invocation_config_digest:"digest".into(),
            submission:AgentSubmission::HostResponse {response:response.clone()},committed_at_ms:300}},
        ApplicationCommand::ReduceAndCommitIdleSubmission {input:nola_application::ReduceAndCommitIdleSubmission {
            conversation_id:"parent".into(),core_config:support::model_identity::config(),submission:AgentSubmission::HostResponse {response},committed_at_ms:300}},
    ];
    for command in commands {
        let error=runtime.execute(command).unwrap_err();assert!(error.contains("terminal Host response requires a terminal receipt"),"{error}");
    }
    assert_eq!(nola_store::NolaStore::open(&path).unwrap().runtime_task("task").unwrap().unwrap().status,"running");
}

#[test]
fn late_terminal_import_uses_true_idle_parent_without_rewriting_its_seal() {
    use nola_agent_engine::agent::host::{HostRequest,HostResponse,HostResponseEnvelope};
    let dir=tempfile::tempdir().unwrap();
    let mut source=support::open_runtime(":memory:").unwrap();
    let admission=support::agent_task_admission::source_fact(&mut source);
    let path=dir.path().join("global.sqlite");
    let mut global=support::open_runtime(&path).unwrap();
    support::agent_task_admission::draft(&mut global,None);
    global.execute(ApplicationCommand::ImportHostAgentTaskAdmission {admission:admission.clone()}).unwrap();
    let mut store=nola_store::NolaStore::open(&path).unwrap();
    let lease=RuntimeTaskLeaseWitness::from(&store.claim_runtime_task("task","worker","generation","token",200,1000).unwrap().unwrap());
    let active:Option<nola_store_port::run_effect::StoredActiveRun>=serde_json::from_value(
        source.execute(ApplicationCommand::ActiveRun {conversation_id:"parent".into()}).unwrap()).unwrap();
    let active=active.unwrap();
    let digest=active.invocation_config_digest.unwrap();
    source.execute(ApplicationCommand::ReduceAndCommitActiveSubmission {input:nola_application::ReduceAndCommitActiveSubmission {
        conversation_id:"parent".into(),run_id:active.run_id.clone(),invocation_config_digest:digest.clone(),
        transition_id:"interrupt-parent".into(),submission:AgentSubmission::Interrupt {
            goal_control: None, cancellation_id:"cancel-parent-terminal-receipt".into(),reason:Some("parent stopped".into())},
        committed_at_ms:700,
    }}).unwrap();
    let requests:nola_application::CommittedHostRequests=serde_json::from_value(source.execute(
        ApplicationCommand::ClaimCommittedHostRequests {input:nola_application::ClaimCommittedHostRequests {
            run_id:active.run_id.clone(),worker_generation:"cancel-generation".into(),lease_owner:"cancel-worker".into(),
            claimed_at_ms:701,lease_expires_at_ms:2000,limit:32,
        }}).unwrap()).unwrap();
    for request in requests.requests {
        let accept = |source: &mut support::ApplicationRuntime, sequence, payload| {
            source.execute(ApplicationCommand::ReduceAndAcceptHostResponse {input:nola_application::ReduceAndAcceptHostResponse {
                conversation_id:"parent".into(),run_id:active.run_id.clone(),invocation_config_digest:digest.clone(),received_at_ms:701+sequence as i64,
                response:HostResponseEnvelope {attempt_id:request.attempt_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),
                    request_id:request.request_id.clone(),effect_id:request.effect_id.clone(),host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),
                    session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),catalog_revision:request.catalog_revision,
                    availability_epoch:request.availability_epoch,delivery:request.delivery.clone(),sequence,payload},
            }}).unwrap();
        };
        accept(&mut source, 1, HostResponse::ExecutionStarted);
        let response = match &request.payload {
            HostRequest::Cancel { target_effect_id, .. } => HostResponse::CancellationConfirmed {target_effect_id:target_effect_id.clone()},
            HostRequest::CancelTask { task_id } => {
                use nola_domain::agent_task_admission::TaskCancellationExecution;
                let execution:TaskCancellationExecution=serde_json::from_value(source.execute(
                    ApplicationCommand::ExecuteTaskCancellationForEffect {request:request.clone(),now_ms:702}).unwrap()).unwrap();
                let TaskCancellationExecution::Transfer {authorization}=execution else {
                    panic!("source without global Task must export exact cancellation authority");
                };
                let requested=global.execute(ApplicationCommand::ImportHostTaskCancellation {authorization,now_ms:702}).unwrap();
                assert_eq!(requested,json!(true));
                assert_eq!(store.runtime_task(task_id).unwrap().unwrap().status,"running",
                    "requesting a stop is not proof the child terminated");
                HostResponse::Completed {output:json!({"task_id":task_id,"cancel_requested":true}),
                    usage:None,full_output_ref:None,artifacts:Vec::new()}
            },
            other => panic!("cancelled parent emitted unexpected new execution: {other:?}"),
        };
        accept(&mut source, 2, response);
    }
    assert!(source.execute(ApplicationCommand::ActiveRun {conversation_id:"parent".into()}).unwrap().is_null());
    let seal_query=||ApplicationCommand::LocalAssistantRunTerminalSeal {
        conversation_id:"parent".into(),run_id:admission.parent_run_id.clone(),
    };
    let seal=source.execute(seal_query()).unwrap();
    assert!(!seal.is_null(),"real parent cancellation must establish terminal evidence");
    // The actual worker acknowledges the earlier stop. A cancelled lease may
    // not publish a later "failed" finish; the Store rejects that stale path.
    assert!(store.finish_runtime_task_detailed_with_event(&lease,"cancelled",Some(7),Some("stopped"),None,None,None,true,
        "cancelled","cancelled",Some("stopped"),"{}",None,800).unwrap());
    let receipt=store.runtime_task_terminal_receipt("task").unwrap().unwrap();
    let fact=projection(&mut global,&receipt.receipt_id);
    let command=||ApplicationCommand::ImportHostAgentTaskTerminalProjection {projection:fact.clone()};
    let checkpoint_query=||ApplicationCommand::LoadCheckpoint {conversation_id:"parent".into()};
    let before:nola_application::ConversationCheckpoint=serde_json::from_value(source.execute(checkpoint_query()).unwrap()).unwrap();
    let ack=source.execute(command()).unwrap();
    assert_eq!(source.execute(command()).unwrap(),ack);
    assert_projected_state(&mut source,&receipt.receipt_id,TaskState::Cancelled);
    assert_eq!(source.execute(seal_query()).unwrap(),seal,"late task state never changes parent Run terminal fact");
    // Replaces the old fabricated idle ClaimTask fixture: a real trusted
    // projection commits and publishes without an external checkpoint effect.
    let after:nola_application::ConversationCheckpoint=serde_json::from_value(source.execute(checkpoint_query()).unwrap()).unwrap();
    assert!(after.revision>before.revision);
    assert!(after.checkpoint_json.len()<100_000);
    let state:serde_json::Value=serde_json::from_str(&after.checkpoint_json).unwrap();
    let pending=state.pointer("/state/host/pending_host_effects/effects").and_then(serde_json::Value::as_object).unwrap();
    assert!(pending.values().all(|effect|effect.pointer("/request/payload/type").and_then(serde_json::Value::as_str)!=Some("persist_checkpoint")));
    let heads:Vec<nola_domain::assistant_journal::AssistantJournalCursor>=serde_json::from_value(source.execute(
        ApplicationCommand::LocalRunAssistantJournalHeads {conversation_id:"parent".into(),run_id:admission.parent_run_id.clone(),after_message_id:None,limit:64},
    ).unwrap()).unwrap();
    assert!(!heads.is_empty());
    assert!(heads.iter().all(|head|head.identity.conversation_id=="parent" && head.identity.run_id==admission.parent_run_id));
}

#[test]
fn completed_worker_uses_global_guidance_authority_without_parent_checkpoint() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("guidance.sqlite");
    let admission=support::agent_task_admission::admitted(&path,None);
    let mut store=nola_store::NolaStore::open(&path).unwrap();
    assert!(store.load_checkpoint("parent").unwrap().is_none());
    let lease=RuntimeTaskLeaseWitness::from(&store.claim_runtime_task("task","worker","generation","token",200,1000).unwrap().unwrap());
    let mut runtime=support::open_runtime(&path).unwrap();
    let result=runtime.execute(ApplicationCommand::RuntimeTask {command:RuntimeTaskCommand::FinishDetailedWithEvent {
        final_observation: None,
        lease,status:"failed".into(),completion_watermark:Some(7),progress_summary:None,result_summary:None,
        error:None,output_path:None,unread:true,event_kind:"failed".into(),event_status:"failed".into(),event_summary:None,event_detail_json:"{}".into(),now_ms:300,
    }}).unwrap();
    assert_eq!(result,json!(true));
    assert!(store.runtime_task_terminal_receipt("task").unwrap().is_some());
    let error=runtime.execute(ApplicationCommand::SubmitTaskGuidance {
        task_id:"task".into(),message_id:"late".into(),parent_turn_id:admission.parent_turn_id,
        content:"too late".into(),submitted_at_ms:350,
    }).unwrap_err();assert!(error.contains("has not started a bound Run"),"{error}");
    assert!(store.runtime_task_guidance("task","late").unwrap().is_none());
}

#[test]
fn worker_terminal_rejects_missing_stale_and_future_completion_watermarks() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("watermark.sqlite");
    let lease=seed(&path,true).unwrap();let mut store=nola_store::NolaStore::open(&path).unwrap();
    let original=store.runtime_task_attempt(&lease.attempt_id).unwrap();
    for watermark in [None,Some(6),Some(8)] {
        assert!(store.finish_runtime_task_detailed_with_event(&lease,"failed",watermark,None,None,None,None,true,"failed","failed",None,"{}",None,300).is_err());
        assert_eq!(store.runtime_task_attempt(&lease.attempt_id).unwrap(),original);
        assert!(store.runtime_task_terminal_receipt("task").unwrap().is_none());
    }
    assert!(finish(&mut store,&lease));
}

#[test]
fn removed_parent_authority_commands_cannot_bypass_durable_agent_terminal_receipts() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("legacy-sweep.sqlite");
    seed(&path,true);let mut runtime=support::open_runtime(&path).unwrap();
    for kind in ["claim_task","heartbeat_task","resume_task_after_permission","recover_task_after_permission",
        "schedule_task_retry","sweep_expired_tasks","queue_task_guidance","acknowledge_task_guidance","resolve_task_guidance"] {
        let submission=json!({"type":kind,"operation_id":"bypass","task_id":"task","ownership_token":"token",
            "worker_id":"worker","now_epoch_ms":1100,"lease_until_epoch_ms":2100,"max_retry_count":0,
            "message_id":"late","parent_session_id":"actual-parent-core","parent_turn_id":"parent-turn","content":"bypass",
            "enqueued_at_epoch_ms":1100,"error":"expired","previous_ownership_token":"old","previous_ownership_tokens":["old"]});
        let error=serde_json::from_value::<AgentSubmission>(submission.clone()).unwrap_err().to_string();
        assert!(error.contains("unknown variant"),"{kind}: {error}");
        assert!(runtime.execute(ApplicationCommand::EnqueueAgentCommand {input:nola_application::AgentCommand {
            session_id:"parent".into(),command_id:"bypass".into(),payload_json:serde_json::to_string(&submission).unwrap(),created_at_ms:1100,
        }}).is_err());
    }
    let store=nola_store::NolaStore::open(&path).unwrap();assert_eq!(store.runtime_task("task").unwrap().unwrap().status,"running");
    assert!(store.runtime_task_terminal_receipt("task").unwrap().is_none());
}
