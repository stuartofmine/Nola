// SPDX-License-Identifier: Apache-2.0
// Internal layer of the deterministic agent runtime. Public clients use AgentRuntime.

use super::*;
use crate::agent::capability::{
    validate_skill_completion, MaterializedSkillRuntimeUpdate, SkillVerificationResult,
};
use crate::agent::host::{validate_host_artifacts, HostCapabilityEffect, HostToolOutcome};
use crate::agent::loop_guard::output_is_structurally_repetitive;
use crate::agent::protocol::ModelToolCall;
use crate::agent::runtime_event::ItemDiscardDisposition;
use crate::agent::turn::DelegatedUsageObservation;
use std::collections::BTreeSet;

fn model_output_is_structurally_repetitive(text: &str) -> bool {
    output_is_structurally_repetitive(text)
}

fn model_output_has_continuation_progress(anchor: &str, continuation: &str) -> bool {
    let normalize = |value: &str| {
        value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let anchor = normalize(anchor);
    let continuation = normalize(continuation);
    if continuation.is_empty()
        || anchor == continuation
        || anchor.contains(continuation.as_str())
        || model_output_is_structurally_repetitive(&continuation)
    {
        return false;
    }
    let prefix = continuation.chars().take(160).collect::<String>();
    !anchor.contains(prefix.as_str())
}

fn loop_checkpoint_protocol_is_selected(tool_calls: &[ModelToolCall]) -> bool {
    if tool_calls.len() != 1 { return false; }
    match tool_calls[0].name.as_str() {
        "requirements_update" => true,
        "request_user_input" => serde_json::from_value::<nola_domain::user_input_wait::QuestionSet>(
            tool_calls[0].arguments.clone(),
        ).is_ok_and(|questions| questions.validate().is_ok()),
        _ => false,
    }
}

fn goal_report_delivers_response(calls: &[ModelToolCall]) -> bool {
    let [call] = calls else { return false; };
    call.name == "goal_complete"
        && call.arguments["status"] == "completed"
        && call.arguments["claims"].as_array().is_some_and(|claims| claims.iter().any(|claim|
            claim["evidence_refs"].as_array().is_some_and(|refs| refs.iter().any(|reference|
                reference.as_str().is_some_and(|value| value == "response" || value.starts_with("response:"))))))
}

impl AgentRuntime {
    fn is_streamed_control_call(&self, turn_id: &TurnId, item_id: &ItemId, calls: &[ModelToolCall]) -> bool {
        matches!(self.session.conversation.projection.parts.get(item_id.as_str()),
            Some(AgentPart::Tool(part)) if part.ownership.turn_id == turn_id.as_str()
                && !part.state.is_terminal()
                && calls.iter().any(|call| call.call_id.as_str() == part.call_id && call.name == part.capability_id))
    }

    pub fn submit(
        &mut self,
        submission: AgentSubmission,
        observed_at_ms: u64,
    ) -> Result<AgentTransition, String> {
        let reduction = self.reduce(submission, observed_at_ms)?;
        Ok(reduction.with_snapshot(self.snapshot()))
    }

    pub fn reduce(
        &mut self,
        submission: AgentSubmission,
        observed_at_ms: u64,
    ) -> Result<AgentReduction, String> {
        let state_before = self.session.clone();
        let previous_time = self.observed_at_ms;
        match self.reduce_in_place(submission, observed_at_ms) {
            Ok(reduction) => Ok(reduction),
            Err(error) => {
                self.session = state_before;
                self.observed_at_ms = previous_time;
                Err(error)
            }
        }
    }

    /// A failed reduction consumes the candidate. Application can only adopt
    /// the returned runtime after its canonical transaction succeeds.
    pub fn reduce_owned(
        mut self: Box<Self>,
        submission: AgentSubmission,
        observed_at_ms: u64,
    ) -> Result<(Box<Self>, AgentReduction), String> {
        let reduction = self.reduce_in_place(submission, observed_at_ms)?;
        Ok((self, reduction))
    }

    fn reduce_in_place(
        &mut self,
        submission: AgentSubmission,
        observed_at_ms: u64,
    ) -> Result<AgentReduction, String> {
        Self::observation_time(observed_at_ms)?;
        self.observed_at_ms = observed_at_ms;
        submission.validate()?;
        if let AgentSubmission::RespondToUserInputWait { input } = &submission {
            if self.user_input_response_is_replay(input)? {
                return Ok(AgentReduction { event_envelopes: Vec::new(), trace_events: Vec::new(), host_requests: Vec::new(), durability: None });
            }
        }
        if let AgentSubmission::ResumeHostEffect { effect_id } = &submission {
            return self.resume_host_effect(effect_id);
        }
        if matches!(&submission, AgentSubmission::CloseSession { .. }) {
            if self.session.journal.session_closed {
                return Ok(AgentReduction {
                    event_envelopes: Vec::new(),
                    trace_events: Vec::new(),
                    host_requests: Vec::new(),
                    durability: None,
                });
            }
            if self.session.journal.session_closing {
                return Ok(AgentReduction {
                    event_envelopes: Vec::new(),
                    trace_events: Vec::new(),
                    host_requests: self.pending_host_requests()?,
                    durability: None,
                });
            }
        }
        let should_replay_after_submission =
            matches!(&submission, AgentSubmission::UpdateHostCatalog { .. });
        let submission_id = submission.idempotency_key().map(str::to_string);
        let input_fingerprint = matches!(&submission, AgentSubmission::UserDelegation { .. } | AgentSubmission::UserInput { .. }
            | AgentSubmission::GoalUserInput { .. } | AgentSubmission::AutomatedInput { .. })
            .then(|| serde_json::to_value(&submission).map(|value| crate::agent::input_queue::input_digest(&value)))
            .transpose().map_err(|error| error.to_string())?;
        if let (Some(id), Some(fingerprint)) = (&submission_id, &input_fingerprint) {
            if let Some(existing) = self.session.journal.accepted_input_fingerprints.get(id) {
                if existing != fingerprint { return Err("input submission identity was reused with different content or delivery".into()); }
                return Ok(AgentReduction { event_envelopes: Vec::new(), trace_events: Vec::new(), host_requests: Vec::new(), durability: None });
            }
        }
        if submission_id.as_ref().is_some_and(|submission_id| {
            self.session.journal.has_accepted_submission(submission_id)
        }) {
            return Ok(AgentReduction {
                event_envelopes: Vec::new(),
                trace_events: Vec::new(),
                host_requests: Vec::new(),
                durability: None,
            });
        }
        if self.session.journal.session_closed
            && !matches!(
                &submission,
                AgentSubmission::HostResponse { .. } | AgentSubmission::ResumeHostEffect { .. }
            )
        {
            return Err("the session is already closed".to_string());
        }
        if self.session.journal.session_closing
            && !matches!(
                &submission,
                AgentSubmission::HostResponse { .. }
                    | AgentSubmission::ResumeHostEffect { .. }
                    | AgentSubmission::CloseSession { .. }
            )
        {
            return Err("the session is closing".to_string());
        }
        match self.dispatch(submission.into_command()?) {
            Ok(mut transition) => {
                if let Some(id) = submission_id {
                    self.session.journal.record_accepted_submission(id.clone());
                    if let Some(fingerprint) = input_fingerprint {
                        self.session.journal.accepted_input_fingerprints.insert(id, fingerprint);
                    }
                }
                if should_replay_after_submission {
                    let existing_effect_ids = transition
                        .host_requests
                        .iter()
                        .map(|request| request.effect_id.0.clone())
                        .collect::<std::collections::HashSet<_>>();
                    transition.host_requests.extend(
                        self.pending_host_requests()?
                            .into_iter()
                            .filter(|request| !existing_effect_ids.contains(&request.effect_id.0)),
                    );
                }
                Ok(transition)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn dispatch(&mut self, command: AgentCommand) -> Result<AgentReduction, String> {
        // Host observations are validated/correlated before becoming one of
        // the reducer's internal commands. Route them outside the large
        // command match: retaining that frame while the Host adapter calls
        // back into dispatch duplicates every branch's debug-stack storage.
        // Move the owned envelope; there is no reason to clone its payload.
        match command {
            AgentCommand::HostResponded { response } => self.dispatch_host_response(response),
            command => self.dispatch_command(command, None),
        }
    }

    #[inline(never)]
    pub(super) fn dispatch_command(&mut self, command: AgentCommand,
        promotion_evidence: Option<nola_domain::execution_evidence::ExecutionEvidenceObservation>,
    ) -> Result<AgentReduction, String> {
        if promotion_evidence.is_some() && !matches!(&command, AgentCommand::TaskPromoted { .. }) {
            return Err("Task promotion evidence cannot be attached to another command".into());
        }
        if let AgentCommand::GoalRuntime { envelope } | AgentCommand::Cancel { goal_control: Some(envelope), .. } = &command {
            if self.session.goal_projection.operation_replayed(envelope)? {
                return Ok(AgentReduction { event_envelopes: Vec::new(), trace_events: Vec::new(),
                    host_requests: Vec::new(), durability: None });
            }
        }
        let command = match command {
            AgentCommand::MemoryWorkerCompleted {
                operation_id,
                job_id,
                ownership_token,
                now_epoch_ms,
                completion_watermark,
                output,
            } => {
                let plan = self.memory_worker_plan(&job_id)?;
                if plan.ownership_token != ownership_token {
                    return Err(format!(
                        "memory job {job_id} ownership token does not match"
                    ));
                }
                let output =
                    super::super::memory_worker::materialize_memory_worker_output(&plan, output)?;
                AgentCommand::MemoryRuntime {
                    envelope: super::super::memory_runtime::MemoryRuntimeCommandEnvelope {
                        operation_id,
                        command: super::super::memory_runtime::MemoryRuntimeCommand::Complete {
                            job_id,
                            ownership_token,
                            now_epoch_ms,
                            completion_watermark,
                            output,
                        },
                    },
                }
            }
            command => command,
        };
        // Enforce the same contract for every Host, including non-streaming
        // adapters. Reject the entire response before budgets or side effects
        // are mutated. A bypassing Host receives a settled failure, not an Err
        // that would leave its already projected drafts pending indefinitely.
        let oversized_turn = match &command {
            AgentCommand::ModelCompleted { turn_id, tool_calls, .. }
                if super::super::model_tool_batch::validate_model_tool_batch_size(tool_calls.len()).is_err() =>
                Some(turn_id.clone()),
            AgentCommand::ModelToolCallDelta { turn_id, part_id, .. } => {
                let turn = self.require_turn(turn_id, AgentTurnStatus::AwaitingModel)?;
                let count = turn.streamed_tool_part_ids.len()
                    + usize::from(!turn.streamed_tool_part_ids.contains(part_id));
                super::super::model_tool_batch::validate_model_tool_batch_size(count)
                    .err().map(|_| turn_id.clone())
            }
            _ => None,
        };
        let command = if let Some(turn_id) = oversized_turn {
            AgentCommand::ModelFailed {
                turn_id,
                error: super::super::model_tool_batch::MODEL_TOOL_BATCH_LIMIT_EXCEEDED.to_string(),
            }
        } else { command };
        match &command {
            AgentCommand::ToolCompleted { artifacts, .. }
            | AgentCommand::DelegatedAgentToolCompleted { artifacts, .. } => {
                validate_host_artifacts(artifacts)?;
            }
            _ => {}
        }
        let checkpoint_reason = match &command {
            AgentCommand::ResolveUnknownEffects { .. } => Some(CheckpointReason::Manual),
            AgentCommand::RespondToUserInputWait { .. } => Some(CheckpointReason::Manual),
            AgentCommand::RebaseConversation { .. } | AgentCommand::RevertConversation { .. } => {
                Some(CheckpointReason::Manual)
            }
            AgentCommand::UserDelegation { .. } | AgentCommand::SubmitInput { .. } | AgentCommand::PromoteQueuedInput { .. } => Some(CheckpointReason::SessionStarted),
            AgentCommand::ModelCompleted { .. }
            | AgentCommand::ModelFailed { .. }
            | AgentCommand::ToolCompleted { .. }
            | AgentCommand::DelegatedAgentToolCompleted { .. } => {
                Some(CheckpointReason::TurnCompleted)
            }
            AgentCommand::Cancel { .. } | AgentCommand::HostCancellationConfirmed { .. } => Some(CheckpointReason::CancellationProgress),
            AgentCommand::TaskPromoted { .. }
            | AgentCommand::TaskUpdated { .. }
            | AgentCommand::ReconcileTaskTerminal { .. }
            | AgentCommand::UnresolvedTaskGuidance
            | AgentCommand::ReconcileTaskGuidance { .. }
            | AgentCommand::ApplyTaskGuidanceInput { .. }
            | AgentCommand::ObserveTaskGuidanceDelivery { .. }
            | AgentCommand::UpdateTask { .. }
            | AgentCommand::RequestTaskCancellation { .. } => Some(CheckpointReason::TaskUpdated),
            AgentCommand::GoalRuntime { .. } => Some(CheckpointReason::GoalUpdated),
            AgentCommand::CreateMemory { .. }
            | AgentCommand::UpdateMemory { .. }
            | AgentCommand::DeleteMemory { .. }
            | AgentCommand::MemoryRuntime { .. } => Some(CheckpointReason::MemoryUpdated),
            AgentCommand::MemoryWorkerCompleted { .. } => {
                unreachable!("memory worker completion is normalized before dispatch")
            }
            AgentCommand::SkillRuntime { .. } => Some(CheckpointReason::SkillUpdated),
            AgentCommand::PermissionResolved { .. } | AgentCommand::PermissionIntervened { .. } => {
                Some(CheckpointReason::PermissionResolved)
            }
            AgentCommand::UpdateSessionConfiguration { .. } | AgentCommand::ApplyPermissionPolicy { .. } => {
                Some(CheckpointReason::ConfigurationChanged)
            }
            AgentCommand::DelegatedAgentSessionStarted { .. } => {
                Some(CheckpointReason::ConfigurationChanged)
            }
            AgentCommand::CompactionCompleted { .. } => Some(CheckpointReason::CompactionCompleted),
            AgentCommand::CloseSession => Some(CheckpointReason::SessionClosing),
            AgentCommand::SessionClosed => Some(CheckpointReason::SessionClosed),
            AgentCommand::HostDisconnected { .. } => Some(CheckpointReason::HostDisconnected),
            AgentCommand::RecoverContextLimit { .. }
            | AgentCommand::RequestCompaction { .. }
            | AgentCommand::CompactionFailed { .. }
            | AgentCommand::Reset => Some(CheckpointReason::Manual),
            AgentCommand::CodeModeToolRequested { .. } | AgentCommand::CodeModeOutput { .. } => Some(CheckpointReason::Manual),
            AgentCommand::HostResponded { .. }
            | AgentCommand::HostProgress { .. }
            | AgentCommand::LifecycleHookUpdated { .. }
            | AgentCommand::ToolCallProgress { .. }
            | AgentCommand::UpdateHostCatalog { .. }
            | AgentCommand::HostHeartbeat { .. }
            | AgentCommand::ModelDelta { .. }
            | AgentCommand::ModelToolCallDelta { .. }
            | AgentCommand::ModelContinuationUpdated { .. }
            | AgentCommand::ModelStreamRestarted { .. }
            | AgentCommand::DelegatedAgentTextDelta { .. }
            | AgentCommand::DelegatedAgentToolStarted { .. }
            | AgentCommand::DelegatedAgentPermissionRequested { .. }
            | AgentCommand::DelegatedAgentUsage { .. } => None,
        };
        let completed_model_turn = match &command {
            AgentCommand::ModelCompleted { turn_id, .. } => Some(turn_id.clone()),
            _ => None,
        };
        let mut events: Vec<RuntimeEvent> = Vec::new();
        let mut effects = Vec::new();
        if !self.session.journal.session_started_emitted {
            self.session.journal.session_started_emitted = true;
            events.push(RuntimeEvent::SessionStarted {
                session_id: self.session.session_id.clone(),
            });
        }
        match command {
            AgentCommand::RebaseConversation {
                items,
                item_ownership,
            } => {
                self.rebase_conversation(items, item_ownership)?;
            }
            AgentCommand::RevertConversation {
                through_message_id,
                keep_message,
            } => {
                self.revert_conversation(&through_message_id, keep_message)?;
            }
            AgentCommand::HostResponded { .. } => unreachable!(),
            AgentCommand::CodeModeToolRequested { turn_id, request } => {
                let turn = self.session.execution.active_turn.as_mut().ok_or("no active code cell turn")?;
                if turn.turn_id != turn_id { return Err("code cell Turn mismatch".into()); }
                let call = turn.admit_code_tool_call(request)?;
                let item = AgentItem::ToolCall {
                    id: ItemId::from_string(self.session.allocate_id("code-tool-call")),
                    call_id: call.call_id, name: call.name, arguments: call.arguments,
                };
                self.session.record_turn_items(&turn_id, [item]);
                self.schedule_tools(&turn_id, &mut events, &mut effects)?;
            }
            AgentCommand::CodeModeOutput { turn_id, effect_id, frame } => {
                let turn = self.session.execution.active_turn.as_mut().ok_or("no active code cell turn")?;
                if turn.turn_id != turn_id { return Err("code output Turn mismatch".into()); }
                turn.code_cells.get_mut(&frame.cell.cell_id).ok_or("unknown code output cell")?
                    .append_output(frame.clone())?;
                events.push(RuntimeEvent::HostProgress {
                    turn_id: Some(turn_id), effect_id, title: "code_mode_exec".into(),
                    detail: Some(frame.chunk.text),
                    metadata: serde_json::json!({"cell_id":frame.cell.cell_id,"output_sequence":frame.chunk.sequence}),
                });
            }
            AgentCommand::HostProgress {
                turn_id,
                effect_id,
                title,
                detail,
                metadata,
            } => {
                events.push(RuntimeEvent::HostProgress {
                    turn_id,
                    effect_id,
                    title,
                    detail,
                    metadata,
                });
            }
            AgentCommand::LifecycleHookUpdated {
                turn_id,
                projection,
            } => {
                events.push(RuntimeEvent::LifecycleHookUpdated {
                    turn_id,
                    projection,
                });
            }
            AgentCommand::ToolCallProgress {
                turn_id,
                call_id,
                channel: _,
                title,
                detail,
                delta,
            } => {
                events.push(RuntimeEvent::ToolCallProgress {
                    turn_id,
                    call_id,
                    title,
                    detail,
                    delta,
                });
            }
            AgentCommand::SkillRuntime { envelope } => {
                self.apply_skill_runtime_command(envelope, &mut events, &mut effects)?;
            }
            AgentCommand::GoalRuntime { envelope } => {
                if envelope.operation_id.trim().is_empty() {
                    return Err("Goal operation id must not be empty".into());
                }
                let original_operation = envelope.clone();
                match envelope.command {
                    crate::agent::goal::GoalRuntimeCommand::ApplyAudit { mut report } => {
                        if report.findings.iter().any(|finding| finding.status == crate::agent::goal::GoalRecordStatus::Completed
                            || finding.assessment_method != crate::agent::goal::GoalAssessmentMethod::ModelJudgment) {
                            return Err("Positive Goal acceptance requires canonical Core evidence; external inspection claims are not authority".into());
                        }
                        report.assessor = crate::agent::goal::GoalAssessor::RuntimeOperation { operation_id: envelope.operation_id.clone() };
                        self.session.goal_projection.apply_audit(report)?;
                    }
                    crate::agent::goal::GoalRuntimeCommand::AcceptCompletion {
                        expected_revision, contract_id, claims, now_epoch_ms,
                    } => self.accept_goal_completion(expected_revision, &contract_id, claims, now_epoch_ms)?,
                    command => self.session.goal_projection.apply(crate::agent::goal::GoalRuntimeCommandEnvelope {
                        operation_id: envelope.operation_id, command,
                    })?,
                }
                self.session.goal_projection.record_operation(&original_operation)?;
                events.push(RuntimeEvent::GoalUpdated {
                    goal_projection_revision: self.session.goal_projection.revision,
                    goal: self.session.goal_projection.clone(),
                });
            }
            AgentCommand::UpdateHostCatalog { catalog } => {
                let current = &self.session.host.catalog;
                if catalog.host_id != current.host_id && self.session.host.pending_effects.len() > 0
                {
                    return Err(
                        "cannot replace the active Host while effects are pending".to_string()
                    );
                }
                if catalog.host_id == current.host_id
                    && (catalog.revision < current.revision
                        || catalog.availability_epoch < current.availability_epoch)
                {
                    return Err("Host capability catalog moved backwards".to_string());
                }
                self.config.host_catalog = Some(catalog.clone());
                self.session.host.catalog = catalog.clone();
                events.push(RuntimeEvent::HostCatalogUpdated {
                    host_id: catalog.host_id.0,
                    revision: catalog.revision,
                    availability_epoch: catalog.availability_epoch,
                });
            }
            AgentCommand::HostHeartbeat {
                host_id,
                availability_epoch,
                lease_id,
                observed_at_ms,
                extend_until_ms,
            } => {
                let catalog = &mut self.session.host.catalog;
                if catalog.host_id.0 != host_id || catalog.availability_epoch != availability_epoch
                {
                    return Err("Host heartbeat belongs to a stale Host epoch".to_string());
                }
                let lease = catalog
                    .lease
                    .as_mut()
                    .ok_or_else(|| "in-process Host does not accept heartbeat".to_string())?;
                if lease.lease_id != lease_id {
                    return Err("Host heartbeat belongs to a different lease".to_string());
                }
                if observed_at_ms < lease.last_heartbeat_ms {
                    return Err("Host heartbeat timestamp moved backwards".to_string());
                }
                if extend_until_ms <= observed_at_ms {
                    return Err("Host heartbeat expiry must be after observation".to_string());
                }
                lease.last_heartbeat_ms = observed_at_ms;
                lease.expires_at_ms = extend_until_ms;
                self.config.host_catalog = Some(catalog.clone());
                events.push(RuntimeEvent::HostLeaseRenewed {
                    host_id,
                    availability_epoch,
                    lease_id,
                    observed_at_ms,
                    expires_at_ms: extend_until_ms,
                });
            }
            AgentCommand::ApplyPermissionPolicy { policy } => {
                self.apply_active_permission_policy(policy, &mut events, &mut effects)?;
            }
            AgentCommand::UpdateSessionConfiguration { patch } => {
                if self
                    .session
                    .execution
                    .active_turn
                    .as_ref()
                    .is_some_and(|turn| !turn.status.is_terminal())
                {
                    return Err(
                        "session configuration can only change while the thread is idle"
                            .to_string(),
                    );
                }
                let model_changed = patch
                    .model
                    .as_ref()
                    .is_some_and(|model| model != &self.session.configuration.model);
                let mut next_config = self.config.clone();
                if let Some(model) = patch.model.clone() {
                    next_config.model = model;
                }
                if let Some(tools) = patch.tools.clone() {
                    next_config.tools = tools;
                }
                if let Some(context_window) = patch.context_window.clone() {
                    next_config.context_window = context_window.normalized();
                }
                if let Some(memory_policy) = patch.memory_policy.clone() {
                    next_config.memory_policy = memory_policy;
                }
                if let Some(turn_identity) = patch.turn_identity.clone() {
                    next_config.turn_identity = turn_identity;
                }
                Self::validate_config(&next_config)?;
                self.session.configuration.apply_patch(patch);
                if model_changed {
                    self.session.conversation.provider_continuation = None;
                }
                self.session.conversation.context_window.config =
                    self.session.configuration.context_window.clone();
                self.config = next_config;
                events.push(RuntimeEvent::SessionConfigurationUpdated {
                    model: self.session.configuration.model.clone(),
                    tool_count: self.session.configuration.tools.len(),
                    context_window_tokens: self
                        .session
                        .configuration
                        .context_window
                        .context_window_tokens,
                    prompt_token_limit: self
                        .session
                        .configuration
                        .context_window
                        .prompt_token_limit(),
                });
            }
            AgentCommand::HostDisconnected {
                host_id,
                availability_epoch,
                reason,
            } => {
                if self.session.host.catalog.host_id.0 != host_id {
                    return Err("Host disconnect belongs to a different Host".to_string());
                }
                if availability_epoch < self.session.host.catalog.availability_epoch {
                    return Err("stale Host disconnect notification".to_string());
                }
                self.session.host.catalog.availability_epoch = availability_epoch;
                for capability in self.session.host.catalog.capabilities.values_mut() {
                    capability.availability = CapabilityAvailability::RequiresConnection;
                }
                self.config.host_catalog = Some(self.session.host.catalog.clone());
                events.push(RuntimeEvent::HostDisconnected {
                    host_id,
                    availability_epoch,
                    reason,
                });
            }
            AgentCommand::PromoteQueuedInput { admission_id, expected_previous_turn_id, successor_turn_id, turn_context_components } => {
                self.promote_queued_input(&admission_id, &expected_previous_turn_id, successor_turn_id,
                    turn_context_components, &mut events, &mut effects)?;
            }
            AgentCommand::RespondToUserInputWait { input } => {
                self.respond_to_user_input_wait(input, &mut events, &mut effects)?;
            }
            AgentCommand::UserDelegation { agent_type, prompt, submission_id, submitted_at_epoch_ms } => {
                self.submit_user_delegation(agent_type, prompt, submission_id, submitted_at_epoch_ms, &mut events, &mut effects)?;
            }
            AgentCommand::SubmitInput {
                origin,
                items,
                turn_context_items,
                turn_context_platform,
                turn_context_components,
                submission_id,
                submitted_at_epoch_ms,
                delivery,
            } => {
                self.submit_input(
                    origin,
                    items,
                    turn_context_items,
                    turn_context_platform,
                    turn_context_components,
                    submission_id,
                    submitted_at_epoch_ms,
                    delivery,
                    &mut events,
                    &mut effects,
                )?;
            }
            AgentCommand::ModelDelta {
                turn_id,
                item_id,
                kind,
                delta,
                presentation_committed,
            } => {
                let projected_delta = {
                    let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                    if kind != AgentItemDeltaKind::AssistantText
                        || !turn.final_answer_audit_required
                        || (turn.context.configuration.turn_identity.supports_reviewed_text_parts
                            && !(turn.final_answer_draft.is_some() && turn.final_answer_audit_report.is_none()))
                    {
                        Some(delta)
                    } else if turn.final_answer_draft.is_none() {
                        // Tool-round and first-synthesis text is a private
                        // candidate. ModelCompleted owns the immutable draft;
                        // no provisional bytes may flash into the UI.
                        None
                    } else if turn.final_answer_audit_report.is_some() {
                        None
                    } else {
                        // Constraint audit JSON is private protocol state.
                        None
                    }
                };
                if let Some(delta) =
                    projected_delta.filter(|delta| !delta.is_empty() || presentation_committed)
                {
                    let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                    if !turn.open_stream_items.contains(&item_id) {
                        turn.open_stream_items.push(item_id.clone());
                        events.push(RuntimeEvent::ItemStarted {
                            turn_id: turn_id.clone(),
                            item_id: item_id.clone(),
                            kind,
                        });
                    }
                    if presentation_committed && !turn.committed_stream_items.contains(&item_id) {
                        turn.committed_stream_items.push(item_id.clone());
                    }
                    events.push(RuntimeEvent::ItemDelta {
                        turn_id,
                        item_id,
                        kind,
                        delta,
                        presentation_committed,
                    });
                }
            }
            AgentCommand::ModelToolCallDelta {
                turn_id,
                part_id,
                call_id,
                capability_id,
                arguments_delta,
            } => {
                let item_id = ItemId::from_string(part_id.clone());
                let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                turn.streamed_tool_part_ids.insert(part_id.clone());
                if !turn.open_stream_items.contains(&item_id) {
                    turn.open_stream_items.push(item_id);
                }
                events.push(RuntimeEvent::ToolInputDelta {
                    turn_id,
                    part_id,
                    call_id,
                    capability_id,
                    arguments_delta,
                });
            }
            AgentCommand::ModelContinuationUpdated {
                turn_id,
                continuation,
            } => {
                let expected_model = self
                    .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .context
                    .configuration
                    .model
                    .clone();
                if !continuation.is_for_model(expected_model.as_deref()) {
                    return Err(
                        "provider continuation model does not match active Turn".to_string()
                    );
                }
                self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .pending_provider_continuation = Some(continuation);
            }
            AgentCommand::ModelStreamRestarted { turn_id } => {
                let provisional_items = self
                    .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .open_stream_items
                    .clone();
                for item_id in provisional_items {
                    events.push(RuntimeEvent::ItemDiscarded {
                        turn_id: turn_id.clone(),
                        item_id,
                        disposition: ItemDiscardDisposition::Hidden,
                    });
                }
                let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                turn.clear_stream_items();
                turn.pending_provider_continuation = None;
            }
            AgentCommand::DelegatedAgentSessionStarted {
                turn_id,
                backend_id,
                backend_session_id,
            } => {
                let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                if turn.context.runtime_snapshot.execution_backend.backend_id != backend_id {
                    return Err(
                        "delegated Agent session belongs to a different backend".to_string()
                    );
                }
                turn.context
                    .runtime_snapshot
                    .execution_backend
                    .backend_session_id = Some(backend_session_id.clone());
                turn.context
                    .configuration
                    .turn_identity
                    .execution_backend
                    .backend_session_id = Some(backend_session_id.clone());
                self.session
                    .configuration
                    .turn_identity
                    .execution_backend
                    .backend_session_id = Some(backend_session_id);
            }
            AgentCommand::DelegatedAgentTextDelta {
                turn_id,
                target_effect_id,
                item_id,
                delta,
            } => {
                let runaway = self
                    .require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .observe_delegated_text_delta(&delta);
                if runaway {
                    let failure =
                        "delegated Agent output became structurally repetitive"
                            .to_string();
                    let open_items = {
                        let turn =
                            self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                        turn.status = AgentTurnStatus::Cancelling;
                        turn.failure = Some(failure.clone());
                        turn.open_stream_items.clone()
                    };
                    for open_item in open_items {
                        events.push(RuntimeEvent::ItemDiscarded {
                            turn_id: turn_id.clone(),
                            item_id: open_item,
                            disposition: ItemDiscardDisposition::Hidden,
                        });
                    }
                    effects.push(AgentEffect::CancelHostEffect {
                        turn_id: turn_id.clone(),
                        target_effect_id: target_effect_id.clone(),
                    });
                    events.push(RuntimeEvent::HostProgress {
                        turn_id: Some(turn_id), effect_id: target_effect_id,
                        title: "stopping_failed_execution".into(), detail: Some(failure), metadata: serde_json::Value::Null,
                    });
                } else if !delta.is_empty() {
                    let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                    if !turn.open_stream_items.contains(&item_id) {
                        turn.open_stream_items.push(item_id.clone());
                        events.push(RuntimeEvent::ItemStarted {
                            turn_id: turn_id.clone(),
                            item_id: item_id.clone(),
                            kind: AgentItemDeltaKind::AssistantText,
                        });
                    }
                    events.push(RuntimeEvent::ItemDelta {
                        turn_id,
                        item_id,
                        kind: AgentItemDeltaKind::AssistantText,
                        delta,
                        presentation_committed: false,
                    });
                }
            }
            AgentCommand::DelegatedAgentToolStarted {
                turn_id,
                target_effect_id,
                call_id,
                capability_id,
                mut input,
            } => {
                crate::agent::tool_lifecycle::canonicalize_tool_arguments(&mut input,
                    self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                        .context.capability_projection.frozen_host_catalog()
                        .resolve_available(&capability_id));
                let repeated = {
                    let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                    turn.has_executed_tools = true;
                    turn.observe_delegated_tool_call(ModelToolCall {
                        call_id: call_id.clone(),
                        name: capability_id.clone(),
                        arguments: input.clone(),
                        permission_scope: None,
                        permission_rationale: None,
                    })
                };
                // The accepted external start is the source of the call,
                // just as a native model's tool call is. A projected Tool Part
                // alone is not a transcript or permission/execution receipt.
                let item = AgentItem::ToolCall {
                    id: ItemId::from_string(self.session.allocate_id("tool_call")),
                    call_id: call_id.clone(), name: capability_id.clone(), arguments: input.clone(),
                };
                events.push(RuntimeEvent::ItemCompleted {turn_id:turn_id.clone(),item:item.clone()});
                self.session.record_turn_items(&turn_id,[item]);
                events.push(RuntimeEvent::ToolStarted {
                    turn_id: turn_id.clone(),
                    call_id: call_id.clone(),
                    tool_name: capability_id.clone(),
                    arguments: input.clone(),
                });
                if repeated {
                    let failure = format!(
                        "delegated Agent repeatedly requested the same tool and arguments: {capability_id}"
                    );
                    let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                    turn.status = AgentTurnStatus::Cancelling;
                    turn.failure = Some(failure.clone());
                    effects.push(AgentEffect::CancelHostEffect {
                        turn_id: turn_id.clone(),
                        target_effect_id: target_effect_id.clone(),
                    });
                    events.push(RuntimeEvent::HostProgress {
                        turn_id: Some(turn_id), effect_id: target_effect_id,
                        title: "stopping_failed_execution".into(), detail: Some(failure), metadata: serde_json::Value::Null,
                    });
                }
            }
            AgentCommand::DelegatedAgentToolCompleted {
                turn_id,
                call_id,
                capability_id,
                output,
                outcome,
                full_output_ref,
                artifacts,
            } => {
                let call = self
                    .require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .take_delegated_tool_call(&call_id)
                    .ok_or("delegated tool completion has no accepted start")?;
                if call.name != capability_id { return Err("delegated tool completion changed its capability".into()); }
                let arguments = call.arguments.clone();
                let (effect, resource_projection) = self
                    .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .context
                    .capability_projection
                    .frozen_host_catalog()
                    .resolve_available(&capability_id)
                    .map(|descriptor| {
                        (
                            descriptor.effect_for(&arguments),
                            descriptor.input_normalization.resource_projection.clone(),
                        )
                    })
                    .unwrap_or((
                        crate::agent::host::HostCapabilityEffect::Execute,
                        crate::agent::host::HostResourceProjectionDeclaration::default(),
                    ));
                let is_error = outcome.is_error();
                let failure_summary = is_error.then(|| {
                    let value = output.to_string();
                    value.chars().take(2_000).collect::<String>()
                });
                let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                turn.has_executed_tools = true;
                let previous_engineering = turn.engineering.clone();
                // Delegated outputs have not crossed the normal tool-result
                // normalizer. Never trust a business-data _nola_execution key.
                nola_host_protocol::tool_result::validate_completion(&output, outcome)?;
                let normalized_output = nola_host_protocol::tool_result::core_tool_output(output.clone())?;
                let receipt = normalized_output.get("_nola_execution").cloned()
                    .map(serde_json::from_value::<nola_host_protocol::tool_result::ExecutionReceipt>)
                    .transpose().map_err(|error| error.to_string())?;
                let execution_skipped = super::tool_driver::execution_was_skipped(&normalized_output, receipt.as_ref());
                if outcome == HostToolOutcome::Succeeded
                    && !execution_skipped
                    && !receipt.as_ref().is_some_and(crate::agent::turn::receipt_is_known_noop)
                    && matches!(effect, crate::agent::host::HostCapabilityEffect::Mutate
                        | crate::agent::host::HostCapabilityEffect::MutateAndValidate)
                {
                    turn.successful_mutation_calls.insert(call_id.as_str().to_string());
                }
                turn.engineering
                    .observe_tool_result_for_call_with_resources(
                        &capability_id,
                        effect,
                        matches!(
                            effect,
                            crate::agent::host::HostCapabilityEffect::Mutate
                                | crate::agent::host::HostCapabilityEffect::MutateAndValidate
                        ),
                        is_error,
                        failure_summary,
                        execution_skipped,
                        Some(&arguments),
                        &resource_projection,
                    );
                if !execution_skipped && outcome != HostToolOutcome::Deferred {
                    let mutation = matches!(effect, crate::agent::host::HostCapabilityEffect::Mutate
                        | crate::agent::host::HostCapabilityEffect::MutateAndValidate);
                    turn.engineering.reconcile_execution_receipt(&previous_engineering, receipt.as_ref(),
                        effect, mutation && turn.engineering.scratch_mutation_count == previous_engineering.scratch_mutation_count,
                        !is_error, &crate::agent::loop_guard::declared_effect_resources(&arguments, &resource_projection, false));
                }
                let item = AgentItem::ToolResult {
                    id: ItemId::from_string(self.session.allocate_id("tool_result")),
                    call_id:call_id.clone(),name:capability_id.clone(),output:normalized_output,
                    is_error,full_output_ref:full_output_ref.clone(),artifacts:artifacts.clone(),
                };
                self.session.record_turn_items(&turn_id,[item]);
                events.push(RuntimeEvent::ToolCompleted {
                    turn_id,
                    call_id,
                    tool_name: capability_id,
                    output,
                    outcome,
                    full_output_ref,
                    artifacts,
                });
            }
            AgentCommand::DelegatedAgentPermissionRequested {
                occurrence,
                turn_id,
                target_effect_id,
                request_id,
                call_id,
                capability_id,
                resource,
                reason,
                input,
            } => {
                self.request_delegated_permission(
                    occurrence,
                    turn_id,
                    target_effect_id,
                    request_id,
                    call_id,
                    capability_id,
                    resource,
                    reason,
                    input,
                    &mut events,
                    &mut effects,
                )?;
            }
            AgentCommand::DelegatedAgentUsage {
                turn_id,
                used_tokens,
                context_window_tokens,
                input_tokens: _,
                output_tokens: _,
            } => {
                let used_tokens = usize::try_from(used_tokens)
                    .map_err(|_| "delegated used-token count exceeds Core range".to_string())?;
                let context_window_tokens = usize::try_from(context_window_tokens)
                    .map_err(|_| "delegated context window exceeds Core range".to_string())?;
                let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                turn.pending_delegated_usage = Some(DelegatedUsageObservation {
                    used_tokens,
                    context_window_tokens,
                });
                self.session
                    .conversation
                    .context_window
                    .record_provider_context_usage(used_tokens, context_window_tokens);
            }
            AgentCommand::ModelCompleted {
                turn_id,
                items,
                tool_calls,
                usage,
                output_limited,
                effect_elapsed_millis,
            } => {
                let delegated_usage = self
                    .require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .pending_delegated_usage
                    .take();
                let delegated_context_commit = {
                    let turn = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?;
                    let backend = &turn.context.runtime_snapshot.execution_backend;
                    if backend.backend_id == "nola" {
                        None
                    } else {
                        let backend_session_id = backend
                            .backend_session_id
                            .clone()
                            .filter(|value| !value.trim().is_empty())
                            .ok_or_else(|| {
                                "delegated Agent completed without a backend session identity"
                                    .to_string()
                            })?;
                        let anchor_item_id = items.iter().rev().find_map(|item| match item {
                            AgentItem::Message {
                                id,
                                role: AgentRole::Assistant,
                                ..
                            } => Some(id.as_str().to_string()),
                            _ => None,
                        });
                        anchor_item_id.map(|anchor_item_id| {
                            (
                                backend_session_id,
                                anchor_item_id,
                                backend.context_identity_digest.clone().unwrap_or_default(),
                            )
                        })
                    }
                };
                if let Some((backend_session_id, anchor_item_id, identity_digest)) =
                    delegated_context_commit
                {
                    let selection = &mut self.session.configuration.turn_identity.execution_backend;
                    selection.backend_session_id = Some(backend_session_id.clone());
                    selection.context_anchor_item_id = Some(anchor_item_id.clone());
                    selection.context_identity_digest = Some(identity_digest.clone());
                    let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                    for selection in [
                        &mut turn.context.runtime_snapshot.execution_backend,
                        &mut turn.context.configuration.turn_identity.execution_backend,
                    ] {
                        selection.backend_session_id = Some(backend_session_id.clone());
                        selection.context_anchor_item_id = Some(anchor_item_id.clone());
                        selection.context_identity_digest = Some(identity_digest.clone());
                    }
                }
                self.record_active_skill_elapsed(
                    &turn_id,
                    effect_elapsed_millis,
                    "model",
                    &mut events,
                    &mut effects,
                )?;
                self.record_active_skill_model_turns(&turn_id, &mut events, &mut effects)?;
                let model_effect_capability = if self
                    .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .context
                    .runtime_snapshot
                    .execution_backend
                    .backend_id
                    == "nola"
                {
                    "model.stream"
                } else {
                    "agent.delegate"
                };
                if self.session.execution.requirements.model_source.is_none() {
                    if let Some(pending) = self.session.host.pending_effects.iter().find(|pending|
                        pending.turn_id.as_deref() == Some(turn_id.as_str()) && pending.capability_id == model_effect_capability) {
                        self.session.execution.requirements.model_source = Some(nola_domain::core_requirements::CoreDecisionSource::Model {
                            effect_id: pending.effect_id.clone(), request_id: pending.request_id.clone(),
                            attempt_id: None, call_id: None,
                        });
                    }
                }
                // Host completion removes the pending effect before reducing
                // this command. Its already-validated decision source retains
                // the exact request/physical Attempt; querying pending work here
                // loses both provenance and persisted cache diagnostics.
                let usage_request = self.session.execution.requirements.model_source.as_ref()
                    .and_then(|source| match source {
                        nola_domain::core_requirements::CoreDecisionSource::Model { request_id, attempt_id, .. } =>
                            Some(nola_domain::ContextUsageMeasurement {
                                request_id: request_id.clone(), attempt_id: attempt_id.clone(),
                                context_generation: Some(self.session.conversation.context_window.compaction_generation),
                            }),
                        nola_domain::core_requirements::CoreDecisionSource::Runtime { .. } => None,
                    });
                self.session.host.pending_effects.complete_matching(
                    Some(turn_id.as_str()),
                    Some(model_effect_capability),
                    None,
                );
                self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?;
                let mut tool_calls = tool_calls;
                // Account for the completed attempt even when recovery rejects
                // its output and terminates before normal content processing.
                self.session.conversation.context_window.record_provider_usage(&usage);
                if let Some(observation) = delegated_usage {
                    self.session.conversation.context_window.record_provider_context_usage(
                        observation.used_tokens, observation.context_window_tokens,
                    );
                }
                self.session.conversation.context_window.measurement = usage_request.clone();
                self.session.conversation.context_window.prompt_cache_measurement =
                    usage.prompt_cache_diagnostic.as_ref().and(usage_request);
                let loop_interruption_reason = self
                    .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .engineering
                    .no_progress_recovery.reason().map(str::to_owned);
                if let Some(reason) = loop_interruption_reason.as_ref() {
                    let selected_required_protocol =
                        loop_checkpoint_protocol_is_selected(&tool_calls);
                    let direct_answer = tool_calls.is_empty() && items.iter().any(|item|
                        matches!(item, AgentItem::Message { role: AgentRole::Assistant, content, .. }
                            if !content.trim().is_empty()));
                    if !selected_required_protocol && !direct_answer && tool_calls.is_empty() {
                        // No progress is not evidence that the user owes us a
                        // decision. Never synthesize a question or execute the
                        // rejected calls. This recovery response exhausts the
                        // current checkpoint instead of starting another loop.
                        self.conclude_no_progress(&turn_id, reason.clone(),
                            nola_domain::core_requirements::RecoveryStopCause::EmptyModelContinuation,
                            &mut events, &mut effects)?;
                        return self.finish_transition_after_model(events, effects, checkpoint_reason, completed_model_turn.as_ref());
                    }
                }
                let item_continuation = items.iter().rev().find_map(|item| match item {
                    AgentItem::Reasoning {
                        provider_continuation: Some(continuation),
                        ..
                    } => Some(continuation.clone()),
                    _ => None,
                });
                let pending_continuation = self
                    .session
                    .execution
                    .active_turn
                    .as_mut()
                    .and_then(|turn| turn.pending_provider_continuation.take());
                if let Some(continuation) = item_continuation.or(pending_continuation) {
                    let expected_model = self
                        .session
                        .execution
                        .active_turn
                        .as_ref()
                        .and_then(|turn| turn.context.configuration.model.as_deref());
                    if !continuation.is_for_model(expected_model) {
                        return Err("completed provider continuation does not match active Turn"
                            .to_string());
                    }
                    self.session.conversation.provider_continuation = Some(continuation);
                }
                let mut items = items;
                if tool_calls.iter().any(|call| call.name == super::execution_constraints::TOOL) {
                    let offered = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                        .last_model_tool_projection.as_ref().is_some_and(|projection|
                            projection.capability_ids.iter().any(|id| id == super::execution_constraints::TOOL));
                    let result = if offered && !output_limited && tool_calls[0].name == super::execution_constraints::TOOL
                        && tool_calls.iter().filter(|call|call.name == super::execution_constraints::TOOL).count() == 1
                        && !tool_calls.iter().skip(1).any(|call|matches!(call.name.as_str(),super::requirements::TOOL|super::task_scope::TOOL|super::goal_work::TOOL|super::model_planner::REQUEST_COMPLETION_REVIEW_TOOL)) {
                        self.apply_model_execution_constraint(&turn_id,&tool_calls[0].call_id,&tool_calls[0].arguments)
                    } else {
                        Err("execution_constraints_update must be offered and be the first, single constraint update in its batch; no calls in this batch executed".into())
                    };
                    // Usage and provider continuation above remain accounted
                    // once. Record one control pair; successful same-batch
                    // actions continue through ordinary admission below.
                    if result.is_ok() && tool_calls.len() > 1 {
                        for item in items.iter().filter(|item|matches!(item,AgentItem::Reasoning {..})) {
                            events.push(RuntimeEvent::ItemCompleted { turn_id:turn_id.clone(),item:item.clone() });
                            self.session.record_item(item.clone());
                        }
                        items.retain(|item|!matches!(item,AgentItem::Reasoning {..}));
                        let call = tool_calls.remove(0);
                        let item = AgentItem::ToolCall { id:ItemId::from_string(self.session.allocate_id("tool_call")),
                            call_id:call.call_id.clone(),name:call.name.clone(),arguments:call.arguments };
                        events.push(RuntimeEvent::ItemCompleted { turn_id:turn_id.clone(),item:item.clone() });
                        self.session.record_item(item);
                        let item = AgentItem::ToolResult { id:ItemId::from_string(self.session.allocate_id("tool_result")),
                            call_id:call.call_id,name:call.name,output:result.expect("successful constraint result"),
                            is_error:false,full_output_ref:None,artifacts:Vec::new() };
                        events.push(RuntimeEvent::ItemCompleted { turn_id:turn_id.clone(),item:item.clone() });
                        self.session.record_item(item);
                    } else {
                        let open_items = self.require_turn(&turn_id,AgentTurnStatus::AwaitingModel)?.open_stream_items.clone();
                        for item_id in open_items {
                        // A final control call resolves its existing streamed Tool Part.
                        // Discarding it first would terminalize it and create a second owner.
                        if self.is_streamed_control_call(&turn_id, &item_id, &tool_calls) { continue; }
                            events.push(RuntimeEvent::ItemDiscarded { turn_id:turn_id.clone(),item_id,disposition:ItemDiscardDisposition::Process });
                        }
                        for item in items.iter().filter(|item|matches!(item,AgentItem::Reasoning {..})) {
                            events.push(RuntimeEvent::ItemCompleted { turn_id:turn_id.clone(),item:item.clone() });
                            self.session.record_item(item.clone());
                        }
                        for call in &tool_calls {
                            let item = AgentItem::ToolCall { id:ItemId::from_string(self.session.allocate_id("tool_call")),
                                call_id:call.call_id.clone(),name:call.name.clone(),arguments:call.arguments.clone() };
                            events.push(RuntimeEvent::ItemCompleted { turn_id:turn_id.clone(),item:item.clone() });
                            self.session.record_item(item);
                            let item = AgentItem::ToolResult { id:ItemId::from_string(self.session.allocate_id("tool_result")),
                                call_id:call.call_id.clone(),name:call.name.clone(),
                                output:match &result { Ok(value)=>value.clone(),Err(message)=>json!({"error":"execution_constraint_update_rejected","message":message,"tool_execution_skipped":true}) },
                                is_error:result.is_err(),full_output_ref:None,artifacts:Vec::new() };
                            events.push(RuntimeEvent::ItemCompleted { turn_id:turn_id.clone(),item:item.clone() });
                            self.session.record_item(item);
                        }
                        self.require_turn_mut(&turn_id,AgentTurnStatus::AwaitingModel)?.clear_stream_items();
                        self.request_model(&turn_id,&mut events,&mut effects)?;
                        return self.finish_transition_after_model(events,effects,checkpoint_reason,completed_model_turn.as_ref());
                    }
                }
                if tool_calls.iter().any(|call| matches!(call.name.as_str(), super::requirements::TOOL | super::task_scope::TOOL)) {
                    let control = tool_calls.iter().find(|call| matches!(call.name.as_str(), super::requirements::TOOL | super::task_scope::TOOL)).expect("control call").name.clone();
                    let offered = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                        .last_model_tool_projection.as_ref().is_some_and(|projection|
                            projection.capability_ids.iter().any(|id| id == &control));
                    let has_final_body=items.iter().any(|item|matches!(item,AgentItem::Message{role:AgentRole::Assistant,content,..} if !content.trim().is_empty()));
                    let result = if offered && !output_limited && tool_calls.len() == 1 && !has_final_body {
                        if control == super::task_scope::TOOL {
                            self.apply_model_task_scope(&turn_id, &tool_calls[0].call_id, &tool_calls[0].arguments, &mut events)
                        } else {
                            self.apply_model_requirement_control(&turn_id, &tool_calls[0].call_id,
                                &tool_calls[0].arguments, &mut events)
                        }
                    } else {
                        Err(format!("{control} must be offered and called alone; no calls in this batch were executed"))
                    };
                    self.session.execution.requirements.control_recovery = match &result {
                        Ok(_) => super::requirements::RequirementControlRecovery::Clear,
                        Err(detail) => super::requirements::RequirementControlRecovery::Rejected {
                            call_id: tool_calls.iter().find(|call| call.name == control)
                                .expect("requirements control branch contains its call").call_id.as_str().to_owned(),
                            detail: detail.clone(),
                        },
                    };
                    let open_items = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?.open_stream_items.clone();
                    for item_id in open_items {
                        // A final control call resolves its existing streamed Tool Part.
                        // Discarding it first would terminalize it and create a second owner.
                        if self.is_streamed_control_call(&turn_id, &item_id, &tool_calls) { continue; }
                        events.push(RuntimeEvent::ItemDiscarded { turn_id: turn_id.clone(), item_id,
                            disposition: ItemDiscardDisposition::Process });
                    }
                    for item in items.iter().filter(|item| matches!(item, AgentItem::Reasoning { .. })) {
                        events.push(RuntimeEvent::ItemCompleted { turn_id: turn_id.clone(), item: item.clone() });
                        self.session.record_item(item.clone());
                    }
                    for call in &tool_calls {
                        let item = AgentItem::ToolCall { id: ItemId::from_string(self.session.allocate_id("tool_call")),
                            call_id: call.call_id.clone(), name: call.name.clone(), arguments: call.arguments.clone() };
                        events.push(RuntimeEvent::ItemCompleted { turn_id: turn_id.clone(), item: item.clone() });
                        self.session.record_item(item);
                        let item = AgentItem::ToolResult { id: ItemId::from_string(self.session.allocate_id("tool_result")),
                            call_id: call.call_id.clone(), name: call.name.clone(),
                            output: match &result { Ok((value,_)) => value.clone(), Err(error) => json!({"error":error}) },
                            is_error: result.is_err(), full_output_ref: None, artifacts: Vec::new() };
                        events.push(RuntimeEvent::ItemCompleted { turn_id: turn_id.clone(), item: item.clone() });
                        self.session.record_item(item);
                    }
                    self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?.clear_stream_items();
                    if let Ok((_,Some(answer))) = &result {
                        let item = AgentItem::Message { id: ItemId::from_string(self.session.allocate_id("answer")),
                            role: AgentRole::Assistant, content: answer.clone() };
                        events.push(RuntimeEvent::ItemCompleted { turn_id: turn_id.clone(), item: item.clone() });
                        self.session.record_item(item);
                    }
                    if result.is_ok() && self.session.execution.requirements.proposed.is_some() {
                        self.complete_turn(&turn_id, &mut events, &mut effects)?;
                    } else if let Some(reason) = loop_interruption_reason {
                        self.conclude_no_progress(&turn_id, reason,
                            nola_domain::core_requirements::RecoveryStopCause::UnfinishedRequirementControl {
                                call_id: tool_calls.iter().find(|call| call.name == control)
                                    .expect("requirements branch").call_id.as_str().to_owned(),
                            }, &mut events, &mut effects)?;
                    } else {
                        self.request_model(&turn_id, &mut events, &mut effects)?;
                    }
                    return self.finish_transition_after_model(events, effects, checkpoint_reason, completed_model_turn.as_ref());
                }
                if tool_calls.iter().any(|call| call.name == super::goal_work::TOOL) {
                    let offered = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                        .last_model_tool_projection.as_ref().is_some_and(|projection|
                            projection.capability_ids.iter().any(|id| id == super::goal_work::TOOL));
                    let result = if offered && !output_limited && tool_calls.len() == 1 {
                        let call = &tool_calls[0];
                        super::goal_work::apply_model_update(&mut self.session.goal_projection,
                            &call.arguments, &format!("{}:{}", turn_id.as_str(), call.call_id.as_str()),
                            self.observed_at_ms.min(i64::MAX as u64) as i64)
                    } else {
                        Err("goal_update must be offered for this Goal turn and called alone; no calls in this batch were executed".into())
                    };
                    if result.is_ok() {
                        events.push(RuntimeEvent::GoalUpdated {
                            goal_projection_revision: self.session.goal_projection.revision,
                            goal: self.session.goal_projection.clone(),
                        });
                    }
                    let open_items = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?.open_stream_items.clone();
                    for item_id in open_items {
                        // A final control call resolves its existing streamed Tool Part.
                        // Discarding it first would terminalize it and create a second owner.
                        if self.is_streamed_control_call(&turn_id, &item_id, &tool_calls) { continue; }
                        events.push(RuntimeEvent::ItemDiscarded { turn_id: turn_id.clone(), item_id,
                            disposition: ItemDiscardDisposition::Process });
                    }
                    for item in items.iter().filter(|item| matches!(item, AgentItem::Reasoning { .. })) {
                        events.push(RuntimeEvent::ItemCompleted { turn_id: turn_id.clone(), item: item.clone() });
                        self.session.record_item(item.clone());
                    }
                    for call in &tool_calls {
                        let item = AgentItem::ToolCall {
                            id: ItemId::from_string(self.session.allocate_id("tool_call")),
                            call_id: call.call_id.clone(), name: call.name.clone(), arguments: call.arguments.clone(),
                        };
                        events.push(RuntimeEvent::ItemCompleted { turn_id: turn_id.clone(), item: item.clone() });
                        self.session.record_item(item);
                        let item = AgentItem::ToolResult {
                            id: ItemId::from_string(self.session.allocate_id("tool_result")),
                            call_id: call.call_id.clone(), name: call.name.clone(),
                            output: match &result {
                                Ok(()) => json!({"updated":true,"revision":self.session.goal_projection.revision}),
                                Err(error) => json!({"error":error,"revision":self.session.goal_projection.revision}),
                            },
                            is_error: result.is_err(), full_output_ref: None, artifacts: Vec::new(),
                        };
                        events.push(RuntimeEvent::ItemCompleted { turn_id: turn_id.clone(), item: item.clone() });
                        self.session.record_item(item);
                    }
                    self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?.clear_stream_items();
                    self.request_model(&turn_id, &mut events, &mut effects)?;
                    return self.finish_transition_after_model(events, effects, checkpoint_reason, completed_model_turn.as_ref());
                }
                if tool_calls.iter().any(|call| {
                    call.name == super::model_planner::REQUEST_COMPLETION_REVIEW_TOOL
                }) {
                    let review_is_offered = self
                        .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                        .last_model_tool_projection
                        .as_ref()
                        .is_some_and(|projection| projection.capability_ids.iter().any(|id| {
                            id == super::model_planner::REQUEST_COMPLETION_REVIEW_TOOL
                        }));
                    let review_ready = {
                        let turn = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?;
                        !turn.engineering.validation_required && !turn.engineering.artifact_completion_pending()
                    };
                    let candidate = (review_is_offered && review_ready && !output_limited && tool_calls.len() == 1)
                        .then(|| tool_calls[0].arguments.get("candidate_answer")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|text| !text.is_empty() && text.len() <= 100_000))
                        .flatten()
                        .map(str::to_string);
                    let open_items = self
                        .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                        .open_stream_items
                        .clone();
                    let supports_reviewed_text = self
                        .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                        .context.configuration.turn_identity.supports_reviewed_text_parts;
                    let candidate_part_id = if supports_reviewed_text {
                        candidate.as_deref().map(|candidate| {
                            self.capture_review_candidate(&turn_id, &open_items, candidate,
                                &mut events)
                        }).transpose()?
                    } else { None };
                    for item_id in open_items {
                        // A final control call resolves its existing streamed Tool Part.
                        // Discarding it first would terminalize it and create a second owner.
                        if self.is_streamed_control_call(&turn_id, &item_id, &tool_calls) { continue; }
                        if candidate_part_id.as_ref() == Some(&item_id) {
                            continue;
                        }
                        events.push(RuntimeEvent::ItemDiscarded {
                            turn_id: turn_id.clone(),
                            item_id,
                            disposition: ItemDiscardDisposition::Hidden,
                        });
                    }
                    for item in items.iter().filter(|item| matches!(item, AgentItem::Reasoning { .. })) {
                        events.push(RuntimeEvent::ItemCompleted {
                            turn_id: turn_id.clone(),
                            item: item.clone(),
                        });
                        self.session.record_item(item.clone());
                    }
                    let valid = candidate.is_some();
                    for call in &tool_calls {
                        let call_item = AgentItem::ToolCall {
                            id: ItemId::from_string(self.session.allocate_id("tool_call")),
                            call_id: call.call_id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        };
                        events.push(RuntimeEvent::ItemCompleted {
                            turn_id: turn_id.clone(), item: call_item.clone(),
                        });
                        self.session.record_item(call_item);
                        let result_item = AgentItem::ToolResult {
                            id: ItemId::from_string(self.session.allocate_id("tool_result")),
                            call_id: call.call_id.clone(),
                            name: call.name.clone(),
                            output: if valid { json!({"review_requested": true}) } else {
                                json!({"error":"invalid_review_request","message":"Review requires no pending validation or artifact completion. Request it as the only tool call with a nonempty candidate_answer of at most 100000 bytes; no other tool in this batch was executed."})
                            },
                            is_error: !valid,
                            full_output_ref: None,
                            artifacts: Vec::new(),
                        };
                        events.push(RuntimeEvent::ItemCompleted {
                            turn_id: turn_id.clone(), item: result_item.clone(),
                        });
                        self.session.record_item(result_item);
                    }
                    let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                    turn.clear_stream_items();
                    if let Some(candidate) = candidate {
                        turn.final_answer_audit_required = true;
                        turn.final_answer_retry_requested = true;
                        turn.final_answer_draft = Some(candidate);
                        turn.final_answer_draft_part_id = candidate_part_id;
                        turn.final_answer_audit_report = None;
                    }
                    self.request_model(&turn_id, &mut events, &mut effects)?;
                    return self.finish_transition_after_model(events, effects, checkpoint_reason, completed_model_turn.as_ref());
                }
                let visible_text_in_items = items.iter().any(|item| {
                    matches!(
                        item,
                        AgentItem::Message {
                            role: AgentRole::Assistant,
                            content,
                            ..
                        } if !content.trim().is_empty()
                    )
                });
                let reasoning_in_items = items.iter().any(|item| {
                    matches!(
                        item,
                        AgentItem::Reasoning { content, .. } if !content.trim().is_empty()
                    )
                });
                let (
                    current_stream_item_ids,
                    current_stream_has_visible_text,
                    has_executed_tools,
                    final_answer_audit_required,
                    final_answer_retry_requested,
                    final_answer_draft,
                    final_answer_audit_report,
                    reasoning_only_retry_requested,
                    empty_completion_retry_requested,
                    output_limit_retry_requested,
                    output_limit_recovery_anchor,
                    user_denied_permission,
                    iteration_budget_final_synthesis,
                    tool_free_request,
                    engineering,
                ) = {
                    let turn = self
                        .session
                        .execution
                        .active_turn
                        .as_ref()
                        .expect("active turn was validated above");
                    let ids = turn.open_stream_items.clone();
                    let has_visible_text = ids.iter().any(|item_id| {
                        matches!(
                            self.session
                                .conversation
                                .projection
                                .parts
                                .get(item_id.as_str()),
                            Some(AgentPart::Text(part)) if !part.text.trim().is_empty()
                        )
                    });
                    (
                        ids,
                        has_visible_text,
                        turn.has_executed_tools,
                        turn.final_answer_audit_required,
                        turn.final_answer_retry_requested,
                        turn.final_answer_draft.clone(),
                        turn.final_answer_audit_report.clone(),
                        turn.reasoning_only_retry_requested,
                        turn.empty_completion_retry_requested,
                        turn.output_limit_retry_requested,
                        turn.output_limit_recovery_anchor.clone(),
                        turn.user_denied_permission,
                        turn.iteration_budget_final_synthesis,
                        turn.last_model_tool_projection.as_ref().is_some_and(|projection| {
                            projection.capability_ids.is_empty()
                        }),
                        turn.engineering.clone(),
                    )
                };
                let has_visible_answer = visible_text_in_items || current_stream_has_visible_text;
                let skill_completion_pending =
                    !self.session.pending_skill_completion_steps().is_empty();
                let current_answer_text = {
                    let item_text = items
                        .iter()
                        .filter_map(|item| match item {
                            AgentItem::Message {
                                role: AgentRole::Assistant,
                                content,
                                ..
                            } if !content.trim().is_empty() => Some(content.trim()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    if !item_text.is_empty() {
                        item_text
                    } else {
                        current_stream_item_ids
                            .iter()
                            .filter_map(|item_id| {
                                match self
                                    .session
                                    .conversation
                                    .projection
                                    .parts
                                    .get(item_id.as_str())
                                {
                                    Some(AgentPart::Text(part)) if !part.text.trim().is_empty() => {
                                        Some(part.text.trim())
                                    }
                                    _ => None,
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    }
                };
                let mut normalized_report = normalize_final_answer_audit(&current_answer_text);
                if let Some(report) = normalized_report.as_ref() {
                    let value: Value = serde_json::from_str(report).expect("normalized audit");
                    if !audit_receipts_valid(&value, &self.session.execution.active_turn.as_ref().expect("active turn").successful_mutation_calls) {
                        // An invalid audit is not evidence of unfinished work.
                        // Retry this tool-free audit, retaining its candidate;
                        // never synthesize an execute decision that could replay
                        // a successful non-idempotent operation.
                        normalized_report = None;
                    }
                }
                let completion =
                    nola_kernel::plan_model_completion(&nola_domain::ModelCompletionFacts {
                        iteration_budget_final_synthesis,
                        has_tool_calls: !tool_calls.is_empty(),
                        tool_free_request,
                        has_visible_answer,
                        structurally_repetitive: model_output_is_structurally_repetitive(
                            &current_answer_text,
                        ),
                        continuation_made_no_progress: output_limit_recovery_anchor
                            .as_deref()
                            .is_some_and(|anchor| {
                                !model_output_has_continuation_progress(
                                    anchor,
                                    &current_answer_text,
                                )
                            }),
                        output_limited,
                        output_limit_retry_requested,
                        user_denied_permission,
                        final_answer_retry_requested,
                        final_answer_draft_present: final_answer_draft.is_some(),
                        final_answer_audit_report_present: final_answer_audit_report.is_some(),
                        normalized_audit_report_valid: normalized_report.is_some(),
                        audit_accepts_candidate: normalized_report.as_deref().is_some_and(|report| {
                            let report: Value = serde_json::from_str(report).expect("normalized audit");
                            matches!(report["decision"].as_str(), Some("accept" | "clarify"))
                                && audit_receipts_valid(&report, &self.session.execution.active_turn.as_ref()
                                    .expect("active turn").successful_mutation_calls)
                        }),
                        audit_blocks_task: normalized_report.as_deref().is_some_and(|report| {
                            serde_json::from_str::<Value>(report).expect("normalized audit")["decision"] == "blocked"
                        }),
                        audit_requests_input: normalized_report.as_deref().is_some_and(|report| {
                            serde_json::from_str::<Value>(report).expect("normalized audit")["decision"] == "clarify"
                        }),
                        audit_requires_execution: normalized_report.as_deref().is_some_and(|report| {
                            let report: Value = serde_json::from_str(report).expect("normalized audit");
                            report["decision"] == "execute" || !audit_receipts_valid(&report,
                                &self.session.execution.active_turn.as_ref().expect("active turn").successful_mutation_calls)
                        }),
                        completion_recovery_count: self.session.execution.active_turn.as_ref()
                            .expect("active turn").completion_recovery_count,
                        answer_revision_count: self.session.execution.active_turn.as_ref()
                            .expect("active turn").answer_revision_count,
                        embedded_tool_protocol: contains_embedded_tool_call_protocol(
                            &current_answer_text,
                        ),
                        final_answer_audit_required,
                        has_executed_tools,
                        reasoning_in_items,
                        reasoning_only_retry_requested,
                        empty_completion_retry_requested,
                        no_progress_count: self
                            .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                            .final_answer_no_progress_count,
                        validation_required: engineering.validation_required
                            || engineering.artifact_completion_pending(),
                        recovery_required: engineering.has_protocol_recovery(),
                        skill_completion_pending,
                    });
                let stream_review = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .context.configuration.turn_identity.supports_reviewed_text_parts;
                let mut retained_candidate_id = None;
                if stream_review && matches!(completion.update.audit,
                    nola_domain::ModelCompletionAuditUpdate::SaveDraft
                    | nola_domain::ModelCompletionAuditUpdate::ReviseDraft)
                {
                    let id = self.capture_review_candidate(&turn_id, &current_stream_item_ids,
                        &current_answer_text, &mut events)?;
                    self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?
                        .final_answer_draft_part_id = Some(id.clone());
                    retained_candidate_id = Some(id);
                }
                if stream_review && matches!(completion.update.audit,
                    nola_domain::ModelCompletionAuditUpdate::ResumeExecution
                    | nola_domain::ModelCompletionAuditUpdate::AcceptReport)
                    && !matches!(completion.action, nola_domain::ModelCompletionAction::PublishReviewedCandidate { .. })
                {
                    if let Some(id) = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?
                        .final_answer_draft_part_id.take()
                    {
                        events.push(RuntimeEvent::TextReviewChanged { turn_id: turn_id.clone(), item_id: id,
                            state: crate::agent::parts::TextReviewState::Rejected });
                    }
                }
                {
                    let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                    if let Some(count) = completion.update.no_progress_count {
                        turn.final_answer_no_progress_count = count;
                    }
                    turn.final_answer_retry_requested |= completion.update.request_final_answer;
                    turn.reasoning_only_retry_requested |=
                        completion.update.request_reasoning_continuation;
                    turn.empty_completion_retry_requested |=
                        completion.update.request_empty_continuation;
                    match completion.update.audit {
                        nola_domain::ModelCompletionAuditUpdate::ResumeExecution => {
                            // A review that found missing work reopens execution
                            // under the admitted capability and response contracts.
                            turn.completion_recovery_count = turn.completion_recovery_count.saturating_add(1);
                            turn.completion_recovery_feedback = normalized_report;
                            turn.final_answer_draft = None;
                            turn.final_answer_audit_report = None;
                            turn.final_answer_audit_required = false;
                            turn.final_answer_retry_requested = false;
                            turn.completion_outcome = Some(nola_domain::TaskCompletionOutcome::Incomplete);
                        }
                        nola_domain::ModelCompletionAuditUpdate::Unchanged => {}
                        nola_domain::ModelCompletionAuditUpdate::AcceptReport => {
                            turn.final_answer_audit_report = normalized_report;
                        }
                        nola_domain::ModelCompletionAuditUpdate::ReviseDraft => {
                            turn.answer_revision_count = turn.answer_revision_count.saturating_add(1);
                            turn.final_answer_draft = Some(if stream_review { current_answer_text.clone() } else { current_answer_text.trim().to_string() });
                            turn.final_answer_audit_report = None;
                        }
                        nola_domain::ModelCompletionAuditUpdate::SaveDraft => {
                            turn.final_answer_draft = Some(if stream_review { current_answer_text.clone() } else { current_answer_text.trim().to_string() });
                            turn.final_answer_audit_report = None;
                        }
                    }
                }
                match completion.action {
                    nola_domain::ModelCompletionAction::PublishReviewedCandidate { blocked } => {
                        for item_id in current_stream_item_ids {
                            events.push(RuntimeEvent::ItemDiscarded {
                                turn_id: turn_id.clone(), item_id,
                                disposition: ItemDiscardDisposition::Hidden,
                            });
                        }
                        let draft = final_answer_draft.expect("reviewed candidate exists");
                        let id = if let Some(id) = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?.final_answer_draft_part_id.clone() {
                            events.push(RuntimeEvent::TextReviewChanged { turn_id: turn_id.clone(), item_id: id.clone(),
                                state: crate::agent::parts::TextReviewState::Approved });
                            id
                        } else {
                            ItemId::from_string(self.session.allocate_id("reviewed-answer"))
                        };
                        let item = AgentItem::assistant_message(id, draft);
                        events.push(RuntimeEvent::ItemCompleted { turn_id: turn_id.clone(), item: item.clone() });
                        self.session.record_item(item);
                        let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                        turn.clear_stream_items();
                        let report: Value = serde_json::from_str(turn.final_answer_audit_report.as_deref().expect("accepted report")).expect("normalized audit");
                        turn.completion_outcome = Some(serde_json::from_value(report["outcome"].clone()).expect("validated outcome"));
                        if blocked {
                            self.complete_turn(&turn_id, &mut events, &mut effects)?;
                        } else if report["outcome"] == "needs_input" {
                            let questions = serde_json::from_value(report["question_set"].clone()).expect("validated question schema");
                            self.open_review_user_input_wait(&turn_id, questions, &mut events)?;
                        } else {
                            self.complete_turn(&turn_id, &mut events, &mut effects)?;
                        }
                    }
                    action @ (nola_domain::ModelCompletionAction::DiscardAndRequestModel
                    | nola_domain::ModelCompletionAction::ConcludeIncomplete { .. }
                    | nola_domain::ModelCompletionAction::DiscardAndFail(_)
                    | nola_domain::ModelCompletionAction::DiscardAndComplete) => {
                        for item_id in current_stream_item_ids {
                            if retained_candidate_id.as_ref() == Some(&item_id) {
                                continue;
                            }
                            events.push(RuntimeEvent::ItemDiscarded {
                                turn_id: turn_id.clone(),
                                item_id,
                                disposition: ItemDiscardDisposition::Hidden,
                            });
                        }
                        self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?
                            .open_stream_items
                            .clear();
                        match action {
                            nola_domain::ModelCompletionAction::ConcludeIncomplete { reason } => {
                                let scope = self.session.execution.requirements.scope.as_ref()
                                    .ok_or("review conclusion has no execution requirement scope")?;
                                let code = match reason {
                                    nola_domain::ModelCompletionIncompleteReason::OutputLimitRecoveryExhausted => "model.output_incomplete",
                                    nola_domain::ModelCompletionIncompleteReason::ExecutionRecoveryBudget => "review.execution_recovery_budget",
                                    nola_domain::ModelCompletionIncompleteReason::ReviewRevisionBudget => "review.revision_budget",
                                };
                                self.session.execution.requirements.proposed = Some(nola_domain::run_result::RunResultValue::Incomplete {
                                    reason: nola_domain::run_result::RunResultReason { code: code.into(), detail: self.session.execution.active_turn.as_ref().and_then(|turn| turn.final_answer_audit_report.clone()) },
                                    remaining_requirement_refs: scope.requirements.iter().map(|requirement| requirement.requirement_id.clone()).collect(),
                                    obligation_refs: Vec::new(),
                                });
                                self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?.completion_outcome = Some(nola_domain::TaskCompletionOutcome::Incomplete);
                                self.complete_turn(&turn_id, &mut events, &mut effects)?;
                            }
                            nola_domain::ModelCompletionAction::DiscardAndRequestModel => {
                                self.request_model(&turn_id, &mut events, &mut effects)?
                            }
                            nola_domain::ModelCompletionAction::DiscardAndComplete => {
                                self.complete_turn(&turn_id, &mut events, &mut effects)?
                            }
                            nola_domain::ModelCompletionAction::DiscardAndFail(failure) => {
                                let error = failure.to_string();
                                let session_id = self.session.session_id.clone();
                                let turn = self
                                    .require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                                turn.status = AgentTurnStatus::Failed;
                                turn.failure = Some(error.clone());
                                events.push(RuntimeEvent::TurnFailed {
                                    session_id,
                                    turn_id: turn_id.clone(),
                                    error,
                                });
                            }
                            _ => unreachable!("matched discard action"),
                        }
                    }
                    nola_domain::ModelCompletionAction::CommitAndRecoverOutputLimit => {
                        for item in items {
                            events.push(RuntimeEvent::ItemCompleted {
                                turn_id: turn_id.clone(),
                                item: item.clone(),
                            });
                            self.session.record_item(item);
                        }
                        let turn =
                            self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                        turn.open_stream_items.clear();
                        turn.output_limit_retry_requested = true;
                        turn.output_limit_recovery_anchor = Some(current_answer_text);
                        self.request_model(&turn_id, &mut events, &mut effects)?;
                    }
                    nola_domain::ModelCompletionAction::CommitCompletion { has_tool_calls } => {
                        // A response cited by a Goal completion report is the
                        // deliverable, not a preamble to an external action.
                        // Preserve its public text without bypassing review or
                        // treating the unverified report as Goal acceptance.
                        let goal_response = !final_answer_audit_required && goal_report_delivers_response(&tool_calls);
                        let mut retained_stream_response = false;
                        if has_tool_calls && !goal_response && !current_answer_text.trim().is_empty() {
                            let candidate = current_answer_text.trim().to_string();
                            let turn =
                                self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                            if !turn
                                .provisional_assistant_drafts
                                .iter()
                                .any(|draft| draft == &candidate)
                            {
                                turn.provisional_assistant_drafts.push(candidate);
                            }
                        }
                        if !tool_calls.is_empty() {
                            let (provisional_text_items, committed_text_items) = self
                                .session
                                .execution
                                .active_turn
                                .as_ref()
                                .map(|turn| {
                                    (
                                        turn.open_stream_items.clone(),
                                        turn.committed_stream_items.clone(),
                                    )
                                })
                                .unwrap_or_default();
                            for item_id in provisional_text_items {
                                let text = match self
                                    .session
                                    .conversation
                                    .projection
                                    .parts
                                    .get(item_id.as_str())
                                {
                                    Some(AgentPart::Text(part)) => Some(part.text.clone()),
                                    _ => None,
                                };
                                if let Some(text) = text {
                                    if (committed_text_items.contains(&item_id) || goal_response) && !text.is_empty() {
                                        retained_stream_response |= goal_response;
                                        let item = AgentItem::assistant_message(item_id, text);
                                        events.push(RuntimeEvent::ItemCompleted {
                                            turn_id: turn_id.clone(),
                                            item: item.clone(),
                                        });
                                        self.session.record_item(item);
                                    } else {
                                        events.push(RuntimeEvent::ItemDiscarded {
                                            turn_id: turn_id.clone(),
                                            item_id,
                                            disposition: ItemDiscardDisposition::Process,
                                        });
                                    }
                                }
                            }
                        }
                        for item in items {
                            if has_tool_calls && (!goal_response || retained_stream_response)
                                && matches!(
                                    item,
                                    AgentItem::Message {
                                        role: AgentRole::Assistant,
                                        ..
                                    }
                                )
                            {
                                // Text emitted by a tool-capable step is private
                                // planning/progress, not a durable user answer.
                                // The explicit post-tool synthesis step owns the
                                // first assistant message that may be committed.
                                continue;
                            }
                            events.push(RuntimeEvent::ItemCompleted {
                                turn_id: turn_id.clone(),
                                item: item.clone(),
                            });
                            self.session.record_item(item);
                        }
                        // Normalize executable calls before journaling, dedup,
                        // dependency tracking, permission checks and dispatch.
                        // Core control calls above retain their strict original
                        // wire representation for Store provenance validation.
                        let catalog = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                            .context.capability_projection.frozen_host_catalog();
                        for call in &mut tool_calls {
                            crate::agent::tool_lifecycle::canonicalize_tool_arguments(
                                &mut call.arguments, catalog.resolve_available(&call.name));
                        }
                        for call in &tool_calls {
                            let item = AgentItem::ToolCall {
                                id: ItemId::from_string(self.session.allocate_id("tool_call")),
                                call_id: call.call_id.clone(),
                                name: call.name.clone(),
                                arguments: call.arguments.clone(),
                            };
                            events.push(RuntimeEvent::ItemCompleted {
                                turn_id: turn_id.clone(),
                                item: item.clone(),
                            });
                            self.session.record_item(item);
                        }
                        // A prior deterministic failure belongs to the concrete
                        // call, not to the tool name. Blocking every later call to
                        // that tool made a corrected `bash { command: "pwd" }`
                        // inherit an unrelated rejected heredoc. The canonical
                        // batch guard below already compares tool names *and*
                        // arguments, so materially different recovery calls must
                        // be admitted and observed normally.
                        // Waiting for an owned, live task observes external work;
                        // identical arguments do not establish a failed strategy.
                        // Keep ordinary calls in mixed batches protected, and do
                        // not reset their history when a wait intervenes.
                        let guarded_calls = tool_calls.iter().filter(|call| {
                            !self.session.task_projection.is_live_wait_call(
                                call, &self.session.session_id, &turn_id)
                        }).cloned().collect::<Vec<_>>();
                        let repeated_tool_batch = {
                            let turn =
                                self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                            tool_calls.iter().any(|call| turn.engineering.rejects_strategy_call(call))
                                || (!guarded_calls.is_empty() && turn.observes_doom_loop(&guarded_calls))
                        };
                        if repeated_tool_batch {
                            let repeated_count = self
                                .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                                .repeated_tool_batch_count;
                            let capabilities = tool_calls
                                .iter()
                                .map(|call| call.name.as_str())
                                .collect::<BTreeSet<_>>()
                                .into_iter()
                                .collect::<Vec<_>>()
                                .join(", ");
                            let failure = format!(
                                "The identical typed tool batch ({capabilities}) was requested {repeated_count} consecutive times without progress."
                            );
                            for call in &tool_calls {
                                let output = json!({
                                    "error": "repeated_tool_batch",
                                    "message": failure.clone(),
                                    "capabilities": capabilities,
                                    "consecutive_count": repeated_count,
                                    "tool_execution_skipped": true,
                                    "next_action": "choose_a_materially_different_strategy_or_report_incomplete",
                                });
                                let result_item = AgentItem::ToolResult {
                                    id: ItemId::from_string(
                                        self.session.allocate_id("tool_result"),
                                    ),
                                    call_id: call.call_id.clone(),
                                    name: call.name.clone(),
                                    output: output.clone(),
                                    is_error: true,
                                    full_output_ref: None,
                                    artifacts: Vec::new(),
                                };
                                self.session.record_item(result_item);
                                events.push(RuntimeEvent::ToolCompleted {
                                    turn_id: turn_id.clone(),
                                    call_id: call.call_id.clone(),
                                    tool_name: call.name.clone(),
                                    output,
                                    outcome: HostToolOutcome::Failed,
                                    full_output_ref: None,
                                    artifacts: Vec::new(),
                                });
                            }
                            {
                                let turn = self
                                    .require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                                turn.engineering.record_rejected_strategy_calls(&tool_calls);
                                turn.engineering.require_no_progress_recovery(format!(
                                    "the identical typed tool batch ({capabilities}) was requested {repeated_count} consecutive times"
                                ));
                                turn.clear_stream_items();
                                turn.has_executed_tools = true;
                                // A new model batch does not own the lifetime
                                // of an early-returned cell's pending children.
                                turn.tools.retain(|tool| turn.code_cells.values().any(|cell|
                                    !cell.is_settled() && cell.owns_execution_call(tool.call_id.as_str())));
                            }
                            self.request_model(&turn_id, &mut events, &mut effects)?;
                        } else {
                            {
                                let turn = self
                                    .require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                                turn.clear_stream_items();
                                turn.has_executed_tools |= !tool_calls.is_empty();
                                let next = turn.admit_tool_calls(tool_calls);
                                turn.tools.retain(|tool| turn.code_cells.values().any(|cell|
                                    !cell.is_settled() && cell.owns_execution_call(tool.call_id.as_str())));
                                turn.tools.extend(next);
                            }
                            if self
                                .session
                                .execution
                                .active_turn
                                .as_ref()
                                .is_some_and(|turn| turn.tools.is_empty())
                            {
                                self.complete_turn(&turn_id, &mut events, &mut effects)?;
                            } else {
                                self.schedule_tools(&turn_id, &mut events, &mut effects)?;
                            }
                        }
                    }
                }
            }
            AgentCommand::ModelFailed { turn_id, error } => {
                let model_effect_capability = if self
                    .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .context
                    .runtime_snapshot
                    .execution_backend
                    .backend_id
                    == "nola"
                {
                    "model.stream"
                } else {
                    "agent.delegate"
                };
                self.session.host.pending_effects.complete_matching(
                    Some(turn_id.as_str()),
                    Some(model_effect_capability),
                    None,
                );
                let session_id = self.session.session_id.clone();
                let turn = self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?;
                turn.pending_provider_continuation = None;
                turn.status = AgentTurnStatus::Failed;
                turn.failure = Some(error.clone());
                events.push(RuntimeEvent::TurnFailed {
                    session_id,
                    turn_id,
                    error,
                });
            }
            AgentCommand::RecoverContextLimit { turn_id, error: _ } => {
                let model_effect_capability = if self
                    .require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .context
                    .runtime_snapshot
                    .execution_backend
                    .backend_id
                    == "nola"
                {
                    "model.stream"
                } else {
                    "agent.delegate"
                };
                self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .pending_provider_continuation = None;
                self.session.host.pending_effects.complete_matching(
                    Some(turn_id.as_str()),
                    Some(model_effect_capability),
                    None,
                );
                if let Err(compaction_error) = self.request_compaction(
                    CompactionReason::ContextLimitRecovery,
                    &mut events,
                    &mut effects,
                ) {
                    let session_id = self.session.session_id.clone();
                    let failure = format!("context limit recovery failed: {compaction_error}");
                    let turn = self
                        .session
                        .execution
                        .active_turn
                        .as_mut()
                        .filter(|turn| turn.turn_id == turn_id)
                        .ok_or_else(|| "context recovery lost the active turn".to_string())?;
                    turn.status = AgentTurnStatus::Failed;
                    turn.failure = Some(failure.clone());
                    events.push(RuntimeEvent::TurnFailed {
                        session_id,
                        turn_id,
                        error: failure,
                    });
                }
            }
            AgentCommand::PermissionResolved {
                request_id,
                decision,
                grant,
            } => {
                self.resolve_permission(request_id, decision, grant, &mut events, &mut effects)?;
            }
            AgentCommand::PermissionIntervened {
                request_id,
                intervention,
            } => {
                self.intervene_permission(request_id, intervention, &mut events, &mut effects)?;
            }
            AgentCommand::ToolCompleted {
                turn_id,
                call_id,
                capability_id,
                evidence,
                mut output,
                mut outcome,
                activation,
                full_output_ref,
                mut artifacts,
                skill_invocation,
                skill_runtime_update,
                effect_elapsed_millis,
            } => {
                self.session.host.pending_effects.complete_matching(
                    Some(turn_id.as_str()),
                    None,
                    Some(call_id.as_str()),
                );
                let materialized_skill_capabilities = skill_invocation.as_ref().map(|invocation| {
                    invocation
                        .required_tools
                        .iter()
                        .map(|reference| reference.model_capability_id())
                        .collect::<Vec<_>>()
                });
                if skill_invocation
                    .as_ref()
                    .is_some_and(|invocation| !invocation.required_tools.is_empty())
                {
                    let activation = activation.as_ref().ok_or_else(|| {
                        "Skill Host response omitted required capability descriptors".to_string()
                    })?;
                    let required = materialized_skill_capabilities
                        .as_ref()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .collect::<std::collections::BTreeSet<_>>();
                    let activated = activation
                        .capability_ids
                        .iter()
                        .cloned()
                        .collect::<std::collections::BTreeSet<_>>();
                    if activation.source != "skill" || activated != required {
                        return Err(
                            "Skill capability activation does not match required_tools".to_string()
                        );
                    }
                    self.install_capability_activation_descriptors(&turn_id, activation)?;
                }
                if let Some(invocation) = skill_invocation {
                    self.start_materialized_skill(
                        &turn_id,
                        &call_id,
                        invocation,
                        &mut events,
                        &mut effects,
                    )?;
                } else if let Some(update) = skill_runtime_update {
                    self.record_active_skill_elapsed(
                        &turn_id,
                        effect_elapsed_millis,
                        call_id.as_str(),
                        &mut events,
                        &mut effects,
                    )?;
                    if let Err(error) = self.apply_materialized_skill_runtime_update(
                        &turn_id,
                        &call_id,
                        update,
                        &mut events,
                        &mut effects,
                    ) {
                        let pending_skill_updates = self.session.pending_skill_completion_steps();
                        // A syntactically valid Skill update can still violate the active
                        // completion contract (for example, by using a filename instead of
                        // the declared artifact id). That is a recoverable model protocol
                        // error, not a corrupt Host response or a fatal runtime failure.
                        // Return it through the ordinary failed-tool path so the model can
                        // inspect the active contract and retry with corrected arguments.
                        output = serde_json::json!({
                            "error": {
                                "code": "invalid_skill_runtime_update",
                                "message": error,
                            },
                            "tool_execution_skipped": true,
                            "recovery": "correct_arguments",
                            "next_action": "Retry only the exact pending skill_update calls supplied by Core. Do not search for tool documentation or repeat completed execution and validation.",
                            "pending_skill_updates": pending_skill_updates,
                        });
                        outcome = nola_host_protocol::HostToolOutcome::Failed;
                        artifacts.clear();
                    }
                } else {
                    self.record_active_skill_elapsed(
                        &turn_id,
                        effect_elapsed_millis,
                        call_id.as_str(),
                        &mut events,
                        &mut effects,
                    )?;
                    self.record_active_skill_tool_calls(
                        &turn_id,
                        &call_id,
                        &capability_id,
                        &mut events,
                        &mut effects,
                    )?;
                    if !outcome.is_error() {
                        self.register_unambiguous_skill_artifact_from_tool(
                            &turn_id,
                            &call_id,
                            &capability_id,
                            &artifacts,
                            &mut events,
                            &mut effects,
                        )?;
                    }
                }
                self.complete_tool(
                    &turn_id,
                    &call_id,
                    evidence,
                    output,
                    outcome,
                    activation,
                    full_output_ref,
                    artifacts,
                    materialized_skill_capabilities,
                    false,
                    &mut events,
                    &mut effects,
                )?;
            }
            AgentCommand::TaskPromoted {
                effect_id,
                call_id,
                task,
            } => {
                let task = self.session.task_projection.promote(task)?;
                let task_id = task.id.clone();
                if self.session.execution.active_turn.as_ref().is_some_and(|turn| {
                    matches!(turn.context.input_origin,
                        crate::agent::protocol::AgentInputOrigin::GoalInteractiveUser
                        | crate::agent::protocol::AgentInputOrigin::GoalContinuation)
                }) {
                    if let Some(goal) = self.session.goal_projection.active.as_mut().filter(|goal| !goal.status.is_terminal()) {
                        goal.work.owned_task_ids.insert(task_id.clone());
                        self.session.goal_projection.revision = self.session.goal_projection.revision.saturating_add(1);
                        events.push(RuntimeEvent::GoalUpdated {
                            goal_projection_revision: self.session.goal_projection.revision,
                            goal: self.session.goal_projection.clone(),
                        });
                    }
                }
                events.push(RuntimeEvent::TaskUpdated {
                    task_projection_revision: self.session.task_projection.revision,
                    task: task.clone(),
                });
                self.complete_tool(
                    &task.owner_turn_id,
                    &call_id,
                    promotion_evidence,
                    json!({
                        "type": "task_handle",
                        "task_id": task_id,
                        "task": task,
                        "effect_id": effect_id,
                    }),
                    HostToolOutcome::Deferred,
                    None,
                    None,
                    Vec::new(),
                    None,
                    false,
                    &mut events,
                    &mut effects,
                )?;
            }
            AgentCommand::TaskUpdated { task } => {
                let task = self.session.task_projection.replace(task)?;
                events.push(RuntimeEvent::TaskUpdated {
                    task_projection_revision: self.session.task_projection.revision,
                    task,
                });
            }
            AgentCommand::UnresolvedTaskGuidance => return Err("guidance FIFO marker requires Application admission".into()),
            AgentCommand::ReconcileTaskGuidance {fact} => {
                let task=self.session.task_projection.reconcile_guidance(&fact)?;
                events.push(RuntimeEvent::TaskUpdated {task_projection_revision:self.session.task_projection.revision,task});
            }
            AgentCommand::ObserveTaskGuidanceDelivery {expected_run_id} => {
                // Read/flush only: never enqueue input, restart the model, or
                // rely on the bounded operation ledger to suppress delivery.
                if self.session.execution.active_turn.as_ref().map(|turn|turn.turn_id.as_str())!=Some(expected_run_id.as_str()) {
                    return Err("guidance replay no longer belongs to the original Run".into());
                }
            }
            AgentCommand::ApplyTaskGuidanceInput {message_id,expected_run_id,content} => {
                let turn=self.session.execution.active_turn.as_ref().ok_or("guidance requires the original live Run")?;
                if turn.turn_id.as_str()!=expected_run_id || !matches!(turn.status,
                    super::super::AgentTurnStatus::AwaitingModel | super::super::AgentTurnStatus::AwaitingTools
                    | super::super::AgentTurnStatus::AwaitingPermission | super::super::AgentTurnStatus::Compacting)
                    || message_id.trim().is_empty() || content.trim().is_empty() {
                    return Err("guidance cannot create or replace a Run".into());
                }
                let turn_id=turn.turn_id.clone();
                let items=vec![super::super::AgentItem::user_message(super::super::ItemId::from_string(format!("task-guidance:{message_id}")),content)];
                let admission_id = self.session.allocate_id("input-admission");
                let batch = self.session.execution.input_queue.admit(admission_id, Some(message_id.clone()),
                    super::super::AgentInputOrigin::DelegatedAgent, super::super::AgentInputDelivery::CurrentTurn,
                    Some(turn_id.as_str().into()), crate::agent::input_queue::input_digest(&serde_json::json!({"source":"task_guidance","message_id":message_id,"items":items})), items.clone())?;
                events.push(RuntimeEvent::InputAccepted { admission: batch.admission.clone() });
                let item_count=self.session.execution.input_queue.enqueue_batch(batch)?;
                events.push(RuntimeEvent::InputQueued {turn_id,delivery:super::super::AgentInputDelivery::CurrentTurn,item_count,items});
            }
            AgentCommand::ReconcileTaskTerminal { fact } => {
                let task=self.session.task_projection.reconcile_terminal(&fact,&self.session.session_id)?;
                self.session.host.pending_effects.complete_task(&task.id);
                events.push(RuntimeEvent::TaskUpdated {
                    task_projection_revision:self.session.task_projection.revision,task,
                });
            }
            AgentCommand::UpdateTask {
                task_id,
                state,
                progress,
                current_step,
                result_ref,
                cancel_requested,
                ownership_token,
                completion_watermark,
            } => {
                let current = self
                    .session
                    .task_projection
                    .tasks
                    .get(&task_id)
                    .ok_or_else(|| "Task update references an unknown Task".to_string())?;
                if current.durability.ownership_token.is_none() {
                    return Err("Task worker must claim the Task before updating it".to_string());
                }
                let task = self.session.task_projection.next_handle_attested(
                    &task_id,
                    state,
                    progress,
                    current_step,
                    result_ref,
                    cancel_requested,
                    Some(&ownership_token),
                    completion_watermark,
                )?;
                let task = self.session.task_projection.replace(task)?;
                if task.state.is_terminal() {
                    self.session.host.pending_effects.complete_task(&task.id);
                }
                events.push(RuntimeEvent::TaskUpdated {
                    task_projection_revision: self.session.task_projection.revision,
                    task,
                });
            }
            AgentCommand::RequestTaskCancellation { task_id } => {
                let tasks = self
                    .session
                    .task_projection
                    .request_cancellation_tree(&task_id)?;
                for (task_projection_revision, task) in tasks {
                    events.push(RuntimeEvent::TaskUpdated {
                        task_projection_revision,
                        task: task.clone(),
                    });
                    effects.push(AgentEffect::CancelTask {
                        turn_id: task.owner_turn_id,
                        task_id: task.id,
                    });
                }
            }
            AgentCommand::CreateMemory { record } => {
                let update = self
                    .session
                    .memory_projection
                    .apply(crate::memory::MemoryUpdate::Created { record })?;
                events.push(RuntimeEvent::MemoryUpdated {
                    memory_projection_revision: self.session.memory_projection.revision,
                    update,
                });
            }
            AgentCommand::UpdateMemory { record } => {
                let update = self
                    .session
                    .memory_projection
                    .apply(crate::memory::MemoryUpdate::Updated { record })?;
                events.push(RuntimeEvent::MemoryUpdated {
                    memory_projection_revision: self.session.memory_projection.revision,
                    update,
                });
            }
            AgentCommand::DeleteMemory { memory_id } => {
                let update = self
                    .session
                    .memory_projection
                    .apply(crate::memory::MemoryUpdate::Deleted { memory_id })?;
                events.push(RuntimeEvent::MemoryUpdated {
                    memory_projection_revision: self.session.memory_projection.revision,
                    update,
                });
            }
            AgentCommand::MemoryWorkerCompleted { .. } => {
                unreachable!("memory worker completion is normalized before reduction")
            }
            AgentCommand::MemoryRuntime { envelope } => {
                if let crate::agent::memory_runtime::MemoryRuntimeCommand::RecordUsage { entry } =
                    &envelope.command
                {
                    let session_id = self.session.session_id.as_str();
                    if entry.session_id != session_id {
                        return Err(format!(
                            "memory usage session {} does not match active session {session_id}",
                            entry.session_id
                        ));
                    }
                }
                // Completion and record projection are one Core transaction.
                // Hosts may execute provider and persistence I/O, but must not
                // own a second memory state machine or leave a completed job
                // whose validated records were never made recallable.
                let completed_records = match &envelope.command {
                    crate::agent::memory_runtime::MemoryRuntimeCommand::Complete {
                        output, ..
                    } => match output {
                        crate::agent::memory_runtime::MemoryJobOutput::Extraction(output) => output
                            .memories
                            .iter()
                            .map(|memory| memory.record.clone())
                            .collect::<Vec<_>>(),
                        crate::agent::memory_runtime::MemoryJobOutput::Consolidation(output) => {
                            output.records.clone()
                        }
                    },
                    _ => Vec::new(),
                };

                let mut next_runtime = self.session.memory_runtime.clone();
                let apply_result = next_runtime.apply(envelope)?;
                let mut next_projection = self.session.memory_projection.clone();
                let mut projected_updates = Vec::new();
                if !apply_result.duplicate {
                    for record in completed_records {
                        let update = match next_projection.get(&record.id) {
                            Some(existing) if existing == &record => continue,
                            Some(_) => crate::memory::MemoryUpdate::Updated { record },
                            None => crate::memory::MemoryUpdate::Created { record },
                        };
                        let update = next_projection.apply(update)?;
                        projected_updates.push((next_projection.revision, update));
                    }
                }

                self.session.memory_runtime = next_runtime;
                self.session.memory_projection = next_projection;
                for (memory_projection_revision, update) in projected_updates {
                    events.push(RuntimeEvent::MemoryUpdated {
                        memory_projection_revision,
                        update,
                    });
                }
            }
            AgentCommand::RequestCompaction { operation_id } => {
                // Public manual maintenance is not an interrupt or a queued
                // control intent. In particular it must not replace a model,
                // permission wait, or cancellation drain with Compacting.
                // Automatic compaction enters request_compaction directly at
                // the model boundary, after the previous effect has settled.
                if self.session.execution.active_turn.as_ref()
                    .is_some_and(|turn| !turn.status.is_terminal())
                    || !self.session.host.pending_effects.is_quiescent()
                {
                    return Err("manual compaction busy: active execution must settle before retrying; no compaction was started or queued".into());
                }
                let maintenance_turn_id = if self
                    .session
                    .execution
                    .active_turn
                    .as_ref()
                    .is_none_or(|turn| turn.status.is_terminal())
                {
                    self.start_turn(
                        crate::agent::protocol::AgentInputOrigin::Maintenance,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        crate::context_engine::ContextPlatform::Unknown,
                        Vec::new(),
                        Some(operation_id),
                        None,
                        None,
                        &mut events,
                        &mut effects,
                    )?;
                    self.session
                        .execution
                        .active_turn
                        .as_ref()
                        .map(|turn| turn.turn_id.clone())
                } else {
                    None
                };
                self.request_compaction(CompactionReason::Manual, &mut events, &mut effects)?;
                if let Some(turn_id) = maintenance_turn_id {
                    if !self
                        .session
                        .conversation
                        .context_window
                        .compaction_in_progress
                    {
                        self.complete_turn(&turn_id, &mut events, &mut effects)?;
                    }
                }
            }
            AgentCommand::CompactionCompleted { summary, usage } => {
                if summary.trim().is_empty() {
                    return Err("compaction summary must not be empty".into());
                }
                if !self.session.conversation.context_window.compaction_in_progress {
                    return Err("no context compaction is active".into());
                }
                let plan = self
                    .session
                    .conversation
                    .context_window
                    .pending_compaction
                    .as_ref()
                    .ok_or_else(|| "context compaction has no Core-owned pending plan".to_string())?;
                let retained_items = {
                    let configuration = self.session.execution.active_turn.as_ref()
                        .map(|turn| &turn.context.configuration.context_window)
                        .unwrap_or(&self.session.configuration.context_window);
                    plan.validate_summary_for_prompt(&summary, configuration.prompt_token_limit(),
                        &self.session.conversation.model_context)?;
                    plan.retained_items(&self.session.conversation.model_context)?
                        .into_iter().cloned().collect::<Vec<_>>()
                };
                self.session.host.pending_effects.complete_matching(
                    None,
                    Some("context.compact"),
                    None,
                );
                let pending_compaction = self
                    .session
                    .conversation
                    .context_window
                    .take_pending_compaction()
                    .ok_or_else(|| {
                        "context compaction has no Core-owned pending plan".to_string()
                    })?;
                let replaced_item_count = pending_compaction.replaced_item_count;
                let compaction_reason = pending_compaction.reason;
                let source_item_ids = pending_compaction.source_item_ids;
                let source_fingerprint = pending_compaction.source_fingerprint;
                let original_token_count = pending_compaction.original_token_count;
                let source_item_count = source_item_ids.len();
                if self
                    .session
                    .conversation
                    .provider_continuation
                    .as_ref()
                    .is_some_and(|continuation| !continuation.survives_compaction())
                {
                    self.session.conversation.provider_continuation = None;
                }
                let compaction_item_id =
                    ItemId::from_string(self.session.allocate_id("compaction"));
                let compaction_item_key = compaction_item_id.as_str().to_string();
                let (compaction_turn_id, compaction_message_id) = self
                    .session
                    .execution
                    .active_turn
                    .as_ref()
                    .map(|turn| {
                        let turn_id = turn.turn_id.as_str().to_string();
                        let message_id = format!("{turn_id}:assistant");
                        (turn_id, message_id)
                    })
                    .unwrap_or_else(|| {
                        (
                            format!("{}:maintenance", self.session.session_id.as_str()),
                            compaction_item_key.clone(),
                        )
                    });
                self.session.item_ownership.insert(
                    compaction_item_key.clone(),
                    super::super::session::AgentItemOwnership {
                        source: Default::default(),
                        turn_id: compaction_turn_id,
                        message_id: compaction_message_id,
                    },
                );
                let compaction_item = AgentItem::Compaction {
                    id: compaction_item_id.clone(),
                    summary: summary.clone(),
                    replaced_item_count,
                    source_item_ids,
                    source_fingerprint: source_fingerprint.clone(),
                    original_token_count,
                };
                self.session
                    .conversation
                    .transcript
                    .push(compaction_item.clone());
                let (mut model_context, retained_tail): (Vec<_>, Vec<_>) = retained_items
                    .into_iter()
                    .partition(|item| matches!(item,
                        AgentItem::Compaction { .. } | AgentItem::Message {
                            role: AgentRole::System | AgentRole::Developer, ..
                        }));
                model_context.push(compaction_item.clone());
                model_context.extend(retained_tail);
                self.session.conversation.model_context = model_context;
                self.session
                    .conversation
                    .context_window
                    .finish_compaction(&self.session.conversation.model_context, &usage);
                events.push(RuntimeEvent::CompactionCompleted {
                    summary,
                    replaced_item_count,
                    remaining_item_count: self.session.conversation.model_context.len(),
                    reason: Some(compaction_reason),
                    compaction_item_id: Some(compaction_item_id),
                    source_fingerprint: Some(source_fingerprint),
                    source_item_count,
                    original_token_count,
                });
                if let Some(turn_id) = self
                    .session
                    .execution
                    .active_turn
                    .as_ref()
                    .filter(|turn| !turn.status.is_terminal())
                    .map(|turn| turn.turn_id.clone())
                {
                    events.push(RuntimeEvent::ItemCompleted {
                        turn_id: turn_id.clone(),
                        item: compaction_item,
                    });
                    let is_maintenance =
                        self.session
                            .execution
                            .active_turn
                            .as_ref()
                            .is_some_and(|turn| {
                                turn.context.input_origin
                                    == crate::agent::protocol::AgentInputOrigin::Maintenance
                            });
                    if is_maintenance {
                        self.complete_turn(&turn_id, &mut events, &mut effects)?;
                    } else {
                        self.request_model_after_compaction(&mut events, &mut effects)?;
                    }
                }
            }
            AgentCommand::CompactionFailed { error } => {
                self.session.host.pending_effects.complete_matching(
                    None,
                    Some("context.compact"),
                    None,
                );
                if !self
                    .session
                    .conversation
                    .context_window
                    .compaction_in_progress
                {
                    return Err("no context compaction is active".to_string());
                }
                let optional_maintenance = self.session.conversation.context_window.pending_compaction
                    .as_ref().is_some_and(|plan| plan.reason == CompactionReason::ThresholdReached)
                    && self.session.execution.active_turn.as_ref()
                        .is_some_and(|turn| turn.status == AgentTurnStatus::Compacting);
                self.session.conversation.context_window.fail_compaction();
                if optional_maintenance {
                    // A rejected summary never replaces the source context or
                    // replays completed tools. Preserve the failure diagnostic,
                    // then recompile and enforce the full request's hard limit.
                    // Its automatic count remains consumed, so a failed optional
                    // optimization cannot loop through the same model request.
                    events.push(RuntimeEvent::CompactionFailed { error });
                    self.request_model_after_compaction(&mut events, &mut effects)?;
                } else if let Some(turn) = self.session.execution.active_turn.as_mut() {
                    let turn_id = turn.turn_id.clone();
                    let session_id = self.session.session_id.clone();
                    turn.status = AgentTurnStatus::Failed;
                    turn.failure = Some(error.clone());
                    events.push(RuntimeEvent::TurnFailed {
                        session_id,
                        turn_id,
                        error,
                    });
                } else {
                    events.push(RuntimeEvent::CompactionFailed { error });
                }
            }
            AgentCommand::HostCancellationConfirmed { fact } => {
                let nola_domain::core_requirements::CoreRuntimeDecisionFactKind::HostCancellation { cancellation_id, .. } = &fact else {
                    return Err("invalid Host cancellation source".into());
                };
                let turn_id=self.session.execution.active_turn.as_ref().ok_or("no active cancellation Turn")?.turn_id.clone();
                self.session.execution.requirements.cancellation_id=Some(cancellation_id.clone());
                if self.session.execution.requirements.scope.is_some() {
                    self.record_result_fact(&turn_id,fact,&mut events);
                }
                self.cancel(Some("returned_to_composer".into()),&mut events,&mut effects)?;
            }
            AgentCommand::Cancel { goal_control, cancellation_id, reason } => {
                let mut accepted = true;
                let guarded_goal = goal_control.is_some();
                if let Some(envelope) = goal_control {
                    if !matches!(&envelope.command, crate::agent::GoalRuntimeCommand::GuardedControl { control, .. }
                        if matches!(control.as_ref(), crate::agent::GoalRuntimeCommand::Pause { status: crate::agent::GoalStatus::Paused, .. } | crate::agent::GoalRuntimeCommand::Cancel { .. })) {
                        return Err("Goal interruption requires a guarded pause or cancel".into());
                    }
                    let operation_id = envelope.operation_id.clone();
                    if cancellation_id != format!("goal-control:{operation_id}") {
                        return Err("Goal cancellation identity differs from its guarded operation".into());
                    }
                    self.session.goal_projection.apply(envelope)?;
                    accepted = self.session.goal_projection.applied_operations.get(&operation_id)
                        .is_some_and(|operation| operation.rejection.is_none());
                    events.push(RuntimeEvent::GoalUpdated { goal_projection_revision: self.session.goal_projection.revision, goal: self.session.goal_projection.clone() });
                }
                let live_turn = self.session.execution.active_turn.as_ref().filter(|turn| !turn.status.is_terminal());
                let owns_live_goal = live_turn.is_some_and(|turn| self.session.goal_projection.execution_owner(turn.turn_id.as_str()).is_some());
                // A Goal may be queued behind another ordinary Run. Its pause
                // commits, but cannot interrupt that unrelated execution.
                if accepted && (!guarded_goal || owns_live_goal) {
                    self.cancel_with_identity(cancellation_id, reason, &mut events, &mut effects)?;
                }
            }
            AgentCommand::ResolveUnknownEffects { decision } => {
                use nola_domain::unknown_effect_recovery::UnknownEffectRecoveryDisposition;
                let turn_id = self.session.execution.active_turn.as_ref()
                    .filter(|turn|!turn.status.is_terminal()).ok_or("unknown effect recovery requires an active Turn")?.turn_id.clone();
                if decision.targets.is_empty() || decision.resolution_id.trim().is_empty() || decision.reason.trim().is_empty() {
                    return Err("unknown effect recovery requires exact targets and explicit user decision".into());
                }
                for target in &decision.targets {
                    if !self.session.host.pending_effects.iter().any(|effect|effect.effect_id==target.effect_id
                        && effect.request_id==target.request_id && effect.turn_id.as_deref()==Some(turn_id.as_str()))
                        || self.session.execution.requirements.observations.values().any(|observation|observation.source.effect_id==target.effect_id) {
                        return Err("unknown effect recovery differs from pending unobserved Turn effect".into());
                    }
                }
                let preserve = decision.disposition==UnknownEffectRecoveryDisposition::KeepUnknownBlocked;
                if preserve {
                    let mut permission_ids = BTreeSet::new();
                    let mut permission_calls = Vec::new();
                    for target in &decision.targets {
                        let effect = self.session.host.pending_effects.iter()
                            .find(|effect| effect.effect_id == target.effect_id && effect.request_id == target.request_id)
                            .ok_or("unknown recovery target is no longer pending")?;
                        if let HostRequest::RequestPermission { permission_request_id, capability_id, resource, input, .. } =
                            self.validated_pending_request(effect)?.payload {
                            let pending = self.session.execution.permissions.pending.iter().find(|pending|
                                pending.request_id.as_str() == permission_request_id
                                    && pending.turn_id == turn_id
                                    && effect.call_id.as_deref() == Some(pending.call_id.as_str())
                                    && pending.capability_id == capability_id && pending.resource == resource
                                    && pending.arguments == input)
                                .ok_or("unknown permission recovery target differs from its pending question")?;
                            permission_ids.insert(permission_request_id);
                            permission_calls.push(pending.call_id.clone());
                        }
                    }
                    let target_ids: BTreeSet<_> = decision.targets.iter().map(|target|target.effect_id.as_str()).collect();
                    let mut covered_controls = BTreeSet::new();
                    for effect in self.session.host.pending_effects.iter() {
                        if let HostRequest::Cancel {target_effect_id,..} = self.validated_pending_request(effect)?.payload {
                            if target_ids.contains(target_effect_id.0.as_str()) { covered_controls.insert(effect.effect_id.as_str()); }
                        }
                    }
                    if self.session.execution.user_input_waits.values().any(|wait|wait.is_open())
                        || self.session.execution.input_queue.len()>0
                        || self.session.execution.permissions.pending.iter()
                            .any(|pending| !permission_ids.contains(pending.request_id.as_str()))
                        || self.session.host.pending_effects.iter()
                            .any(|effect|!target_ids.contains(effect.effect_id.as_str()) && !covered_controls.contains(effect.effect_id.as_str())) {
                        return Err("other independent work or user waits must settle before preserving unknown effects".into());
                    }
                    // Explicitly stopping the exact unknown permission delivery
                    // withdraws only its own question. Interrupted is neither a
                    // grant nor a denial, and the physical outcome stays unknown.
                    self.session.execution.permissions.interrupt_calls(&turn_id,
                        &permission_calls, "unknown_effect_preserved_blocked");
                }
                self.record_result_fact(&turn_id,nola_domain::core_requirements::CoreRuntimeDecisionFactKind::UnknownEffectRecovery { decision },&mut events);
                if preserve { self.complete_turn(&turn_id,&mut events,&mut effects)?; }
            }
            AgentCommand::CloseSession => {
                if self.session.journal.session_closing {
                    return Err("the session is already closing".to_string());
                }
                if self
                    .session
                    .execution
                    .active_turn
                    .as_ref()
                    .is_some_and(|turn| !turn.status.is_terminal())
                    || self.session.execution.input_queue.len() > 0
                    || self.session.host.pending_effects.len() > 0
                {
                    return Err("the session can only close while idle".to_string());
                }
                self.session.journal.session_closing = true;
                self.start_turn(
                    crate::agent::protocol::AgentInputOrigin::Maintenance,
                    Vec::new(), Vec::new(), Vec::new(),
                    crate::context_engine::ContextPlatform::Unknown,
                    Vec::new(), None, None, None, &mut events, &mut effects,
                )?;
                effects.push(AgentEffect::RunLifecycleHook {
                    event: crate::lifecycle_hooks::LifecycleHookEvent {
                        id: self.session.allocate_id("lifecycle_event"),
                        name: crate::lifecycle_hooks::LifecycleHookEventName::SessionEnd,
                        session_id: self.session.session_id.as_str().to_string(),
                        turn_id: self.session.execution.active_turn.as_ref()
                            .map(|turn| turn.turn_id.as_str().to_owned()),
                        tool_name: None,
                        tool_call_id: None,
                    },
                });
            }
            AgentCommand::SessionClosed => {
                if !self.session.journal.session_closing {
                    return Err("the session is not closing".to_string());
                }
                self.session.journal.session_closing = false;
                self.session.journal.session_closed = true;
                let turn_id = self.session.execution.active_turn.as_ref()
                    .ok_or("session close has no owning maintenance Turn")?.turn_id.clone();
                self.complete_turn(&turn_id, &mut events, &mut effects)?;
            }
            AgentCommand::Reset => {
                self.withdraw_user_input_waits("session reset", &mut events);
                let user_input_waits = std::mem::take(&mut self.session.execution.user_input_waits);
                for admission in self.session.execution.input_queue.withdraw_pending("session reset") {
                    let turn_id = admission
                        .anchor_turn_id
                        .ok_or("queued input has no original Turn")?;
                    events.push(RuntimeEvent::InputWithdrawn {
                        turn_id: super::super::TurnId::from_string(turn_id),
                        admission_ids: vec![admission.admission_id],
                        reason: "session reset".into(),
                    });
                }
                let input_queue = std::mem::take(&mut self.session.execution.input_queue);
                let input_fingerprints = std::mem::take(&mut self.session.journal.accepted_input_fingerprints);
                let session_id = self.session.session_id.clone();
                let revision = self.session.journal.revision;
                let next_event_sequence = self.session.journal.next_event_sequence;
                let next_id_sequence = self.session.journal.next_id_sequence;
                let next_turn_sequence = self.session.journal.next_turn_sequence;
                let configuration = self.session.configuration.clone();
                self.session = AgentThreadState::new(session_id, configuration);
                self.session.execution.input_queue = input_queue;
                self.session.execution.user_input_waits = user_input_waits;
                self.session.journal.accepted_input_fingerprints = input_fingerprints;
                self.session.journal.revision = revision;
                self.session.journal.next_event_sequence = next_event_sequence;
                self.session.journal.next_id_sequence = next_id_sequence;
                self.session.journal.next_turn_sequence = next_turn_sequence;
                self.session.journal.session_started_emitted = true;
                self.session.execution.permission_rules = self.config.permission_rules.clone();
                if let Some(host_catalog) = self.config.host_catalog.clone() {
                    self.session.host.catalog = host_catalog;
                }
            }
        }

        self.finish_transition_after_model(events, effects, checkpoint_reason, completed_model_turn.as_ref())
    }

    pub(super) fn observation_time(
        observed_at_ms: u64,
    ) -> Result<chrono::DateTime<chrono::Utc>, String> {
        i64::try_from(observed_at_ms)
            .ok()
            .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
            .ok_or_else(|| "Core observation timestamp is outside the supported range".to_string())
    }

    pub(super) fn finish_transition(
        &mut self,
        events: Vec<RuntimeEvent>,
        effects: Vec<AgentEffect>,
        checkpoint_reason: Option<CheckpointReason>,
    ) -> Result<AgentReduction, String> {
        self.finish_transition_after_model(events, effects, checkpoint_reason, None)
    }

    fn finish_transition_after_model(
        &mut self,
        mut events: Vec<RuntimeEvent>,
        mut effects: Vec<AgentEffect>,
        checkpoint_reason: Option<CheckpointReason>,
        completed_model_turn: Option<&TurnId>,
    ) -> Result<AgentReduction, String> {
        // Validate before projection or journal mutation. The trace clock is
        // part of the deterministic transition, not a wall-clock fallback.
        let emitted_at = Self::observation_time(self.observed_at_ms)?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        self.capture_authorized_question_effects(&mut events, &mut effects)?;
        // Observations can finish a drained cell and exhaust the final model
        // budget. Resolve them before terminal arbitration so that this same
        // transition records the Run result and closes outstanding input.
        self.reconcile_interrupted_code_calls()?;
        self.resolve_immediate_code_observations(&mut events, &mut effects)?;
        if let Some(turn_id) = self.session.execution.active_turn.as_ref()
            .filter(|turn| turn.status.is_terminal()).map(|turn| turn.turn_id.clone()) {
            self.withdraw_user_input_waits("owning Turn terminated", &mut events);
            let admission_ids = self.session.execution.input_queue.withdraw_current(
                turn_id.as_str(), "destination Turn ended before input application");
            if !admission_ids.is_empty() {
                events.push(RuntimeEvent::InputWithdrawn { turn_id, admission_ids,
                    reason: "destination Turn ended before input application".into() });
            }
        }
        self.close_stream_items_at_terminal_boundary(&mut events);
        if let Some(turn_id) = self.session.execution.active_turn.as_ref().filter(|turn| turn.status.is_terminal()).map(|turn|turn.turn_id.clone()) {
            self.decide_requirement_result(&turn_id, &mut events)?;
        }
        // Answers are reduced first. Only a committed answer for this exact
        // execution can wake its Goal; clients never issue a second Resume.
        if let Some(turn) = self.session.execution.active_turn.as_ref() {
            let answered = events.iter().any(|event| match event {
                RuntimeEvent::UserInputWaitChanged { wait } => wait.turn_id == turn.turn_id.as_str()
                    && matches!(wait.state, nola_domain::user_input_wait::UserInputWaitState::Resolved { .. }),
                RuntimeEvent::PermissionResolved { turn_id, .. } => turn_id == &turn.turn_id,
                _ => false,
            });
            let ready = !matches!(turn.status, AgentTurnStatus::AwaitingInput | AgentTurnStatus::AwaitingPermission
                | AgentTurnStatus::Cancelling | AgentTurnStatus::Cancelled)
                && !self.session.execution.user_input_waits.values().any(|wait|
                    wait.turn_id == turn.turn_id.as_str() && wait.is_open());
            if answered && ready && self.session.goal_projection.resume_answered_execution(turn.turn_id.as_str()) {
                events.push(RuntimeEvent::GoalUpdated {
                    goal_projection_revision: self.session.goal_projection.revision,
                    goal: self.session.goal_projection.clone(),
                });
            }
        }
        // Only the canonical terminal boundary (after every stop receipt)
        // releases an interrupted Goal contract for a future round. This does
        // not audit or satisfy any requirement.
        let goal_cancelled = self.session.execution.active_turn.as_ref().is_some_and(|turn|
            matches!(turn.context.input_origin, crate::agent::AgentInputOrigin::GoalInteractiveUser | crate::agent::AgentInputOrigin::GoalContinuation)
            && events.iter().any(|event| matches!(event, RuntimeEvent::TurnCancelled { turn_id, .. } if turn_id == &turn.turn_id)));
        if goal_cancelled {
            if let Some(goal) = self.session.goal_projection.active.as_mut() {
                if let Some(contract) = goal.contracts.last_mut().filter(|contract| contract.status == crate::agent::GoalContractStatus::Executing) {
                    contract.status = crate::agent::GoalContractStatus::Interrupted;
                    self.session.goal_projection.revision = self.session.goal_projection.revision.saturating_add(1);
                    events.push(RuntimeEvent::GoalUpdated { goal_projection_revision: self.session.goal_projection.revision, goal: self.session.goal_projection.clone() });
                }
            }
        }
        self.finalize_skills_from_turn_events(&mut events, &mut effects)?;
        // Reconcile from canonical states, not only Host completions: schema
        // rejection and permission denial also settle a JS tool promise.
        if let Some(turn) = self.session.execution.active_turn.as_mut() {
            effects.extend(turn.take_code_tool_results()?.into_iter()
                .map(|result| AgentEffect::ResolveCodeTool { result }));
        }
        self.schedule_code_observation_polls(&mut effects);
        self.emit_context_update(&mut events);
        self.project_events(&mut events)?;
        if let Some(turn_id) = completed_model_turn {
            self.complete_model_reasoning_parts(turn_id, &mut events)?;
        }
        let host_requests = self.wrap_host_requests(&effects)?;
        for request in &host_requests {
            if let Some(turn_id) = request.turn_id.as_ref() {
                let assistant_message_id = self.session.conversation.projection.active_message_segments
                    .get(turn_id).cloned().unwrap_or_else(|| crate::agent::parts::PartOwnership::for_turn(turn_id).message_id);
                events.push(RuntimeEvent::AssistantEffectBound {
                    turn_id: TurnId::from_string(turn_id.clone()), assistant_message_id,
                    effect_id: request.effect_id.0.clone(),
                });
            }
        }
        let events = events
            .into_iter()
            .filter_map(RuntimeEvent::into_public)
            .collect::<Vec<_>>();
        self.session.journal.revision = self.session.journal.revision.saturating_add(1);
        let event_envelopes = self.envelope_events(events);
        // Trace buffers are persisted in the checkpoint. A replay must not
        // replace their timestamps with the wall clock at recovery time.
        let trace_events = event_envelopes
            .iter()
            .flat_map(|envelope| {
                self.session
                    .journal
                    .trace_context
                    .project_agent_events_at(&envelope.event, envelope.cursor.sequence, emitted_at.clone())
            })
            .collect();
        Ok(AgentReduction {
            event_envelopes,
            trace_events,
            host_requests,
            durability: checkpoint_reason
                .map(|checkpoint_reason| AgentDurabilityIntent { checkpoint_reason }),
        })
    }

    /// Enforces a single terminal boundary for every streamed Part. Individual
    /// failure paths only decide the Turn outcome; this reducer boundary owns
    /// stream cleanup so a newly added failure path cannot strand a loading UI.
    fn close_stream_items_at_terminal_boundary(&mut self, events: &mut Vec<RuntimeEvent>) {
        let terminal = events
            .iter()
            .enumerate()
            .find_map(|(index, event)| match event {
                RuntimeEvent::TurnCompleted { turn_id, .. }
                | RuntimeEvent::TurnFailed { turn_id, .. }
                | RuntimeEvent::TurnCancelled { turn_id, .. } => Some((index, turn_id.clone())),
                _ => None,
            });
        let Some((terminal_index, turn_id)) = terminal else {
            return;
        };
        let Some(turn) = self
            .session
            .execution
            .active_turn
            .as_ref()
            .filter(|turn| turn.turn_id == turn_id)
        else {
            return;
        };
        let mut open_items = turn.open_stream_items.clone();
        if let Some(id) = &turn.final_answer_draft_part_id {
            if !open_items.contains(id) {
                open_items.push(id.clone());
            }
        }
        let committed_items = turn
            .committed_stream_items
            .iter()
            .map(|item_id| item_id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        let settled_items = events
            .iter()
            .filter_map(|event| match event {
                RuntimeEvent::ItemCompleted { item, .. } => Some(item.id().as_str().to_string()),
                RuntimeEvent::ItemDiscarded { item_id, .. } => Some(item_id.as_str().to_string()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();

        let mut settlements = Vec::new();
        for item_id in open_items {
            if settled_items.contains(item_id.as_str()) {
                continue;
            }
            let projected_item = self
                .session
                .conversation
                .projection
                .parts
                .get(item_id.as_str())
                .and_then(|part| match part {
                    AgentPart::Text(part) if !part.text.is_empty() && part.review.is_none() => Some(AgentItem::Message {
                        id: item_id.clone(),
                        role: AgentRole::Assistant,
                        content: part.text.clone(),
                    }),
                    AgentPart::Reasoning(part)
                        if !part.title.is_empty() || !part.detail.is_empty() =>
                    {
                        Some(AgentItem::Reasoning {
                            id: item_id.clone(),
                            summary: part.title.clone(),
                            content: part.detail.clone(),
                            provider_continuation: None,
                        })
                    }
                    _ => None,
                });
            // A non-empty projected Part has already crossed the UI boundary,
            // even on legacy HostResponse::ModelDelta. Explicit committed IDs
            // additionally protect future non-text Part kinds.
            if committed_items.contains(item_id.as_str()) || projected_item.is_some() {
                let item = projected_item;
                if let Some(item) = item {
                    self.session.record_item(item.clone());
                    settlements.push(RuntimeEvent::ItemCompleted {
                        turn_id: turn_id.clone(),
                        item,
                    });
                    continue;
                }
            }
            settlements.push(RuntimeEvent::ItemDiscarded {
                turn_id: turn_id.clone(),
                item_id,
                disposition: ItemDiscardDisposition::Hidden,
            });
        }
        if let Some(turn) = self.session.execution.active_turn.as_mut() {
            turn.clear_stream_items();
        }
        events.splice(terminal_index..terminal_index, settlements);
    }
}

impl AgentRuntime {
    fn capture_review_candidate(
        &mut self,
        turn_id: &TurnId,
        stream_ids: &[ItemId],
        text: &str,
        events: &mut Vec<RuntimeEvent>,
    ) -> Result<ItemId, String> {
        use crate::agent::parts::TextReviewState;
        let existing = stream_ids.iter().find_map(|id| match self.session.conversation.projection.parts.get(id.as_str()) {
            Some(AgentPart::Text(part)) if part.review == Some(TextReviewState::Pending) => Some((id.clone(), part.text.clone())),
            _ => None,
        });
        let (id, prefix) = match existing {
            Some((id, prefix)) if text.starts_with(&prefix) => (id, prefix),
            previous => {
                if let Some((id, _)) = previous {
                    events.push(RuntimeEvent::TextReviewChanged { turn_id: turn_id.clone(), item_id: id,
                        state: TextReviewState::Rejected });
                }
                let id = ItemId::from_string(self.session.allocate_id("answer-candidate"));
                events.push(RuntimeEvent::ItemStarted { turn_id: turn_id.clone(), item_id: id.clone(), kind: AgentItemDeltaKind::AssistantText });
                (id, String::new())
            }
        };
        let suffix = &text[prefix.len()..];
        if !suffix.is_empty() {
            events.push(RuntimeEvent::ItemDelta { turn_id: turn_id.clone(), item_id: id.clone(),
                kind: AgentItemDeltaKind::AssistantText, delta: suffix.to_string(), presentation_committed: true });
        }
        events.push(RuntimeEvent::TextReviewChanged { turn_id: turn_id.clone(), item_id: id.clone(), state: TextReviewState::Pending });
        Ok(id)
    }
}

fn normalize_final_answer_audit(content: &str) -> Option<String> {
    let trimmed = content.trim();
    let unfenced = if trimmed.starts_with("```json") || trimmed.starts_with("```JSON") {
        trimmed
            .split_once('\n')
            .map(|(_, body)| body)
            .unwrap_or("")
            .strip_suffix("```")
            .unwrap_or_else(|| trimmed.split_once('\n').map(|(_, body)| body).unwrap_or(""))
            .trim()
    } else {
        trimmed
    };
    let json_text = match (unfenced.find('{'), unfenced.rfind('}')) {
        (Some(start), Some(end)) if start <= end => &unfenced[start..=end],
        _ => return None,
    };
    let mut report: Value = serde_json::from_str(json_text).ok()?;
    // Constraints are an explanatory index, not an authorization/completion
    // decision. A valid evidence-bearing verdict must not spend another model
    // round solely to supply this redundant empty array. Wrong types still fail.
    report.as_object_mut()?.entry("constraints").or_insert_with(|| json!([]));
    let decision = report.get("decision")?.as_str()?;
    if !matches!(decision, "accept" | "revise" | "execute" | "clarify" | "blocked")
        || !report.get("constraints")?.is_array()
        || !report.get("items")?.is_array()
        || !report.get("violations")?.is_array()
    {
        return None;
    }
    let expected = match decision {
        "accept" => "answered",
        "clarify" => "needs_input",
        "blocked" => "blocked",
        _ => "incomplete",
    };
    let outcome = report.get("outcome")?.as_str()?;
    if decision == "clarify" {
        let questions: nola_domain::user_input_wait::QuestionSet = serde_json::from_value(report.get("question_set")?.clone()).ok()?;
        questions.validate().ok()?;
    }
    if outcome != expected && !(decision == "accept" && outcome == "action_completed") {
        return None;
    }
    if matches!(decision, "accept" | "clarify" | "blocked")
        && !report["violations"].as_array()?.is_empty()
    {
        return None;
    }
    let outcome = outcome.to_string();
    if decision == "accept" && report["items"].as_array()?.is_empty() {
        // A protocol-shaped empty object is not a completed review. Even a
        // greeting must identify the requirement the candidate satisfies.
        return None;
    }
    for item in report.get("items")?.as_array()? {
        if item.get("label")?.as_str()?.trim().is_empty()
            || !matches!(
                item.get("eligibility")?.as_str()?,
                "eligible" | "rejected" | "unknown"
            )
            || item.get("evidence")?.as_str()?.trim().is_empty()
            || (decision == "accept" && item.get("eligibility")?.as_str()? != "eligible")
        {
            return None;
        }
    }
    report["outcome"] = Value::String(outcome);
    serde_json::to_string(&report).ok()
}

fn audit_receipts_valid(report: &Value, receipts: &std::collections::BTreeSet<String>) -> bool {
    if report["outcome"] == "answered" {
        // Completion accounting follows observed effects, not prompt keywords.
        // A successful mutation cannot disappear behind a pure-answer label.
        return receipts.is_empty();
    }
    if report["outcome"] != "action_completed" {
        return true;
    }
    report.get("effect_call_ids").and_then(Value::as_array).is_some_and(|ids| {
        !ids.is_empty() && ids.iter().all(|id| id.as_str().is_some_and(|id| receipts.contains(id)))
    })
}

fn contains_embedded_tool_call_protocol(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    (normalized.contains("<tool_calls") && normalized.contains("</tool_calls>"))
        || (normalized.contains("<tool_call") && normalized.contains("</tool_call>"))
        || (normalized.contains("<tool name=") && normalized.contains("</tool>"))
        || (normalized.contains("<dots_function_call")
            && normalized.contains("</dots_function_call>"))
}

#[cfg(test)]
mod skill_verification_source_tests {
    use super::*;
    use crate::agent::capability::*;
    use nola_domain::execution_evidence::*;
    use nola_domain::assistant_journal_operations::JournalNumber;

    fn fixture(method: SkillVerificationMethod) -> (AgentRuntime, SkillVerificationResult) {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            session_id: Some(SessionId::from_string("skill-evidence-session".into())),
            turn_identity: crate::agent::model_capability::test_host_identity(), ..Default::default()
        }).unwrap();
        runtime.session.skill_runtime.apply(SkillRuntimeCommandEnvelope { operation_id: "start-skill".into(),
            command: SkillRuntimeCommand::Start { run_id: "skill-run".into(),
                owner: SkillRunOwner { session_id: "skill-evidence-session".into(), parent_turn_id: "turn".into(), child_turn_id: None },
                descriptor: SkillDescriptor { skill_id: "review".into(), version: "1.0.0".into(), display_name: "Review".into(),
                    instructions_ref: format!("skill://review/1.0.0/SKILL.md#sha256={}", "a".repeat(64)), instructions_digest: "a".repeat(64),
                    required_tools: vec![], default_mode: Some(SkillExecutionMode::Inline),
                    execution_contract: SkillExecutionContract { verifiers: vec![SkillVerifierContract {
                        id: "validation".into(), description: "Check current output".into(), method, required: true }], ..Default::default() } },
                mode: SkillExecutionMode::Inline, budget: SkillRunBudget::default() } }).unwrap();
        let observation = ExecutionEvidenceObservation { evidence_id: "validation-evidence".into(), call_id: "validation-call".into(),
            tool_name: "validator".into(), source: ExecutionEvidenceSource { effect_id: "effect".into(), request_id: "request".into(),
                host_id: "host".into(), attempt_id: None, response_sequence: JournalNumber(1), catalog_revision: JournalNumber(1), availability_epoch: JournalNumber(0) },
            outcome: HostToolOutcome::Succeeded,
            receipt: Some(ExecutionReceipt { local_stop: None, state: ExecutionState::Completed,
                effects: vec![ResourceEffect { resource_uri: "file:///workspace/report".into(), operation: "validation".into(),
                    before_version: Some("sha256:v1".into()), after_version: Some("sha256:v1".into()) }],
                checks: vec![PostconditionCheck { kind: ExecutionCheckKind::Postcondition,
                    resource_uri: "file:///workspace/report".into(), version: "sha256:v1".into(), predicate: "rendered_pages_checked".into(), passed: true }] }) };
        runtime.session.execution.requirements.observations.insert(observation.evidence_id.clone(), (&observation, "turn", 10).into());
        let result = SkillVerificationResult { verifier_id: "validation".into(), passed: true,
            replaces_evidence: None,
            evidence: SkillVerificationEvidence::HostCheck { source: SkillEvidenceReference::ToolCall { call_id: "validation-call".into() },
                resource_uri: "file:///workspace/report".into(), version: "sha256:v1".into(), check_kind: ExecutionCheckKind::Postcondition,
                predicate: "rendered_pages_checked".into() }, summary: "Actual current resource check".into() };
        (runtime, result)
    }

    #[test]
    fn host_check_requires_an_exact_owner_receipt_predicate_version_and_verdict() {
        let (mut runtime, result) = fixture(SkillVerificationMethod::HostCheck);
        assert!(runtime.validate_skill_verification_sources("skill-run", &result).is_ok());
        let mut invented = result.clone();
        if let SkillVerificationEvidence::HostCheck { source, .. } = &mut invented.evidence {
            *source = SkillEvidenceReference::ExecutionEvidence { evidence_id: "self-described-search-evidence".into() };
        }
        assert!(runtime.validate_skill_verification_sources("skill-run", &invented).is_err());
        let mut unrelated_check = result.clone();
        if let SkillVerificationEvidence::HostCheck { predicate, .. } = &mut unrelated_check.evidence { *predicate = "all_requirements_passed".into(); }
        assert!(runtime.validate_skill_verification_sources("skill-run", &unrelated_check).is_err());
        let mut false_verdict = result.clone(); false_verdict.passed = false;
        assert!(runtime.validate_skill_verification_sources("skill-run", &false_verdict).is_err());
        runtime.session.execution.requirements.observations.get_mut("validation-evidence").unwrap().turn_id = "another-owner".into();
        assert!(runtime.validate_skill_verification_sources("skill-run", &result).is_err());
        let observation = runtime.session.execution.requirements.observations.get_mut("validation-evidence").unwrap();
        observation.turn_id = "turn".into(); observation.receipt = None;
        assert!(runtime.validate_skill_verification_sources("skill-run", &result).is_err(), "Success without a check receipt cannot become verification");
    }

    #[test]
    fn newer_resource_version_blocks_completion_without_rewriting_prior_verification() {
        let (mut runtime, result) = fixture(SkillVerificationMethod::HostCheck);
        runtime.apply_skill_runtime_command(SkillRuntimeCommandEnvelope { operation_id: "verify".into(),
            command: SkillRuntimeCommand::RecordVerification { run_id: "skill-run".into(), result: result.clone() } }, &mut vec![], &mut vec![]).unwrap();
        let mut newer = runtime.session.execution.requirements.observations["validation-evidence"].clone();
        newer.evidence_id = "changed-resource".into(); newer.call_id = "write-later".into(); newer.recorded_core_revision = 11;
        let receipt = newer.receipt.as_mut().unwrap();
        receipt.checks.clear(); receipt.effects[0].after_version = Some("sha256:v2".into());
        runtime.session.execution.requirements.observations.insert(newer.evidence_id.clone(), newer);
        assert!(runtime.validate_skill_verification_sources("skill-run", &result).is_err());
        assert!(runtime.apply_skill_runtime_command(SkillRuntimeCommandEnvelope { operation_id: "complete".into(),
            command: SkillRuntimeCommand::Complete { run_id: "skill-run".into(), result_ref: None } }, &mut vec![], &mut vec![]).is_err());
        assert_eq!(runtime.session.skill_runtime.runs["skill-run"].verifications["validation"], result);
        assert_eq!(runtime.session.skill_runtime.runs["skill-run"].status, SkillRunStatus::Running);
        let mut current = runtime.session.execution.requirements.observations["validation-evidence"].clone();
        current.evidence_id = "current-check".into(); current.call_id = "current-check-call".into(); current.recorded_core_revision = 12;
        let receipt = current.receipt.as_mut().unwrap();
        receipt.effects[0].after_version = Some("sha256:v2".into());
        receipt.checks[0].version = "sha256:v2".into();
        runtime.session.execution.requirements.observations.insert(current.evidence_id.clone(), current);
        let mut replacement = result.clone();
        if let SkillVerificationEvidence::HostCheck { source, version, .. } = &mut replacement.evidence {
            *source = SkillEvidenceReference::ExecutionEvidence { evidence_id: "current-check".into() };
            *version = "sha256:v2".into();
        }
        assert!(runtime.apply_skill_runtime_command(SkillRuntimeCommandEnvelope { operation_id: "unrelated-success".into(),
            command: SkillRuntimeCommand::RecordVerification { run_id: "skill-run".into(), result: replacement.clone() } }, &mut vec![], &mut vec![]).is_err());
        replacement.replaces_evidence = Some(result.evidence.clone());
        let mut invented_replacement = replacement.clone();
        if let SkillVerificationEvidence::HostCheck { source, .. } = &mut invented_replacement.evidence {
            *source = SkillEvidenceReference::ToolCall { call_id: "unobserved-new-check".into() };
        }
        assert!(runtime.apply_skill_runtime_command(SkillRuntimeCommandEnvelope { operation_id: "replace-with-unverified-source".into(),
            command: SkillRuntimeCommand::RecordVerification { run_id: "skill-run".into(), result: invented_replacement } }, &mut vec![], &mut vec![]).is_err());
        assert_eq!(runtime.session.skill_runtime.runs["skill-run"].verifications["validation"], result,
            "Naming the prior evidence does not validate a replacement source");
        runtime.apply_skill_runtime_command(SkillRuntimeCommandEnvelope { operation_id: "replace-old-check".into(),
            command: SkillRuntimeCommand::RecordVerification { run_id: "skill-run".into(), result: replacement.clone() } }, &mut vec![], &mut vec![]).unwrap();
        runtime.apply_skill_runtime_command(SkillRuntimeCommandEnvelope { operation_id: "complete-current-check".into(),
            command: SkillRuntimeCommand::Complete { run_id: "skill-run".into(), result_ref: None } }, &mut vec![], &mut vec![]).unwrap();
        assert_eq!(runtime.session.skill_runtime.runs["skill-run"].verifications["validation"], replacement);
        assert_eq!(runtime.session.skill_runtime.runs["skill-run"].status, SkillRunStatus::Completed);
    }

    #[test]
    fn model_judgment_is_explicit_and_supplied_sources_still_have_to_resolve() {
        let (mut runtime, mut result) = fixture(SkillVerificationMethod::ModelJudgment);
        result.evidence = SkillVerificationEvidence::ModelJudgment { sources: vec![SkillEvidenceReference::ToolCall { call_id: "validation-call".into() }] };
        runtime.apply_skill_runtime_command(SkillRuntimeCommandEnvelope { operation_id: "judge".into(),
            command: SkillRuntimeCommand::RecordVerification { run_id: "skill-run".into(), result } }, &mut vec![], &mut vec![]).unwrap();
        let (_, mut invented) = fixture(SkillVerificationMethod::ModelJudgment);
        invented.evidence = SkillVerificationEvidence::ModelJudgment { sources: vec![SkillEvidenceReference::ToolCall { call_id: "invented-call".into() }] };
        assert!(runtime.validate_skill_verification_sources("skill-run", &invented).is_err());
        let mut wire = serde_json::to_value(&runtime.session.execution.requirements.observations["validation-evidence"]).unwrap();
        wire.as_object_mut().unwrap().remove("turn_id");
        assert!(serde_json::from_value::<super::super::requirements::ObservedEvidenceReference>(wire).is_err());
        assert!(serde_json::from_value::<SkillVerificationResult>(serde_json::json!({"verifierId":"validation","passed":true,"evidenceRef":"invented"})).is_err());
    }
}

#[cfg(test)]
mod observation_clock_tests {
    use super::*;

    #[test]
    fn completed_model_seals_unmatched_process_reasoning_before_next_phase() {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("reasoning-boundary".into())),
            ..Default::default()
        }).unwrap();
        let turn = TurnId::from_string("reasoning-turn".into());
        let item = ItemId::from_string("preamble".into());
        let events = vec![
            RuntimeEvent::ItemDelta { turn_id: turn.clone(), item_id: item.clone(),
                kind: AgentItemDeltaKind::AssistantText, delta: "Now write the implementation.".into(),
                presentation_committed: false },
            RuntimeEvent::ItemDiscarded { turn_id: turn.clone(), item_id: item,
                disposition: ItemDiscardDisposition::Process },
        ];
        runtime.finish_transition_after_model(events, vec![], None, Some(&turn)).unwrap();
        let AgentPart::Reasoning(part) = &runtime.session.conversation.projection.parts["preamble:process"] else {
            panic!("provider preamble must remain visible as reasoning");
        };
        assert!(part.is_complete, "Model completion, not whole-turn completion, seals the preamble");
        assert_eq!(part.detail, "Now write the implementation.");
        let revision = runtime.session.conversation.projection.revision;
        runtime.complete_model_reasoning_parts(&turn, &mut vec![]).unwrap();
        assert_eq!(runtime.session.conversation.projection.revision, revision, "Sealing is idempotent");
    }

    #[test]
    fn unsupported_observation_clock_cannot_mutate_a_borrowed_reducer() {
        let config = AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("clock-boundary".into())),
            ..Default::default()
        };
        let mut runtime = AgentRuntime::try_new(config.clone()).unwrap();
        runtime.reduce(AgentSubmission::Reset, 1_234).unwrap();
        let checkpoint = runtime.checkpoint(CheckpointReason::Manual);
        let before = serde_json::to_value(&checkpoint).unwrap();
        let maximum_time = u64::try_from(chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp_millis())
            .unwrap();
        for invalid_time in [maximum_time + 1, i64::MAX as u64, u64::MAX] {
            let error = runtime.reduce(AgentSubmission::Reset, invalid_time).unwrap_err();
            assert!(error.contains("observation timestamp is outside the supported range"), "{error}");
            assert_eq!(runtime.observed_at_ms, 1_234);
            assert_eq!(serde_json::to_value(runtime.checkpoint(CheckpointReason::Manual)).unwrap(), before);
        }
        let mut clean = AgentRuntime::restore(config, checkpoint).unwrap();
        let resumed = runtime.reduce(AgentSubmission::Reset, maximum_time).unwrap();
        let expected = clean.reduce(AgentSubmission::Reset, maximum_time).unwrap();
        assert_eq!(serde_json::to_value((&resumed.event_envelopes, &resumed.trace_events,
            &resumed.host_requests, &resumed.durability)).unwrap(),
            serde_json::to_value((&expected.event_envelopes, &expected.trace_events,
                &expected.host_requests, &expected.durability)).unwrap());
        assert_eq!(serde_json::to_value(runtime.checkpoint(CheckpointReason::Manual)).unwrap(),
            serde_json::to_value(clean.checkpoint(CheckpointReason::Manual)).unwrap());
        assert_eq!(runtime.observed_at_ms, maximum_time);
    }
}

#[cfg(test)]
mod embedded_tool_protocol_tests {
    use super::{
        contains_embedded_tool_call_protocol,
        loop_checkpoint_protocol_is_selected,
    };
    use crate::agent::protocol::ModelToolCall;
    use crate::agent::CallId;

    #[test]
    fn audit_omitted_advisory_constraints_do_not_require_another_model_round() {
        use super::normalize_final_answer_audit;
        let mut report = json!({
            "decision": "accept", "outcome": "answered", "effect_call_ids": [],
            "items": [{"label": "current question", "eligibility": "eligible", "evidence": "candidate acknowledges unknown timing"}],
            "violations": []
        });
        let normalized: serde_json::Value = serde_json::from_str(&normalize_final_answer_audit(&report.to_string()).unwrap()).unwrap();
        assert_eq!(normalized["constraints"], json!([]));
        report["constraints"] = json!("invalid type");
        assert!(normalize_final_answer_audit(&report.to_string()).is_none());
        report.as_object_mut().unwrap().remove("constraints");
        report.as_object_mut().unwrap().remove("violations");
        assert!(normalize_final_answer_audit(&report.to_string()).is_none());
    }
    use serde_json::json;

    #[test]
    fn task_outcomes_and_receipts_are_not_label_filters() {
        use super::{normalize_final_answer_audit, audit_receipts_valid};
        assert!(normalize_final_answer_audit(r#"{"decision":"accept","constraints":[],"items":[],"violations":[]}"#).is_none());
        let invalid = json!({"decision":"accept", "outcome":"incomplete", "constraints":[], "items":[], "violations":[]});
        assert!(normalize_final_answer_audit(&invalid.to_string()).is_none());
        let invalid = json!({"decision":"accept", "outcome":"answered", "constraints":[], "items":[], "violations":["not executed"]});
        assert!(normalize_final_answer_audit(&invalid.to_string()).is_none());
        let mut report = json!({"outcome":"action_completed", "effect_call_ids":["mutation-1"]});
        let receipts = ["mutation-1".to_string()].into_iter().collect();
        assert!(audit_receipts_valid(&report, &receipts));
        assert!(!audit_receipts_valid(&report, &Default::default()));
        report["effect_call_ids"] = json!(["status-1"]);
        assert!(!audit_receipts_valid(&report, &receipts));
        report["outcome"] = json!("answered");
        assert!(!audit_receipts_valid(&report, &receipts));
        assert!(audit_receipts_valid(&report, &Default::default()));
    }

    #[test]
    fn empty_or_unresolved_review_cannot_accept_a_candidate() {
        use super::normalize_final_answer_audit;
        let mut report = json!({"decision":"accept", "outcome":"answered", "constraints":[], "items":[], "violations":[]});
        assert!(normalize_final_answer_audit(&report.to_string()).is_none());
        for eligibility in ["rejected", "unknown"] {
            report["items"] = json!([{"label":"coverage", "eligibility":eligibility, "evidence":"Only inventory was inspected"}]);
            assert!(normalize_final_answer_audit(&report.to_string()).is_none());
        }
        report["items"] = json!([{"label":"coverage", "eligibility":"eligible", "evidence":" "}]);
        assert!(normalize_final_answer_audit(&report.to_string()).is_none());
        report["items"][0]["evidence"] = json!("Candidate explicitly limits its result to the observed file inventory");
        assert!(normalize_final_answer_audit(&report.to_string()).is_some());
    }

    #[test]
    fn detects_provider_serialized_invoke_envelopes_without_matching_normal_xml() {
        assert!(contains_embedded_tool_call_protocol(
            "<dots_function_call><invoke name=\"memory_search\"><parameter name=\"query\">x</parameter></invoke></dots_function_call>"
        ));
        assert!(contains_embedded_tool_call_protocol(
            "<dots_function_call> name<arg_key>write_file</arg_key><arg_value>{}</arg_value></dots_function_call>"
        ));
        assert!(!contains_embedded_tool_call_protocol(
            "<article><invoke>ordinary quoted XML</invoke></article>"
        ));
    }

    #[test]
    fn loop_checkpoint_accepts_only_explicit_control_calls() {
        let mut call = ModelToolCall {
            call_id: CallId::from_string("decision".into()),
            name: "request_user_input".into(), arguments: json!({}),
            permission_scope: None, permission_rationale: None,
        };
        assert!(!loop_checkpoint_protocol_is_selected(std::slice::from_ref(
            &call
        )));
        call.name = "requirements_update".into();
        assert!(loop_checkpoint_protocol_is_selected(std::slice::from_ref(&call)));
        assert!(!loop_checkpoint_protocol_is_selected(&[]));
        assert!(!loop_checkpoint_protocol_is_selected(&[ModelToolCall {
            call_id: CallId::from_string("retry".to_string()),
            name: "write_file".to_string(),
            arguments: json!({}),
            permission_scope: None,
            permission_rationale: None,
        }]));
    }
}

impl AgentRuntime {
    fn resolve_skill_execution_evidence(&self, source: &crate::agent::capability::SkillEvidenceReference)
        -> Result<&super::requirements::ObservedEvidenceReference, String> {
        use crate::agent::capability::SkillEvidenceReference;
        let observations = &self.session.execution.requirements.observations;
        match source {
            SkillEvidenceReference::ExecutionEvidence { evidence_id } => observations.get(evidence_id)
                .ok_or_else(|| "Skill verification cites unknown execution evidence".into()),
            SkillEvidenceReference::ToolCall { call_id } => {
                let mut matching = observations.values().filter(|observation| observation.call_id == *call_id);
                let observed = matching.next().ok_or("Skill verification cites an unobserved tool call")?;
                if matching.next().is_some() { return Err("Skill tool-call evidence identity is ambiguous".into()); }
                Ok(observed)
            }
        }
    }

    fn validate_skill_verification_sources(&self, run_id: &str, result: &SkillVerificationResult) -> Result<(), String> {
        use crate::agent::capability::SkillVerificationEvidence;
        result.evidence.validate()?;
        let run = self.session.skill_runtime.runs.get(run_id).ok_or("Unknown Skill verification owner")?;
        if run.owner.session_id != self.session.session_id.as_str() {
            return Err("Skill verification owner differs from the current Core session".into());
        }
        for source in result.evidence.sources() {
            let observed = self.resolve_skill_execution_evidence(source)?;
            if observed.turn_id != run.owner.parent_turn_id && run.owner.child_turn_id.as_deref() != Some(observed.turn_id.as_str()) {
                return Err("Skill verification evidence belongs to another execution owner".into());
            }
        }
        if let SkillVerificationEvidence::HostCheck { source, resource_uri, version, check_kind, predicate } = &result.evidence {
            let observed = self.resolve_skill_execution_evidence(source)?;
            let receipt = observed.receipt.as_ref().ok_or("Skill Host verification requires an actual Host check receipt")?;
            if result.passed && (observed.outcome != HostToolOutcome::Succeeded
                || receipt.state != nola_domain::execution_evidence::ExecutionState::Completed) {
                return Err("Skill Host verification cannot turn an unsuccessful or uncertain execution into a passed check".into());
            }
            if !receipt.checks.iter().any(|check| check.resource_uri == *resource_uri && check.version == *version
                && check.kind == *check_kind && check.predicate == *predicate && check.passed == result.passed) {
                return Err("Skill verification does not match the exact observed resource, version, method and verdict".into());
            }
            for newer in self.session.execution.requirements.observations.values().filter(|newer|
                newer.recorded_core_revision > observed.recorded_core_revision) {
                if newer.receipt.as_ref().is_some_and(|receipt|
                    receipt.effects.iter().any(|effect| effect.resource_uri == *resource_uri && effect.after_version.as_ref() != Some(version))
                    || receipt.checks.iter().any(|check| check.resource_uri == *resource_uri && check.version != *version)) {
                    return Err(format!("Skill verification references a resource version superseded by a later execution; obtain a current check and use replaces_evidence={}",
                        serde_json::to_string(&result.evidence).map_err(|error| error.to_string())?));
                }
            }
        }
        Ok(())
    }

    pub(super) fn apply_skill_runtime_command(
        &mut self,
        envelope: SkillRuntimeCommandEnvelope,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        if !self.session.skill_runtime.accepted_operation_ids.contains(&envelope.operation_id) {
            match &envelope.command {
                SkillRuntimeCommand::RecordVerification { run_id, result } => self.validate_skill_verification_sources(run_id, result)?,
                SkillRuntimeCommand::Complete { run_id, .. } => {
                    if let Some(run) = self.session.skill_runtime.runs.get(run_id) {
                        for result in run.verifications.values() { self.validate_skill_verification_sources(run_id, result)?; }
                    }
                }
                _ => {},
            }
        }
        let transition = self
            .session
            .skill_runtime
            .apply(envelope)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        for update in transition.events {
            let run_id = match &update.event {
                SkillRuntimeEvent::Started { run } => Some(run.run_id.as_str()),
                SkillRuntimeEvent::CapabilityRefreshRequested { run_id, .. }
                | SkillRuntimeEvent::Resumed { run_id, .. }
                | SkillRuntimeEvent::UsageUpdated { run_id, .. }
                | SkillRuntimeEvent::ProgressUpdated { run_id, .. }
                | SkillRuntimeEvent::ArtifactRegistered { run_id, .. }
                | SkillRuntimeEvent::VerificationRecorded { run_id, .. }
                | SkillRuntimeEvent::ResumePointUpdated { run_id, .. }
                | SkillRuntimeEvent::BudgetExceeded { run_id, .. }
                | SkillRuntimeEvent::Completed { run_id, .. }
                | SkillRuntimeEvent::Failed { run_id, .. }
                | SkillRuntimeEvent::Cancelled { run_id, .. } => Some(run_id.as_str()),
                SkillRuntimeEvent::CatalogReplaced { .. } => None,
            };
            let turn_id = run_id
                .and_then(|run_id| self.session.skill_runtime.runs.get(run_id))
                .map(|run| TurnId::from_string(run.owner.parent_turn_id.clone()));
            if let SkillRuntimeEvent::CapabilityRefreshRequested { run_id, errors } = &update.event
            {
                effects.push(AgentEffect::RefreshCapabilityCatalog {
                    turn_id: turn_id
                        .clone()
                        .ok_or_else(|| "capability refresh has no owning Skill Turn".to_string())?,
                    run_id: run_id.clone(),
                    catalog_revision: self.session.skill_runtime.catalog.revision,
                    errors: errors.clone(),
                });
            }
            events.push(RuntimeEvent::SkillRuntimeUpdated {
                turn_id,
                skill_runtime_revision: transition.revision,
                update: update.event,
            });
        }
        Ok(())
    }

    pub(super) fn active_skill_run_ids(&self, turn_id: &TurnId) -> Vec<String> {
        self.session
            .skill_runtime
            .runs
            .values()
            .filter(|run| {
                run.owner.parent_turn_id == turn_id.as_str()
                    && run.status == SkillRunStatus::Running
            })
            .map(|run| run.run_id.clone())
            .collect()
    }

    fn resolve_active_skill_run_id(
        &self,
        turn_id: &TurnId,
        requested_run_id: &str,
    ) -> Result<String, String> {
        let active_runs = self
            .session
            .skill_runtime
            .runs
            .values()
            .filter(|run| {
                run.owner.parent_turn_id == turn_id.as_str()
                    && run.status == SkillRunStatus::Running
            })
            .collect::<Vec<_>>();
        if let Some(run) = active_runs
            .iter()
            .find(|run| run.run_id == requested_run_id)
        {
            return Ok(run.run_id.clone());
        }
        let matching_skill_runs = active_runs
            .iter()
            .filter(|run| run.descriptor.skill_id == requested_run_id)
            .collect::<Vec<_>>();
        if matching_skill_runs.len() == 1 {
            return Ok(matching_skill_runs[0].run_id.clone());
        }
        let active_ids = active_runs
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "skill_update run_id `{requested_run_id}` does not identify one active SkillRun owned by the current turn; active run ids: [{active_ids}]"
        ))
    }

    fn nonterminal_skill_run_ids(&self, turn_id: &TurnId) -> Vec<String> {
        self.session
            .skill_runtime
            .runs
            .values()
            .filter(|run| run.owner.parent_turn_id == turn_id.as_str() && !run.status.is_terminal())
            .map(|run| run.run_id.clone())
            .collect()
    }

    fn synchronize_skill_catalog(
        &mut self,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let host = &self.session.host.catalog;
        let operation_id = format!(
            "host_catalog:{}:{}:{}",
            host.host_id.0, host.revision, host.availability_epoch
        );
        let mcp = if self.session.skill_runtime.catalog.mcp.revision == host.revision {
            self.session.skill_runtime.catalog.mcp.clone()
        } else {
            McpCapabilityCatalog {
                revision: host.revision,
                servers: Default::default(),
            }
        };
        let catalog = CapabilityCatalog {
            revision: host.revision,
            builtin_tools: host
                .capabilities
                .iter()
                .filter(|(_, capability)| {
                    capability.route == HostCapabilityRoute::Tool
                        && matches!(
                            capability.exposure,
                            HostCapabilityExposure::Visible | HostCapabilityExposure::Deferred
                        )
                })
                .map(|(capability_id, _)| capability_id.clone())
                .collect(),
            mcp,
        };
        if catalog == self.session.skill_runtime.catalog {
            return Ok(());
        }
        self.apply_skill_runtime_command(
            SkillRuntimeCommandEnvelope {
                operation_id,
                command: SkillRuntimeCommand::ReplaceCatalog { catalog },
            },
            events,
            effects,
        )
    }

    fn start_materialized_skill(
        &mut self,
        turn_id: &TurnId,
        call_id: &CallId,
        invocation: MaterializedSkillInvocation,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        self.synchronize_skill_catalog(events, effects)?;
        let run_id = format!("skill:{}:{}", turn_id.as_str(), call_id.as_str());
        self.apply_skill_runtime_command(
            SkillRuntimeCommandEnvelope {
                operation_id: format!("skill_start:{}", call_id.as_str()),
                command: SkillRuntimeCommand::Start {
                    run_id,
                    owner: SkillRunOwner {
                        session_id: self.session.session_id.as_str().to_string(),
                        parent_turn_id: turn_id.as_str().to_string(),
                        child_turn_id: None,
                    },
                    descriptor: invocation.descriptor(),
                    mode: invocation.mode.unwrap_or(SkillExecutionMode::Inline),
                    budget: invocation.budget.unwrap_or_else(SkillRunBudget::default),
                },
            },
            events,
            effects,
        )
    }

    fn record_active_skill_model_turns(
        &mut self,
        turn_id: &TurnId,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let revision = self.session.journal.revision;
        for run_id in self.active_skill_run_ids(turn_id) {
            self.apply_skill_runtime_command(
                SkillRuntimeCommandEnvelope {
                    operation_id: format!("skill_model_turn:{run_id}:{revision}"),
                    command: SkillRuntimeCommand::RecordModelTurn { run_id },
                },
                events,
                effects,
            )?;
        }
        Ok(())
    }

    fn record_active_skill_elapsed(
        &mut self,
        turn_id: &TurnId,
        elapsed_millis: u64,
        source: &str,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        if elapsed_millis == 0 {
            return Ok(());
        }
        let revision = self.session.journal.revision;
        for run_id in self.active_skill_run_ids(turn_id) {
            self.apply_skill_runtime_command(
                SkillRuntimeCommandEnvelope {
                    operation_id: format!("skill_elapsed:{run_id}:{source}:{revision}"),
                    command: SkillRuntimeCommand::RecordElapsed {
                        run_id,
                        elapsed_millis,
                    },
                },
                events,
                effects,
            )?;
        }
        Ok(())
    }

    fn record_active_skill_tool_calls(
        &mut self,
        turn_id: &TurnId,
        call_id: &CallId,
        capability_id: &str,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        for run_id in self.active_skill_run_ids(turn_id) {
            self.apply_skill_runtime_command(
                SkillRuntimeCommandEnvelope {
                    operation_id: format!("skill_tool_call:{run_id}:{}", call_id.as_str()),
                    command: SkillRuntimeCommand::RecordToolCall {
                        run_id,
                        tool: CapabilityToolReference::Builtin {
                            tool_name: capability_id.to_string(),
                        },
                    },
                },
                events,
                effects,
            )?;
        }
        Ok(())
    }

    fn apply_materialized_skill_runtime_update(
        &mut self,
        turn_id: &TurnId,
        call_id: &CallId,
        update: MaterializedSkillRuntimeUpdate,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let requested_run_id = match &update {
            MaterializedSkillRuntimeUpdate::Progress { run_id, .. }
            | MaterializedSkillRuntimeUpdate::Artifact { run_id, .. }
            | MaterializedSkillRuntimeUpdate::Verification { run_id, .. }
            | MaterializedSkillRuntimeUpdate::ResumePoint { run_id, .. } => run_id.as_str(),
        };
        let run_id = self.resolve_active_skill_run_id(turn_id, requested_run_id)?;
        let command = match update {
            MaterializedSkillRuntimeUpdate::Progress {
                step_id, completed, ..
            } => SkillRuntimeCommand::RecordProgress {
                run_id: run_id.clone(),
                step_id,
                completed,
            },
            MaterializedSkillRuntimeUpdate::Artifact {
                artifact_id,
                artifact_ref,
                ..
            } => SkillRuntimeCommand::RegisterArtifact {
                run_id: run_id.clone(),
                artifact_id,
                artifact_ref,
            },
            MaterializedSkillRuntimeUpdate::Verification {
                verifier_id,
                passed,
                evidence,
                replaces_evidence,
                summary,
                ..
            } => SkillRuntimeCommand::RecordVerification {
                run_id: run_id.clone(),
                result: SkillVerificationResult {
                    verifier_id,
                    passed,
                    evidence,
                    replaces_evidence,
                    summary,
                },
            },
            MaterializedSkillRuntimeUpdate::ResumePoint { resume_point, .. } => {
                SkillRuntimeCommand::SetResumePoint {
                    run_id: run_id.clone(),
                    resume_point,
                }
            }
        };
        self.apply_skill_runtime_command(
            SkillRuntimeCommandEnvelope {
                operation_id: format!("skill_update:{run_id}:{}", call_id.as_str()),
                command,
            },
            events,
            effects,
        )?;
        self.complete_skill_if_contract_satisfied(
            &run_id,
            format!("skill_contract_satisfied:{run_id}:{}", call_id.as_str()),
            events,
            effects,
        )
    }

    /// A Skill execution contract is declarative: once all required records
    /// exist, Core owns the terminal transition. Waiting for a later model
    /// response to imply completion leaves a satisfied run artificially active
    /// and lets stale bookkeeping calls turn successful work into recovery.
    fn complete_skill_if_contract_satisfied(
        &mut self,
        run_id: &str,
        operation_id: String,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let satisfied = self
            .session
            .skill_runtime
            .runs
            .get(run_id)
            .is_some_and(|run| {
                run.status == SkillRunStatus::Running && validate_skill_completion(run).is_ok()
            });
        if !satisfied {
            return Ok(());
        }
        self.apply_skill_runtime_command(
            SkillRuntimeCommandEnvelope {
                operation_id,
                command: SkillRuntimeCommand::Complete {
                    run_id: run_id.to_string(),
                    result_ref: None,
                },
            },
            events,
            effects,
        )
    }

    /// Connect a Host-observed output to a Skill contract when the mapping is
    /// structurally unambiguous. This keeps capability contracts composable:
    /// a successful writer's canonical artifact is evidence, so the model
    /// does not have to copy its URI through a second fallible protocol call.
    fn register_unambiguous_skill_artifact_from_tool(
        &mut self,
        turn_id: &TurnId,
        call_id: &CallId,
        capability_id: &str,
        artifacts: &[nola_host_protocol::HostArtifact],
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let [artifact] = artifacts else {
            return Ok(());
        };
        let produces_durable_output = self
            .session
            .execution
            .active_turn
            .as_ref()
            .and_then(|turn| {
                let descriptor = turn
                    .context
                    .capability_projection
                    .descriptor(capability_id)?;
                let arguments = turn
                    .tools
                    .iter()
                    .find(|tool| &tool.call_id == call_id)
                    .map(|tool| &tool.arguments)?;
                Some(descriptor.effect_for(arguments))
            })
            .is_some_and(|effect| {
                matches!(
                    effect,
                    HostCapabilityEffect::Mutate | HostCapabilityEffect::MutateAndValidate
                )
            });
        if !produces_durable_output {
            return Ok(());
        }
        let candidates = self
            .session
            .skill_runtime
            .runs
            .values()
            .filter(|run| {
                run.owner.parent_turn_id == turn_id.as_str()
                    && run.status == SkillRunStatus::Running
            })
            .filter_map(|run| {
                let missing = run
                    .descriptor
                    .execution_contract
                    .artifacts
                    .iter()
                    .filter(|required| {
                        required.required && !run.artifacts.contains_key(&required.id)
                    })
                    .collect::<Vec<_>>();
                (missing.len() == 1
                    && artifact.mime_type.as_deref() == Some(missing[0].media_type.as_str()))
                .then(|| (run.run_id.clone(), missing[0].id.clone()))
            })
            .collect::<Vec<_>>();
        let [(run_id, artifact_id)] = candidates.as_slice() else {
            return Ok(());
        };
        self.apply_skill_runtime_command(
            SkillRuntimeCommandEnvelope {
                operation_id: format!("skill_artifact_from_tool:{run_id}:{}", call_id.as_str()),
                command: SkillRuntimeCommand::RegisterArtifact {
                    run_id: run_id.clone(),
                    artifact_id: artifact_id.clone(),
                    artifact_ref: artifact.uri.clone(),
                },
            },
            events,
            effects,
        )?;
        self.complete_skill_if_contract_satisfied(
            run_id,
            format!(
                "skill_contract_satisfied_from_tool:{run_id}:{}",
                call_id.as_str()
            ),
            events,
            effects,
        )
    }

    fn finalize_skills_from_turn_events(
        &mut self,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        // A goal pause ends the interactive turn, not the SkillRun. The Skill
        // remains Core-owned and resumable while the user or a background task
        // supplies the evidence required by its execution contract.
        let paused_goal_turns = events
            .iter()
            .filter_map(|event| match event {
                RuntimeEvent::ToolCompleted {
                    turn_id,
                    tool_name,
                    output,
                    ..
                } if tool_name == "goal_complete"
                    && output
                        .get("goal_status")
                        .and_then(Value::as_str)
                        .is_some_and(|status| matches!(status, "needs_user" | "blocked")) =>
                {
                    Some(turn_id.clone())
                }
                RuntimeEvent::PartUpdated { turn_id, part: AgentPart::Tool(part), .. }
                    if part.capability_id == "goal_complete"
                        && part.structured_output.as_ref().and_then(|output| output.get("goal_status"))
                            .and_then(Value::as_str)
                            .is_some_and(|status| matches!(status, "needs_user" | "blocked")) =>
                    Some(turn_id.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let terminal = events
            .iter()
            .filter_map(|event| match event {
                RuntimeEvent::TurnCompleted { turn_id, .. } => Some((turn_id.clone(), None, false)),
                RuntimeEvent::TurnFailed { turn_id, error, .. } => {
                    Some((turn_id.clone(), Some(error.clone()), false))
                }
                RuntimeEvent::TurnCancelled {
                    turn_id, reason, ..
                } => Some((turn_id.clone(), Some(reason.clone()), true)),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (turn_id, failure, cancelled) in terminal {
            if failure.is_none() && !cancelled && !self.session.execution.requirements.decision.as_ref()
                .is_some_and(|decision| matches!(decision.value, nola_domain::run_result::RunResultValue::ActionCompleted { .. })) {
                // Closing a conversational Turn is not a Skill acceptance.
                continue;
            }
            if failure.is_none()
                && !cancelled
                && paused_goal_turns.iter().any(|paused| paused == &turn_id)
            {
                continue;
            }
            for run_id in self.nonterminal_skill_run_ids(&turn_id) {
                let command = if cancelled {
                    SkillRuntimeCommand::Cancel {
                        run_id: run_id.clone(),
                        reason: failure.clone(),
                    }
                } else if let Some(error) = failure.clone() {
                    SkillRuntimeCommand::Fail {
                        run_id: run_id.clone(),
                        error,
                    }
                } else {
                    SkillRuntimeCommand::Complete {
                        run_id: run_id.clone(),
                        result_ref: None,
                    }
                };
                let completion = self.apply_skill_runtime_command(
                    SkillRuntimeCommandEnvelope {
                        operation_id: format!("skill_terminal:{run_id}"),
                        command,
                    },
                    events,
                    effects,
                );
                if let Err(error) = completion {
                    if failure.is_none() && !cancelled {
                        self.apply_skill_runtime_command(
                            SkillRuntimeCommandEnvelope {
                                operation_id: format!("skill_terminal_failed:{run_id}"),
                                command: SkillRuntimeCommand::Fail {
                                    run_id,
                                    error: format!(
                                        "Skill completion contract was not satisfied: {error}"
                                    ),
                                },
                            },
                            events,
                            effects,
                        )?;
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }
}
