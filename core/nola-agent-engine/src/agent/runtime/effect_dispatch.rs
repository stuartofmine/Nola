// SPDX-License-Identifier: Apache-2.0
// Internal effect delivery and replay layer. Public clients use AgentCore.

use super::*;

impl AgentRuntime {
    pub(crate) fn restore_committed_model_input(&mut self, request: &HostRequestEnvelope) -> Result<(), String> {
        if request.session_id != self.session.session_id.as_str() {
            return Err("frozen model input belongs to another session".into());
        }
        self.session.host.pending_effects.restore_committed_model_input(request)
    }

    pub(crate) fn bind_committed_host_effect(
        &mut self,
        request: &HostRequestEnvelope,
    ) -> Result<(), String> {
        let control = nola_domain::host_control::HostEffectControl::from_request(request)?;
        self.bind_committed_host_control(&control)
    }

    pub(crate) fn bind_committed_host_response_control(&mut self, control: &nola_domain::host_control::HostEffectControl) -> Result<(), String> {
        if control.session_id != self.session.session_id.as_str() {
            return Err("committed Host response belongs to another session".into());
        }
        self.session.host.pending_effects.bind_response_control(control)
    }

    pub(crate) fn bind_committed_host_control(
        &mut self,
        control: &nola_domain::host_control::HostEffectControl,
    ) -> Result<(), String> {
        if control.session_id != self.session.session_id.as_str() {
            return Err("committed Host effect belongs to another session".into());
        }
        self.session.host.pending_effects.bind_control(control)
    }

    pub(crate) fn resume_committed_host_effect(
        &mut self,
        request: &HostRequestEnvelope,
        observed_at_ms: u64,
    ) -> Result<AgentReduction, String> {
        // This borrowed entry point does not use reduce's candidate rollback.
        // Reject an unsupported trace clock before rebinding or allocating IDs.
        Self::observation_time(observed_at_ms)?;
        self.observed_at_ms = observed_at_ms;
        self.bind_committed_host_effect(request)?;
        self.resume_host_effect(&request.effect_id.0)
    }

    pub(super) fn effect_can_replay_automatically(&self, effect: &PendingHostEffect) -> bool {
        if let Ok(control) = effect.validated_control() {
            if let nola_domain::host_control::HostResponsePolicy::ResolveCodeTool { cell, .. }
                | nola_domain::host_control::HostResponsePolicy::PollCodeCell { cell } = &control.payload.policy {
                // Only redeliver a receipt to its original isolate. This never
                // reissues the nested operation or starts a replacement cell.
                return effect.replay_safety == HostReplaySafety::Idempotent
                    && cell.host_id == self.session.host.catalog.host_id.0
                    && cell.conversation_id == self.session.session_id.as_str();
            }
        }
        let resolution = effect.validated_control().ok().is_some_and(|control|
            matches!(control.payload.policy,nola_domain::host_control::HostResponsePolicy::ResolvePermissionResources {..}));
        replay_safety_allows_automatic_delivery(effect.replay_safety)
            && self
                .session
                .host
                .catalog
                .capabilities
                .get(&effect.capability_id)
                .is_some_and(|capability| {
                    capability.availability == CapabilityAvailability::Available
                        && (resolution || capability.replay_safety == effect.replay_safety)
                })
    }

