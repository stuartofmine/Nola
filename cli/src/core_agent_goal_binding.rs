// SPDX-License-Identifier: Apache-2.0
//! Host-private Goal descriptor adapter. Core owns the immutable Goal binding,
//! admission transaction, control state and round lifecycle.
use super::*;
use nola_application::ApplicationCommand;
use nola_application::goal_execution::GoalExecutionBinding;
use nola_domain::goal_execution::GoalModelSelection;


pub(super) fn start(
    request: &HostInvocationRequest,
    envelope: GoalRuntimeCommandEnvelope,
) -> Result<nola_core::agent::protocol::AgentControlSnapshot, String> {
    let session_id = request.session.session_id.as_deref().filter(|id| !id.trim().is_empty())
        .ok_or("Goal requires a durable conversation identity")?;
    if !request.session.save_session { return Err("Goal requires durable execution binding".into()); }
    let GoalRuntimeCommand::Start { id, .. } = &envelope.command else {
        return Err("Goal binding admission requires Start".into());
    };
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let existing: Option<GoalExecutionBinding> = application.execute(ApplicationCommand::LoadGoalExecutionBinding {
        conversation_id: session_id.into(), goal_id: id.clone(),
    })?;
    if let Some(bound) = &existing {
        if bound.identity.operation_id != envelope.operation_id {
            return Err("Goal Start operation differs from its immutable binding".into());
        }
        let projection = crate::session::load_session_goal_projection(session_id)?
            .ok_or("Goal Start binding has no committed projection")?;
        let receipt = projection.applied_operations.get(&envelope.operation_id)
            .ok_or("Goal Start has no committed operation receipt")?;
        if receipt.command_sha256 != nola_core::agent::goal::goal_command_digest(&envelope.command)? {
            return Err("Goal Start operation content changed".into());
        }
        if let Some(reason) = &receipt.rejection { return Err(reason.clone()); }
        // This is reconciliation of the original committed command, not a
        // new execution admission. Current defaults and unadmitted UI request
        // fields cannot replace or invalidate that immutable receipt.
        let execution: nola_core::agent::HostExecutionState = application.execute(
            ApplicationCommand::LoadHostExecutionState { conversation_id: session_id.into() })?;
        return Ok(execution.control);
    }
    let core_config=capture_start_configuration(request)?;
    let source_host_id=core_config.host_catalog.as_ref().ok_or("Goal Host catalog unavailable")?.host_id.0.clone();
    let invocation=core_config.turn_identity.host_invocation.as_ref().ok_or("Goal requires a frozen invocation")?;
    let selected=nola_application::GoalSelectedModel {profile:request.runtime.selected_profile.clone(),
        primary:invocation.primary.clone(),generation:invocation.generation.clone(),output_admission:invocation.output_admission.clone()};
    let result: serde_json::Value = application.execute(ApplicationCommand::StepGoal {
        conversation_id: session_id.into(), source_host_id,
        control: Some(envelope), start_configuration: Some(core_config), selected_model: Some(selected),
        user_prompt: String::new(), host_available: Some(true),
    })?;
    if let Some(error) = result["error"].as_str() { return Err(error.into()); }
    let execution: nola_core::agent::HostExecutionState = application.execute(
        ApplicationCommand::LoadHostExecutionState { conversation_id: session_id.into() })?;
    Ok(execution.control)
}

fn capture_start_configuration(request:&HostInvocationRequest)->Result<Box<AgentCoreConfig>,String> {
    let host = acquire_core_agent_host(request, Vec::new())?;
    let mut host = host.lock().map_err(|_| "Goal Host lock is poisoned")?;
    // Bind the same execution-tier surface used immediately before dispatch.
    // Host assembly may have promoted an explicit tool allowlist afterward.
    let settings = host.execution_config.host_execution_settings();
    host.runtime.effects.driver_mut().set_execution_settings(settings);
    let schemas = host.admission_tool_schemas.clone();
    let mut core_config = host.runtime.new_run_admission_configuration(&schemas)?;
    core_config.initial_items.clear();
    if core_config.automation_execution_spec.is_some() {
        return Err("Goal cannot reuse automation execution authority".into());
    }
    drop(host);
    // Assembly is not an execution owner. The admitted Run will restore its
    // exact frozen Host through the same path as every other step consumer.
    invalidate_core_agent_session(request.session.session_id.as_deref().ok_or("Goal requires a conversation")?)?;
    Ok(Box::new(core_config))
}

