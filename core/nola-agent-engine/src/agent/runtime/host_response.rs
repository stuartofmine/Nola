// SPDX-License-Identifier: Apache-2.0
// Internal layer of the deterministic agent runtime. Public clients use AgentRuntime.

use super::*;
use crate::agent::capability::MaterializedSkillRuntimeUpdate;
use crate::agent::host::validate_host_artifacts;
use crate::agent::PermissionUserIntervention;
use crate::agent::TokenUsage;
use nola_host_protocol::{
    HostPermissionAuthorizationDecision, HostPermissionIntervention, HostPermissionResolution,
};

// Post-admission dispatch facts only. The complete request is validated by the
// effect journal before this projection is read; this cannot authorize a call.
struct AcceptedEffectContext {
    turn_id: Option<String>,
    call_id: Option<String>,
    task_id: Option<String>,
    capability_id: String,
    replay_safety: HostReplaySafety,
    cancelling_target: bool,
}

impl AgentRuntime {
    fn finish_quiescent_cancellation(&mut self) -> Result<Option<AgentReduction>, String> {
        let cancelling_turn = self
            .session
            .execution
            .active_turn
            .as_ref()
            .filter(|turn| turn.status == AgentTurnStatus::Cancelling)
            .map(|turn| turn.turn_id.clone());
        let cancellation_complete = cancelling_turn.as_ref().is_some_and(|turn_id| {
            !self
                .session
                .host
                .pending_effects
                .iter()
                .any(|pending| pending.turn_id.as_deref() == Some(turn_id.as_str()))
        });
        if cancellation_complete {
            let turn = self
                .session
                .execution
                .active_turn
                .as_mut()
                .expect("cancelling Turn exists");
            turn.confirm_code_cells_cancelled()?;
            let turn_id = turn.turn_id.clone();
            let event = if let Some(error) = turn.failure.clone() {
                turn.status = AgentTurnStatus::Failed;
                RuntimeEvent::TurnFailed { session_id: self.session.session_id.clone(), turn_id, error }
            } else {
                turn.status = AgentTurnStatus::Cancelled;
                let reason = turn.cancellation_reason.take().unwrap_or_else(|| "cancelled by user".to_string());
                RuntimeEvent::TurnCancelled { session_id: self.session.session_id.clone(), turn_id, reason }
            };
            return self.finish_transition(vec![event], Vec::new(), Some(CheckpointReason::TurnCompleted)).map(Some);
        }
        Ok(None)
    }

    fn finish_host_cancellation(&mut self, turn_id:TurnId, detail:String)->Result<AgentReduction,String> {
        if let Some(cancellation_id)=self.session.execution.requirements.cancellation_id.clone() {
            self.dispatch(AgentCommand::Cancel { goal_control: None, cancellation_id,reason:Some(detail) })
        } else {
            self.dispatch(AgentCommand::ModelFailed {turn_id,error:format!("Host execution stopped without an admitted cancellation: {detail}")})
        }
    }
}

fn completed_model_items(
    items: Vec<serde_json::Value>,
    independent_review: bool,
) -> Result<Vec<AgentItem>, String> {
    let mut items = items
        .into_iter()
        .map(serde_json::from_value)
        .collect::<Result<Vec<AgentItem>, _>>()
        .map_err(|error| format!("invalid model item: {error}"))?;
    if independent_review {
        // Review is a separate model invocation, never the conversation's
        // continuation owner, even if its Host includes protocol metadata.
        for item in &mut items {
            if let AgentItem::Reasoning {
                provider_continuation,
                ..
            } = item
            {
                *provider_continuation = None;
            }
        }
    }
    Ok(items)
}

impl AgentRuntime {
    pub(super) fn envelope_events(
        &mut self,
        events: Vec<AgentEvent>,
    ) -> Vec<CoreEventEnvelope<AgentEvent>> {
        events
            .into_iter()
            .map(|event| {
                let sequence = self.session.journal.next_event_sequence;
                self.session.journal.next_event_sequence =
                    self.session.journal.next_event_sequence.saturating_add(1);
                CoreEventEnvelope {
                    session_id: self.session.session_id.as_str().to_string(),
                    turn_id: match &event {
                        AgentEvent::PermissionSettled { receipt } => Some(receipt.turn_id.clone()),
                        AgentEvent::UserInputWaitChanged { wait } => Some(wait.turn_id.clone()),
                        _ => Self::event_turn_id(&event).map(|turn_id| turn_id.as_str().to_string()),
                    },
                    cursor: EventCursor { sequence },
                    revision: self.session.journal.revision,
                    event,
                }
            })
            .collect()
    }

    pub(super) fn event_turn_id(event: &AgentEvent) -> Option<&TurnId> {
        match event {
            AgentEvent::PermissionSettled { .. } => None,
            AgentEvent::UserInputWaitChanged { .. } => None,
            AgentEvent::AssistantSegmentStarted { turn_id, .. }
            | AgentEvent::AssistantEffectBound { turn_id, .. }
            | AgentEvent::AssistantActivityBound { turn_id, .. }
            | AgentEvent::TurnStarted { turn_id, .. }
            | AgentEvent::RuntimeDecisionFactObserved { turn_id, .. }
            | AgentEvent::RequirementScopeChanged { turn_id, .. }
            | AgentEvent::RequirementAssessed { turn_id, .. }
            | AgentEvent::RunResultDecided { turn_id, .. }
            | AgentEvent::InputQueued { turn_id, .. }
            | AgentEvent::InputApplied { turn_id, .. }
            | AgentEvent::InputWithdrawn { turn_id, .. }
            | AgentEvent::PartUpdated { turn_id, .. }
            | AgentEvent::CapabilitiesActivated { turn_id, .. }
            | AgentEvent::ExecutionEvidenceObserved { turn_id, .. }
            | AgentEvent::EngineeringLoopUpdated { turn_id, .. } => Some(turn_id),
            AgentEvent::TaskUpdated { task, .. } => Some(&task.owner_turn_id),
            AgentEvent::SkillRuntimeUpdated { turn_id, .. } => turn_id.as_ref(),
            AgentEvent::LifecycleHookUpdated { turn_id, .. } => turn_id.as_ref(),
            AgentEvent::TurnCompleted { turn_id, .. }
            | AgentEvent::TurnCancelled { turn_id, .. }
            | AgentEvent::TurnFailed { turn_id, .. } => Some(turn_id),
            AgentEvent::HostProgress { turn_id, .. } => turn_id.as_ref(),
            AgentEvent::SessionStarted { .. }
            | AgentEvent::InputAccepted { .. }
            | AgentEvent::GoalUpdated { .. }
            | AgentEvent::MemoryUpdated { .. }
            | AgentEvent::HostCatalogUpdated { .. }
            | AgentEvent::HostLeaseRenewed { .. }
            | AgentEvent::SessionConfigurationUpdated { .. }
            | AgentEvent::HostDisconnected { .. }
            | AgentEvent::ContextUpdated { .. }
            | AgentEvent::TurnContextCaptured { .. }
            | AgentEvent::CompactionRequested { .. }
            | AgentEvent::CompactionCompleted { .. }
            | AgentEvent::CompactionFailed { .. } => None,
        }
    }