    pub(super) fn resume_host_effect(&mut self, effect_id: &str) -> Result<AgentReduction, String> {
        let effect = self
            .session
            .host
            .pending_effects
            .get(effect_id)
            .cloned()
            .ok_or_else(|| "Host effect is no longer pending".to_string())?;
        let capability_title = if matches!(effect.validated_control().ok().map(|control| &control.payload.policy),
            Some(nola_domain::host_control::HostResponsePolicy::ResolveCodeTool { .. }
                | nola_domain::host_control::HostResponsePolicy::PollCodeCell { .. })) {
            "Code cell control".to_string()
        } else if effect.capability_id == "lifecycle.session_end" {
            "Session lifecycle".to_string()
        } else {
            self.session
                .host
                .catalog
                .resolve_available(&effect.capability_id)
                .ok_or_else(|| "Host capability is unavailable for recovery".to_string())?
                .title
                .clone()
        };
        let mut request = self.validated_pending_request(&effect)?;
        if matches!(&request.payload, HostRequest::StartCodeCell { .. }) {
            return Err("code cell source cannot be replayed; reconcile its committed child effects before starting a new cell".into());
        }
        let catalog = self.session.host.catalog.clone();
        request.request_id = self.session.allocate_id("host_request");
        request.host_id = catalog.host_id.clone();
        request.lease_id = catalog.lease.as_ref().map(|lease| lease.lease_id.clone());
        request.catalog_revision = catalog.revision;
        request.availability_epoch = catalog.availability_epoch;
        request.resume_after_sequence = effect.last_response_sequence;
        request.created_at_ms = self.observed_at_ms;
        request.deadline_ms = host_request_deadline_ms(&request.payload, request.created_at_ms);
        let request_value = serde_json::to_value(&request).map_err(|error| error.to_string())?;
        self.session.host.pending_effects.rebind_for_replay(
            &effect.effect_id,
            request.request_id.clone(),
            request.host_id.0.clone(),
            request.lease_id.clone(),
            request.catalog_revision,
            request.availability_epoch,
            request_value,
        ).map_err(|error| format!("Host effect {} ({}) cannot be rebound for recovery: {error}",
            effect.effect_id, effect.capability_id))?;
        let mut events = vec![RuntimeEvent::HostProgress {
            turn_id: effect.turn_id.clone().map(TurnId::from_string),
            effect_id: effect.effect_id,
            title: "recovery_started".to_string(),
            detail: Some(capability_title),
            metadata: serde_json::Value::Null,
        }];
        if let HostRequest::RequestPermission {
            permission_request_id,
            input,
            ..
        } = &request.payload
        {
            let pending = self
                .session
                .execution
                .permissions
                .pending
                .iter()
                .find(|pending| pending.request_id.as_str() == permission_request_id)
                .ok_or_else(|| "recovered permission effect has no pending ask".to_string())?;
            // Re-publish the committed waiting Part under a new event cursor.
            // Re-applying an identical permission mutation is correctly a
            // projection no-op and would not produce a fresh delivery trace.
            let part = self.session.conversation.projection.parts.values().find(|part| {
                matches!(part, AgentPart::Tool(tool) if tool.call_id == pending.call_id.as_str()
                    && tool.permission.as_ref().is_some_and(|permission| permission.request_id == *permission_request_id))
            }).cloned().ok_or_else(|| "recovered permission has no canonical waiting Part".to_string())?;
            let AgentPart::Tool(tool) = &part else {
                unreachable!()
            };
            if tool.input != *input || tool.state != ToolPartState::WaitingForPermission {
                return Err("recovered permission Part differs from the committed ask".into());
            }
            events.push(RuntimeEvent::PartUpdated {
                turn_id: pending.turn_id.clone(),
                projection_revision: self.session.conversation.projection.revision,
                update: PartUpdate::ToolPermission {
                    part_id: tool.id.clone(),
                    permission: tool.permission.clone().expect("matched permission"),
                },
                part,
            });
        }
        let mut transition = self.finish_transition(events, Vec::new(), None)?;
        transition.host_requests.insert(0, request);
        Ok(transition)
    }

    pub(super) fn wrap_host_requests(
        &mut self,
        effects: &[AgentEffect],
    ) -> Result<Vec<HostRequestEnvelope>, String> {
        let mut requests = effects
            .iter()
            .map(|effect| self.wrap_host_request(effect))
            .collect::<Result<Vec<_>, _>>()?;
        nola_host_protocol::prioritize_code_cell_controls(&mut requests);
        self.annotate_concurrent_tool_batches(&mut requests)?;
        Ok(requests)
    }