pub(super) fn restore(conversation_id: &str, goal_id: &str) -> Result<HostInvocationRequest, String> {
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let binding: GoalExecutionBinding = application.execute(
        ApplicationCommand::PrepareGoalExecution {conversation_id:conversation_id.into(),goal_id:goal_id.into()})?;
    if binding.identity.conversation_id != conversation_id || binding.identity.goal_id != goal_id {
        return Err("Goal execution owner changed".into());
    }
    let original = binding.template.core_config.turn_identity.host_invocation.as_ref()
        .ok_or("Goal execution binding has no Host invocation")?;
    let reference = binding.template.core_config.turn_identity.host_recovery.as_ref()
        .ok_or("Goal execution binding has no Host recovery descriptor")?;
    let mut request = host_recovery::restore(reference, conversation_id, original)?;
    if let nola_host_protocol::HostModelRouteBinding::Native(route) = &original.primary {
        // The logical selection remains immutable. The owning native adapter
        // freshly admits its physical instance; no old endpoint/key survives.
        let (endpoint, credential) = crate::desktop_host_control::resolve_goal_native_model_channel(goal_id, route)?;
        request.model_settings.endpoint = endpoint;
        request.model_settings.api_key = credential;
        request.model_settings.model = route.model_id.clone();
    }

    restore_host_memory_admission(&mut request, &binding.template.core_config.memory_policy)?;
    original.require_exact(&host_invocation_binding(&request)?).map_err(str::to_string)?;
    let profile = match binding.template.model_selection {
        GoalModelSelection::Profile {profile_id,revision} => Some(nola_host_protocol::HostModelProfileReference {
            profile_id,admitted_revision:revision,
        }),
        GoalModelSelection::DirectRoute => None,
    };
    if request.runtime.selected_profile != profile {
        return Err("Goal descriptor profile identity differs from Core binding".into());
    }
    request.session.goal_execution_id = Some(goal_id.into());
    request.session.host_submission_id = None;
    request.session.host_user_message_id = None;
    request.session.host_assistant_message_id = None;
    Ok(request)
}

fn validated_binding(conversation_id: &str, goal_id: &str, request: &HostInvocationRequest)
    -> Result<GoalExecutionBinding, String> {
    let binding: GoalExecutionBinding = crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::PrepareGoalExecution {conversation_id:conversation_id.into(),goal_id:goal_id.into()})?;
    let expected = binding.template.core_config.turn_identity.host_invocation.as_ref()
        .ok_or("Goal has no frozen Host invocation")?;
    expected.require_exact(&host_invocation_binding(request)?).map_err(str::to_string)?;
    let selected = match &request.runtime.selected_profile {
        Some(profile) => GoalModelSelection::Profile {profile_id:profile.profile_id.clone(),revision:profile.admitted_revision},
        None => GoalModelSelection::DirectRoute,
    };
    if selected != binding.template.model_selection { return Err("Goal Run profile differs from its original selection".into()); }
    Ok(binding)
}