    pub(super) fn dispatch_host_response(
        &mut self,
        mut response: HostResponseEnvelope,
    ) -> Result<AgentReduction, String> {
        match &response.payload {
            HostResponse::CodeModeObservationSuspended { cell, call_id } => {
                let observer = self.session.host.pending_effects.get(&response.effect_id.0)
                    .ok_or("code observation has no pending effect")?;
                let current = self.session.execution.code_cell(&cell.cell_id)
                    .ok_or("unknown observed code cell")?;
                let requested_cell = self.session.execution.active_turn.as_ref()
                    .and_then(|turn| turn.tools.iter().find(|tool| tool.call_id.as_str() == call_id))
                    .and_then(|tool| tool.arguments.get("cell_id"))
                    .and_then(Value::as_str);
                if observer.capability_id != "code_mode_wait"
                    || observer.call_id.as_deref() != Some(call_id.as_str())
                    || requested_cell != Some(cell.cell_id.as_str())
                    || current.identity() != cell
                    || cell.host_id != response.host_id.0
                    || cell.conversation_id != self.session.session_id.as_str() {
                    return Err("code observation suspension identity mismatch".into());
                }
            }
            HostResponse::CodeModeYielded { cell, output_cursor } => {
                let parent = self.session.host.pending_effects.get(&response.effect_id.0)
                    .ok_or("code yield has no pending parent effect")?;
                if parent.capability_id != "code_mode_exec"
                    || parent.call_id.as_deref() != Some(cell.parent_call_id.as_str())
                    || cell.host_id != response.host_id.0
                    || cell.conversation_id != self.session.session_id.as_str() {
                    return Err("code yield parent identity mismatch".into());
                }
                let turn = self.session.execution.active_turn.as_ref().ok_or("no active code cell turn")?;
                let mut candidate = turn.code_cells.get(&cell.cell_id).ok_or("unknown code yield cell")?.clone();
                candidate.record_yield(cell, *output_cursor)?;
            }
            HostResponse::CodeModeOutput { frame } => {
                let parent = self.session.host.pending_effects.get(&response.effect_id.0)
                    .ok_or("code output has no pending parent effect")?;
                if parent.capability_id != "code_mode_exec"
                    || parent.call_id.as_deref() != Some(frame.cell.parent_call_id.as_str())
                    || frame.cell.host_id != response.host_id.0
                    || frame.cell.conversation_id != self.session.session_id.as_str() {
                    return Err("code output parent identity mismatch".into());
                }
                let turn = self.session.execution.active_turn.as_ref().ok_or("no active code cell turn")?;
                let mut candidate = turn.code_cells.get(&frame.cell.cell_id)
                    .ok_or("unknown code output cell")?.clone();
                candidate.append_output(frame.clone())?;
            }
            HostResponse::CodeModeToolRequested { request } => {
                let parent = self.session.host.pending_effects.get(&response.effect_id.0)
                    .ok_or("code cell callback has no pending parent effect")?;
                if parent.capability_id != "code_mode_exec"
                    || parent.call_id.as_deref() != Some(request.cell.parent_call_id.as_str())
                    || request.cell.host_id != response.host_id.0
                    || request.cell.conversation_id != self.session.session_id.as_str() {
                    return Err("code cell callback parent identity mismatch".into());
                }
                let turn = self.session.execution.active_turn.as_ref().ok_or("no active code cell turn")?;
                let cell = turn.code_cells.get(&request.cell.cell_id).ok_or("unknown code cell callback")?;
                // Validate sequence/budget before consuming the Host response
                // cursor. The actual transition uses the same registration.
                let mut candidate = cell.clone();
                candidate.register_call(request.clone())?;
            }
            HostResponse::CompactionCompleted { summary, usage }
                if !self.session.host.pending_effects.get(&response.effect_id.0).is_some_and(|pending|
                    pending.state == PendingHostEffectState::Cancelling && pending.target_effect_id.is_none()) => {
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
                {
                    let configuration = self.session.execution.active_turn.as_ref()
                        .map(|turn| &turn.context.configuration.context_window)
                        .unwrap_or(&self.session.configuration.context_window);
                    plan.validate_summary_for_prompt(summary, configuration.prompt_token_limit(),
                        &self.session.conversation.model_context)?;
                }
                // Validate before advancing the response cursor or consuming
                // the pending effect. A corrected response must remain usable.
                serde_json::from_value::<TokenUsage>(usage.clone())
                    .map_err(|error| format!("invalid compaction usage: {error}"))?;
            }
            HostResponse::ToolCompleted {
                output,
                outcome,
                artifacts,
                ..
            }
            | HostResponse::DelegatedAgentEvent {
                event:
                    nola_host_protocol::DelegatedAgentEventV1::ToolCompleted {
                        output,
                        outcome,
                        artifacts,
                        ..
                    },
            } => {
                nola_host_protocol::tool_result::validate_completion(output, *outcome)?;
                nola_host_protocol::tool_result::core_tool_output(output.clone())?;
                validate_host_artifacts(artifacts)?;
            }
            HostResponse::Completed { artifacts, .. } => {
                validate_host_artifacts(artifacts)?;
            }
            _ => {}
        }
        // In-process Hosts have no replaceable lease. Their response envelope
        // may omit the optional Turn echo, so bind it from the authoritative
        // pending effect. Leased remote/process Hosts must continue to echo an
        // exact Turn identity and are rejected when it is missing or wrong.
        if response.turn_id.is_none() && response.lease_id.is_none() {
            if let Some(pending) = self
                .session
                .host
                .pending_effects
                .get(&response.effect_id.0)
                .filter(|pending| pending.lease_id.is_none())
            {
                response.turn_id = pending.turn_id.clone();
            }
        }
        if let Some(response_call_id) = match &response.payload {
            HostResponse::ToolCallProgress { call_id, .. }
            | HostResponse::ToolCompleted { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        } {
            if let Some(pending) = self.session.host.pending_effects.get(&response.effect_id.0) {
                let expected_call_id = pending.call_id.as_deref().ok_or_else(|| {
                    format!(
                        "rejected Host response: code=tool_call_identity_missing effect_id={} received_call_id={response_call_id}",
                        response.effect_id.0
                    )
                })?;
                if response_call_id != expected_call_id {
                    return Err(format!(
                        "rejected Host response: code=tool_call_identity_mismatch effect_id={} expected_call_id={expected_call_id} received_call_id={response_call_id}",
                        response.effect_id.0
                    ));
                }
            }
        }
        let pending_identity = self
            .session
            .host
            .pending_effects
            .get(&response.effect_id.0)
            .map(|effect| {
                let request_type = effect
                    .request
                    .as_ref()
                    .and_then(|value| value.get("payload"))
                    .and_then(|value| value.get("type"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown");
                (
                    effect.turn_id.as_deref().unwrap_or("none").to_string(),
                    request_type.to_string(),
                    effect.last_response_sequence,
                )
            });
        let acceptance = self.session.host.pending_effects.accept_response(&response);
        if acceptance != HostEffectAcceptance::Accepted {
            let (expected_turn_id, request_type, last_response_sequence) = pending_identity
                .unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string(), None));
            let response_type = serde_json::to_value(&response.payload)
                .ok()
                .and_then(|value| {
                    value
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "unknown".to_string());
            return Err(format!(
                "rejected Host response: code={} category={} effect_id={} request_id={} host_id={} turn_id={} expected_turn_id={} request_type={} response_type={} response_sequence={} last_response_sequence={}",
                acceptance.code(),
                acceptance.category(),
                response.effect_id.0,
                response.request_id,
                response.host_id.0,
                response.turn_id.as_deref().unwrap_or("none"),
                expected_turn_id,
                request_type,
                response_type,
                response.sequence,
                last_response_sequence.map_or_else(|| "none".to_string(), |value| value.to_string()),
            ));
        }
        let batch_id = self
            .session
            .host
            .pending_effects
            .batch_id_for_effect(&response.effect_id.0);
        if self
            .session
            .host
            .pending_effects
            .defer_out_of_order_batch_terminal(&response)?
        {
            return Ok(AgentReduction {
                event_envelopes: Vec::new(),
                trace_events: Vec::new(),
                host_requests: Vec::new(),
                durability: None,
            });
        }
        let mut transition = self.dispatch_accepted_host_response(response)?;
        if let Some(batch_id) = batch_id {
            while let Some(next_response) = self
                .session
                .host
                .pending_effects
                .take_ready_buffered_batch_terminal(&batch_id)?
            {
                let next = self.dispatch_accepted_host_response(next_response)?;
                transition.event_envelopes.extend(next.event_envelopes);
                transition.trace_events.extend(next.trace_events);
                transition.host_requests.extend(next.host_requests);
                if next.durability.is_some() {
                    transition.durability = next.durability;
                }
            }
        }
        Ok(transition)
    }

    fn dispatch_accepted_host_response(
        &mut self,
        mut response: HostResponseEnvelope,
    ) -> Result<AgentReduction, String> {
        let pending = self
            .session
            .host
            .pending_effects
            .get(&response.effect_id.0)
            .ok_or_else(|| "Host response has no pending effect".to_string())?;
        let control = pending.validated_control()?;
        let resolution_request = if let nola_domain::host_control::HostResponsePolicy::ResolvePermissionResources { call_id } = &control.payload.policy {
            self.session.execution.active_turn.as_ref()
                .and_then(|turn|turn.tools.iter().find(|tool|tool.call_id.as_str() == call_id))
                .and_then(|tool|tool.permission_resolution_request.clone())
        } else { None };
        let independent_review = matches!(
            control.payload.policy,
            nola_domain::host_control::HostResponsePolicy::ModelStream {
                response_mode: crate::agent::host::ModelResponseMode::StructuredAudit,
            }
        );
        let task_cancellation = matches!(control.payload.policy, nola_domain::host_control::HostResponsePolicy::CancelTask {});
        let effect_elapsed_millis = self.observed_at_ms.saturating_sub(control.created_at_ms);
        let controlled_cell = match &control.payload.policy {
            nola_domain::host_control::HostResponsePolicy::PollCodeCell { cell }
            | nola_domain::host_control::HostResponsePolicy::ResolveCodeTool { cell, .. } => Some(cell.clone()),
            _ => None,
        };
        let pending = AcceptedEffectContext {
            turn_id: pending.turn_id.clone(),
            call_id: pending.call_id.clone(),
            task_id: pending.task_id.clone(),
            capability_id: pending.capability_id.clone(),
            replay_safety: pending.replay_safety,
            cancelling_target: pending.state == PendingHostEffectState::Cancelling
                && pending.target_effect_id.is_none(),
        };
        let turn_id = response
            .turn_id
            .clone()
            .or(pending.turn_id.clone())
            .map(TurnId::from_string);
        let effect_id = response.effect_id.0.clone();
        if matches!(&response.payload, HostResponse::ModelCompleted { .. } | HostResponse::ModelOutputLimited { .. }) {
            self.session.execution.requirements.model_source = Some(nola_domain::core_requirements::CoreDecisionSource::Model {
                effect_id: effect_id.clone(), request_id: response.request_id.clone(),
                attempt_id: response.attempt_id.clone(), call_id: None,
            });
        }
        if let HostResponse::ExecutionRejected {
            code,
            message,
            retryable,
        } = &response.payload
        {
            response.payload = HostResponse::Failed {
                error: crate::agent::host::HostError {
                    code: code.clone(),
                    message: message.clone(),
                    retryable: *retryable,
                    http_status: None,
                },
            };
        }
        if pending.cancelling_target {
            if response.payload.is_terminal() {
                self.session.host.pending_effects.complete(&effect_id);
            }
            return Ok(AgentReduction {
                event_envelopes: Vec::new(),
                trace_events: Vec::new(),
                host_requests: Vec::new(),
                durability: None,
            });
        }
        match response.payload {
            HostResponse::PermissionResourcesResolved { call_id, facts } => {
                let request = resolution_request.ok_or("resource resolution response has no frozen request")?;
                facts.subject_for(&request).map_err(|error| error.to_string())?;
                let turn_id = turn_id.ok_or("resource resolution response has no Turn")?;
                let turn = self.session.execution.active_turn.as_mut()
                    .filter(|turn| turn.turn_id == turn_id && !turn.status.is_terminal())
                    .ok_or("resource resolution belongs to an inactive Turn")?;
                let tool = turn.tools.iter_mut().find(|tool| tool.call_id.as_str() == call_id)
                    .ok_or("resource resolution has no matching tool call")?;
                let catalog = &self.session.host.catalog;
                let plan = catalog.resolve_available(&pending.capability_id)
                    .and_then(|capability| capability.input_normalization.permission_contract.resource_plan_for(&tool.arguments))
                    .ok_or("resource resolution has no current frozen catalog declaration")?;
                if tool.status != ToolCallStatus::Running || tool.name != pending.capability_id
                    || response.catalog_revision != catalog.revision || response.availability_epoch != catalog.availability_epoch
                    || request.execution_target != catalog.host_id.0 || request.capability != plan.capability
                    || request.operation != plan.operation || request.demands != plan.extract(&tool.arguments).map_err(|error|error.to_string())?
                    || request.invocation_digest != nola_domain::permission_resource_plan::PermissionResolutionRequest::input_digest(&tool.arguments) {
                    return Err("resource resolution differs from the pending invocation".into());
                }
                tool.permission_resolution = Some((request, facts));
                tool.permission_resolution_request = None;
                tool.status = ToolCallStatus::Pending;
                self.session.host.pending_effects.complete(&effect_id);
                let mut events = Vec::new();
                let mut effects = Vec::new();
                self.schedule_tools(&turn_id, &mut events, &mut effects)?;
                self.finish_transition(events, effects, None)
            }
            HostResponse::Accepted => {
                self.session.host.pending_effects.mark_accepted(&effect_id);
                self.dispatch(AgentCommand::HostProgress {
                    turn_id,
                    effect_id,
                    title: "accepted".to_string(),
                    detail: None,
                    metadata: serde_json::Value::Null,
                })
            }
            HostResponse::ExecutionStarted => {
                self.session.host.pending_effects.mark_accepted(&effect_id);
                self.dispatch(AgentCommand::HostProgress {
                    turn_id,
                    effect_id,
                    title: "execution_started".to_string(),
                    detail: None,
                    metadata: serde_json::Value::Null,
                })
            }
            HostResponse::ExecutionRejected { .. } => {
                unreachable!("execution rejection is normalized before Core reduction")
            }
            HostResponse::DelegatedAgentEvent { event } => {
                event.validate().map_err(str::to_string)?;
                if let nola_host_protocol::DelegatedAgentEventV1::Completed { items, usage } = &event {
                    // validate() checks the event shape, but Completed carries
                    // opaque JSON. Validate its typed payload before admitting a
                    // decision witness or retiring the original effect.
                    for item in items {
                        serde_json::from_value::<AgentItem>(item.clone())
                            .map_err(|error| format!("invalid delegated Agent item: {error}"))?;
                    }
                    serde_json::from_value::<TokenUsage>(usage.clone())
                        .map_err(|error| format!("invalid delegated Agent usage: {error}"))?;
                    self.session.execution.requirements.model_source = Some(nola_domain::core_requirements::CoreDecisionSource::Model {
                        effect_id: effect_id.clone(), request_id: response.request_id.clone(),
                        attempt_id: response.attempt_id.clone(), call_id: None,
                    });
                }
                if event.is_terminal() {
                    self.session.host.pending_effects.complete(&effect_id);
                } else {
                    self.session.host.pending_effects.mark_streaming(&effect_id);
                }
                let delegated_turn_id = turn_id
                    .ok_or_else(|| "delegated Agent event is missing turn id".to_string())?;
                match event {
                    nola_host_protocol::DelegatedAgentEventV1::SessionStarted {
                        backend_id,
                        backend_session_id,
                    } => self.dispatch(AgentCommand::DelegatedAgentSessionStarted {
                        turn_id: delegated_turn_id,
                        backend_id,
                        backend_session_id,
                    }),
                    nola_host_protocol::DelegatedAgentEventV1::TextDelta { part_id, delta } => self
                        .dispatch(AgentCommand::DelegatedAgentTextDelta {
                            turn_id: delegated_turn_id,
                            target_effect_id: effect_id,
                            item_id: ItemId::from_string(part_id),
                            delta,
                        }),
                    nola_host_protocol::DelegatedAgentEventV1::ReasoningDelta {
                        part_id,
                        delta,
                    } => self.dispatch(AgentCommand::ModelDelta {
                        turn_id: delegated_turn_id,
                        item_id: ItemId::from_string(part_id),
                        kind: AgentItemDeltaKind::ReasoningContent,
                        delta,
                        presentation_committed: false,
                    }),
                    nola_host_protocol::DelegatedAgentEventV1::ToolStarted {
                        call_id,
                        capability_id,
                        input,
                    } => self.dispatch(AgentCommand::DelegatedAgentToolStarted {
                        turn_id: delegated_turn_id,
                        target_effect_id: effect_id,
                        call_id: CallId::from_string(call_id),
                        capability_id,
                        input,
                    }),
                    nola_host_protocol::DelegatedAgentEventV1::ToolProgress {
                        call_id,
                        channel,
                        title,
                        detail,
                        delta,
                        metadata: _,
                    } => self.dispatch(AgentCommand::ToolCallProgress {
                        turn_id: delegated_turn_id,
                        call_id: CallId::from_string(call_id),
                        channel,
                        title,
                        detail,
                        delta,
                    }),
                    nola_host_protocol::DelegatedAgentEventV1::ToolCompleted {
                        call_id,
                        capability_id,
                        output,
                        outcome,
                        full_output_ref,
                        artifacts,
                    } => {
                        nola_host_protocol::tool_result::validate_completion(&output, outcome)?;
                        let output = nola_host_protocol::tool_result::core_tool_output(output)?;
                        self.dispatch(AgentCommand::DelegatedAgentToolCompleted {
                            turn_id: delegated_turn_id,
                            call_id: CallId::from_string(call_id),
                            capability_id,
                            output,
                            outcome,
                            full_output_ref,
                            artifacts,
                        })
                    }
                    nola_host_protocol::DelegatedAgentEventV1::PermissionRequested {
                        occurrence,
                        request_id,
                        call_id,
                        capability_id,
                        resource,
                        reason,
                        input,
                    } => self.dispatch(AgentCommand::DelegatedAgentPermissionRequested {
                        occurrence,
                        turn_id: delegated_turn_id,
                        target_effect_id: effect_id,
                        request_id: PermissionRequestId::from_string(request_id),
                        call_id: CallId::from_string(call_id),
                        capability_id,
                        resource,
                        reason,
                        input,
                    }),
                    nola_host_protocol::DelegatedAgentEventV1::Usage {
                        used_tokens,
                        context_window_tokens,
                        input_tokens,
                        output_tokens,
                    } => self.dispatch(AgentCommand::DelegatedAgentUsage {
                        turn_id: delegated_turn_id,
                        used_tokens,
                        context_window_tokens,
                        input_tokens,
                        output_tokens,
                    }),
                    nola_host_protocol::DelegatedAgentEventV1::Accounting { snapshot } => self.dispatch(AgentCommand::HostProgress {
                        turn_id: Some(delegated_turn_id), effect_id, title: "external_usage".into(),
                        detail: None, metadata: serde_json::to_value(snapshot).map_err(|error|error.to_string())?,
                    }),
                    nola_host_protocol::DelegatedAgentEventV1::Completed { items, usage } => self
                        .dispatch(AgentCommand::ModelCompleted {
                            turn_id: delegated_turn_id,
                            items: items
                                .into_iter()
                                .map(serde_json::from_value)
                                .collect::<Result<Vec<_>, _>>()
                                .map_err(|error| {
                                    format!("invalid delegated Agent item: {error}")
                                })?,
                            tool_calls: Vec::new(),
                            usage: serde_json::from_value(usage).map_err(|error| {
                                format!("invalid delegated Agent usage: {error}")
                            })?,
                            output_limited: false,
                            effect_elapsed_millis,
                        }),
                    nola_host_protocol::DelegatedAgentEventV1::Failed { error } => {
                        self.dispatch(AgentCommand::ModelFailed {
                            turn_id: delegated_turn_id,
                            error: format!("{}: {}", error.code, error.message),
                        })
                    }
                    nola_host_protocol::DelegatedAgentEventV1::CancellationConfirmed => self
                        .finish_host_cancellation(delegated_turn_id, "delegated Agent cancellation confirmed".into()),
                }
            }
            HostResponse::ModelDelta {
                channel,
                part_id,
                delta,
            } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                self.dispatch(AgentCommand::ModelDelta {
                    turn_id: turn_id.ok_or_else(|| "model delta is missing turn id".to_string())?,
                    item_id: ItemId::from_string(part_id),
                    kind: match channel {
                        ModelDeltaChannel::Text => AgentItemDeltaKind::AssistantText,
                        ModelDeltaChannel::Reasoning => AgentItemDeltaKind::ReasoningContent,
                    },
                    delta,
                    presentation_committed: false,
                })
            }
            HostResponse::ModelCommittedDelta { part_id, delta } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                self.dispatch(AgentCommand::ModelDelta {
                    turn_id: turn_id
                        .ok_or_else(|| "committed model delta is missing turn id".to_string())?,
                    item_id: ItemId::from_string(part_id),
                    kind: AgentItemDeltaKind::AssistantText,
                    delta,
                    presentation_committed: true,
                })
            }
            HostResponse::ModelToolCallDelta {
                part_id,
                call_id,
                capability_id,
                arguments_delta,
            } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                self.dispatch(AgentCommand::ModelToolCallDelta {
                    turn_id: turn_id
                        .ok_or_else(|| "model tool delta is missing turn id".to_string())?,
                    part_id,
                    call_id: CallId::from_string(call_id),
                    capability_id,
                    arguments_delta,
                })
            }
            HostResponse::ModelContinuation { continuation } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                if independent_review {
                    self.finish_transition(Vec::new(), Vec::new(), None)
                } else {
                    self.dispatch(AgentCommand::ModelContinuationUpdated {
                        turn_id: turn_id
                            .ok_or_else(|| "model continuation is missing turn id".to_string())?,
                        continuation,
                    })
                }
            }
            HostResponse::ModelUsageReported { .. } => {
                // Application commits this typed accounting fact to the exact
                // physical Attempt. It never changes context or output state.
                self.finish_transition(Vec::new(), Vec::new(), None)
            }
            HostResponse::ModelAttemptStarted {
                provider,
                model,
                reason,
            } => {
                let compaction = self
                    .session
                    .host
                    .pending_effects
                    .get(&effect_id)
                    .and_then(|entry| entry.request.clone())
                    .and_then(|value| serde_json::from_value::<HostRequestEnvelope>(value).ok())
                    .is_some_and(|request| matches!(request.payload, HostRequest::Compact { .. }));
                self.session.host.pending_effects.mark_streaming(&effect_id);
                if compaction {
                    self.dispatch(AgentCommand::HostProgress {
                        turn_id,
                        effect_id,
                        title: "model_attempt_started".into(),
                        detail: Some(format!("{provider} / {model}: {reason}")),
                        metadata: serde_json::Value::Null,
                    })
                } else {
                    self.dispatch(AgentCommand::ModelStreamRestarted {
                        turn_id: turn_id
                            .ok_or_else(|| "model attempt is missing turn id".to_string())?,
                    })
                }
            }
            HostResponse::ToolProgress {
                title,
                detail,
                metadata,
            } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                self.dispatch(AgentCommand::HostProgress {
                    turn_id,
                    effect_id,
                    title,
                    detail,
                    metadata,
                })
            }
            HostResponse::CodeModeToolRequested { request } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                self.dispatch(AgentCommand::CodeModeToolRequested {
                    turn_id: turn_id.ok_or("code cell callback has no Turn")?, request,
                })
            }
            HostResponse::CodeModeYielded { cell, output_cursor } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                let mut events = Vec::new();
                let mut effects = Vec::new();
                self.accept_code_cell_yield(&cell, output_cursor, &mut events, &mut effects)?;
                self.finish_transition(events, effects, Some(CheckpointReason::Manual))
            }
            HostResponse::CodeModeObservationSuspended { .. } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                // No ToolResult and no model wake-up: this is an internal
                // scheduling boundary, not the end of the observation budget.
                self.finish_transition(Vec::new(), Vec::new(), Some(CheckpointReason::Manual))
            }
            HostResponse::CodeModeOutput { frame } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                self.dispatch(AgentCommand::CodeModeOutput {
                    turn_id: turn_id.ok_or("code output has no Turn")?, effect_id, frame,
                })
            }
            HostResponse::LifecycleHookUpdated { projection } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                self.dispatch(AgentCommand::LifecycleHookUpdated {
                    turn_id,
                    projection,
                })
            }
            HostResponse::ToolCallProgress {
                call_id,
                channel,
                title,
                detail,
                delta,
                metadata: _,
            } => {
                self.session.host.pending_effects.mark_streaming(&effect_id);
                self.dispatch(AgentCommand::ToolCallProgress {
                    turn_id: turn_id
                        .ok_or_else(|| "tool progress is missing turn id".to_string())?,
                    call_id: CallId::from_string(call_id),
                    channel,
                    title,
                    detail,
                    delta,
                })
            }
            HostResponse::TaskPromoted {
                task_id,
                input_watermark,
                kind,
                state,
                progress,
                current_step,
                mut lineage,
            } => {
                let turn_id = turn_id
                    .ok_or_else(|| "Task promotion is missing its owning Turn".to_string())?;
                let call_id = pending
                    .call_id
                    .clone()
                    .ok_or_else(|| "Task promotion is not attached to a tool call".to_string())?;
                let original_request: HostRequestEnvelope = serde::Deserialize::deserialize(
                    self.session
                        .host
                        .pending_effects
                        .get(&effect_id)
                        .and_then(|effect| effect.request.as_ref())
                        .ok_or_else(|| "Task promotion has no original Host request".to_string())?,
                )
                .map_err(|error| format!("invalid promoted Host request: {error}"))?;
                if !matches!(original_request.payload, HostRequest::ExecuteTool { .. }) {
                    return Err("Only an executing tool may be promoted to a Task".to_string());
                }
                let call_id = CallId::from_string(call_id);
                if lineage
                    .parent_call_id
                    .as_ref()
                    .is_some_and(|parent_call_id| parent_call_id != &call_id)
                {
                    return Err("Task lineage does not match the promoted tool call".to_string());
                }
                lineage.parent_call_id = Some(call_id.clone());
                if !self
                    .session
                    .host
                    .pending_effects
                    .bind_task(&effect_id, &task_id)
                {
                    return Err("Host effect is already promoted or Task id is invalid".to_string());
                }
                let evidence = nola_domain::execution_evidence::ExecutionEvidenceObservation {
                    evidence_id: self.session.allocate_id("execution_evidence"),
                    call_id: call_id.as_str().to_owned(),
                    tool_name: pending.capability_id.clone(),
                    source: nola_domain::execution_evidence::ExecutionEvidenceSource {
                        effect_id: response.effect_id.0,
                        request_id: response.request_id,
                        host_id: response.host_id.0,
                        attempt_id: response.attempt_id,
                        response_sequence: nola_domain::assistant_journal_operations::JournalNumber(response.sequence),
                        catalog_revision: nola_domain::assistant_journal_operations::JournalNumber(response.catalog_revision),
                        availability_epoch: nola_domain::assistant_journal_operations::JournalNumber(response.availability_epoch),
                    },
                    // The foreground handoff happened; child execution and
                    // verification have not. Never manufacture a receipt.
                    receipt: None,
                    outcome: nola_domain::host_effect::HostToolOutcome::Deferred,
                };
                evidence.validate()?;
                let transition = self.dispatch_command(AgentCommand::TaskPromoted {
                    effect_id: effect_id.clone(),
                    call_id,
                    task: TaskHandle {
                        id: task_id,
                        owner_session_id: self.session.session_id.clone(),
                        owner_turn_id: turn_id,
                        kind,
                        state,
                        progress,
                        current_step,
                        result_ref: None,
                        cancel_requested: false,
                        cancel_requested_at_epoch_ms: None,
                        lineage,
                        durability: super::super::task::TaskDurability { input_watermark, ..Default::default() },
                    },
                }, Some(evidence))?;
                // Promotion transfers execution ownership to the durable Task.
                // The foreground effect is terminal even while that Task runs;
                // keep its completed delivery identity for duplicate rejection.
                self.session.host.pending_effects.complete(&effect_id)
                    .ok_or("promoted foreground Host effect disappeared before handoff")?;
                Ok(transition)
            }
            HostResponse::TaskUpdated {
                task_id,
                state,
                progress,
                current_step,
                result_ref,
                cancel_requested,
                ownership_token,
                completion_watermark,
            } => {
                if pending.task_id.as_deref() != Some(task_id.as_str()) {
                    return Err("Task update does not match the promoted Host effect".to_string());
                }
                let task = self.session.task_projection.next_handle_attested(
                    &task_id,
                    state,
                    progress,
                    current_step,
                    result_ref,
                    cancel_requested,
                    ownership_token.as_deref(),
                    completion_watermark,
                )?;
                if state.is_terminal() {
                    self.session.host.pending_effects.complete_task(&task_id);
                } else {
                    self.session.host.pending_effects.mark_streaming(&effect_id);
                }
                self.dispatch(AgentCommand::TaskUpdated { task })
            }
            HostResponse::DelegatedPermissionClaimed { delivery_id }
            | HostResponse::DelegatedPermissionInterrupted { delivery_id } => {
                let mut transition = self.dispatch(AgentCommand::HostProgress {
                    turn_id,
                    effect_id,
                    title: "delegated_permission_delivery".into(),
                    detail: Some(delivery_id),
                    metadata: serde_json::Value::Null,
                })?;
                transition.durability = Some(AgentDurabilityIntent {
                    checkpoint_reason: CheckpointReason::PermissionTransportInterrupted,
                });
                Ok(transition)
            }
            HostResponse::DelegatedPermissionTransportWritten { delivery_id } => {
                self.session.host.pending_effects.complete(&effect_id);
                self.dispatch(AgentCommand::HostProgress {
                    turn_id,
                    effect_id,
                    title: "delegated_permission_transport_written".into(),
                    detail: Some(delivery_id),
                    metadata: serde_json::Value::Null,
                })
            }
            HostResponse::DelegatedExecutionInterrupted { reason } => {
                self.session
                    .host
                    .pending_effects
                    .mark_delegated_interrupted(&effect_id, reason);
                let mut transition = self.dispatch(AgentCommand::HostProgress {
                    turn_id, effect_id, title: "delegated_execution_interrupted".into(),
                    detail: Some("The admitted external operation has no attached worker; recovery evidence is required before execution can continue.".into()),
                    metadata: serde_json::Value::Null,
                })?;
                transition.durability = Some(AgentDurabilityIntent {
                    checkpoint_reason: CheckpointReason::DelegatedExecutionInterrupted,
                });
                Ok(transition)
            }
            HostResponse::PermissionTransportInterrupted { reason } => {
                self.session
                    .host
                    .pending_effects
                    .mark_permission_interrupted(&effect_id, reason);
                let mut transition = self.dispatch(AgentCommand::HostProgress {
                    turn_id,
                    effect_id,
                    title: "permission_transport_interrupted".to_string(),
                    detail: Some(match reason {
                        nola_host_protocol::PermissionTransportInterruptionReason::Timeout => "timeout",
                        nola_host_protocol::PermissionTransportInterruptionReason::Disconnected => "disconnected",
                        nola_host_protocol::PermissionTransportInterruptionReason::Unavailable => "unavailable",
                    }.to_string()),
                    metadata: serde_json::Value::Null,
                })?;
                transition.durability = Some(AgentDurabilityIntent {
                    checkpoint_reason: CheckpointReason::PermissionTransportInterrupted,
                });
                Ok(transition)
            }
            HostResponse::PermissionResolved {
                request_id,
                resolution,
            } => {
                self.session.host.pending_effects.complete(&effect_id);
                match resolution {
                    HostPermissionResolution::Intervention {
                        intervention: HostPermissionIntervention::ReturnToComposer,
                    } => self.dispatch(AgentCommand::HostCancellationConfirmed {
                        fact:nola_domain::core_requirements::CoreRuntimeDecisionFactKind::HostCancellation {
                            cancellation_id:format!("host_response:{}:{}",response.request_id,response.sequence),
                            request_id:response.request_id.clone(),effect_id,host_id:response.host_id.0.clone(),
                            response_sequence:nola_domain::assistant_journal_operations::JournalNumber(response.sequence),
                        }
                    }),
                    HostPermissionResolution::Intervention {
                        intervention: HostPermissionIntervention::ProvideInstruction { instruction },
                    } => {
                        let instruction = instruction.trim().to_string();
                        if instruction.is_empty() {
                            return Err("permission instruction is empty".to_string());
                        }
                        self.dispatch(AgentCommand::PermissionIntervened {
                            request_id: PermissionRequestId::from_string(request_id),
                            intervention: PermissionUserIntervention::ProvideInstruction {
                                instruction,
                            },
                        })
                    }
                    HostPermissionResolution::Authorization { decision, grant } => {
                        self.dispatch(AgentCommand::PermissionResolved {
                            request_id: PermissionRequestId::from_string(request_id),
                            decision: match decision {
                                HostPermissionAuthorizationDecision::AllowConversation => {
                                    PermissionUserDecision::AllowConversation
                                }
                                HostPermissionAuthorizationDecision::AllowOnce => {
                                    PermissionUserDecision::AllowOnce
                                }
                                HostPermissionAuthorizationDecision::Deny => {
                                    PermissionUserDecision::Deny
                                }
                            },
                            grant,
                        })
                    }
                }
            }
            HostResponse::ModelCompleted {
                items,
                tool_calls,
                usage,
            } => {
                self.session.host.pending_effects.complete(&effect_id);
                self.dispatch(AgentCommand::ModelCompleted {
                    turn_id: turn_id
                        .ok_or_else(|| "model completion is missing turn id".to_string())?,
                    items: completed_model_items(items, independent_review)?,
                    tool_calls: tool_calls
                        .into_iter()
                        .map(serde_json::from_value)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| format!("invalid model tool call: {error}"))?,
                    usage: serde_json::from_value(usage)
                        .map_err(|error| format!("invalid model usage: {error}"))?,
                    output_limited: false,
                    effect_elapsed_millis,
                })
            }
            HostResponse::ModelOutputLimited {
                items,
                tool_calls,
                usage,
            } => {
                self.session.host.pending_effects.complete(&effect_id);
                self.dispatch(AgentCommand::ModelCompleted {
                    turn_id: turn_id
                        .ok_or_else(|| "limited model completion is missing turn id".to_string())?,
                    items: completed_model_items(items, independent_review)?,
                    tool_calls: tool_calls
                        .into_iter()
                        .map(serde_json::from_value)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| format!("invalid limited model tool call: {error}"))?,
                    usage: serde_json::from_value(usage)
                        .map_err(|error| format!("invalid limited model usage: {error}"))?,
                    output_limited: true,
                    effect_elapsed_millis,
                })
            }
            HostResponse::ToolCompleted {
                call_id,
                output,
                outcome,
                activation,
                full_output_ref,
                artifacts,
            } => {
                nola_host_protocol::tool_result::validate_completion(&output, outcome)?;
                let output = nola_host_protocol::tool_result::core_tool_output(output)?;
                let evidence = nola_domain::execution_evidence::ExecutionEvidenceObservation {
                    evidence_id: self.session.allocate_id("execution_evidence"),
                    call_id: call_id.clone(),
                    tool_name: pending.capability_id.clone(),
                    source: nola_domain::execution_evidence::ExecutionEvidenceSource {
                        effect_id: response.effect_id.0,
                        request_id: response.request_id,
                        host_id: response.host_id.0,
                        attempt_id: response.attempt_id,
                        response_sequence: nola_domain::assistant_journal_operations::JournalNumber(response.sequence),
                        catalog_revision: nola_domain::assistant_journal_operations::JournalNumber(response.catalog_revision),
                        availability_epoch: nola_domain::assistant_journal_operations::JournalNumber(response.availability_epoch),
                    },
                    receipt: output.get("_nola_execution").cloned()
                        .map(serde_json::from_value).transpose()
                        .map_err(|error| format!("invalid normalized execution receipt: {error}"))?,
                    outcome,
                };
                self.session.host.pending_effects.complete(&effect_id);
                let capability_id = pending.capability_id.clone();
                let skill_invocation = if !outcome.is_error()
                    && matches!(capability_id.as_str(), "Skill" | "skill_invoke")
                {
                    if output.get("type").and_then(Value::as_str) != Some("invoked_skill") {
                        return Err(
                            "Skill Host response is missing the invoked_skill protocol payload"
                                .to_string(),
                        );
                    }
                    Some(
                        serde_json::from_value::<MaterializedSkillInvocation>(output.clone())
                            .map_err(|error| {
                                format!("invalid materialized Skill invocation: {error}")
                            })?,
                    )
                } else {
                    None
                };
                let skill_runtime_update = if !outcome.is_error() && capability_id == "skill_update"
                {
                    if output.get("type").and_then(Value::as_str) != Some("skill_runtime_update") {
                        return Err(
                            "skill_update Host response is missing the skill_runtime_update protocol payload"
                                .to_string(),
                        );
                    }
                    let mut update_payload = output.clone();
                    if let Value::Object(object) = &mut update_payload {
                        object.retain(|key, _| {
                            matches!(
                                key.as_str(),
                                "run_id"
                                    | "action"
                                    | "step_id"
                                    | "completed"
                                    | "artifact_id"
                                    | "artifact_ref"
                                    | "verifier_id"
                                    | "passed"
                                    | "evidence"
                                    | "replaces_evidence"
                                    | "summary"
                                    | "resume_point"
                            )
                        });
                    }
                    Some(
                        serde_json::from_value::<MaterializedSkillRuntimeUpdate>(update_payload)
                            .map_err(|error| {
                                format!("invalid materialized Skill runtime update: {error}")
                            })?,
                    )
                } else {
                    None
                };
                self.dispatch(AgentCommand::ToolCompleted {
                    turn_id: turn_id
                        .ok_or_else(|| "tool completion is missing turn id".to_string())?,
                    call_id: CallId::from_string(call_id),
                    capability_id,
                    evidence: Some(evidence),
                    output,
                    outcome,
                    activation,
                    full_output_ref,
                    artifacts,
                    skill_invocation,
                    skill_runtime_update,
                    effect_elapsed_millis,
                })
            }
            HostResponse::CompactionCompleted { summary, usage } => {
                self.session.host.pending_effects.complete(&effect_id);
                self.dispatch(AgentCommand::CompactionCompleted {
                    summary,
                    usage: serde_json::from_value(usage)
                        .map_err(|error| format!("invalid compaction usage: {error}"))?,
                })
            }
            HostResponse::Completed {
                output,
                usage: _,
                full_output_ref: _,
                artifacts: _,
            } => {
                self.session.host.pending_effects.complete(&effect_id);
                if let Some(reduction) = self.finish_quiescent_cancellation()? { return Ok(reduction); }
                if pending.capability_id == "code_mode.poll" {
                    let mut events = Vec::new();
                    let mut effects = Vec::new();
                    self.advance_deferred_code_completion(&mut events, &mut effects)?;
                    self.finish_transition(events, effects, Some(CheckpointReason::Manual))
                } else if pending.capability_id == "lifecycle.session_end" {
                    self.dispatch(AgentCommand::SessionClosed)
                } else {
                    self.dispatch(AgentCommand::HostProgress {
                        turn_id,
                        effect_id,
                        title: "completed".to_string(),
                        detail: Some(output.to_string()),
                        metadata: serde_json::Value::Null,
                    })
                }
            }
            HostResponse::ContextFetched { components } => {
                self.session.host.pending_effects.complete(&effect_id);
                self.dispatch(AgentCommand::HostProgress {
                    turn_id,
                    effect_id,
                    title: "context_fetched".to_string(),
                    detail: Some(format!("{} components", components.len())),
                    metadata: serde_json::Value::Null,
                })
            }
            HostResponse::CapabilityCatalogRefreshed { catalog } => {
                let refreshed = catalog.host_catalog;
                let current = &self.session.host.catalog;
                if refreshed.host_id != current.host_id
                    || refreshed.revision < current.revision
                    || refreshed.availability_epoch < current.availability_epoch
                    || (refreshed.revision == current.revision && refreshed != *current)
                {
                    return Err("invalid Host catalog refresh identity or revision".into());
                }
                self.session.host.pending_effects.complete(&effect_id);
                self.config.host_catalog = Some(refreshed.clone());
                self.session.host.catalog = refreshed.clone();
                if let Some(turn) = self.session.execution.active_turn.as_mut() {
                    let previous = turn.context.capability_projection.clone();
                    turn.context.capability_projection.refresh_host_catalog(&refreshed);
                    super::model_planner::rebase_host_tool_working_set(&previous,&turn.context.capability_projection,
                        &mut turn.model_host_tool_working_set);
                }
                let mut events = vec![RuntimeEvent::HostCatalogUpdated {
                    host_id: refreshed.host_id.0, revision: refreshed.revision,
                    availability_epoch: refreshed.availability_epoch,
                }];
                let mut effects = Vec::new();
                self.apply_skill_runtime_command(SkillRuntimeCommandEnvelope {
                        operation_id: format!("host:{effect_id}:capability_catalog"),
                        command: SkillRuntimeCommand::ReplaceCatalog { catalog: catalog.catalog },
                    }, &mut events, &mut effects)?;
                self.finish_transition(events, effects, None)
            }
            HostResponse::Failed { error } => {
                self.session.host.pending_effects.complete(&effect_id);
                if task_cancellation {
                    // A failed stop-intent delivery is not child termination.
                    // Commit the failure so its attempt settles and later work
                    // can proceed; preserve the task's exact terminal authority.
                    return self.finish_transition(vec![RuntimeEvent::HostProgress {
                        turn_id, effect_id, title:"task_cancellation_failed".into(),
                        detail:Some(error.message), metadata:json!({"task_id":pending.task_id,
                            "code":error.code,"retryable":error.retryable,"cancellation_confirmed":false}),
                    }], Vec::new(), Some(CheckpointReason::Manual));
                }
                if let Some(cell) = controlled_cell {
                    // A result-delivery failure has the same uncertainty as
                    // a failed poll: neither proves that its isolate stopped.
                    self.session.execution.active_turn.as_mut().ok_or("code poll failure has no Turn")?
                        .code_observation_failures.insert(cell.cell_id.clone());
                    // Observation failure is not evidence that the original
                    // worker stopped. Commit it once and retain its execution
                    // journal for reconciliation; never replay source or spin
                    // through failed observer effects automatically.
                    return self.finish_transition(vec![RuntimeEvent::HostProgress {
                        turn_id, effect_id, title:"code_cell_observation_failed".into(),
                        detail:Some(error.message),
                        metadata:json!({"code":error.code,"cell_id":cell.cell_id,
                            "control":pending.capability_id,"requires_execution_reconciliation":true,
                            "source_replay_allowed":false}),
                    }], Vec::new(), Some(CheckpointReason::Manual));
                } else if pending.capability_id == "permission.ask"
                    && !(error.code == "cancelled" && error.message == "permission_custom_input") {
                    // Failure of the approval request is not completion of the
                    // guarded tool: it has not crossed the execution boundary.
                    // Keep the Host diagnostic without inventing a user decision.
                    let turn_id = turn_id.ok_or("permission request failure has no Turn")?;
                    let call_id = CallId::from_string(pending.call_id.ok_or("permission request failure has no call")?);
                    let request_id = self.session.execution.active_turn.as_mut()
                        .filter(|turn| turn.turn_id == turn_id)
                        .and_then(|turn| turn.tools.iter_mut().find(|tool| tool.call_id == call_id))
                        .filter(|tool| tool.status == ToolCallStatus::AwaitingPermission)
                        .and_then(|tool| tool.permission_request_id.take())
                        .ok_or("permission request failure does not match a pending tool permission")?;
                    self.session.execution.permissions.remove(&request_id);
                    let mut events = Vec::new();
                    let mut effects = Vec::new();
                    self.complete_tool(&turn_id, &call_id, None, json!({
                        "error": error.message, "code": error.code, "retryable": error.retryable,
                        "tool_execution_skipped": true,
                        "_nola_execution": {"state":"not_started", "effects":[], "checks":[]},
                        "next_action": "The permission request failed before tool execution. Diagnose the reported Host error before retrying the request. No user denial or authorization grant was recorded; any retry still requires normal permission checks."
                    }), nola_host_protocol::HostToolOutcome::Failed, None, None,
                        Vec::new(), None, true, &mut events, &mut effects)?;
                    return self.finish_transition(events, effects, None);
                }
                if error.code == "cancelled" && error.message == "permission_custom_input" {
                    return Err(
                        "permission custom input must use the structured permission decision protocol"
                            .to_string(),
                    );
                }
                if pending.capability_id == "model.stream" {
                    let turn_id =
                        turn_id.ok_or_else(|| "model failure is missing turn id".to_string())?;
                    if error.code == "context_length_exceeded" {
                        self.dispatch(AgentCommand::RecoverContextLimit {
                            turn_id,
                            error: error.message,
                        })
                    } else if error.code == "cancelled" {
                        self.finish_host_cancellation(turn_id,error.message)
                    } else {
                        self.dispatch(AgentCommand::ModelFailed {
                            turn_id,
                            error: error.message,
                        })
                    }
                } else if pending.capability_id == "context.compact" {
                    if error.code == "cancelled" && self.session.execution.requirements.cancellation_id.is_some() {
                        self.finish_host_cancellation(turn_id.ok_or("compaction cancellation is missing Turn")?,error.message)
                    } else {
                        self.dispatch(AgentCommand::CompactionFailed { error: error.message })
                    }
                } else if pending.capability_id == "capability.catalog.refresh" {
                    self.dispatch(AgentCommand::HostProgress {
                        turn_id,
                        effect_id,
                        title: "capability_refresh_failed".to_string(),
                        detail: Some(error.message),
                        metadata: serde_json::Value::Null,
                    })
                } else if let Some(call_id) = pending.call_id {
                    let (recovery, next_action) = match (
                        error.retryable,
                        pending.replay_safety,
                    ) {
                        (true, HostReplaySafety::Idempotent) => (
                            "retry_idempotent",
                            "Retry this tool once with the same validated arguments. If it fails again, inspect the new error and choose a materially different approach.",
                        ),
                        (true, HostReplaySafety::Recoverable) => (
                            "inspect_then_retry",
                            "Inspect the affected state before retrying. The previous attempt may have partially completed, so reconcile the current state first.",
                        ),
                        (true, HostReplaySafety::RequiresRecoveryDecision) => (
                            "recovery_decision_required",
                            "Do not automatically repeat this tool call. Inspect the affected state or request a recovery decision because the previous attempt may have produced side effects.",
                        ),
                        (false, _) => (
                            "correct_or_alternate",
                            "Do not repeat the unchanged call. Correct the reported root cause or choose a different supported capability.",
                        ),
                    };
                    self.dispatch(AgentCommand::ToolCompleted {
                        turn_id: turn_id
                            .ok_or_else(|| "tool failure is missing turn id".to_string())?,
                        call_id: CallId::from_string(call_id),
                        capability_id: pending.capability_id,
                        evidence: None,
                        output: json!({
                            "error": error.message,
                            "code": error.code,
                            "retryable": error.retryable,
                            "replay_safety": pending.replay_safety,
                            "recovery": recovery,
                            "next_action": next_action,
                        }),
                        outcome: nola_host_protocol::HostToolOutcome::Failed,
                        activation: None,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                        skill_invocation: None,
                        skill_runtime_update: None,
                        effect_elapsed_millis,
                    })
                } else {
                    Err(error.message)
                }
            }
            HostResponse::CancellationConfirmed { target_effect_id } => {
                let acceptance = self
                    .session
                    .host
                    .pending_effects
                    .acknowledge_cancellation(&effect_id, &target_effect_id.0);
                if acceptance != HostEffectAcceptance::Accepted {
                    return Err(format!(
                        "rejected Host cancellation acknowledgement: {acceptance:?}"
                    ));
                }
                // The journal authenticates the receipt first. Only then may
                // its exact target discharge a cell's durable stop obligation.
                if let Some(turn) = self.session.execution.active_turn.as_mut() {
                    for cell in turn.code_cells.values_mut() {
                        cell.acknowledge_cancelled_effect(&target_effect_id.0);
                    }
                }
                if let Some(reduction) = self.finish_quiescent_cancellation()? { return Ok(reduction); }
                let mut events = Vec::new();
                let mut effects = Vec::new();
                self.settle_confirmed_code_stops(&mut events, &mut effects)?;
                self.advance_deferred_code_completion(&mut events, &mut effects)?;
                self.finish_transition(events, effects, Some(CheckpointReason::CancellationProgress))
            }
        }
    }
}

