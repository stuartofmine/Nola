// SPDX-License-Identifier: Apache-2.0
use nola_core::agent::{AgentCore,AgentCoreConfig,AgentSubmission,SessionId,TaskDurability,TaskKind,TaskState,TurnId};
use nola_core::agent::task::{TaskHandle,TaskTerminalReconciliation};
use nola_core::agent::recovery::CheckpointReason;
#[path = "support/model_identity.rs"]
mod model_identity;

fn config()->AgentCoreConfig {AgentCoreConfig {turn_identity:model_identity::host_identity(),session_id:Some(SessionId::from_string("parent".into())),..Default::default()}}
fn fixture(owner:Option<(&str,&str)>)->AgentCore {
    let mut cp=AgentCore::try_new(config()).unwrap().checkpoint(CheckpointReason::Manual);
    cp.state.task_projection.order.push("task".into());
    cp.state.task_projection.tasks.insert("task".into(),TaskHandle {
        id:"task".into(),owner_session_id:SessionId::from_string("parent".into()),owner_turn_id:TurnId::from_string("turn".into()),
        kind:TaskKind::Agent,state:if owner.is_some(){TaskState::Running}else{TaskState::Queued},progress:None,current_step:None,result_ref:None,
        cancel_requested:false,cancel_requested_at_epoch_ms:None,lineage:serde_json::from_value(serde_json::json!({"child_session_id":"child","parent_call_id":"call"})).unwrap(),
        durability:TaskDurability {worker_id:owner.map(|(_,worker)|worker.into()),ownership_token:owner.map(|(token,_)|token.into()),
            lease_until_epoch_ms:owner.map(|_|100),input_watermark:Some(7),..Default::default()},
    });
    AgentCore::restore(config(),cp).unwrap()
}
fn fact()->TaskTerminalReconciliation {
    // Pure reducer fixture: this sealed identity is not an authenticated export.
    // Application tests prove original source admission across Store boundaries.
    let run_result: nola_domain::run_result::RunResult = serde_json::from_value(serde_json::json!({
        "result_id":"child-result","run_id":"child-run","conversation_id":"child",
        "originating_submission_id":"child-input","input":{"watermark":"1","applications_digest":"00".repeat(32)},
        "decided_at_revision":"4","decision_event_cursor":"5","requirement_revision":"0","evidence_frontier":"5",
        "value":{"kind":"answered","answer_part_refs":["answer"]}
    })).unwrap();
    TaskTerminalReconciliation {protocol_version:1,authority_revision:2,ended_at_ms:100,
        result_digest:Some(run_result.computed_digest().unwrap()),run_result:Some(run_result),
        admission:nola_domain::agent_task_admission::AgentTaskAdmission {
            predecessor_task_id: None,
            protocol_version:nola_domain::agent_task_admission::AGENT_TASK_ADMISSION_VERSION,task_id:"task".into(),parent_conversation_id:"conversation".into(),
            parent_core_session_id:"parent".into(),parent_run_id:"run".into(),parent_turn_id:"turn".into(),
            child_conversation_id:"child".into(),parent_tool_call_id:"call".into(),parent_effect_id:"effect".into(),
            input_watermark:Some(7),source_store_epoch:"epoch".into(),source_transition_id:"transition".into(),
            source_host_event_id:"host-event".into(),source_checkpoint_revision:1,core_revision:1,
            core_event_cursor:1,admission_digest:String::new(),
        }.seal().unwrap(),receipt_id:"receipt".into(),input_watermark:Some(7),completion_watermark:Some(7),
        guidance_results:vec![],
        status:"sealed".into(),result_ref:Some("result".into())}
}

fn unbound_cancellation_fact()->TaskTerminalReconciliation {
    // Store-authorized cancellation before child Run admission has no RunResult.
    // A bound child's cancellation instead remains sealed with a typed outcome.
    let mut cancelled=fact();
    cancelled.status="cancelled".into();
    cancelled.run_result=None;
    cancelled.result_digest=None;
    cancelled.result_ref=None;
    cancelled
}

