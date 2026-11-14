// SPDX-License-Identifier: Apache-2.0
//! Stateful Application object used by embedded platform adapters.
//!
//! A platform owns this runtime, while the runtime exclusively owns the Store.
//! Commands and product projections are the only public communication surface.

use nola_agent_engine::agent::host::HostResponseEnvelope;
use nola_agent_engine::agent::{AgentCore, AgentEvent, AgentSubmission};
#[cfg(test)]
use nola_store::NolaStore;
use nola_store_port::protocol_journal::CommandTransactionRepository;
use nola_store_port::run_effect::RunEffectRepository;
use serde_json::Value;

use crate::runtime_supervisor::{RetainHotRun, RuntimeSupervisor};
use crate::{
    execute_application_command, load_committed_host_requests,
    reduce_and_accept_host_response_with_core, reduce_and_commit_active_submission_with_core,
    ApplicationCommand, CommittedHostRequests,
};

pub struct ApplicationRuntime<R> {
    pub(crate) store: R,
    pub(crate) content_upload_store:
        Box<dyn nola_store_port::content_upload::ContentUploadRepository<Error = String> + Send>,
    supervisor: RuntimeSupervisor,
}

// Keep crash-recovery checkpoints bounded without turning every provider
// delta into a SQLite transaction. This is deliberately a byte budget rather
// than a timer so deterministic hosts and tests observe identical behavior.
const STREAM_COMMIT_BYTE_BUDGET: usize = 64 * 1024;

impl<R> ApplicationRuntime<R> {
    /// Storage adapters are opened by the owning Service composition, never by Application.
    /// Every repository capability belongs to this same transaction owner.
    pub fn from_store(
        store: R,
        content_upload_store: Box<
            dyn nola_store_port::content_upload::ContentUploadRepository<Error = String> + Send,
        >,
    ) -> Self {
        Self {
            store,
            content_upload_store,
            supervisor: RuntimeSupervisor::new(),
        }
    }

    pub fn has_hot_overlay(&self) -> bool {
        self.supervisor.has_hot_overlay()
    }
}

impl<R: crate::repositories::RuntimeExecutionRepositories> ApplicationRuntime<R> {
    pub fn execute(&mut self, command: ApplicationCommand) -> Result<Value, String> {
        self.supervisor.ensure_usable()?;
        let store_has_unit_of_work = self
            .store
            .command_protocol_repository()
            .has_write_unit_of_work();
        if self.supervisor.has_unit_of_work() {
            if !store_has_unit_of_work {
                self.supervisor
                    .poison("Runtime overlay lost its canonical transaction".into());
                return Err(
                    "Runtime overlay lost its canonical transaction; reopen required".into(),
                );
            }
            return self.execute_in_unit_of_work(command);
        }
        if store_has_unit_of_work {
            return Err("bare Runtime command cannot adopt an externally owned transaction".into());
        }
        if !self.requires_durable_flush(&command)? {
            let result = self.execute_in_unit_of_work(command);
            if result.is_ok() {
                self.supervisor.commit_hot_command();
            } else {
                self.rollback_runtime_unit_of_work()?;
            }
            return result;
        }
        // Bare calls and receipt calls use one commit/adoption order. Opening
        // this scope must not change the 64-KiB hot-only delta contract.
        let mut protocol = self.store.command_protocol_repository();
        protocol
            .begin_write_unit_of_work()
            .map_err(|error| format!("begin durable Runtime flush failed: {error}"))?;
        drop(protocol);
        if let Err(error) = self.supervisor.begin_durable_flush() {
            let rollback = self
                .store
                .command_protocol_repository()
                .rollback_write_unit_of_work();
            if let Err(rollback) = rollback {
                self.supervisor
                    .poison(format!("Runtime begin rollback failed: {rollback}"));
            }
            return Err(error);
        }
        let result = self.execute_in_unit_of_work(command);
        match result {
            Ok(value) => {
                let commit = self
                    .store
                    .command_protocol_repository()
                    .commit_write_unit_of_work();
                if let Err(error) = commit {
                    // A failed acknowledgement does not prove that COMMIT did
                    // not reach storage. Never publish or reuse this overlay.
                    let _ = self
                        .store
                        .command_protocol_repository()
                        .rollback_write_unit_of_work();
                    self.supervisor
                        .poison(format!("durable Runtime commit outcome unknown: {error}"));
                    return Err(format!(
                        "durable Runtime commit outcome unknown; reopen required: {error}"
                    ));
                }
                if let Err(error) = self.supervisor.commit_unit_of_work() {
                    self.supervisor
                        .poison(format!("durable Runtime adoption failed: {error}"));
                    return Err(format!(
                        "durable Runtime adoption failed; reopen required: {error}"
                    ));
                }
                Ok(value)
            }
            Err(error) => {
                let rollback = self
                    .store
                    .command_protocol_repository()
                    .rollback_write_unit_of_work();
                if let Err(rollback) = rollback {
                    self.supervisor
                        .poison(format!("durable Runtime rollback failed: {rollback}"));
                    return Err(format!(
                        "{error}; rollback failed; reopen required: {rollback}"
                    ));
                }
                self.rollback_runtime_unit_of_work()?;
                Err(error)
            }
        }
    }

    fn requires_durable_flush(&mut self, command: &ApplicationCommand) -> Result<bool, String> {
        match command {
            ApplicationCommand::ReduceAndAcceptHostResponse { input } => {
                let revision = self
                    .store
                    .checkpoint_revision(&input.conversation_id)
                    .map_err(|error| format!("load flush checkpoint revision failed: {error}"))?;
                let deferred =
                    self.supervisor
                        .deferred_bytes(&input.run_id, &input.conversation_id, revision);
                Ok(host_response_requires_recovery_boundary(&input.response)
                    || input.response.payload.is_terminal()
                    || deferred.saturating_add(estimated_response_bytes(&input.response))
                        >= STREAM_COMMIT_BYTE_BUDGET)
            }
            ApplicationCommand::ExpertPeerTaskAction {..}|ApplicationCommand::AgentRoomAtSource {..} => Ok(!command.is_read_only_query()),
            ApplicationCommand::AdmitExpertConversationMessage {..}
            | ApplicationCommand::PrepareEditedExpertResend {..}
            | ApplicationCommand::AdmitExpertPeer {..}
            | ApplicationCommand::ClarifyExpertPeer {..}
            | ApplicationCommand::PrepareExpertPeerTask {..}
            | ApplicationCommand::AbortExpertPeerTaskPreparation {..}
            | ApplicationCommand::ReceiveRemoteExpertParentAdmission { .. }
            | ApplicationCommand::BeginAssistantTurn { .. }
            | ApplicationCommand::PromoteQueuedAssistantTurn { .. }
            | ApplicationCommand::BeginQueuedUserTurn { .. }
            | ApplicationCommand::RespondToConversationWait { .. }
            | ApplicationCommand::ImportHostAgentTaskAdmission { .. }
            | ApplicationCommand::ImportHostAgentTaskTerminalProjection { .. }
            | ApplicationCommand::ApplyPendingTaskNotice { .. }
            | ApplicationCommand::ImportHostAgentTaskGuidanceProjection { .. }
            | ApplicationCommand::BeginTaskAssistantTurn { .. }
            | ApplicationCommand::BeginAutomationAssistantTurn { .. }
            | ApplicationCommand::ReconcileAutomationExecution { .. }
            | ApplicationCommand::ResolveUnknownEffectRecovery { .. }
            | ApplicationCommand::InspectOwnedTaskRecovery { .. }
            | ApplicationCommand::ResolveOwnedTaskRecovery { .. }
            | ApplicationCommand::EnqueueTaskTerminalReconciliation { .. }
            | ApplicationCommand::SubmitTaskGuidance { .. }
            | ApplicationCommand::ApplyTaskGuidance { .. }
            | ApplicationCommand::ApplyActivePermissionPolicy { .. }
            | ApplicationCommand::RecoverExpiredAgentTask { .. }
            | ApplicationCommand::ClaimAgentTaskFair { .. }
            | ApplicationCommand::BeginMaintenanceTurn { .. }
            | ApplicationCommand::ReduceAndCommitActiveSubmission { .. }
            | ApplicationCommand::ReduceAndCommitIdleSubmission { .. }
            | ApplicationCommand::RequestRunCancellation { .. }
            | ApplicationCommand::RecoverObservedOrphanedRun { .. }
            | ApplicationCommand::RecoverExpiredRunDeliveries { .. }
            | ApplicationCommand::PrepareRunEffectRecovery { .. }
            | ApplicationCommand::RecoverOrphanedRuns { .. } => Ok(true),
            ApplicationCommand::RuntimeTask {
                command:
                    crate::RuntimeTaskCommand::FinishDetailedWithEvent { .. }
                    | crate::RuntimeTaskCommand::ReconcileBoundRunTerminal { .. },
            } => Ok(true),
            _ => Ok(false),
        }
    }