    fn annotate_concurrent_tool_batches(&mut self, requests: &mut [HostRequestEnvelope]) -> Result<(), String> {
        // Most model deltas emit no Host request. A batch needs at least two
        // requests; do not clone the frozen tool catalog on every streamed
        // token (or every token replayed while restoring a conversation).
        if requests.len() < 2 {
            return Ok(());
        }
        let catalog = self
            .session
            .execution
            .active_turn
            .as_ref()
            .map(|turn| turn.context.capability_projection.frozen_host_catalog())
            .unwrap_or_else(|| self.session.host.catalog.clone());
        let mut start = 0;
        while start < requests.len() {
            if !request_is_concurrent_safe_tool(&requests[start], &catalog) {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < requests.len() && request_is_concurrent_safe_tool(&requests[end], &catalog)
            {
                end += 1;
            }
            let size = end - start;
            if size > 1 {
                let batch_id = self.session.allocate_id("host_batch");
                for (index, request) in requests[start..end].iter_mut().enumerate() {
                    request.batch = Some(crate::agent::host::HostRequestBatchIdentity {
                        batch_id: batch_id.clone(),
                        index,
                        size,
                    });
                    // Registration precedes batch planning. Retain the exact same
                    // batch in Core's bounded control before Application commits it.
                    self.bind_committed_host_effect(request)?;
                }
            }
            start = end;
        }
        Ok(())
    }

    pub(super) fn wrap_host_request(
        &mut self,
        effect: &AgentEffect,
    ) -> Result<HostRequestEnvelope, String> {
        let effect_id = HostEffectId(self.session.allocate_id("host_effect"));
        let session_id = self.session.session_id.as_str().to_string();
        let frozen_turn_id = match effect {
            AgentEffect::PollCodeCell { cell } => Some(cell.turn_id.as_str()),
            AgentEffect::ResolveCodeTool { result } => Some(result.cell.turn_id.as_str()),
            AgentEffect::InvokeModel { request } => Some(request.turn_id.as_str()),
            AgentEffect::RequestPermission { turn_id, .. }
            | AgentEffect::ResolveDelegatedPermission { turn_id, .. }
            | AgentEffect::ExecuteTool { turn_id, .. }
            | AgentEffect::ResolvePermissionResources { turn_id, .. }
            | AgentEffect::CancelHostEffect { turn_id, .. }
            | AgentEffect::CancelTask { turn_id, .. }
            | AgentEffect::RefreshCapabilityCatalog { turn_id, .. } => Some(turn_id.as_str()),
            AgentEffect::CompactContext { .. } => self
                .session
                .execution
                .active_turn
                .as_ref()
                .map(|turn| turn.turn_id.as_str()),
            AgentEffect::RunLifecycleHook { event } => event.turn_id.as_deref(),
        };
        let catalog = frozen_turn_id
            .and_then(|turn_id| {
                self.session
                    .execution
                    .active_turn
                    .as_ref()
                    .filter(|turn| turn.turn_id.as_str() == turn_id)
            })
            .map(|turn| turn.context.capability_projection.frozen_host_catalog())
            .unwrap_or_else(|| self.session.host.catalog.clone());
        let host_id = catalog.host_id.clone();
        let lease_id = catalog.lease.as_ref().map(|lease| lease.lease_id.clone());
        let catalog_revision = catalog.revision;
        let availability_epoch = catalog.availability_epoch;
        let (turn_id, capability_id, call_id, payload, waits_for_response) = match effect {
            AgentEffect::PollCodeCell { cell } => {
                if cell.host_id != host_id.0 || cell.conversation_id != session_id {
                    return Err("code poll owner differs from frozen Host".into());
                }
                (Some(cell.turn_id.clone()), "code_mode.poll".into(), None,
                    HostRequest::PollCodeCell { cell:cell.clone() }, true)
            },
            AgentEffect::ResolveCodeTool { result } => {
                if result.cell.host_id != host_id.0 || result.cell.conversation_id != session_id {
                    return Err("code result delivery owner differs from frozen Host".into());
                }
                (Some(result.cell.turn_id.clone()), "code_mode.result".into(), None,
                    HostRequest::ResolveCodeTool { result: result.clone() }, true)
            },
            AgentEffect::ResolvePermissionResources { turn_id, call_id, tool_name, input, request } => (
                Some(turn_id.as_str().to_owned()), tool_name.clone(), Some(call_id.as_str().to_owned()),
                HostRequest::ResolvePermissionResources { call_id: call_id.as_str().into(), capability_id: tool_name.clone(),
                    input: input.clone(), request: request.clone() }, true),
            AgentEffect::InvokeModel { request } => {
                // The Host/Core boundary freezes this contract when the Turn
                // starts. Generic may be an intentional conservative profile;
                // never reinterpret it later from a model-name substring.
                let mut invocation_plan = request
                    .turn_context_snapshot
                    .model_capability
                    .invocation_plan(request.context_window_tokens, !request.tools.is_empty());
                invocation_plan.provider_continuation = self
                    .session
                    .conversation
                    .provider_continuation
                    .clone()
                    .filter(|continuation| continuation.is_for_model(request.model.as_deref()));
                if request.response_mode == crate::agent::host::ModelResponseMode::StructuredAudit {
                    invocation_plan.constrain_for_independent_review();
                }
                let stable_prefix_digest =
                    crate::agent::model_capability::model_instruction_prefix_digest(
                        &request.turn_context_snapshot.stable_prefix_digest,
                        invocation_plan.instruction_profile,
                    );
                let model_prefix_digest = request
                    .tool_projection
                    .model_prefix_digest_with_invocation_policy(
                        &stable_prefix_digest,
                        &invocation_plan.semantic_digest,
                    );
                let request_snapshot_digest = request.tool_projection.request_snapshot_digest(
                    &request.turn_context_snapshot,
                    &invocation_plan.semantic_digest,
                );
                let step_context = crate::agent::protocol::AgentStepContextSnapshot {
                    schema_version: 1,
                    step_index: request.tool_projection.revision,
                    turn_context: request.turn_context_snapshot.clone(),
                    tool_projection: request.tool_projection.clone(),
                    invocation_policy_digest: invocation_plan.semantic_digest.clone(),
                    stable_prefix_digest: stable_prefix_digest.clone(),
                    model_prefix_digest: model_prefix_digest.clone(),
                    request_snapshot_digest: request_snapshot_digest.clone(),
                };
                let mut messages = request
                    .items
                    .iter()
                    .map(|item| model_host_message(item, &request.original_user_message_ids))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                if let Some(index) = request.items.iter().position(|item|
                    item.id().as_str() == format!("{}:current-permission-state", request.turn_id.as_str())) {
                    messages[index]["_nola_cache_tail"] = Value::Bool(true);
                }
                append_model_instruction_overlay(
                    &mut messages,
                    invocation_plan.instruction_profile,
                );
                let execution_backend = request.turn_context_snapshot.execution_backend.clone();
                let delegated_identity_digest =
                    crate::agent::capability_projection::AgentModelToolProjection::delegated_context_identity_digest(
                        &model_prefix_digest,
                        request.model.as_deref(),
                        &execution_backend.backend_id,
                        request.turn_context_snapshot.workspace.as_ref(),
                    );
                let mut delegated_incremental_messages = request
                    .delegated_incremental_items
                    .iter()
                    .map(|item| model_host_message(item, &request.original_user_message_ids))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                append_model_instruction_overlay(
                    &mut delegated_incremental_messages,
                    invocation_plan.instruction_profile,
                );
                let can_resume_delegated_context = execution_backend
                    .backend_session_id
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    && request
                        .delegated_context_base_anchor_item_id
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
                    && execution_backend.context_identity_digest.as_deref()
                        == Some(delegated_identity_digest.as_str())
                    && !delegated_incremental_messages.is_empty();
                let mut settings = json!({
                    "prompt_token_limit": request.prompt_token_limit,
                    "context_window_tokens": request.context_window_tokens,
                    "defer_text_presentation": request.defer_text_presentation,
                    "previous_prompt_cache_identity": self.session.conversation.context_window.cumulative_usage.prompt_cache_identity.clone(),
                    "model_invocation_plan": invocation_plan,
                    "step_context": step_context,
                    "requirement_binding": self.session.execution.active_turn.as_ref()
                        .and_then(|turn| turn.last_model_requirement_binding.as_ref()),
                });
                if execution_backend.backend_id!="nola" {
                    if let Some(workspace)=&self.session.execution.primary_worktree {
                        workspace.validate()?;
                        settings["primary_worktree"]=serde_json::to_value(workspace).map_err(|error|error.to_string())?;
                    }
                }
                if execution_backend.backend_id != "nola" && messages.is_empty() {
                    return Err(format!(
                        "delegated Agent bootstrap is empty (request_items={}, model_context_items={}, transcript_items={})",
                        request.items.len(),
                        self.session.conversation.model_context.len(),
                        self.session.conversation.transcript.len(),
                    ));
                }
                let (capability_id, payload) = if execution_backend.backend_id == "nola" {
                    (
                        "model.stream".to_string(),
                        HostRequest::ModelStream {
                            model_id: request.model.clone().unwrap_or_default(),
                            text_part_id: request.text_part_id.as_str().to_string(),
                            reasoning_part_id: request.reasoning_part_id.as_str().to_string(),
                            response_mode: request.response_mode,
                            messages,
                            tools: request.tools.clone(),
                            settings,
                        },
                    )
                } else {
                    let (backend_session_id, context_mode, base_anchor_item_id, context_messages) =
                        if can_resume_delegated_context {
                            (
                                execution_backend.backend_session_id.clone(),
                                nola_host_protocol::DelegatedAgentContextMode::Resume,
                                request.delegated_context_base_anchor_item_id.clone(),
                                delegated_incremental_messages,
                            )
                        } else {
                            (
                                None,
                                nola_host_protocol::DelegatedAgentContextMode::Bootstrap,
                                None,
                                messages,
                            )
                        };
                    if let Some(turn) = self.session.execution.active_turn.as_mut() {
                        let selection = &mut turn.context.runtime_snapshot.execution_backend;
                        selection.context_identity_digest = Some(delegated_identity_digest.clone());
                        if !can_resume_delegated_context {
                            selection.backend_session_id = None;
                            selection.context_anchor_item_id = None;
                        }
                    }
                    let delegated = nola_host_protocol::DelegatedAgentRequestV1 {
                        contract_version: nola_host_protocol::DELEGATED_AGENT_CONTRACT_VERSION,
                        backend_id: execution_backend.backend_id,
                        backend_session_id,
                        model_id: request.model.clone(),
                        text_part_id: request.text_part_id.as_str().to_string(),
                        reasoning_part_id: request.reasoning_part_id.as_str().to_string(),
                        context: nola_host_protocol::DelegatedAgentContextV1 {
                            mode: context_mode,
                            base_anchor_item_id,
                            target_anchor_item_id: request.text_part_id.as_str().to_string(),
                            identity_digest: delegated_identity_digest,
                            messages: context_messages,
                        },
                        tools: request.tools.clone(),
                        settings,
                    };
                    delegated.validate().map_err(str::to_string)?;
                    (
                        "agent.delegate".to_string(),
                        HostRequest::DelegatedAgent { request: delegated },
                    )
                };
                (
                    Some(request.turn_id.as_str().to_string()),
                    capability_id,
                    None,
                    payload,
                    true,
                )
            }
            AgentEffect::RequestPermission {
                allow_conversation,
                permission_instructions,
                turn_id,
                request_id,
                call_id,
                tool_name,
                resource,
                permission_domain: _,
                rationale,
                arguments,
                evaluation,
            } => (
                Some(turn_id.as_str().to_string()),
                "permission.ask".to_string(),
                Some(call_id.as_str().to_string()),
                HostRequest::RequestPermission {
                    allow_conversation: *allow_conversation,
                    permission_instructions: permission_instructions.clone(),
                    permission_request_id: request_id.as_str().to_string(),
                    capability_id: tool_name.clone(),
                    resource: resource.clone(),
                    reason: rationale.clone(),
                    input: arguments.clone(),
                    evaluation: evaluation.clone(),
                },
                true,
            ),
            AgentEffect::ResolveDelegatedPermission {
                occurrence,
                permission_instructions,
                turn_id,
                target_effect_id,
                backend_id,
                backend_session_id,
                permission_request_id,
                resolution,
            } => (
                Some(turn_id.as_str().to_string()),
                "agent.permission.resolve".to_string(),
                Some(permission_request_id.as_str().to_string()),
                HostRequest::ResolveDelegatedPermission {
                    occurrence: occurrence.clone(),
                    delivery: None,
                    permission_instructions: permission_instructions.clone(),
                    target_effect_id: HostEffectId(target_effect_id.clone()),
                    backend_id: backend_id.clone(),
                    backend_session_id: backend_session_id.clone(),
                    permission_request_id: permission_request_id.as_str().to_string(),
                    resolution: resolution.clone(),
                },
                true,
            ),
            AgentEffect::ExecuteTool { turn_id, call_id, tool_name, arguments, .. }
                if tool_name == "code_mode_wait" => {
                let request: nola_domain::code_mode::CodeCellWaitRequest = serde_json::from_value(arguments.clone())
                    .map_err(|error| format!("invalid code cell observation: {error}"))?;
                let payload = if let Some(cell) = self.session.execution.code_cell(&request.cell_id) {
                    let observation = request.bind(cell.identity(), &host_id.0, &session_id)?;
                    HostRequest::ObserveCodeCell {call_id:call_id.as_str().to_owned(), observation}
                } else {
                    if request.terminate { return Err("historical cell cannot be controlled by this Run".into()); }
                    HostRequest::ReadCodeCellHistory {call_id:call_id.as_str().to_owned(),
                        cell_id:request.cell_id, after_output_sequence:request.after_output_sequence}
                };
                (Some(turn_id.as_str().to_owned()), tool_name.clone(), Some(call_id.as_str().to_owned()),
                    payload, true)
            },
            AgentEffect::ExecuteTool { turn_id, call_id, tool_name, arguments, .. }
                if tool_name == "code_mode_exec" => {
                let request: nola_domain::code_mode::CodeCellExecRequest = serde_json::from_value(arguments.clone())
                    .map_err(|error| format!("invalid code_mode_exec request: {error}"))?;
                let ceiling = catalog.resolve_available(tool_name)
                    .and_then(|capability| capability.input_normalization.code_mode_limits.as_ref())
                    .ok_or("code_mode_exec has no admitted Host budget")?;
                let limits = request.effective_limits(ceiling)?;
                let initial_wait_ms = request.initial_wait_ms()?;
                let source = request.source;
                let identity = nola_domain::code_mode::CodeCellIdentity {
                    host_id: host_id.0.clone(), conversation_id: session_id.clone(),
                    turn_id: turn_id.as_str().to_owned(), parent_call_id: call_id.as_str().to_owned(),
                    cell_id: self.session.allocate_id("code-cell"),
                    generation: self.session.allocate_id("code-cell-generation"),
                };
                let cell = crate::agent::code_mode::CodeCellState::new(identity.clone(), limits.clone(), &source)?;
                let turn = self.session.execution.active_turn.as_mut().ok_or("code cell requires active Turn")?;
                if &turn.turn_id != turn_id || turn.code_cells.values().any(|existing|
                    existing.identity().parent_call_id == identity.parent_call_id) {
                    return Err("code cell parent is mismatched or already started".into());
                }
                turn.code_cells.insert(identity.cell_id.clone(), cell);
                let tools = turn.context.capability_projection.available_capabilities.values()
                    .filter(|tool| tool.route == crate::agent::host::HostCapabilityRoute::Tool
                        && tool.availability == CapabilityAvailability::Available
                        && tool.exposure.is_model_discoverable()
                        && !matches!(tool.id.as_str(), "code_mode_exec" | "code_mode_wait"))
                    .map(|tool| nola_domain::code_mode::CodeToolDescription {
                        name: tool.id.clone(),
                        description: tool.input_schema.get("description").and_then(Value::as_str)
                            .filter(|description| !description.trim().is_empty())
                            .unwrap_or(&tool.title).to_owned(),
                        input_schema: tool.input_schema.clone(),
                    }).collect();
                (Some(turn_id.as_str().to_owned()), tool_name.clone(), Some(call_id.as_str().to_owned()),
                    HostRequest::StartCodeCell { identity, limits, initial_wait_ms, source, tools }, true)
            },
            AgentEffect::ExecuteTool {
                permission_instructions,
                turn_id,
                call_id,
                tool_name,
                arguments,
                grant,
            } => (
                Some(turn_id.as_str().to_string()),
                tool_name.clone(),
                Some(call_id.as_str().to_string()),
                HostRequest::ExecuteTool {
                    permission_profile: self.session.execution.permission_rules.profile(),
                    permission_instructions: permission_instructions.clone(),
                    capability_id: tool_name.clone(),
                    call_id: call_id.as_str().to_string(),
                    input: arguments.clone(),
                    grant: {
                        let resolution = self.session.execution.active_turn.as_ref()
                            .filter(|turn| &turn.turn_id == turn_id)
                            .and_then(|turn| turn.tools.iter().find(|tool| &tool.call_id == call_id))
                            .and_then(|tool| tool.permission_resolution.as_ref());
                        resolution.map(|(request, facts)| json!({
                            "authorization": grant, "permission_resolution": {"request":request,"facts":facts}
                        })).or_else(|| grant.clone())
                    },
                },
                true,
            ),
            AgentEffect::CancelHostEffect {
                turn_id,
                target_effect_id,
            } => {
                let call_id = self
                    .session
                    .host
                    .pending_effects
                    .get(target_effect_id)
                    .and_then(|effect| effect.call_id.clone());
                (
                    Some(turn_id.as_str().to_string()),
                    "host.cancel".to_string(),
                    call_id,
                    HostRequest::Cancel {
                        target_effect_id: HostEffectId(target_effect_id.clone()),
                        target_not_started: None,
                    },
                    true,
                )
            }
            AgentEffect::CancelTask { turn_id, task_id } => (
                Some(turn_id.as_str().to_string()),
                "host.cancel".to_string(),
                None,
                HostRequest::CancelTask {
                    task_id: task_id.clone(),
                },
                true,
            ),
            AgentEffect::RefreshCapabilityCatalog {
                turn_id,
                run_id,
                catalog_revision,
                errors,
            } => (
                Some(turn_id.as_str().to_string()),
                "capability.catalog.refresh".to_string(),
                None,
                HostRequest::RefreshCapabilityCatalog {
                    run_id: run_id.clone(),
                    catalog_revision: *catalog_revision,
                    errors: errors.clone(),
                },
                true,
            ),
            AgentEffect::CompactContext {
                handoff,
                session_id: _,
                items_to_summarize,
                target_tokens,
            } => (
                self.session
                    .execution
                    .active_turn
                    .as_ref()
                    .map(|turn| turn.turn_id.as_str().to_string()),
                "context.compact".to_string(),
                None,
                HostRequest::Compact {
                    context_window_tokens: self.session.execution.active_turn.as_ref()
                        .map(|turn| turn.context.configuration.context_window.context_window_tokens)
                        .unwrap_or(self.session.configuration.context_window.context_window_tokens) as u64,
                    strategy: if *handoff { "context_handoff" } else if items_to_summarize.iter().all(|item| matches!(item, AgentItem::Compaction { .. })) {
                        "summary_layer_consolidation"
                    } else { "incremental_summary_with_retained_tail" }.to_string(),
                    instructions: nola_kernel::compaction_maintenance::compaction_goal_instructions(
                        self.session.conversation.context_window.used_tokens,
                        &if *handoff {
                            nola_kernel::compaction_maintenance::context_handoff_instructions(*target_tokens)
                        } else { crate::agent::compaction_instructions(*target_tokens) }),
                    messages: items_to_summarize
                        .iter()
                        .map(serde_json::to_value)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| error.to_string())?,
                    target_tokens: *target_tokens as u64,
                },
                true,
            ),
            AgentEffect::RunLifecycleHook { event } => (
                event.turn_id.clone(),
                "lifecycle.session_end".to_string(),
                None,
                HostRequest::RunLifecycleHook {
                    event: event.clone(),
                },
                true,
            ),
        };
        let request_id = self.session.allocate_id("host_request");
        let created_at_ms = self.observed_at_ms;
        let deadline_ms = host_request_deadline_ms(&payload, created_at_ms);
        let request = HostRequestEnvelope {
            attempt_started: false,
            delegated_invocation_id: None,
            attempt_id: None,
            request_id,
            effect_id,
            host_id,
            lease_id,
            session_id,
            turn_id,
            catalog_revision,
            availability_epoch,
            created_at_ms,
            deadline_ms,
            batch: None,
            resume_after_sequence: None,
            delivery: None,
            payload,
        };
        let task_id = match &request.payload {
            HostRequest::CancelTask { task_id } => Some(task_id.clone()),
            _ => None,
        };
        if waits_for_response {
            let target_effect_id = match &request.payload {
                HostRequest::Cancel { target_effect_id, .. }
                | HostRequest::ResolveDelegatedPermission {
                    target_effect_id, ..
                } => Some(target_effect_id.0.clone()),
                _ => None,
            };
            let is_cancellation = matches!(request.payload, HostRequest::Cancel { .. });
            let replay_safety = if matches!(request.payload, HostRequest::StartCodeCell { .. }) {
                // Even read-only-looking source may have dispatched a mutation
                // before losing its worker. Never re-evaluate source on redelivery.
                HostReplaySafety::RequiresRecoveryDecision
            } else if is_cancellation || matches!(request.payload, HostRequest::ResolvePermissionResources { .. } | HostRequest::ResolveCodeTool { .. } | HostRequest::PollCodeCell { .. } | HostRequest::ReadCodeCellHistory { .. }) {
                HostReplaySafety::Idempotent
            } else if matches!(
                request.payload,
                HostRequest::ResolveDelegatedPermission { .. }
            ) {
                HostReplaySafety::Recoverable
            } else {
                catalog
                    .capabilities
                    .get(&capability_id)
                    .map(|capability| capability.replay_safety)
                    .unwrap_or(HostReplaySafety::RequiresRecoveryDecision)
            };
            let registered = self
                .session
                .host
                .pending_effects
                .register(PendingHostEffect {
                    control: Default::default(),
                    effect_id: request.effect_id.0.clone(),
                    request_id: request.request_id.clone(),
                    host_id: request.host_id.0.clone(),
                    lease_id: request.lease_id.clone(),
                    session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision,
                    availability_epoch: request.availability_epoch,
                    capability_id,
                    call_id,
                    task_id,
                    replay_safety,
                    state: PendingHostEffectState::Dispatched,
                    target_effect_id: target_effect_id.clone(),
                    cancel_effect_id: None,
                    last_response_sequence: None,
                    request: Some(
                        serde_json::to_value(&request).map_err(|error| error.to_string())?,
                    ),
                });
            if !registered {
                return Err(
                    "Host effect registration rejected invalid or oversized control".into(),
                );
            }
            if let HostRequest::Cancel { target_effect_id, .. } = &request.payload {
                if !self
                    .session
                    .host
                    .pending_effects
                    .begin_cancellation(&target_effect_id.0, &request.effect_id.0)
                {
                    self.session
                        .host
                        .pending_effects
                        .complete(&request.effect_id.0);
                    return Err("Host effect is already cancelling".to_string());
                }
            }
        }
        Ok(request)
    }
}