#[test]
fn sealed_task_preserves_each_business_outcome_without_claiming_parent_acceptance() {
    use nola_domain::run_result::{RunResultReason, RunResultValue};
    for value in [
        RunResultValue::Answered { answer_part_refs: vec!["answer".into()] },
        RunResultValue::ActionCompleted { requirement_acceptance_refs: vec!["assessment".into()] },
        RunResultValue::Incomplete { reason: RunResultReason { code: "remaining".into(), detail: None }, remaining_requirement_refs: vec!["requirement".into()], obligation_refs: vec![] },
        RunResultValue::Blocked { reason: RunResultReason { code: "dependency".into(), detail: None }, blocking_fact_refs: vec!["dependency".into()], resolution_condition: "dependency becomes available".into() },
        RunResultValue::Cancelled { cancellation_id: "cancel".into(), observed_effect_refs: vec![], unknown_effect_refs: vec![] },
        RunResultValue::Failed { error_ref: "failure".into(), observed_effect_refs: vec![], unknown_effect_refs: vec![] },
    ] {
        let mut core = fixture(None);
        let mut source = fact();
        source.run_result.as_mut().unwrap().value = value.clone();
        source.run_result.as_mut().unwrap().requirement_revision = nola_domain::assistant_journal_operations::JournalNumber(1);
        source.result_digest = Some(source.run_result.as_ref().unwrap().computed_digest().unwrap());
        let committed = core.submit(AgentSubmission::ReconcileTaskTerminal {
            operation_id: "terminal:receipt".into(), fact: source.clone(),
        }).unwrap();
        let task = &committed.snapshot.tasks.tasks["task"];
        assert_eq!(task.state, TaskState::Sealed);
        assert_eq!(task.durability.run_result, source.run_result);
        assert_eq!(task.durability.result_digest, source.result_digest);
        assert_ne!(task.progress, Some(1.0), "settlement does not attest business progress");
        let before = core.snapshot().tasks;
        let mut changed = source.clone();
        changed.run_result.as_mut().unwrap().result_id = "changed".into();
        assert!(core.submit(AgentSubmission::ReconcileTaskTerminal { operation_id: "tampered".into(), fact: changed }).is_err());
        assert_eq!(core.snapshot().tasks, before);
        source.run_result = None;
        source.result_digest = None;
        assert!(core.submit(AgentSubmission::ReconcileTaskTerminal { operation_id: "missing".into(), fact: source }).is_err());
        assert_eq!(core.snapshot().tasks, before);
    }
}

#[test]
fn terminal_projection_does_not_require_parent_lease_mirroring_and_survives_restore() {
    for owner in [("original","worker-original"),("successor","worker-successor")] {
        let mut core=fixture(Some(owner));
        let op=AgentSubmission::ReconcileTaskTerminal {operation_id:"terminal:receipt".into(),fact:fact()};
        let result=core.submit(op.clone()).unwrap();
        assert_eq!(result.snapshot.tasks.tasks["task"].state,TaskState::Sealed);
        core=AgentCore::restore(config(),core.checkpoint(CheckpointReason::Manual)).unwrap();
        assert_eq!(core.submit(op).unwrap().snapshot.tasks,result.snapshot.tasks);
        let mut different=fact();different.receipt_id="other-receipt".into();
        assert!(core.submit(AgentSubmission::ReconcileTaskTerminal {operation_id:"other".into(),fact:different}).is_err());
    }
}

#[test]
fn terminal_receipt_cannot_borrow_a_foreign_owner_or_watermark() {
    for field in ["session","turn","child","call"] {
        let mut core=fixture(None);let before=core.snapshot().tasks;
        let mut wrong=fact();
        match field {
            "session"=>wrong.admission.parent_core_session_id="foreign".into(),
            "turn"=>wrong.admission.parent_turn_id="foreign".into(),
            "child"=>wrong.admission.child_conversation_id="foreign".into(),
            _=>wrong.admission.parent_tool_call_id="foreign".into(),
        }
        wrong.admission=wrong.admission.seal().unwrap();
        assert!(core.submit(AgentSubmission::ReconcileTaskTerminal {operation_id:"terminal:receipt".into(),fact:wrong}).is_err());
        assert_eq!(core.snapshot().tasks,before);
    }
    let mut core=fixture(Some(("original","worker-original")));let mut wrong=fact();wrong.input_watermark=Some(8);
    assert!(core.submit(AgentSubmission::ReconcileTaskTerminal {operation_id:"wrong-watermark".into(),fact:wrong}).is_err());
}