    #[inline(never)]
    fn execute_in_unit_of_work(&mut self, mut command: ApplicationCommand) -> Result<Value, String> {
        if matches!(&command,ApplicationCommand::ReduceAndAcceptHostResponse{..}) {
            crate::task_guidance::prepare_command(&mut self.store,&mut command)?;
            crate::task_terminal::validate_command(&self.store,&command)?;
            let ApplicationCommand::ReduceAndAcceptHostResponse{input}=command else {unreachable!()};
            return self.reduce_host_response_in_unit_of_work(Box::new(input));
        }
        // These commands have always used the repository dispatcher. Keep the
        // common worker/private Host adapter out of the much larger supervisor
        // reducer frame on small native/background thread stacks.
        if matches!(&command,
            ApplicationCommand::ClaimCommittedHostRequests{..}|ApplicationCommand::ClaimApplicationControlRequests{..}
            |ApplicationCommand::LoadMobileTaskWakePlan{..}|ApplicationCommand::LoadMobileTaskChildConfiguration{..}
            |ApplicationCommand::AdmitMobileTaskChild{..}|ApplicationCommand::CaptureMobileDelegationConfiguration{..}
            |ApplicationCommand::PrepareMobileDelegatedTask{..}|ApplicationCommand::ObserveMobileTask{..}
            |ApplicationCommand::PrepareMobileTaskGuidance{..}|ApplicationCommand::InspectMobilePermissionRequest{..}
            |ApplicationCommand::InspectMobileMcpRequest{..}
            |ApplicationCommand::AbortExpertPeerTaskPreparation{..}|ApplicationCommand::LoadExpertPeerForEffect{..}
            |ApplicationCommand::ReadExpertExchangeCapability|ApplicationCommand::DeriveExpertPeerAuthority{..}|ApplicationCommand::PrepareExpertPeerTask{..}
            |ApplicationCommand::ExpertPeerTaskAction{..}|ApplicationCommand::ExpertPeerDirectory{..}|ApplicationCommand::PreviewExpertPeer{..}
            |ApplicationCommand::AdmitExpertPeer{..}|ApplicationCommand::LoadExpertPeerForSubmission{..}|ApplicationCommand::LoadExpertPeer{..}|ApplicationCommand::ClarifyExpertPeer{..}) {
            crate::task_guidance::prepare_command(&mut self.store,&mut command)?;
            crate::task_terminal::validate_command(&self.store,&command)?;
            return crate::execute_application_command(&mut self.store,command);
        }
        self.execute_supervised_command_in_unit_of_work(Box::new(command))
    }

    #[inline(never)]
    fn execute_supervised_command_in_unit_of_work(
        &mut self,
        mut command: Box<ApplicationCommand>,
    ) -> Result<Value, String> {
        if let ApplicationCommand::ReduceAndCommitActiveSubmission { input } = command.as_ref() {
            if matches!(input.submission, AgentSubmission::ApplyPermissionPolicy { .. }) {
                return Err("permission policy requires its dedicated claimed-notification command".into());
            }
            if matches!(input.submission, AgentSubmission::RespondToUserInputWait { .. }) {
                return Err("wait responses require the dedicated conversation wait command".into());
            }
            if matches!(
                input.submission,
                AgentSubmission::ApplyTaskGuidanceInput { .. }
            ) {
                return Err(
                    "guidance input requires the dedicated ApplyTaskGuidance transaction".into(),
                );
            }
        }
        crate::task_guidance::prepare_command(&mut self.store, &mut *command)?;
        crate::task_terminal::validate_command(&self.store, command.as_ref())?;
        match command.as_ref() {
            ApplicationCommand::PromoteQueuedAssistantTurn {..}
            | ApplicationCommand::BeginAssistantTurn {..}
            | ApplicationCommand::BeginTaskAssistantTurn {..}
            | ApplicationCommand::BeginAutomationAssistantTurn {..} => self.execute_admission_command(command),
            ApplicationCommand::ApplyPendingTaskNotice {..}
            | ApplicationCommand::InspectOwnedTaskRecovery {..}
            | ApplicationCommand::ResolveOwnedTaskRecovery {..}
            | ApplicationCommand::ImportHostAgentTaskGuidanceProjection {..}
            | ApplicationCommand::ImportHostAgentTaskTerminalProjection {..}
            | ApplicationCommand::RecoverExpiredAgentTask {..}
            | ApplicationCommand::ClaimAgentTaskFair {..}
            | ApplicationCommand::ApplyTaskGuidance {..} => self.execute_expert_task_control_command(command),
            ApplicationCommand::RespondToConversationWait {..}
            | ApplicationCommand::ReadPermissionConfiguration {..}
            | ApplicationCommand::LoadConversationPermissionProjectionV1 {..}
            | ApplicationCommand::ApplyActivePermissionPolicy {..} => self.execute_permission_control_command(command),
            ApplicationCommand::ResolveUnknownEffectRecovery {..}
            | ApplicationCommand::RequestRunCancellation {..}
            | ApplicationCommand::LoadRunCancellationDeliveryFacts {..}
            | ApplicationCommand::ClaimCancellationHostRequests {..}
            | ApplicationCommand::InspectCancellationProcessOwners {..}
            | ApplicationCommand::ValidateCancellationProcessObservations {..}
            | ApplicationCommand::RecoverObservedOrphanedRun {..}
            | ApplicationCommand::PrepareRunEffectRecovery {..}
            | ApplicationCommand::RecoverExpiredRunDeliveries {..}
            | ApplicationCommand::RecoverOrphanedRuns {..} => self.execute_recovery_control_command(command),
            ApplicationCommand::LoadSessionGoalProjection {..}
            | ApplicationCommand::LoadHostExecutionState {..}
            | ApplicationCommand::LoadCommittedHostRequests {..} => self.execute_current_read_command(command),
            ApplicationCommand::ReduceAndCommitActiveSubmission {..} => self.execute_active_submission_command(command),
            ApplicationCommand::StageArchiveManifest {..}
            | ApplicationCommand::PublishUploadedTranscript {..}
            | ApplicationCommand::BeginContentUpload {..}
            | ApplicationCommand::AppendContentChunk {..}
            | ApplicationCommand::ContentUploadProgress {..}
            | ApplicationCommand::ReadContentUploadChunk {..}
            | ApplicationCommand::SealContentUpload {..}
            | ApplicationCommand::CancelContentUpload {..} => self.execute_uploaded_content_command(command),
            _ => execute_application_command(&mut self.store,*command),
        }
    }