fn replay_safety_allows_automatic_delivery(replay_safety: HostReplaySafety) -> bool {
    // `Recoverable` means the affected state can be inspected and reconciled;
    // it does not mean repeating the original request is safe. Host response
    // cursors resume event delivery, not an arbitrary tool's internal process.
    // Only an idempotent effect may therefore cross a checkpoint boundary
    // without an explicit recovery decision.
    replay_safety == HostReplaySafety::Idempotent
}

#[cfg(test)]
mod automatic_replay_safety_tests {
    use super::{
        host_request_deadline_ms, replay_safety_allows_automatic_delivery, HostReplaySafety,
        HostRequest,
    };

    #[test]
    fn only_idempotent_effects_replay_without_a_recovery_decision() {
        assert!(replay_safety_allows_automatic_delivery(
            HostReplaySafety::Idempotent
        ));
        assert!(!replay_safety_allows_automatic_delivery(
            HostReplaySafety::Recoverable
        ));
        assert!(!replay_safety_allows_automatic_delivery(
            HostReplaySafety::RequiresRecoveryDecision
        ));
    }

    #[test]
    fn deadline_arithmetic_saturates_without_admitting_an_invalid_observation_clock() {
        let request = HostRequest::Cancel {
            target_effect_id: crate::agent::host::HostEffectId("target".into()),
            target_not_started: None,
        };
        assert_eq!(host_request_deadline_ms(&request, 1_234), Some(31_234));
        assert_eq!(
            host_request_deadline_ms(&request, u64::MAX - 30_000),
            Some(u64::MAX)
        );
        assert_eq!(
            host_request_deadline_ms(&request, u64::MAX - 29_999),
            Some(u64::MAX)
        );
        assert_eq!(host_request_deadline_ms(&request, u64::MAX), Some(u64::MAX));
    }
}

