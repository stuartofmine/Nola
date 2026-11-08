// SPDX-License-Identifier: Apache-2.0
//! Actual ephemeral source and independent durable task authority.
mod support;
use nola_application::{ApplicationCommand, CommittedHostRequests};
use nola_agent_engine::agent::{AgentSubmission, host::{HostResponse, HostResponseEnvelope}};
use nola_domain::agent_task_admission::TaskCancellationExecution;
use serde_json::json;

#[test]
fn ephemeral_source_cancel_transfers_only_exact_started_control_to_task_store() {
    let directory = tempfile::tempdir().unwrap();
    let mut source = support::open_runtime(":memory:").unwrap();
    let admission = support::agent_task_admission::source_fact(&mut source);
    let mut target = support::open_runtime(&directory.path().join("tasks.sqlite")).unwrap();
    support::agent_task_admission::draft(&mut target, None);
    target.execute(ApplicationCommand::ImportHostAgentTaskAdmission { admission: admission.clone() }).unwrap();
    let active: nola_application::ActiveRun = serde_json::from_value(source.execute(
        ApplicationCommand::ActiveRun { conversation_id: "parent".into() }).unwrap()).unwrap();
    let digest = active.invocation_config_digest.unwrap();
    source.execute(ApplicationCommand::ReduceAndCommitActiveSubmission { input:nola_application::ReduceAndCommitActiveSubmission {
        conversation_id:"parent".into(), run_id:active.run_id.clone(), invocation_config_digest:digest.clone(),
        transition_id:"request-child-stop".into(), submission:AgentSubmission::RequestTaskCancellation {task_id:"task".into()}, committed_at_ms:1100,
    }}).unwrap();
    let controls:CommittedHostRequests = serde_json::from_value(source.execute(ApplicationCommand::ClaimApplicationControlRequests {
        input:nola_application::ClaimCommittedHostRequests {run_id:active.run_id.clone(),worker_generation:"control-generation".into(),
            lease_owner:"control-owner".into(),claimed_at_ms:1200,lease_expires_at_ms:100000,limit:64}
    }).unwrap()).unwrap();
    assert_eq!(controls.requests.len(),1);
    let request=controls.requests[0].clone();
    assert!(source.execute(ApplicationCommand::ExecuteTaskCancellationForEffect {request:request.clone(),now_ms:1250}).is_err());
    source.execute(ApplicationCommand::ReduceAndAcceptHostResponse {input:nola_application::ReduceAndAcceptHostResponse {
        conversation_id:"parent".into(),run_id:active.run_id.clone(),invocation_config_digest:digest,received_at_ms:1300,
        response:HostResponseEnvelope {attempt_id:request.attempt_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),
            request_id:request.request_id.clone(),effect_id:request.effect_id.clone(),host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),
            session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),catalog_revision:request.catalog_revision,
            availability_epoch:request.availability_epoch,delivery:request.delivery.clone(),sequence:0,payload:HostResponse::ExecutionStarted},
    }}).unwrap();
    let exported = source.execute(ApplicationCommand::ExecuteTaskCancellationForEffect {request:request.clone(),now_ms:1301}).unwrap();
    let TaskCancellationExecution::Transfer {authorization} = serde_json::from_value(exported).unwrap() else {panic!("source must not invent a local task row")};
    assert_eq!(authorization.admission,admission);
    let mut foreign=request.clone(); foreign.delivery.as_mut().unwrap().lease_token.push_str("foreign");
    assert!(source.execute(ApplicationCommand::ExecuteTaskCancellationForEffect {request:foreign,now_ms:1301}).is_err());
    let mut wrong=authorization.clone();wrong.admission.parent_run_id="another-run".into();
    assert!(target.execute(ApplicationCommand::ImportHostTaskCancellation {authorization:wrong,now_ms:1302}).is_err());
    let mut mismatched = authorization.clone();
    mismatched.admission.parent_run_id = "different-parent-run".into();
    mismatched.admission = mismatched.admission.seal().unwrap();
    mismatched.control_attempt.run_id = "different-parent-run".into();
    mismatched.validate().unwrap();
    assert!(target.execute(ApplicationCommand::ImportHostTaskCancellation {authorization:mismatched,now_ms:1302}).is_err(),
        "A structurally valid transfer cannot replace the destination's immutable source authority");
    assert!(target.execute(ApplicationCommand::ImportHostTaskCancellation {authorization:authorization.clone(),now_ms:40000}).is_err());
    // The trusted variant must not be reachable from general Application JSON.
    assert!(serde_json::from_value::<ApplicationCommand>(json!({"type":"import_host_task_cancellation","authorization":authorization,"now_ms":1302})).is_err());
    assert_eq!(target.execute(ApplicationCommand::ImportHostTaskCancellation {authorization:authorization.clone(),now_ms:1302}).unwrap(),json!(true));
    // Return the durable intent rather than the second UPDATE's false changed bit.
    assert_eq!(target.execute(ApplicationCommand::ImportHostTaskCancellation {authorization,now_ms:1303}).unwrap(),json!(true));
}


#[test]
fn owned_task_cancel_uses_immutable_parent_without_requiring_current_parent_run() {
    use nola_application::{owned_task_cancellation::OwnedTaskCancellation, RuntimeTaskCommand};
    let mut source = support::open_runtime(":memory:").unwrap();
    let admission = support::agent_task_admission::source_fact(&mut source);
    let mut target = support::open_runtime(":memory:").unwrap();
    support::agent_task_admission::draft(&mut target, None);
    target.execute(ApplicationCommand::ImportHostAgentTaskAdmission { admission: admission.clone() }).unwrap();
    // This global task Store has no parent active Run; authority is immutable,
    // so a later parent Run must not receive this cancellation as its FIFO work.
    let owner = OwnedTaskCancellation {parent_conversation_id:admission.parent_conversation_id.clone(),
        parent_run_id:admission.parent_run_id.clone(), task_id:admission.task_id.clone()};
    let before = target.execute(ApplicationCommand::RuntimeTask {command:RuntimeTaskCommand::ReadOwnedCancellation {input:owner.clone()}}).unwrap();
    assert_eq!(before["cancel_requested"], false);
    assert_eq!(target.execute(ApplicationCommand::InspectOwnedTaskRecovery {owner:owner.clone()}).unwrap(), serde_json::Value::Null);
    let mut foreign = owner.clone(); foreign.parent_run_id = "successor-run".into();
    assert!(target.execute(ApplicationCommand::InspectOwnedTaskRecovery {owner:foreign.clone()}).is_err());
    assert!(target.execute(ApplicationCommand::RuntimeTask {command:RuntimeTaskCommand::RequestOwnedCancellation {input:foreign, now_ms:1200}}).is_err());
    let requested = target.execute(ApplicationCommand::RuntimeTask {command:RuntimeTaskCommand::RequestOwnedCancellation {input:owner.clone(), now_ms:1201}}).unwrap();
    assert_eq!(requested["cancel_requested"], true);
    // A cancellation flag/Task terminal cannot manufacture an admitted child Run.
    assert!(target.execute(ApplicationCommand::InspectOwnedTaskRecovery {owner:owner.clone()}).is_err());
    let repeated = target.execute(ApplicationCommand::RuntimeTask {command:RuntimeTaskCommand::RequestOwnedCancellation {input:owner.clone(), now_ms:1202}}).unwrap();
    assert_eq!(requested, repeated);
    assert_eq!(repeated, target.execute(ApplicationCommand::RuntimeTask {command:RuntimeTaskCommand::ReadOwnedCancellation {input:owner}}).unwrap());
}