    #[inline(never)]
    fn execute_admission_command(&mut self,command:Box<ApplicationCommand>)->Result<Value,String>{
        match *command {
            ApplicationCommand::PromoteQueuedAssistantTurn { input } => {
                let promoted = crate::assistant_turn::promote_with_core(&mut self.store, input)?;
                let Some((accepted, core)) = promoted else { return Ok(Value::Null) };
                let active = self.store.active_run(&accepted.begin.conversation_id).map_err(|e| e.to_string())?
                    .ok_or("promoted successor has no active Run")?;
                let checkpoint_revision = self.store.checkpoint_revision(&accepted.begin.conversation_id)
                    .map_err(|e| e.to_string())?.ok_or("promoted successor has no checkpoint")?;
                let encoded = serde_json::to_value(&accepted).map_err(|e| e.to_string())?;
                self.supervisor.retain(RetainHotRun {
                    run_id: accepted.begin.run_id, conversation_id: accepted.begin.conversation_id, core,
                    terminal: accepted.execution.control.turn_status.is_terminal(), checkpoint_revision,
                    next_event_sequence: active.next_event_sequence,
                    invocation_config_digest: accepted.invocation_config_digest,
                    deferred_events: Vec::new(), deferred_host_events: Vec::new(), deferred_traces: Vec::new(), deferred_bytes: 0,
                });
                Ok(encoded)
            }
            ApplicationCommand::BeginAssistantTurn { input } => {
                self.admit_assistant_turn(input, None)
            }
            ApplicationCommand::BeginTaskAssistantTurn { lease, input } => {
                self.admit_assistant_turn(input, Some(lease))
            }
            ApplicationCommand::BeginAutomationAssistantTurn { lease, input } => {
                let (accepted, core) = crate::automation_execution::admit_with_core(&mut self.store, lease, input)?;
                if let Some(core) = core { self.retain_admitted_turn(accepted, core) }
                else { serde_json::to_value(accepted).map_err(|error|error.to_string()) }
            }
            _ => Err("invalid_internal_runtime_command_family".into()),
        }
    }

    #[inline(never)]
    fn execute_expert_task_control_command(&mut self,command:Box<ApplicationCommand>)->Result<Value,String>{
        match *command {
            ApplicationCommand::ApplyPendingTaskNotice { conversation_id, consumer_id, now_ms } => {
                use crate::task_terminal::PreparedTaskTerminalImport;
                let Some((command_id, prepared)) = crate::task_terminal::claim_local_notice(
                    &mut self.store, &conversation_id, &consumer_id, now_ms)? else { return Ok(Value::Null); };
                let result = match prepared {
                    PreparedTaskTerminalImport::Active(input) => self.execute_active_submission_in_unit_of_work(input,
                        |accepted, _| serde_json::to_value(accepted).map_err(|e| e.to_string()))??,
                    PreparedTaskTerminalImport::Idle(input) => {
                        let accepted = crate::reduce_and_commit_idle_submission(&mut self.store, input)?;
                        serde_json::json!({"events":accepted.events,"traces":accepted.traces,"control":accepted.execution.control,
                            "assistant_publications":accepted.assistant_publications})
                    }
                    PreparedTaskTerminalImport::AlreadyApplied(_) => {
                        let state = self.execute_in_unit_of_work(ApplicationCommand::LoadHostExecutionState {
                            conversation_id: conversation_id.clone() })?;
                        serde_json::json!({"events":[],"traces":[],"control":state["control"]})
                    }
                };
                if !nola_store_port::agent_queue::AgentQueueRepository::acknowledge_agent_command(
                    &mut self.store, &conversation_id, &command_id, &consumer_id).map_err(|e| e.to_string())? {
                    return Err("Task terminal delivery lost its claimed acknowledgement".into());
                }
                Ok(result)
            }
            ApplicationCommand::InspectOwnedTaskRecovery { owner } => {
                // Validate immutable parent admission even when no recovery is pending.
                // Renderer/checkpoint cancellation flags are not this query's authority.
                let current = crate::owned_task_cancellation::read(&self.store, &owner)?;
                if !current.cancel_requested {
                    return Ok(serde_json::Value::Null);
                }
                let binding=crate::owned_task_cancellation::recovery_binding(&self.store,&owner)?;
                let run=self.store.inspect_exact_run(&binding.run_id).map_err(|e|e.to_string())?.ok_or("Child Run missing")?;
                self.execute_in_unit_of_work(ApplicationCommand::InspectUnknownEffectRecovery {
                    conversation_id:binding.child_session_id,expected_run_id:binding.run_id,
                    assistant_message_id:run.assistant_message_id.ok_or("Child assistant owner missing")?,
                })
            }
            ApplicationCommand::ResolveOwnedTaskRecovery { owner, input } => {
                let binding=crate::owned_task_cancellation::recovery_binding(&self.store,&owner)?;
                if input.conversation_id!=binding.child_session_id || input.run_id!=binding.run_id
                    || !matches!(input.choice,crate::UnknownEffectRecoveryChoice::KeepUnknownBlocked {..}) {
                    return Err("Task recovery permits only preserving its exact child cancellation unknown".into());
                }
                let now=input.resolved_at_ms;
                let reply=self.execute_in_unit_of_work(ApplicationCommand::ResolveUnknownEffectRecovery {input})?;
                if reply["outcome"]=="unknown_preserved_blocked" {
                    if !self.store.reconcile_runtime_task_bound_run_terminal(&owner.task_id,&binding.run_id,now).map_err(|e|e.to_string())? {
                        return Err("Child result cannot yet be reconciled; its unknown state is retained".into());
                    }
                }
                Ok(reply)
            }
            ApplicationCommand::ImportHostAgentTaskGuidanceProjection { projection } => {
                use crate::task_guidance::PreparedTaskGuidanceImport;
                let revision = match crate::task_guidance::prepare_guidance_import(&self.store, &projection)? {
                    PreparedTaskGuidanceImport::Active(input) => self.execute_active_submission_in_unit_of_work(input,
                        |accepted, _| accepted.control.revision)?,
                    PreparedTaskGuidanceImport::Idle(input) => crate::reduce_and_commit_idle_submission(&mut self.store, input)?.execution.control.revision,
                };
                serde_json::to_value(nola_protocol::TaskGuidanceProjectionApplied {
                    conversation_id: projection.admission.parent_conversation_id,
                    task_id: projection.admission.task_id,
                    message_id: projection.guidance.message_id,
                    settlement: projection.settlement,
                    core_revision: revision,
                }).map_err(|error| error.to_string())
            }
            ApplicationCommand::ImportHostAgentTaskTerminalProjection { projection } => {
                use crate::task_terminal::PreparedTaskTerminalImport;
                let revision = match crate::task_terminal::prepare_import(&self.store, &projection)? {
                    PreparedTaskTerminalImport::Active(input) => self.execute_active_submission_in_unit_of_work(input,
                        |accepted, _| accepted.control.revision)?,
                    PreparedTaskTerminalImport::Idle(input) => crate::reduce_and_commit_idle_submission(&mut self.store, input)?.execution.control.revision,
                    PreparedTaskTerminalImport::AlreadyApplied(ack) => return serde_json::to_value(ack).map_err(|error| error.to_string()),
                };
                serde_json::to_value(crate::TaskTerminalReconciliationApplied {
                    conversation_id: projection.admission.parent_conversation_id,
                    task_id: projection.admission.task_id,
                    receipt_id: projection.receipt_id,
                    core_revision: revision,
                }).map_err(|error| error.to_string())
            }
            ApplicationCommand::RecoverExpiredAgentTask { input } => {
                crate::task_ownership::recover(&mut self.store, input).map(serde_json::Value::Bool)
            }
            ApplicationCommand::ClaimAgentTaskFair { input } => {
                serde_json::to_value(crate::task_ownership::claim(&mut self.store, input)?).map_err(|e|e.to_string())
            }
            ApplicationCommand::ApplyTaskGuidance {
                task_id,
                message_id,
                lease,
                expected_run_id,
                now_ms,
            } => {
                let prepared = crate::task_guidance::prepare_apply(
                    &mut self.store,
                    &task_id,
                    &message_id,
                    lease.as_ref(),
                    &expected_run_id,
                    now_ms,
                )?;
                let accepted = if let Some(input) = prepared.submission {
                    Some(Box::new(
                        self.execute_active_submission_in_unit_of_work(input,
                            |accepted, _| accepted)?,
                    ))
                } else {
                    None
                };
                if accepted.is_some() {
                    crate::task_guidance::finish_apply(&mut self.store, &prepared.guidance)?;
                }
                serde_json::to_value(crate::TaskGuidanceApplyResult {
                    guidance: prepared.guidance,
                    accepted,
                    recovered: prepared.recovered,
                })
                .map_err(|e| e.to_string())
            }
            _ => Err("invalid_internal_runtime_command_family".into()),
        }
    }