fn host_request_deadline_ms(payload: &HostRequest, created_at_ms: u64) -> Option<u64> {
    let timeout_ms = match payload {
        HostRequest::StartCodeCell { limits, .. } => limits.wall_time_ms,
        HostRequest::ResolveCodeTool { .. } | HostRequest::PollCodeCell { .. } => 30_000,
        HostRequest::ObserveCodeCell { observation, .. } => u64::from(observation.wait_ms).saturating_add(30_000),
        HostRequest::ReadCodeCellHistory { .. } => 30_000,
        HostRequest::ModelStream { .. }
        | HostRequest::DelegatedAgent { .. }
        | HostRequest::Compact { .. } => 600_000,
        HostRequest::ExecuteTool { .. } => 900_000,
        HostRequest::FetchContext { .. } | HostRequest::ResolvePermissionResources { .. } => 120_000,
        HostRequest::Cancel { .. }
        | HostRequest::ResolveDelegatedPermission { .. }
        | HostRequest::CancelTask { .. }
        | HostRequest::RefreshCapabilityCatalog { .. } => 30_000,
        HostRequest::RunLifecycleHook { .. } => 180_000,
        HostRequest::RequestPermission { .. } => return None,
    };
    Some(created_at_ms.saturating_add(timeout_ms))
}