#[test]
fn authenticated_cancellation_projection_needs_no_parent_attempt() {
    let mut core=fixture(None);let cancelled=unbound_cancellation_fact();
    let result=core.submit(AgentSubmission::ReconcileTaskTerminal {operation_id:"cancel".into(),fact:cancelled}).unwrap();
    assert_eq!(result.snapshot.tasks.tasks["task"].state,TaskState::Cancelled);
}

#[test]
fn committed_completion_wins_over_later_parent_cancellation_without_erasing_history() {
    let core=fixture(Some(("original","worker-original")));
    let mut checkpoint=core.checkpoint(CheckpointReason::Manual);
    let task=checkpoint.state.task_projection.tasks.get_mut("task").unwrap();
    task.cancel_requested=true;task.cancel_requested_at_epoch_ms=Some(200);
    let mut core=AgentCore::restore(config(),checkpoint).unwrap();
    let result=core.submit(AgentSubmission::ReconcileTaskTerminal {operation_id:"terminal:receipt".into(),fact:fact()}).unwrap();
    let task=&result.snapshot.tasks.tasks["task"];
    assert_eq!(task.state,TaskState::Sealed);assert!(task.cancel_requested);
    assert_eq!(task.cancel_requested_at_epoch_ms,Some(200));
}

#[test]
fn store_authorized_cancel_can_settle_parent_before_its_durable_claim_was_applied() {
    let mut core=fixture(None);let cancelled=unbound_cancellation_fact();
    assert_eq!(core.submit(AgentSubmission::ReconcileTaskTerminal {operation_id:"cancel-before-claim".into(),fact:cancelled}).unwrap()
        .snapshot.tasks.tasks["task"].state,TaskState::Cancelled);
}

#[test]
fn authoritative_cancel_settles_projected_states_but_never_foreign_watermarks() {
    for state in [TaskState::Waiting,TaskState::Suspended,TaskState::Running] {
        let mut cp=fixture(None).checkpoint(CheckpointReason::Manual);
        cp.state.task_projection.tasks.get_mut("task").unwrap().state=state;
        let mut core=AgentCore::restore(config(),cp).unwrap();let cancelled=unbound_cancellation_fact();
        assert_eq!(core.submit(AgentSubmission::ReconcileTaskTerminal {operation_id:"cancel".into(),fact:cancelled}).unwrap().snapshot.tasks.tasks["task"].state,TaskState::Cancelled);
    }
    let mut core=fixture(None);let mut cancelled=unbound_cancellation_fact();
    cancelled.input_watermark=Some(99);
    assert!(core.submit(AgentSubmission::ReconcileTaskTerminal {operation_id:"cancel".into(),fact:cancelled}).is_err());
}

#[test]
fn unclaimed_cancel_initializes_only_an_unassigned_input_version() {
    let mut cp=fixture(None).checkpoint(CheckpointReason::Manual);
    cp.state.task_projection.tasks.get_mut("task").unwrap().durability.input_watermark=None;
    let mut core=AgentCore::restore(config(),cp).unwrap();let cancelled=unbound_cancellation_fact();
    let result=core.submit(AgentSubmission::ReconcileTaskTerminal {operation_id:"cancel-initial".into(),fact:cancelled}).unwrap();
    assert_eq!(result.snapshot.tasks.tasks["task"].durability.input_watermark,Some(7));
    assert_eq!(result.snapshot.tasks.tasks["task"].durability.completion_watermark,Some(7));
}