    #[inline(never)]
    fn execute_permission_control_command(&mut self,command:Box<ApplicationCommand>)->Result<Value,String>{
        match *command {
            ApplicationCommand::RespondToConversationWait { input } => {
                let result = match crate::user_input_wait::prepare(&self.store, &input)? {
                    crate::user_input_wait::PreparedWaitResponse::Settled(result) => result,
                    crate::user_input_wait::PreparedWaitResponse::Apply(command) => {
                        let accepted = self.execute_active_submission_in_unit_of_work(command, |accepted, _| accepted)?;
                        crate::user_input_wait::applied(&self.store, &input, accepted)?
                    }
                };
                serde_json::to_value(result).map_err(|error| error.to_string())
            }
            ApplicationCommand::ReadPermissionConfiguration {
                conversation_id, source_host_id, client_id, unresolved_run_ids,
                source_unconfirmed, minimum_policy_revision,
            } => {
                let supervisor = &self.supervisor;
                self.store.with_consistent_read(|store| {
                    let active = store.active_run(&conversation_id).map_err(|e| e.to_string())?;
                    let revision = store.checkpoint_revision(&conversation_id).map_err(|e| e.to_string())?;
                    let current = active.as_ref().map(|active|
                        supervisor.current_permissions(&active.run_id, &conversation_id, revision))
                        .transpose()?.flatten();
                    crate::application_command::read_permission_configuration(store,
                        &conversation_id, source_host_id.as_deref(), client_id.as_deref(),
                        &unresolved_run_ids, source_unconfirmed, minimum_policy_revision, current.as_ref())
                })
            }
            ApplicationCommand::LoadConversationPermissionProjectionV1 { conversation_id, run_id, scope } => {
                let revision = self.store.checkpoint_revision(&conversation_id).map_err(|e| e.to_string())?;
                if let Some(current) = self.supervisor.current_permissions(&run_id, &conversation_id, revision)? {
                    crate::application_command::read_permission_projection(
                        &self.store, conversation_id, run_id, scope, Some(&current))
                } else {
                    crate::execute_application_command(&mut self.store,
                        ApplicationCommand::LoadConversationPermissionProjectionV1 { conversation_id, run_id, scope })
                }
            }
            ApplicationCommand::ApplyActivePermissionPolicy { conversation_id, policy, now_ms } => {
                let input = crate::permission_policy::prepare_apply(&mut self.store, &conversation_id, &policy, now_ms)?;
                let result = match input {
                    Some(input) => Some(self.execute_active_submission_in_unit_of_work(input, |accepted, _| accepted)?),
                    None => None,
                };
                serde_json::to_value(result).map_err(|error| error.to_string())
            }
            _ => Err("invalid_internal_runtime_command_family".into()),
        }
    }