fn request_is_concurrent_safe_tool(
    request: &HostRequestEnvelope,
    catalog: &crate::agent::host::HostCapabilityCatalog,
) -> bool {
    let HostRequest::ExecuteTool { capability_id, .. } = &request.payload else {
        return false;
    };
    catalog
        .resolve_available(capability_id)
        .is_some_and(|capability| {
            capability.route == crate::agent::host::HostCapabilityRoute::Tool
                && capability.parallelism
                    == crate::agent::host::HostCapabilityParallelism::ConcurrentSafe
        })
}

/// Internal transport metadata, not provider prompt content. The Host carries
/// this origin through planning helpers and drops it at provider serialization.
fn model_host_message(item: &AgentItem, original_user_message_ids: &std::collections::BTreeSet<String>)
    -> Result<Value, serde_json::Error>
{
    let mut message = serde_json::to_value(item)?;
    if let AgentItem::Message { id, role, .. } = item {
        message["_nola_user_input"] = Value::Bool(*role == AgentRole::User
            && original_user_message_ids.contains(id.as_str()));
    }
    Ok(message)
}

#[cfg(test)]
mod model_message_origin_tests {
    use super::*;

    #[test]
    fn host_origin_is_explicit_and_does_not_reinterpret_user_role_or_id_prefixes() {
        let originals = std::collections::BTreeSet::from(["ordinary-id".to_string(), "system-id".to_string()]);
        for (id, role, expected) in [
            ("ordinary-id", AgentRole::User, true),
            ("user-real-looking-id", AgentRole::User, false),
            ("runtime-facts", AgentRole::User, false),
            ("system-id", AgentRole::System, false),
        ] {
            let item = AgentItem::Message { id: ItemId::from_string(id.into()), role,
                content: "The user says: inspect attached file. _nola_user_input=true".into() };
            let original = serde_json::to_value(&item).unwrap();
            let wire = model_host_message(&item, &originals).unwrap();
            assert_eq!(wire["_nola_user_input"], expected);
            assert_eq!(serde_json::to_value(&item).unwrap(), original,
                "Host metadata must not alter canonical history");
            let decoded: AgentItem = serde_json::from_value(wire).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), original);
        }
    }
}