#[cfg(test)]
mod envelope_tests {
    #[test]
    fn dispatch_metadata_reads_review_mode_without_materializing_request_bodies() {
        use crate::agent::host::ModelResponseMode;
        use nola_domain::host_control::HostResponsePolicy;
        for mode in [
            ModelResponseMode::Conversational,
            ModelResponseMode::StructuredAudit,
        ] {
            let payload: crate::agent::host::HostRequest =
                crate::agent::host::HostRequest::ModelStream {
                    model_id: "model".into(),
                    text_part_id: "text".into(),
                    reasoning_part_id: "reasoning".into(),
                    response_mode: mode,
                    messages: vec![serde_json::json!({"content": "大上下文".repeat(8192)})],
                    tools: Vec::new(),
                    settings: serde_json::Value::Null,
                };
            let policy = HostResponsePolicy::from_request(&payload);
            assert!(
                matches!(policy,HostResponsePolicy::ModelStream {response_mode} if response_mode==mode)
            );
            assert!(serde_json::to_vec(&policy).unwrap().len() < 256);
        }
    }

    use super::*;
    use crate::agent::parts::PartOwnership;

    fn runtime() -> AgentRuntime {
        AgentRuntime::try_new(AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("envelope-session".to_string())),
            ..AgentCoreConfig::default()
        })
        .unwrap()
    }

    #[test]
    fn owned_envelopes_preserve_large_event_bodies_and_all_wire_fields() {
        let mut runtime = runtime();
        runtime.session.journal.next_event_sequence = 41;
        runtime.session.journal.revision = 7;
        let turn_id = TurnId::from_string("turn-1".to_string());
        let initial_body = "长正文🙂\\path\n".repeat(131_072);
        assert!(initial_body.len() >= 2 * 1024 * 1024);
        let delta = "追加🙂".repeat(8_192);
        let created_part = AgentPart::Text(TextPart {
            review: None,
            id: "text-1".to_string(),
            ownership: PartOwnership::for_turn("turn-1"),
            text: initial_body.clone(),
            is_complete: false,
        });
        let created_snapshot = created_part.clone();
        let delta_snapshot = AgentPart::Text(TextPart {
            review: None,
            id: "text-1".to_string(),
            ownership: PartOwnership::for_turn("turn-1"),
            text: initial_body + &delta,
            is_complete: false,
        });
        let text_pointer = |part: &AgentPart| match part {
            AgentPart::Text(part) => part.text.as_ptr(),
            _ => panic!("fixture must contain a Text Part"),
        };
        let created_pointer = text_pointer(&created_part);
        let created_snapshot_pointer = text_pointer(&created_snapshot);
        let delta_snapshot_pointer = text_pointer(&delta_snapshot);
        let delta_pointer = delta.as_ptr();
        let events = vec![
            AgentEvent::SessionStarted {
                session_id: SessionId::from_string("envelope-session".to_string()),
            },
            AgentEvent::PartUpdated {
                turn_id: turn_id.clone(),
                projection_revision: 11,
                update: PartUpdate::Created { part: created_part },
                part: created_snapshot,
            },
            AgentEvent::PartUpdated {
                turn_id,
                projection_revision: 12,
                update: PartUpdate::TextDelta {
                    part_id: "text-1".to_string(),
                    delta,
                },
                part: delta_snapshot,
            },
            AgentEvent::HostProgress {
                turn_id: Some(TurnId::from_string("turn-2".to_string())),
                effect_id: "effect-2".to_string(),
                title: "读取文件".to_string(),
                detail: Some("进度🙂".to_string()),
                metadata: serde_json::json!({"ordinal": i64::MAX, "status": "running"}),
            },
        ];
        // The reference retains the previous borrowed-envelope wire behavior.
        let expected = events
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, event)| CoreEventEnvelope {
                session_id: "envelope-session".to_string(),
                turn_id: match index {
                    0 => None,
                    1 | 2 => Some("turn-1".to_string()),
                    3 => Some("turn-2".to_string()),
                    _ => unreachable!(),
                },
                cursor: EventCursor {
                    sequence: 41 + index as u64,
                },
                revision: 7,
                event,
            })
            .collect::<Vec<_>>();

        let actual = runtime.envelope_events(events);
        assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert_eq!(runtime.session.journal.next_event_sequence, 45);
        assert_eq!(runtime.session.journal.revision, 7);
        let AgentEvent::PartUpdated {
            update: PartUpdate::Created { part: created },
            part: created_snapshot,
            ..
        } = &actual[1].event
        else {
            panic!("Created event must retain both original Part values")
        };
        assert_eq!(text_pointer(created), created_pointer);
        assert_eq!(text_pointer(created_snapshot), created_snapshot_pointer);
        let AgentEvent::PartUpdated {
            update: PartUpdate::TextDelta { delta, .. },
            part: delta_snapshot,
            ..
        } = &actual[2].event
        else {
            panic!("Delta event must retain its original delta and resulting Part")
        };
        assert_eq!(delta.as_ptr(), delta_pointer);
        assert_eq!(text_pointer(delta_snapshot), delta_snapshot_pointer);
    }

    #[test]
    fn empty_envelope_batch_does_not_advance_cursor_or_revision() {
        let mut runtime = runtime();
        runtime.session.journal.next_event_sequence = 41;
        runtime.session.journal.revision = 7;
        assert!(runtime.envelope_events(Vec::new()).is_empty());
        assert_eq!(runtime.session.journal.next_event_sequence, 41);
        assert_eq!(runtime.session.journal.revision, 7);
    }
}