    #[inline(never)]
    fn execute_recovery_control_command(&mut self,command:Box<ApplicationCommand>)->Result<Value,String>{
        match *command {
            ApplicationCommand::ResolveUnknownEffectRecovery { input } => {
                let run_id=input.run_id.clone();
                let conversation_id=input.conversation_id.clone();
                let revision=self.store.checkpoint_revision(&conversation_id).map_err(|e|e.to_string())?;
                let hot=self.supervisor.take_current(&run_id,&conversation_id,revision);
                let (core,deferred_events,deferred_host_events,deferred_traces,previous)=match hot {
                    Some(hot)=>(Some(hot.core),hot.deferred_events,hot.deferred_host_events,hot.deferred_traces,
                        Some((hot.checkpoint_revision,hot.next_event_sequence,hot.invocation_config_digest,hot.deferred_bytes))),
                    None=>(None,Vec::new(),Vec::new(),Vec::new(),None),
                };
                let invocation_config_digest=input.invocation_config_digest.clone();
                let resolved=crate::unknown_effect_recovery::resolve_with_core(&mut self.store,input,core,deferred_events,deferred_host_events,deferred_traces)?;
                if let Some(core)=resolved.core {
                    let (checkpoint_revision,next_event_sequence,invocation_config_digest,deferred_bytes,terminal)=match resolved.accepted {
                        Some(accepted)=>(accepted.commit.checkpoint_revision,accepted.commit.next_event_sequence,invocation_config_digest,0,accepted.control.turn_status.is_terminal()),
                        None=>{
                            let (revision,sequence,digest,bytes)=previous.ok_or("recovery overlay lacks original owner")?;
                            (revision,sequence,digest,bytes,false)
                        },
                    };
                    self.supervisor.retain(RetainHotRun{run_id,conversation_id,core,terminal,checkpoint_revision,next_event_sequence,
                        invocation_config_digest,deferred_events:resolved.deferred_events,deferred_host_events:resolved.deferred_host_events,deferred_traces:resolved.deferred_traces,deferred_bytes});
                }
                serde_json::to_value(resolved.reply).map_err(|e|e.to_string())
            }
            ApplicationCommand::RequestRunCancellation { input } => {
                self.retain_cancellation_recovery(&input.conversation_id, &input.expected_run_id)?;
                let revision = self.store.checkpoint_revision(&input.conversation_id)
                    .map_err(|error| format!("read cancellation revision failed: {error}"))?;
                let control = self.supervisor.current_control(&input.expected_run_id,
                    &input.conversation_id, revision);
                let prepared = crate::run_cancellation::prepare(&self.store, input, control)?;
                let receipt = if let Some(input) = prepared.submission {
                    // Reuse the hot submission boundary, including buffered
                    // committed progress and receipt rollback ownership.
                    self.execute_active_submission_in_unit_of_work(input,
                        |accepted, _| accepted.control.turn_status)?;
                    crate::run_cancellation::complete(
                        &self.store,
                        prepared.receipt,
                    )?
                } else {
                    prepared.receipt
                };
                serde_json::to_value(receipt).map_err(|e| e.to_string())
            }
            ApplicationCommand::LoadRunCancellationDeliveryFacts { input } => {
                let checkpoint = self.cancellation_checkpoint(&input)?;
                serde_json::to_value(crate::cancellation_delivery::load_run_cancellation_delivery_facts_with_checkpoint(
                    &self.store, &input, checkpoint.as_ref())?).map_err(|e| e.to_string())
            }
            ApplicationCommand::ClaimCancellationHostRequests { input } => {
                let checkpoint = self.cancellation_checkpoint(&input.identity)?;
                serde_json::to_value(crate::cancellation_delivery::claim_cancellation_host_requests_with_checkpoint(
                    &mut self.store, input, checkpoint.as_ref())?).map_err(|e| e.to_string())
            }
            ApplicationCommand::InspectCancellationProcessOwners { identity, control } => {
                let checkpoint = self.cancellation_checkpoint(&identity)?;
                serde_json::to_value(crate::cancellation_process_observation::inspect_with_checkpoint(
                    &self.store, identity, control, checkpoint.as_ref())?).map_err(|e| e.to_string())
            }
            ApplicationCommand::ValidateCancellationProcessObservations { plan, current_process, observations } => {
                let checkpoint = self.cancellation_checkpoint(&plan.observed.identity)?;
                serde_json::to_value(crate::cancellation_process_observation::validate_with_checkpoint(
                    &self.store, plan, current_process, observations, checkpoint.as_ref())?).map_err(|e| e.to_string())
            }
            ApplicationCommand::RecoverObservedOrphanedRun {
                conversation_id,
                expected_run_id,
                host_invocation,
                observed_session_lease,
                observed_deliveries,
                interrupted_at,
                interrupted_at_ms,
            } => {
                let outcome = crate::run_recovery::recover_observed(
                    &mut self.store,
                    &conversation_id,
                    &expected_run_id,
                    &host_invocation,
                    observed_session_lease.as_ref(),
                    &observed_deliveries,
                    &interrupted_at,
                    interrupted_at_ms,
                )?;
                Ok(serde_json::json!(outcome.changed))
            }
            ApplicationCommand::PrepareRunEffectRecovery { input } => {
                let planned = crate::effect_delivery::plan_run_effect_recovery(&self.store, &input)?;
                let mut prepared = crate::PreparedRunEffectRecovery { rebound_count: planned.len(), traces: Vec::new() };
                for submission in planned {
                    prepared.traces.extend(self.execute_active_submission_in_unit_of_work(submission, |accepted, _| accepted.traces)?);
                }
                serde_json::to_value(prepared).map_err(|error| error.to_string())
            }
            ApplicationCommand::RecoverExpiredRunDeliveries {
                conversation_id, expected_run_id, host_invocation, interrupted_at, interrupted_at_ms,
            } => {
                let outcome = crate::run_recovery::recover_expired(&mut self.store, &conversation_id,
                    &expected_run_id, &host_invocation, &interrupted_at, interrupted_at_ms)?;
                Ok(serde_json::json!(outcome.changed))
            }
            ApplicationCommand::RecoverOrphanedRuns {
                conversation_id,
                reason,
                interrupted_at,
                interrupted_at_ms,
            } => {
                let outcome = crate::run_recovery::recover(
                    &mut self.store,
                    conversation_id.as_deref(),
                    &reason,
                    &interrupted_at,
                    interrupted_at_ms,
                )?;
                Ok(serde_json::json!(outcome.changed))
            }
            _ => Err("invalid_internal_runtime_command_family".into()),
        }
    }

    #[inline(never)]
    fn execute_current_read_command(&mut self,command:Box<ApplicationCommand>)->Result<Value,String>{
        match *command {
            ApplicationCommand::LoadSessionGoalProjection { conversation_id } => {
                let revision = self.store.checkpoint_revision(&conversation_id).map_err(|e| e.to_string())?;
                if let Some(goal) = revision.and_then(|revision| self.supervisor.current_goal(&conversation_id, revision)) {
                    if self.store.checkpoint_revision(&conversation_id).map_err(|e| e.to_string())? != revision {
                        return Err("Goal projection changed during read".into());
                    }
                    serde_json::to_value(Some(goal)).map_err(|e| e.to_string())
                } else {
                    crate::execute_application_command(&mut self.store,
                        ApplicationCommand::LoadSessionGoalProjection { conversation_id })
                }
            }
            ApplicationCommand::LoadHostExecutionState { conversation_id } => {
                let revision = self.store.checkpoint_revision(&conversation_id).map_err(|e| e.to_string())?
                    .ok_or("Host execution state has no committed revision")?;
                let state = match self.supervisor.execution_state(&conversation_id, revision) {
                    Some(state) => state,
                    None => crate::host_execution_state::load(&self.store, &conversation_id)?,
                };
                if self.store.checkpoint_revision(&conversation_id).map_err(|e| e.to_string())? != Some(revision) {
                    return Err("Host execution state changed during read".into());
                }
                serde_json::to_value(state).map_err(|e| e.to_string())
            }
            ApplicationCommand::LoadCommittedHostRequests { run_id } => {
                let committed = self.effective_host_requests(&run_id)?;
                serde_json::to_value(committed).map_err(|error| error.to_string())
            }
            _ => Err("invalid_internal_runtime_command_family".into()),
        }
    }