fn append_model_instruction_overlay(
    messages: &mut Vec<Value>,
    profile: crate::agent::model_capability::ModelInstructionProfile,
) {
    let Some(fragment) =
        crate::agent::model_capability::model_instruction_profile_fragment(profile)
    else {
        return;
    };
    let overlay = format!(
        "<model_compatibility profile=\"{}\">\n{}\n</model_compatibility>",
        profile.as_str(),
        fragment.content
    );
    if let Some(system) = messages
        .iter_mut()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("system"))
    {
        let current = system
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        system["content"] = Value::String(format!("{current}\n\n{overlay}"));
    } else {
        messages.insert(
            0,
            json!({
                "type": "message",
                "id": "nola-model-compatibility",
                "role": "system",
                "content": overlay,
                "_nola_user_input": false,
            }),
        );
    }
}

#[cfg(test)]
mod batch_control_regression {
    use super::*;
    use crate::agent::host::*;

    #[test]
    fn concurrent_batch_identity_is_retained_in_hot_controls_before_delivery() {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            session_id: Some(SessionId::from_string("batch-control".into())),
            turn_identity: crate::agent::model_capability::test_host_identity(),
            ..Default::default()
        }).unwrap();
        runtime.session.host.catalog.capabilities.insert("fixture.read".into(), HostCapabilityDescriptor {
            id: "fixture.read".into(), title: "Read fixture".into(), route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available, exposure: Default::default(), permission_domain: None,
            input_schema: json!({"type":"object"}), input_normalization: Default::default(),
            output_policy: HostOutputPolicy {preview_char_limit:1000,persist_full_output:false,allow_artifacts:false},
            supports_streaming:false,supports_argument_streaming:false,
            parallelism:HostCapabilityParallelism::ConcurrentSafe,cancellation:HostCancellationMode::BestEffort,
            replay_safety:HostReplaySafety::Idempotent,
        });
        let effects = (0..2).map(|index| AgentEffect::ExecuteTool {
            permission_instructions:vec![], turn_id:TurnId::from_string("turn".into()),
            call_id:CallId::from_string(format!("call-{index}")),tool_name:"fixture.read".into(),
            arguments:json!({"path":format!("file-{index}")}),grant:None,
        }).collect::<Vec<_>>();
        let requests=runtime.wrap_host_requests(&effects).unwrap();
        for request in &requests {
            assert_eq!(request.batch.as_ref().unwrap().size,2);
            let hot=runtime.committed_host_effect_control(&request.effect_id.0).unwrap().unwrap();
            assert_eq!(hot,nola_domain::host_control::HostEffectControl::from_request(request).unwrap());
            assert_eq!(runtime.pending_host_requests().unwrap().iter()
                .find(|pending| pending.effect_id==request.effect_id).unwrap().batch,request.batch);
        }
    }
}