pub(super) fn validate_round(conversation_id: &str, goal_id: &str, request: &HostInvocationRequest) -> Result<(), String> {
    validated_binding(conversation_id, goal_id, request).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Environment {
        previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
        conversation: String,
        _root: tempfile::TempDir,
    }
    impl Drop for Environment {
        fn drop(&mut self) {
            let _ = invalidate_core_agent_session(&self.conversation);
            for (key,value) in &self.previous {
                match value {Some(value)=>std::env::set_var(key,value),None=>std::env::remove_var(key)}
            }
        }
    }
    // Start now includes full Core admission. Like the existing permission
    // recovery fixture, nested debug Host/Application frames need an explicit test stack.
    fn run_fixture(run:fn()) {
        std::thread::Builder::new().name("goal-step-fixture".into()).stack_size(16*1024*1024)
            .spawn(run).unwrap().join().unwrap_or_else(|panic|std::panic::resume_unwind(panic));
    }
    fn fixture() -> (Environment, HostInvocationRequest) {
        let root=tempfile::tempdir().unwrap();
        let previous=["NOLA_HOME","NOLA_DB_PATH"].into_iter().map(|key|(key,std::env::var_os(key))).collect();
        std::env::set_var("NOLA_HOME",root.path());
        std::env::set_var("NOLA_DB_PATH",root.path().join("goals.sqlite"));
        let conversation=format!("goal-binding-{}",uuid::Uuid::new_v4());
        let mut request=HostInvocationRequest::for_test().unwrap();
        request.session.session_id=Some(conversation.clone());request.session.save_session=true;
        request.tool_settings.project_root=root.path().into();request.tool_settings.tools_enabled=false;
        request.model_settings.endpoint="https://goal-original.invalid/v1".into();
        request.model_settings.model="goal-original-model".into();
        request.model_settings.api_key="fixture-goal-secret".into();
        request.runtime.memory_enabled=Some(false);request.runtime.selected_profile=None;
        (Environment {previous,conversation,_root:root},request)
    }
    #[test]
    fn goal_binding_exists_before_first_run_and_cold_restore_never_selects_new_defaults() { run_fixture(goal_binding_exists_before_first_run_and_cold_restore_never_selects_new_defaults_inner); }
    fn goal_binding_exists_before_first_run_and_cold_restore_never_selects_new_defaults_inner() {
        let _lock=crate::store::test_env_lock();
        let (fixture,request)=fixture();
        let envelope=nola_application::goal_coordinator::create_goal_command("binding-start","Finish the work","",crate::goal_runtime::goal_now_epoch_ms());
        start(&request,envelope.clone()).unwrap();
        let mut app=crate::application_port::CliApplicationPort::persistent_default().unwrap();
        assert!(app.active_run(&fixture.conversation).unwrap().is_some());
        let load=||ApplicationCommand::LoadGoalExecutionBinding {conversation_id:fixture.conversation.clone(),goal_id:"goal-binding-start".into()};
        let before:GoalExecutionBinding=app.execute::<Option<GoalExecutionBinding>>(load()).unwrap().unwrap();
        start(&request,envelope).unwrap();
        let after:GoalExecutionBinding=app.execute::<Option<GoalExecutionBinding>>(load()).unwrap().unwrap();
        assert_eq!(before.binding_sha256,after.binding_sha256,"replaying Start cannot create a new vault descriptor");
        invalidate_core_agent_session(&fixture.conversation).unwrap();
        let restored=restore(&fixture.conversation,"goal-binding-start").unwrap();
        assert_eq!(restored.model_settings.endpoint,"https://goal-original.invalid/v1");
        assert_eq!(restored.model_settings.model,"goal-original-model");
        assert_eq!(restored.model_settings.api_key,"fixture-goal-secret");
        let again=restore(&fixture.conversation,"goal-binding-start").unwrap();
        assert_eq!(again.model_settings.endpoint,restored.model_settings.endpoint);
        assert!(restore(&fixture.conversation,"different-goal").is_err());
        assert!(restore("different-conversation","goal-binding-start").is_err());
    }
    #[test]
    fn revoked_goal_credential_blocks_execution_but_not_idle_pause() { run_fixture(revoked_goal_credential_blocks_execution_but_not_idle_pause_inner); }
    fn revoked_goal_credential_blocks_execution_but_not_idle_pause_inner() {
        let _lock=crate::store::test_env_lock();
        let (fixture,request)=fixture();
        start(&request,nola_application::goal_coordinator::create_goal_command("revoke-start","Finish the work","",crate::goal_runtime::goal_now_epoch_ms())).unwrap();
        let binding:GoalExecutionBinding=crate::application_port::CliApplicationPort::persistent_default().unwrap()
            .execute::<Option<GoalExecutionBinding>>(ApplicationCommand::LoadGoalExecutionBinding {
                conversation_id:fixture.conversation.clone(),goal_id:"goal-revoke-start".into()}).unwrap().unwrap();
        let store=crate::store::NolaStore::open_default().unwrap();
        let mut vault=crate::host_credentials::HostCredentialVault::open(store.path()).unwrap();
        let reference=binding.template.core_config.turn_identity.host_recovery.unwrap();
        let descriptor:serde_json::Value=serde_json::from_str(&vault.runtime_descriptor(&reference.handle).unwrap().unwrap()).unwrap();
        let handle=descriptor["credentials"][0].as_str().unwrap();
        let record=vault.record(handle).unwrap().unwrap();
        vault.write(handle,&record.identity,record.revision,None,||Ok(())).unwrap();
        assert!(restore(&fixture.conversation,"goal-revoke-start").is_err());
        let projection=crate::session::load_session_goal_projection(&fixture.conversation).unwrap().unwrap();
        let control=GoalRuntimeCommandEnvelope {operation_id:"pause-after-revoke".into(),command:GoalRuntimeCommand::GuardedControl {
            expected_goal_id:"goal-revoke-start".into(),expected_control_token:projection.control_token().unwrap(),
            control:Box::new(GoalRuntimeCommand::Pause {status:nola_core::agent::GoalStatus::Paused,wake_at_epoch_ms:None,now_epoch_ms:crate::goal_runtime::goal_now_epoch_ms()}),
        }};
        crate::goal_control_port::apply_idle(&fixture.conversation,control).unwrap();
        assert_eq!(crate::session::load_session_goal_projection(&fixture.conversation).unwrap().unwrap().active.unwrap().status,
            nola_core::agent::GoalStatus::Paused);
    }
    #[test]
    fn native_control_retries_reuse_receipt_without_touching_new_control_generation() { run_fixture(native_control_retries_reuse_receipt_without_touching_new_control_generation_inner); }
    fn native_control_retries_reuse_receipt_without_touching_new_control_generation_inner() {
        use nola_application::goal_control_intent::{GoalControlAction, PrepareGoalControlIntent};
        let _lock = crate::store::test_env_lock();
        let (fixture, request) = fixture();
        start(&request, nola_application::goal_coordinator::create_goal_command("retry-goal", "Finish", "",
            crate::goal_runtime::goal_now_epoch_ms())).unwrap();
        let projection = crate::session::load_session_goal_projection(&fixture.conversation).unwrap().unwrap();
        let pause = PrepareGoalControlIntent { conversation_id: fixture.conversation.clone(),
            operation_id: "native-pause-fixed".into(), action: GoalControlAction::Pause,
            expected_goal_id: Some("goal-retry-goal".into()), expected_control_token: projection.control_token(),
            objective: String::new(), constraints: Vec::new(), wake_at_epoch_ms: None };
        assert!(!crate::goal_control_port::submit_existing(pause.clone()).unwrap());
        crate::core_agent_runtime::drain_parent_agent_task_inbox(&fixture.conversation).unwrap();
        let paused = crate::session::load_session_goal_projection(&fixture.conversation).unwrap().unwrap();
        assert!(crate::goal_control_port::submit_existing(pause.clone()).unwrap());
        let repeated = crate::session::load_session_goal_projection(&fixture.conversation).unwrap().unwrap();
        assert_eq!(paused.revision, repeated.revision);
        let mut conflicting = pause;
        conflicting.action = GoalControlAction::Cancel;
        assert!(crate::goal_control_port::submit_existing(conflicting).is_err());
        assert_eq!(crate::session::load_session_goal_projection(&fixture.conversation).unwrap().unwrap().revision,
            repeated.revision);
        let mut app = crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let observation: serde_json::Value = app.execute(ApplicationCommand::LoadGoalControlObservation {
            conversation_id: fixture.conversation.clone() }).unwrap();
        assert_eq!(observation["control_token"], repeated.control_token().unwrap());
        let foreign: serde_json::Value = app.execute(ApplicationCommand::LoadGoalExecutionOwner {
            conversation_id: fixture.conversation.clone(), goal_id: "other-goal".into(), run_id: "other-run".into() }).unwrap();
        assert!(foreign.is_null());
    }

    #[test]
    fn competing_sealed_round_finishes_in_same_goal_cycle() { run_fixture(competing_sealed_round_finishes_in_same_goal_cycle_inner); }
    fn competing_sealed_round_finishes_in_same_goal_cycle_inner() {
        use crate::goal_runtime::{GoalRoundDriver,PromptRunResult};
        use nola_core::agent::host::{HostRequest,HostResponse,HostResponseEnvelope};
        let _lock=crate::store::test_env_lock();
        let (fixture,mut request)=fixture();
        struct OtherConsumer {calls:usize}
        impl GoalRoundDriver for OtherConsumer {
            fn resume(&mut self,request:&HostInvocationRequest,run:&str)->Result<Option<PromptRunResult>,String> {
                self.calls+=1;
                assert_eq!(self.calls,1,"a sealed Run must not be dispatched again");
                let owner=request.session.session_id.as_ref().unwrap();
                let mut app=crate::application_port::CliApplicationPort::persistent_default()?;
                let now=crate::goal_runtime::goal_now_epoch_ms();
                let worker=crate::process_observation::current_worker_identity()?.lease_identity()?;
                let physical=app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
                    run_id:run.into(),worker_generation:worker.worker_generation,lease_owner:worker.lease_owner,
                    claimed_at_ms:now,lease_expires_at_ms:now+60000,limit:1,
                })?.requests.into_iter().next().unwrap();
                let HostRequest::ModelStream {text_part_id,..}=&physical.payload else {panic!("model fixture")};
                let digest=app.active_run(owner)?.unwrap().invocation_config_digest.unwrap();
                for (offset,payload) in [HostResponse::ExecutionStarted,
                    HostResponse::ModelAttemptStarted {provider:"fixture".into(),model:"goal-original-model".into(),reason:"initial".into()},
                    HostResponse::ModelCompleted {items:vec![serde_json::to_value(AgentItem::assistant_message(
                        ItemId::from_string(text_part_id.clone()),"Fixture answer delivered")).unwrap()],tool_calls:vec![],usage:serde_json::json!({})},
                ].into_iter().enumerate() {
                    app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                        conversation_id:owner.clone(),run_id:run.into(),invocation_config_digest:digest.clone(),received_at_ms:now+offset as i64,
                        response:HostResponseEnvelope {request_id:physical.request_id.clone(),attempt_id:physical.attempt_id.clone(),
                            delegated_invocation_id:physical.delegated_invocation_id.clone(),effect_id:physical.effect_id.clone(),host_id:physical.host_id.clone(),
                            lease_id:physical.lease_id.clone(),session_id:physical.session_id.clone(),turn_id:physical.turn_id.clone(),catalog_revision:physical.catalog_revision,
                            availability_epoch:physical.availability_epoch,delivery:physical.delivery.clone(),sequence:physical.resume_after_sequence.unwrap_or(0)+1+offset as u64,payload},
                    })?;
                }
                assert!(app.active_run(owner)?.is_none());
                let projection=crate::session::load_session_goal_projection(owner)?.unwrap();
                let goal=projection.active.as_ref().unwrap();
                let contract=goal.contracts.last().unwrap().id.clone();
                let report=nola_core::agent::goal::GoalExecutionReport {contract_id:contract.clone(),summary:"Fixture answer delivered".into(),
                    claimed_record_ids:goal.requirements.keys().cloned().collect(),artifact_refs:vec!["response".into()],blockers:vec![],reported_at_epoch_ms:now+4};
                let mut submit=|operation:&str,command:GoalRuntimeCommand|->Result<(),String>{
                    let execution:nola_core::agent::HostExecutionState=app.execute(ApplicationCommand::LoadHostExecutionState {conversation_id:owner.clone()})?;
                    let _:serde_json::Value=app.execute(ApplicationCommand::ReduceAndCommitIdleSubmission {input:nola_application::ReduceAndCommitIdleSubmission {
                        conversation_id:owner.clone(),core_config:execution.configuration,committed_at_ms:now+5,
                        submission:AgentSubmission::GoalRuntime {envelope:GoalRuntimeCommandEnvelope {operation_id:operation.into(),command}},
                    }})?;Ok(())
                };
                submit("other-consumer-report",GoalRuntimeCommand::RecordExecution {report})?;
                let projection=crate::session::load_session_goal_projection(owner)?.unwrap();
                let claims=projection.active.as_ref().unwrap().requirements.values().map(|requirement|nola_core::agent::GoalCompletionClaim {
                    requirement_id:requirement.id.clone(),requirement_revision:requirement.semantic_revision,summary:"Fixture response delivered".into(),
                    evidence_refs:vec!["response".into()],execution_evidence_refs:vec![],task_replacements:vec![],
                }).collect();
                submit("other-consumer-accept",GoalRuntimeCommand::AcceptCompletion {expected_revision:projection.revision,contract_id:contract,claims,now_epoch_ms:now+6})?;
                let mut output=crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_|{});
                assert!(crate::core_agent_runtime::resume_frozen_core_agent_run(owner,run,&mut output,crate::runtime_control::RuntimeExecutionControl::default())?.is_none());
                Ok(None)
            }
        }
        let mut driver=OtherConsumer {calls:0};
        let result=crate::goal_runtime::run_goal_cycle(&mut request,"Deliver a fixture answer","Deliver a fixture answer","",&mut driver).unwrap();
        assert!(matches!(result,crate::goal_runtime::GoalCycleOutcome::Completed(_)));
        assert_eq!(driver.calls,1);
        let projection=crate::session::load_session_goal_projection(&fixture.conversation).unwrap().unwrap();
        assert_eq!(projection.active.unwrap().contracts.len(),1);
    }

}