    #[inline(never)]
    fn execute_active_submission_command(&mut self,command:Box<ApplicationCommand>)->Result<Value,String>{
        match *command {
            ApplicationCommand::ReduceAndCommitActiveSubmission { input } => {
                let accepted = self.execute_active_submission_in_unit_of_work(input,
                    |accepted, _| accepted)?;
                serde_json::to_value(accepted).map_err(|error| error.to_string())
            }
            _ => Err("invalid_internal_runtime_command_family".into()),
        }
    }

    #[inline(never)]
    fn execute_uploaded_content_command(&mut self,command:Box<ApplicationCommand>)->Result<Value,String>{
        match *command {
            ApplicationCommand::StageArchiveManifest { input } => {
                let result = crate::archive_restore::stage_archive_manifest(
                    &mut self.store,
                    self.content_upload_store.as_ref(),
                    &input,
                    crate::content_upload::MAXIMUM_UPLOAD_BYTES,
                )?;
                serde_json::to_value(result).map_err(|error| error.to_string())
            }
            ApplicationCommand::PublishUploadedTranscript { input } => {
                let result = crate::uploaded_transcript_publication::publish_uploaded_transcript(
                    &mut self.store,
                    self.content_upload_store.as_ref(),
                    &input,
                    crate::content_upload::MAXIMUM_UPLOAD_BYTES,
                )?;
                serde_json::to_value(result).map_err(|error| error.to_string())
            }
            command @ (ApplicationCommand::BeginContentUpload { .. }
            | ApplicationCommand::AppendContentChunk { .. }
            | ApplicationCommand::ContentUploadProgress { .. }
            | ApplicationCommand::ReadContentUploadChunk { .. }
            | ApplicationCommand::SealContentUpload { .. }
            | ApplicationCommand::CancelContentUpload { .. }) => {
                self.execute_content_upload(command)
            }
            _ => Err("invalid_internal_runtime_command_family".into()),
        }
    }

    #[inline(never)]
    fn reduce_host_response_in_unit_of_work(&mut self,input:Box<crate::ReduceAndAcceptHostResponse>)->Result<Value,String>{
        let input=*input;
        if let Some(accepted) = crate::reconcile_committed_terminal_response(&self.store, &input)? {
            return serde_json::to_value(accepted).map_err(|error| error.to_string());
        }
        let run_id = input.run_id.clone();
        let conversation_id = input.conversation_id.clone();
        let checkpoint_revision =
            RunEffectRepository::checkpoint_revision(&self.store, &conversation_id)
                .map_err(|error| {
                    format!("load hot Run checkpoint revision failed: {error}")
                })?;
        let hot =
            self.supervisor
                .take_current(&run_id, &conversation_id, checkpoint_revision);
        let response_bytes = estimated_response_bytes(&input.response);
        let terminal = input.response.payload.is_terminal();
        let delta_only = response_has_incremental_projection(&input.response);
        let (
            hot_core,
            hot_revision,
            hot_sequence,
            hot_digest,
            mut prior_deferred_events,
            mut deferred_host_events,
            mut deferred_traces,
            deferred_bytes,
        ) = match hot {
            Some(run) => (
                Some(run.core),
                Some(run.checkpoint_revision),
                Some(run.next_event_sequence),
                Some(run.invocation_config_digest),
                run.deferred_events,
                run.deferred_host_events,
                run.deferred_traces,
                run.deferred_bytes,
            ),
            None => (None, None, None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        };
        // A durable command receipt may only acknowledge durable state.
        // Otherwise replay after a crash would return the receipt while
        // silently dropping the hot-only delta that it acknowledged.
        // Delegated workers wait for durable per-event acknowledgement before
        // continuing external execution, including below the byte budget.
        // An interrupted permission is a recoverable checkpoint boundary,
        // even though it does not finish the Run or its Host effect.
        // A model restart must also commit the discarded partials before
        // the Host is allowed to start another physical request.
        let recovery_boundary = host_response_requires_recovery_boundary(&input.response);
        let persist = self.supervisor.requires_durable_ack()
            || recovery_boundary
            || terminal
            || deferred_bytes.saturating_add(response_bytes) >= STREAM_COMMIT_BYTE_BUDGET;
        let deferred_events = if persist {
            std::mem::take(&mut prior_deferred_events)
        } else {
            Vec::new()
        };
        let invocation_config_digest = input.invocation_config_digest.clone();
        if persist {
            self.store
                .append_host_event_history(&run_id, &deferred_host_events)
                .map_err(|error| {
                    format!("commit deferred Host responses failed: {error}")
                })?;
            deferred_host_events.clear();
            crate::execution_trace_journal::record_accepted(&mut self.store, &conversation_id, &deferred_traces)?;
            deferred_traces.clear();
        }
        let (mut accepted, core, deferred_host_event) =
            reduce_and_accept_host_response_with_core(
                &mut self.store,
                input,
                hot_core,
                hot_revision,
                hot_sequence,
                hot_digest.as_deref(),
                deferred_events,
                persist,
                persist && !delta_only,
            )?;
        // The reducer supplies its actual binding/normalization facts.
        // A caller must never infer a no-bind fact merely because this
        // response remained below the durable byte boundary.
        if let Some(event) = deferred_host_event {
            deferred_host_events.push(event);
        }
        let next_terminal = core.turn_status().is_terminal();
        let next_checkpoint_revision = accepted.commit.checkpoint_revision;
        let next_event_sequence = accepted.commit.next_event_sequence;
        let next_deferred_events = if persist {
            Vec::new()
        } else {
            deferred_traces.extend(accepted.traces.iter().cloned());
            let mut controls = Vec::new();
            for event in std::mem::take(&mut accepted.events) {
                if !matches!(event.event, AgentEvent::PartUpdated { .. }) {
                    controls.push(event.clone());
                }
                prior_deferred_events.push(event);
            }
            accepted.events = controls;
            prior_deferred_events
        };
        self.supervisor.retain(RetainHotRun {
            run_id,
            conversation_id,
            core,
            terminal: next_terminal,
            checkpoint_revision: next_checkpoint_revision,
            next_event_sequence,
            invocation_config_digest,
            deferred_events: next_deferred_events,
            deferred_traces,
            deferred_host_events,
            deferred_bytes: if persist {
                0
            } else {
                deferred_bytes.saturating_add(response_bytes)
            },
        });
        // Durability and projection shape are independent contracts.
        // A receipt must commit its delta, but must not duplicate the
        // entire growing conversation on every progress/token event.
        // Boundary responses publish scalar control only; permissions
        // are read under their exact witness, and bodies use journals.
        accepted
            .events
            .retain(|event| !matches!(event.event, AgentEvent::PartUpdated { .. }));
        let encoded = serde_json::to_value(&accepted).map_err(|error| error.to_string())?;
        Ok(encoded)
    }

    fn admit_assistant_turn(&mut self, input: crate::BeginAssistantTurn,
        lease: Option<crate::RuntimeTaskLeaseWitness>) -> Result<Value, String>
    {
        let (accepted, core) = crate::assistant_turn::admit_with_core(&mut self.store, input, lease)?;
        self.retain_admitted_turn(accepted, core)
    }

    fn retain_admitted_turn(&mut self, accepted: crate::AcceptedAssistantTurn, core: AgentCore) -> Result<Value, String> {
        let conversation_id = accepted.begin.conversation_id.clone();
        if accepted.execution.control.turn_status.is_terminal() {
            // Immediate Core decisions are admitted and sealed in the same
            // transaction. They are deliberately absent from active_run.
            let exact = self.store.load_exact_run_invocation(
                &conversation_id, &accepted.begin.run_id, &accepted.begin.assistant_message_id,
            ).map_err(|error| error.to_string())?.ok_or("Terminal admission has no canonical Run")?;
            if exact.status != "sealed" || exact.terminal.is_none()
                || exact.invocation_config_digest != accepted.invocation_config_digest {
                return Err("Terminal admission does not match its canonical seal and invocation".into());
            }
            // No effect can consume a hot Core from this terminal admission.
            // Future commands resolve the committed result/checkpoint normally.
            return serde_json::to_value(&accepted).map_err(|error| error.to_string());
        }
        let active = self.store.active_run(&conversation_id).map_err(|error| error.to_string())?
            .ok_or("Admitted Core has no canonical Run")?;
        if active.run_id != accepted.begin.run_id
            || active.invocation_config_digest.as_ref() != Some(&accepted.invocation_config_digest) {
            return Err("Admitted Core does not match canonical Run ownership".into());
        }
        let checkpoint_revision = self.store.checkpoint_revision(&conversation_id)
            .map_err(|error| error.to_string())?.ok_or("Admitted Core has no checkpoint revision")?;
        let encoded = serde_json::to_value(&accepted).map_err(|error| error.to_string())?;
        self.supervisor.retain(RetainHotRun {
            run_id: accepted.begin.run_id, conversation_id, core,
            terminal: accepted.execution.control.turn_status.is_terminal(),
            checkpoint_revision, next_event_sequence: active.next_event_sequence,
            invocation_config_digest: accepted.invocation_config_digest,
            deferred_events: Vec::new(), deferred_host_events: Vec::new(), deferred_traces: Vec::new(), deferred_bytes: 0,
        });
        Ok(encoded)
    }

    /// Shared typed reducer for public submissions and admitted internal work.
    /// Avoid recursive ApplicationCommand/JSON frames on the worker stack.
    fn retain_cancellation_recovery(&mut self, conversation: &str, run: &str) -> Result<(), String> {
        let revision = self.store.checkpoint_revision(conversation).map_err(|e| e.to_string())?;
        if self.supervisor.current_control(run, conversation, revision).is_some() { return Ok(()); }
        let Some(active) = self.store.active_run(conversation).map_err(|e| e.to_string())? else { return Ok(()); };
        // The command's canonical inspection still decides terminal/stale
        // ownership. Never hydrate a different active Run for this request.
        if active.run_id != run { return Ok(()); }
        let frozen = self.store.load_run_invocation_config(run).map_err(|e| e.to_string())?
            .ok_or("cancellation recovery has no frozen invocation")?;
        let stored = crate::load_exact_run_checkpoint(&self.store, conversation, run)?;
        let core = crate::restore_run_recovery(&self.store, run, &frozen.invocation_config_digest, &stored)?;
        self.supervisor.retain_recovered(RetainHotRun {
            run_id: run.into(), conversation_id: conversation.into(), terminal: core.turn_status().is_terminal(), core,
            checkpoint_revision: revision.ok_or("cancellation recovery checkpoint missing")?,
            next_event_sequence: active.next_event_sequence, invocation_config_digest: frozen.invocation_config_digest,
            deferred_events: Vec::new(), deferred_host_events: Vec::new(), deferred_traces: Vec::new(), deferred_bytes: 0,
        });
        Ok(())
    }

    fn cancellation_checkpoint(&mut self, identity: &crate::cancellation_delivery::CancellationRunIdentity)
        -> Result<Option<crate::cancellation_delivery::CancellationCheckpoint>, String> {
        self.retain_cancellation_recovery(&identity.conversation_id, &identity.expected_run_id)?;
        let revision = self.store.checkpoint_revision(&identity.conversation_id).map_err(|e| e.to_string())?;
        Ok(revision.and_then(|revision| self.supervisor.cancellation_checkpoint(
            &identity.expected_run_id, &identity.conversation_id, revision)))
    }

    fn execute_active_submission_in_unit_of_work<T>(
        &mut self,
        input: crate::ReduceAndCommitActiveSubmission,
        project: impl FnOnce(crate::AcceptedActiveSubmission, &AgentCore) -> T,
    ) -> Result<T, String> {
        let invocation_config_digest = input.invocation_config_digest.clone();
        let run_id = input.run_id.clone();
        let conversation_id = input.conversation_id.clone();
        let checkpoint_revision =
            RunEffectRepository::checkpoint_revision(&self.store, &conversation_id)
                .map_err(|error| format!("load hot Run checkpoint revision failed: {error}"))?;
        let hot = self
            .supervisor
            .take_current(&run_id, &conversation_id, checkpoint_revision);
        let (hot_core, deferred_events, deferred_host_events, deferred_traces) = match hot {
            Some(run) => (
                Some(run.core),
                run.deferred_events,
                run.deferred_host_events,
                run.deferred_traces,
            ),
            None => (None, Vec::new(), Vec::new(), Vec::new()),
        };
        self.store
            .append_host_event_history(&run_id, &deferred_host_events)
            .map_err(|error| {
                format!("commit deferred Host responses at submission boundary failed: {error}")
            })?;
        crate::execution_trace_journal::record_accepted(&mut self.store, &conversation_id, &deferred_traces)?;
        let (accepted, core) = reduce_and_commit_active_submission_with_core(
            &mut self.store,
            input,
            hot_core,
            deferred_events,
        )?;
        let terminal = accepted.control.turn_status.is_terminal();
        let checkpoint_revision = accepted.commit.checkpoint_revision;
        let next_event_sequence = accepted.commit.next_event_sequence;
        let result = project(accepted, &core);
        self.supervisor.retain(RetainHotRun {
            run_id,
            conversation_id,
            core,
            terminal,
            checkpoint_revision,
            next_event_sequence,
            invocation_config_digest,
            deferred_events: Vec::new(),
            deferred_traces: Vec::new(),
            deferred_host_events: Vec::new(),
            deferred_bytes: 0,
        });
        Ok(result)
    }

    pub(crate) fn begin_runtime_unit_of_work(&mut self) -> Result<(), String> {
        self.supervisor.begin_unit_of_work()
    }

    pub(crate) fn ensure_runtime_usable(&self) -> Result<(), String> {
        self.supervisor.ensure_usable()
    }

    pub(crate) fn poison_runtime(&mut self, reason: String) {
        self.supervisor.poison(reason);
    }

    pub(crate) fn commit_runtime_unit_of_work(&mut self) -> Result<(), String> {
        self.supervisor.commit_unit_of_work()
    }

    pub(crate) fn rollback_runtime_unit_of_work(&mut self) -> Result<(), String> {
        self.supervisor.rollback_unit_of_work();
        for capsule in self.supervisor.take_rollback_capsules() {
            match crate::restore_hot_rollback(&self.store, capsule) {
                Ok(restored) => self.supervisor.retain(restored),
                Err(error) => {
                    self.supervisor.poison(format!("hot rollback replay failed: {error}"));
                    return Err(format!("hot rollback replay failed; reopen required: {error}"));
                }
            }
        }
        Ok(())
    }

    fn effective_host_requests(&self, run_id: &str) -> Result<CommittedHostRequests, String> {
        let mut committed = load_committed_host_requests(&self.store, run_id)?;
        self.supervisor
            .overlay_resume_cursors(&self.store, run_id, &mut committed)?;
        Ok(committed)
    }
}

fn host_response_requires_recovery_boundary(response: &HostResponseEnvelope) -> bool {
    use nola_agent_engine::agent::host::HostResponse;
    matches!(
        &response.payload,
        HostResponse::Accepted
            | HostResponse::ExecutionStarted
            | HostResponse::ExecutionRejected { .. }
            | HostResponse::DelegatedPermissionClaimed { .. }
            | HostResponse::DelegatedPermissionTransportWritten { .. }
            | HostResponse::DelegatedPermissionInterrupted { .. }
            | HostResponse::DelegatedAgentEvent { .. }
            | HostResponse::DelegatedExecutionInterrupted { .. }
            | HostResponse::PermissionTransportInterrupted { .. }
            | HostResponse::ModelAttemptStarted { .. }
            | HostResponse::ModelUsageReported { .. }
    )
}

/// Publication shape is independent of acknowledgement durability. Clients
/// must share this classification instead of reintroducing full projections
/// for progress events that Application deliberately emits incrementally.
pub fn response_has_incremental_projection(response: &HostResponseEnvelope) -> bool {
    use nola_agent_engine::agent::host::HostResponse;
    matches!(
        response.payload,
        HostResponse::Accepted
            | HostResponse::ExecutionStarted
            | HostResponse::ModelAttemptStarted { .. }
            | HostResponse::ModelUsageReported { .. }
            | HostResponse::ModelDelta { .. }
            | HostResponse::ModelCommittedDelta { .. }
            | HostResponse::ModelToolCallDelta { .. }
            | HostResponse::ModelContinuation { .. }
            | HostResponse::ToolProgress { .. }
            | HostResponse::ToolCallProgress { .. }
    )
}

fn estimated_response_bytes(response: &HostResponseEnvelope) -> usize {
    bounded_serialized_size(response, STREAM_COMMIT_BYTE_BUDGET).max(1_024)
}

/// Measures wire bytes without allocating an encoded copy. Once the commit
/// budget is reached, further counting cannot change the persistence decision.
fn bounded_serialized_size(value: &impl serde::Serialize, limit: usize) -> usize {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes = self.bytes.saturating_add(bytes.len()).min(self.limit);
            if self.bytes >= self.limit {
                return Err(std::io::Error::other("serialization size budget reached"));
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { bytes: 0, limit };
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => counter.bytes,
        // Serialization errors must never underestimate the pending payload.
        Err(_) => limit,
    }
}

#[cfg(test)]
mod response_size_tests {
    use super::*;

    #[test]
    fn measures_heap_strings_and_utf8_escaping_without_an_encoded_buffer() {
        let payload = serde_json::json!({"delta": "\"\n\\\u{4e2d}".repeat(2_000)});
        let expected = serde_json::to_vec(&payload).unwrap().len();
        assert!(expected > 1_024);
        assert_eq!(
            bounded_serialized_size(&payload, STREAM_COMMIT_BYTE_BUDGET),
            expected
        );
    }

    #[test]
    fn a_large_single_response_reaches_the_commit_budget() {
        let payload = serde_json::json!({"delta": "x".repeat(10_000_000)});
        assert_eq!(
            bounded_serialized_size(&payload, STREAM_COMMIT_BYTE_BUDGET),
            STREAM_COMMIT_BYTE_BUDGET
        );
        assert_eq!(bounded_serialized_size(&payload, 0), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::ApplicationRuntime;
    use nola_agent_engine::agent::{AgentCore, AgentCoreConfig, SessionId};

    fn core(session_id: &str) -> AgentCore {
        AgentCore::try_new(AgentCoreConfig {
            session_id: Some(SessionId::from_string(session_id.to_string())),
            ..crate::model_fixture::config()
        })
        .expect("create Core")
    }

    #[test]
    fn hot_core_is_unique_per_conversation_and_terminally_evicted() {
        let directory = tempfile::tempdir().unwrap();
        let mut runtime = ApplicationRuntime::open(directory.path().join("runtime.db")).unwrap();

        runtime.supervisor.retain(super::RetainHotRun {
            run_id: "run-1".to_string(),
            conversation_id: "conversation-1".to_string(),
            core: core("conversation-1"),
            terminal: false,
            checkpoint_revision: 1,
            next_event_sequence: 0,
            invocation_config_digest: String::new(),
            deferred_events: Vec::new(),
            deferred_traces: Vec::new(),
            deferred_host_events: Vec::new(),
            deferred_bytes: 0,
        });
        runtime.supervisor.retain(super::RetainHotRun {
            run_id: "run-2".to_string(),
            conversation_id: "conversation-1".to_string(),
            core: core("conversation-1"),
            terminal: false,
            checkpoint_revision: 2,
            next_event_sequence: 0,
            invocation_config_digest: String::new(),
            deferred_events: Vec::new(),
            deferred_traces: Vec::new(),
            deferred_host_events: Vec::new(),
            deferred_bytes: 0,
        });
        assert_eq!(runtime.supervisor.active_run_count(), 1);
        assert!(runtime.supervisor.contains_run("run-2"));
        assert!(runtime.supervisor.current_control("run-2", "conversation-1", Some(2)).is_some());
        assert!(runtime.supervisor.current_control("run-2", "conversation-1", Some(1)).is_none());
        assert!(runtime.supervisor.current_control("run-2", "foreign-conversation", Some(2)).is_none());
        assert!(runtime.supervisor.current_control("foreign-run", "conversation-1", Some(2)).is_none());
        assert!(runtime.supervisor.contains_run("run-2"), "control inspection must not take the Core");

        runtime.supervisor.retain(super::RetainHotRun {
            run_id: "run-2".to_string(),
            conversation_id: "conversation-1".to_string(),
            core: core("conversation-1"),
            terminal: true,
            checkpoint_revision: 3,
            next_event_sequence: 0,
            invocation_config_digest: String::new(),
            deferred_events: Vec::new(),
            deferred_traces: Vec::new(),
            deferred_host_events: Vec::new(),
            deferred_bytes: 0,
        });
        assert_eq!(runtime.supervisor.active_run_count(), 0);
    }
}

#[cfg(test)]
impl ApplicationRuntime<NolaStore> {
    // Unit-test fixture composition only; absent from the production Application API.
    #[cfg(test)]
    pub(crate) fn open(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let store = NolaStore::open(path).map_err(|error| error.to_string())?;
        let uploads = Box::new(
            nola_store::content_upload_port::LazyContentUploadStore::for_application(
                path,
                crate::content_upload::MAXIMUM_UPLOAD_BYTES,
            ),
        );
        Ok(Self::from_store(store, uploads))
    }
}
