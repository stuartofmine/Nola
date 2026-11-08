// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns Turn creation, steering boundaries, terminal transitions, and context reporting.
// - Boundary: Private deterministic reducer layer; public clients use AgentCore.

use super::*;
use crate::agent::parts::{
    ArtifactPart, ArtifactReference, CompactionPart, ConversationProjection, PartOwnership,
};
use crate::agent::protocol::AgentInputOrigin;
use std::collections::{BTreeMap, BTreeSet};

impl AgentRuntime {
    pub(super) fn revert_conversation(
        &mut self,
        through_message_id: &str,
        keep_message: bool,
    ) -> Result<(), String> {
        if self
            .session
            .execution
            .active_turn
            .as_ref()
            .is_some_and(|turn| !turn.status.is_terminal())
        {
            return Err("conversation can only be reverted while the thread is idle".to_string());
        }
        if self.session.host.pending_effects.len() > 0 {
            return Err(
                "conversation cannot be reverted while Host effects are pending".to_string(),
            );
        }
        if self.session.execution.input_queue.len() > 0 {
            return Err("conversation cannot be reverted while user input is queued".to_string());
        }

        let matching_indexes = self
            .session
            .conversation
            .transcript
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                self.session
                    .item_ownership
                    .get(item.id().as_str())
                    .is_some_and(|ownership| ownership.message_id == through_message_id)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        let first_index = matching_indexes
            .first()
            .copied()
            .ok_or_else(|| format!("unknown Core message boundary {through_message_id}"))?;
        let retained_count = if keep_message {
            matching_indexes
                .last()
                .copied()
                .unwrap_or(first_index)
                .saturating_add(1)
        } else {
            first_index
        };
        let retained_items = self.session.conversation.transcript[..retained_count].to_vec();
        let retained_item_ids = retained_items
            .iter()
            .map(|item| item.id().as_str().to_string())
            .collect::<BTreeSet<_>>();
        let retained_ownership = self
            .session
            .item_ownership
            .iter()
            .filter(|(item_id, _)| retained_item_ids.contains(*item_id))
            .map(|(item_id, ownership)| (item_id.clone(), ownership.clone()))
            .collect();

        let active_compaction_item_id =
            self.session
                .conversation
                .model_context
                .iter()
                .find_map(|item| match item {
                    AgentItem::Compaction { id, .. } => Some(id.as_str().to_string()),
                    _ => None,
                });
        let retained_model_context = if active_compaction_item_id
            .as_ref()
            .is_some_and(|item_id| retained_item_ids.contains(item_id))
        {
            self.session
                .conversation
                .model_context
                .iter()
                .filter(|item| retained_item_ids.contains(item.id().as_str()))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            retained_items
                .iter()
                .filter(|item| !matches!(item, AgentItem::Compaction { .. }))
                .filter(|item| self.session.item_ownership.get(item.id().as_str())
                    .is_some_and(|owner| owner.source.is_model_visible()))
                .cloned()
                .collect::<Vec<_>>()
        };
        let provider_continuation =
            retained_model_context
                .iter()
                .rev()
                .find_map(|item| match item {
                    AgentItem::Reasoning {
                        provider_continuation: Some(continuation),
                        ..
                    } => Some(continuation.clone()),
                    _ => None,
                });

        let retained_projection =
            self.rebuild_historical_projection(&retained_items, &retained_ownership)?;
        self.session.execution.retire_code_cells()?;
        self.session.conversation.transcript = retained_items;
        self.session.conversation.model_context = retained_model_context;
        self.session.conversation.provider_continuation = provider_continuation;
        self.session.conversation.projection = retained_projection;
        self.session
            .conversation
            .context_window
            .rebase_estimate(&self.session.conversation.model_context);
        self.session.item_ownership = retained_ownership;
        self.session.execution.active_turn = None;
        Ok(())
    }

    pub(super) fn rebase_conversation(
        &mut self,
        items: Vec<AgentItem>,
        item_ownership: std::collections::BTreeMap<
            String,
            super::super::session::AgentItemOwnership,
        >,
    ) -> Result<(), String> {
        Self::validate_initial_items(&items)?;
        let item_ids = items
            .iter()
            .map(|item| item.id().as_str())
            .collect::<BTreeSet<_>>();
        let item_ownership = if item_ownership.is_empty() {
            Self::infer_item_ownership(&items)
        } else {
            if let Some(unknown_id) = item_ownership
                .keys()
                .find(|item_id| !item_ids.contains(item_id.as_str()))
            {
                return Err(format!(
                    "conversation ownership references unknown item {unknown_id}"
                ));
            }
            if let Some(missing_id) = item_ids
                .iter()
                .find(|item_id| !item_ownership.contains_key(**item_id))
            {
                return Err(format!(
                    "conversation ownership is missing item {missing_id}"
                ));
            }
            item_ownership
        };
        let provider_continuation = items.iter().rev().find_map(|item| match item {
            AgentItem::Reasoning {
                provider_continuation: Some(continuation),
                ..
            } => Some(continuation.clone()),
            _ => None,
        });
        if self
            .session
            .execution
            .active_turn
            .as_ref()
            .is_some_and(|turn| !turn.status.is_terminal())
        {
            return Err(
                "conversation history can only be rebased while the thread is idle".to_string(),
            );
        }
        if self.session.host.pending_effects.len() > 0 {
            return Err(
                "conversation history cannot be rebased while Host effects are pending".to_string(),
            );
        }
        if self.session.execution.input_queue.len() > 0 {
            return Err(
                "conversation history cannot be rebased while user input is queued".to_string(),
            );
        }

        let projection = self.rebuild_historical_projection(&items, &item_ownership)?;
        self.session.execution.retire_code_cells()?;
        self.session.conversation.transcript = items.clone();
        self.session.conversation.model_context = items.into_iter()
            .filter(|item| item_ownership.get(item.id().as_str())
                .is_some_and(|owner| owner.source.is_model_visible()))
            .collect();
        self.session.conversation.provider_continuation = provider_continuation;
        self.session.item_ownership = item_ownership;
        // Rebase is a canonical-history operation, not a provider-context-only
        // operation. Rebuild the product projection from the validated Agent
        // items and their explicit ownership so every Host restores the same
        // message/Part graph after a cold restart.
        self.session.conversation.projection = projection;
        self.session
            .conversation
            .context_window
            .rebase_estimate(&self.session.conversation.model_context);
        self.session.execution.active_turn = None;
        Ok(())
    }

    fn rebuild_historical_projection(
        &self,
        items: &[AgentItem],
        item_ownership: &BTreeMap<String, super::super::session::AgentItemOwnership>,
    ) -> Result<ConversationProjection, String> {
        let mut projection = ConversationProjection::default();
        let mut tool_parts = BTreeMap::<String, String>::new();

        for item in items {
            let item_id = item.id().as_str();
            let ownership = item_ownership
                .get(item_id)
                .ok_or_else(|| format!("conversation ownership is missing item {item_id}"))?;
            let execution_source = ownership.source.clone();
            let ownership = PartOwnership {
                message_id: ownership.message_id.clone(),
                turn_id: ownership.turn_id.clone(),
            };

            match item {
                AgentItem::Message {
                    role: AgentRole::Assistant,
                    content,
                    ..
                } => Self::apply_historical_part(
                    &mut projection,
                    AgentPart::Text(TextPart {
                        review: None,
                        id: item_id.to_string(),
                        ownership,
                        text: content.clone(),
                        is_complete: true,
                    }),
                )?,
                AgentItem::UiAction { .. } | AgentItem::Message { .. } => {}
                AgentItem::Attachment {
                    uri,
                    name,
                    mime_type,
                    ..
                } => Self::apply_historical_part(
                    &mut projection,
                    AgentPart::Artifact(ArtifactPart {
                        id: item_id.to_string(),
                        ownership,
                        artifact: ArtifactReference {
                            uri: uri.clone(),
                            name: name.clone(),
                            mime_type: mime_type.clone(),
                            operation: Some("attach".to_string()),
                        },
                    }),
                )?,
                AgentItem::Reasoning {
                    summary, content, ..
                } => Self::apply_historical_part(
                    &mut projection,
                    AgentPart::Reasoning(ReasoningPart {
                        id: item_id.to_string(),
                        ownership,
                        title: summary.clone(),
                        detail: content.clone(),
                        is_complete: true,
                    }),
                )?,
                AgentItem::ToolCall {
                    call_id,
                    name,
                    arguments,
                    ..
                } => {
                    let part_id = super::projection::bounded_tool_part_identity(format!("tool:{item_id}"));
                    let (title, origin) = self
                        .session
                        .host
                        .catalog
                        .resolve_available(name)
                        .map(|descriptor| {
                            (
                                descriptor.title.clone(),
                                descriptor.input_normalization.origin.clone(),
                            )
                        })
                        .unwrap_or_else(|| (name.clone(), Default::default()));
                    Self::apply_historical_part(
                        &mut projection,
                        AgentPart::Tool(ToolPart {
                            execution_source,
                            id: part_id.clone(),
                            ownership,
                            call_id: call_id.as_str().to_string(),
                            capability_id: name.clone(),
                            title,
                            summary: None,
                            origin,
                            state: ToolPartState::Pending,
                            input: arguments.clone(),
                            raw_input: String::new(),
                            permission: None,
                            output_preview: None,
                            structured_output: None,
                            full_output_ref: None,
                            artifacts: Vec::new(),
                            started_at_ms: None,
                            finished_at_ms: None,
                        }),
                    )?;
                    tool_parts.insert(call_id.as_str().to_string(), part_id);
                }
                AgentItem::ToolResult {
                    call_id,
                    output,
                    is_error,
                    full_output_ref,
                    artifacts,
                    ..
                } => {
                    let part_id = tool_parts.get(call_id.as_str()).ok_or_else(|| {
                        format!(
                            "conversation history contains an orphan tool result {}",
                            call_id.as_str()
                        )
                    })?;
                    let (output_preview, _) = self.tool_output_preview(
                        projection
                            .parts
                            .get(part_id)
                            .and_then(|part| match part {
                                AgentPart::Tool(part) => Some(part.capability_id.as_str()),
                                _ => None,
                            })
                            .unwrap_or_default(),
                        output,
                    );
                    let artifact_references = artifacts
                        .iter()
                        .map(|artifact| ArtifactReference {
                            uri: artifact.uri.clone(),
                            name: artifact.name.clone(),
                            mime_type: artifact.mime_type.clone(),
                            operation: artifact.operation.clone(),
                        })
                        .collect::<Vec<_>>();
                    projection
                        .apply(PartUpdate::ToolState {
                            part_id: part_id.clone(),
                            state: if *is_error {
                                ToolPartState::Failed
                            } else {
                                ToolPartState::Completed
                            },
                            title: None,
                            summary: self.tool_output_summary(output),
                            output_preview: Some(output_preview),
                            structured_output: Some(output.clone()),
                            full_output_ref: full_output_ref.clone(),
                            artifacts: artifact_references,
                            timestamp_ms: None,
                        })
                        .map_err(|error| {
                            format!("failed to rebuild historical tool Part: {error:?}")
                        })?;
                }
                AgentItem::Compaction {
                    summary,
                    source_item_ids,
                    source_fingerprint,
                    original_token_count,
                    ..
                } => Self::apply_historical_part(
                    &mut projection,
                    AgentPart::Compaction(CompactionPart {
                        id: item_id.to_string(),
                        ownership,
                        summary: summary.clone(),
                        retained_item_ids: source_item_ids
                            .iter()
                            .map(|id| id.as_str().to_string())
                            .collect(),
                        source_fingerprint: source_fingerprint.clone(),
                        original_token_count: u64::try_from(*original_token_count)
                            .unwrap_or(u64::MAX),
                        compacted_token_count: u64::try_from(
                            nola_kernel::item_token_estimate::estimated_item_tokens(
                                nola_domain::item_token_estimate::ItemTokenEstimateFacts::Text(
                                    summary,
                                ),
                            ),
                        )
                        .unwrap_or(u64::MAX),
                    }),
                )?,
            }
        }

        Ok(projection)
    }

    fn apply_historical_part(
        projection: &mut ConversationProjection,
        part: AgentPart,
    ) -> Result<(), String> {
        projection
            .apply(PartUpdate::Created { part })
            .map(|_| ())
            .map_err(|error| format!("failed to rebuild historical Part: {error:?}"))
    }

    pub(super) fn submit_input(
        &mut self,
        origin: AgentInputOrigin,
        mut items: Vec<AgentItem>,
        turn_context_items: Vec<AgentItem>,
        turn_context_platform: crate::context_engine::ContextPlatform,
        turn_context_components: Vec<crate::context_engine::ContextComponent>,
        requested_turn_id: Option<String>,
        submitted_at_epoch_ms: Option<i64>,
        delivery: AgentInputDelivery,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        if delivery == AgentInputDelivery::CurrentTurn && self.session.execution.active_turn.as_ref()
            .is_some_and(|turn| !turn.status.is_terminal() && turn.user_delegation_call.is_some()) {
            return Err("a delegation command accepts successor input only".into());
        }
        let action = nola_kernel::plan_turn_input(nola_domain::TurnInputFacts {
            has_items: !items.is_empty(),
            live_user_authority: origin.carries_live_user_authority(),
            has_context_items: !turn_context_items.is_empty(),
            has_context_components: !turn_context_components.is_empty(),
            active_status: self
                .session
                .execution
                .active_turn
                .as_ref()
                .map(|turn| turn.status),
        })
        .map_err(|error| {
            match error {
                nola_domain::TurnInputRejection::Empty => {
                    "agent input must contain at least one item"
                }
                nola_domain::TurnInputRejection::AutomatedMerge => {
                    "automated input cannot be merged into an active interactive turn"
                }
                nola_domain::TurnInputRejection::ContextAlreadyCaptured => {
                    "turn context can only be captured when starting a new turn"
                }
            }
            .to_string()
        })?;
        if action != nola_domain::TurnInputAction::Enqueue
            && self.session.execution.input_queue.has_next_turn_input() {
            return Err("pending successor input requires explicit admission before a new input Turn".into());
        }
        if origin.is_automated() && origin != AgentInputOrigin::Maintenance {
            let context_digest = crate::agent::protocol::internal_turn_context_digest(
                &turn_context_items, &turn_context_platform, &turn_context_components,
            );
            if items.is_empty() {
                let goal = if origin == AgentInputOrigin::GoalContinuation {
                    let active = self.session.goal_projection.active.as_ref()
                        .filter(|goal| !goal.status.is_terminal())
                        .ok_or("Goal continuation requires its current active Goal")?;
                    Some(crate::agent::protocol::GoalTriggerBinding {
                        goal_id: active.id.clone(),
                        goal_revision: self.session.goal_projection.revision,
                    })
                } else { None };
                let trigger_id = requested_turn_id.as_ref().map(|id| format!("{id}:internal-trigger"))
                    .unwrap_or_else(|| self.session.allocate_id("internal-trigger"));
                items.push(crate::agent::protocol::InternalTurnTrigger {
                    trigger_id,
                    core_session_id: self.session.session_id.as_str().into(),
                    conversation_id: None,
                    origin,
                    context_digest: context_digest.clone(),
                    goal,
                }.item()?);
            }
            if origin == AgentInputOrigin::GoalContinuation {
                if items.len() != 1 {
                    return Err("Goal continuation requires exactly its formal trigger item".into());
                }
                let trigger = crate::agent::protocol::InternalTurnTrigger::from_item(&items[0])?;
                let active = self.session.goal_projection.active.as_ref()
                    .filter(|goal| !goal.status.is_terminal())
                    .ok_or("Goal continuation requires its current active Goal")?;
                if trigger.origin != origin
                    || trigger.core_session_id != self.session.session_id.as_str()
                    || trigger.context_digest != context_digest
                    || trigger.goal.as_ref().is_none_or(|binding|
                        binding.goal_id != active.id || binding.goal_revision != self.session.goal_projection.revision)
                    || requested_turn_id.as_ref().is_some_and(|id|
                        trigger.trigger_id != format!("{id}:internal-trigger"))
                {
                    return Err("Goal continuation trigger differs from its current owner, context or Goal".into());
                }
            }
        } else if items.is_empty() && origin != AgentInputOrigin::Maintenance {
            return Err("context-only input requires an explicit automated origin".into());
        }
        if origin == AgentInputOrigin::GoalInteractiveUser {
            self.record_goal_user_updates(&items, events);
        }
        let anchor = self.session.execution.active_turn.as_ref()
            .filter(|turn| !turn.status.is_terminal()).map(|turn| turn.turn_id.as_str().to_owned());
        let digest_input = serde_json::json!({"origin":origin,"delivery":delivery,"items":items,
            "turn_context_items":turn_context_items,"platform":turn_context_platform,"components":turn_context_components});
        let digest = crate::agent::input_queue::input_digest(&digest_input);
        let admission_id = self.session.allocate_id("input-admission");
        let batch = self.session.execution.input_queue.admit(admission_id, requested_turn_id.clone(),
            origin, delivery, anchor, digest, items.clone())?;
        events.push(RuntimeEvent::InputAccepted { admission: batch.admission.clone() });
        if action == nola_domain::TurnInputAction::Enqueue {
            let turn_id = self
                .session
                .execution
                .active_turn
                .as_ref()
                .expect("Kernel enqueues only into an active turn")
                .turn_id
                .clone();
            let queued_items = items.clone();
            let item_count = self
                .session
                .execution
                .input_queue
                .enqueue_batch(batch)?;
            events.push(RuntimeEvent::InputQueued {
                turn_id,
                delivery,
                item_count,
                items: queued_items,
            });
            return Ok(());
        }
        self.start_turn(
            origin,
            items,
            vec![batch],
            turn_context_items,
            turn_context_platform,
            turn_context_components,
            requested_turn_id,
            submitted_at_epoch_ms,
            None,
            events,
            effects,
        )
    }

    pub(super) fn start_turn(
        &mut self,
        input_origin: AgentInputOrigin,
        items: Vec<AgentItem>,
        input_batches: Vec<crate::agent::input_queue::AdmittedInputBatch>,
        mut turn_context_items: Vec<AgentItem>,
        turn_context_platform: crate::context_engine::ContextPlatform,
        turn_context_components: Vec<crate::context_engine::ContextComponent>,
        requested_turn_id: Option<String>,
        submitted_at_epoch_ms: Option<i64>,
        user_delegation: Option<crate::agent::protocol::ModelToolCall>,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let turn_context_components = crate::context_engine::admit_context_components(
            turn_context_components,
            &self.session.configuration.memory_policy,
        );
        let artifact_contracts = turn_context_components
            .iter()
            .filter(|component| {
                component.id == crate::agent::protocol::TURN_ARTIFACT_CONTRACT_COMPONENT_ID
            })
            .collect::<Vec<_>>();
        if artifact_contracts.len() > 1 {
            return Err("a Turn accepts at most one external artifact contract".to_string());
        }
        let expected_artifact_refs = if let Some(component) = artifact_contracts.first() {
            let contract = serde_json::from_value::<
                crate::agent::protocol::AgentTurnArtifactContract,
            >(component.content.clone())
            .map_err(|error| format!("invalid Turn artifact contract: {error}"))?;
            contract.validate()?;
            contract
                .expected_artifact_refs
                .into_iter()
                .map(|reference| reference.trim().to_string())
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        crate::agent::tool_lifecycle::confirmed_sop_lineages_from_components(
            &turn_context_components,
        )?;
        let turn_id = TurnId::from_string(
            requested_turn_id
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| self.session.allocate_turn_id()),
        );
        self.session.record_turn_items(&turn_id, items.clone());
        let catalog = &self.session.host.catalog;
        let capability_projection =
            crate::agent::capability_projection::AgentTurnCapabilityProjection::resolve(
                &self.session.configuration.tools,
                catalog,
            );
        let prepared_components = crate::context_engine::prepare_context_components(
            turn_context_platform,
            &turn_context_components,
            self.session
                .configuration
                .context_window
                .prompt_token_limit(),
        );
        let mut component_items = prepared_components
            .messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                let content = message.get("content").and_then(Value::as_str)?;
                let role = match message
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("system")
                {
                    "developer" => AgentRole::Developer,
                    "user" => AgentRole::User,
                    "assistant" => AgentRole::Assistant,
                    _ => AgentRole::System,
                };
                let id = message
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{}:context:{index}", turn_id.as_str()));
                Some(AgentItem::Message {
                    id: ItemId::from_string(id),
                    role,
                    content: content.to_string(),
                })
            })
            .collect::<Vec<_>>();
        component_items.append(&mut turn_context_items);
        let context = Box::new(AgentTurnContext {
            input_origin,
            configuration: self.session.configuration.clone(),
            platform: turn_context_platform,
            components: turn_context_components,
            component_metrics: prepared_components.component_metrics,
            component_payload_digest: prepared_components.payload_digest,
            items: component_items,
            host_id: catalog.host_id.0.clone(),
            host_catalog_revision: catalog.revision,
            host_availability_epoch: catalog.availability_epoch,
            capability_projection: capability_projection.clone(),
            runtime_snapshot: crate::agent::protocol::AgentTurnContextSnapshot::capture(
                &self.session.configuration.turn_identity,
                input_origin,
                self.session.configuration.model.clone(),
                &self.session.configuration.context_window,
                catalog.revision,
                &capability_projection,
            ),
        });
        let pinned_skill_activations = self
            .session
            .conversation
            .pinned_skills
            .values()
            .map(|skill| {
                (
                    format!("skill:{}", skill.skill_id),
                    skill.required_tools.clone(),
                )
            })
            .collect::<Vec<_>>();
        let mut active_turn = AgentTurnState::new(turn_id.clone(), context.clone());
        // Only a typed model-selected review request can enter independent
        // review. Tool availability and prior tool execution are not signals.
        active_turn.final_answer_audit_required = false;
        if let Some(call) = user_delegation {
            self.session.record_turn_items(&turn_id, vec![AgentItem::ToolCall {
                id: ItemId::from_string(format!("{}:item", call.call_id.as_str())),
                call_id: call.call_id.clone(), name: call.name.clone(), arguments: call.arguments.clone(),
            }]);
            active_turn.user_delegation_call = Some(call.call_id.clone());
            active_turn.tools = active_turn.admit_tool_calls(vec![call]);
        }
        active_turn
            .engineering
            .require_artifacts(expected_artifact_refs);
        for (source, capability_ids) in pinned_skill_activations {
            active_turn.activate_capabilities(source, capability_ids);
        }
        self.session.execution.retire_code_cells()?;
        self.session.execution.active_turn = Some(Box::new(active_turn));
        // The prior Run's policy ACK must never appear as this Run's ACK.
        // Admitted rules remain the current execution policy; a new Host
        // admission supplies its independently frozen configuration.
        self.session.execution.active_permission_policy = None;
        if matches!(context.input_origin, crate::agent::AgentInputOrigin::GoalInteractiveUser | crate::agent::AgentInputOrigin::GoalContinuation) {
            if let Some(goal) = self.session.goal_projection.active.as_mut().filter(|goal| !goal.status.is_terminal()) {
                goal.work.source_turn_ids.insert(turn_id.as_str().to_owned());
                self.session.goal_projection.revision = self.session.goal_projection.revision.saturating_add(1);
                events.push(RuntimeEvent::GoalUpdated { goal_projection_revision:self.session.goal_projection.revision, goal:self.session.goal_projection.clone() });
            }
        }
        events.push(RuntimeEvent::TurnStarted {
            session_id: self.session.session_id.clone(),
            turn_id: turn_id.clone(),
            assistant_message_id: PartOwnership::for_turn(turn_id.as_str()).message_id,
        });
        if !input_batches.is_empty() {
            let applications = input_batches.iter().map(|batch|
                self.session.execution.input_queue.apply(batch, turn_id.as_str())).collect::<Result<Vec<_>,_>>()?;
            events.push(RuntimeEvent::InputApplied {
                turn_id: turn_id.clone(),
                applications: applications.clone(),
                items: items.clone(),
            });
            self.apply_requirement_inputs(&turn_id, &applications, &items, true, events)?;
        } else {
            let constraints = self.session.execution.requirements.execution_constraints.suspend(
                self.session.execution.requirements.scope.as_ref(), &self.session.execution.input_queue)?;
            self.session.execution.requirements.clear_current_scope();
            self.session.execution.requirements.execution_constraints = constraints;
        }
        let mut memory_usage =
            crate::memory::project_memory_usage(&context.components, &context.component_metrics);
        if let Some(used_at_epoch_ms) = submitted_at_epoch_ms {
            if used_at_epoch_ms < 0 {
                return Err("submitted_at_epoch_ms must not be negative".to_string());
            }
            let session_id = self.session.session_id.as_str().to_string();
            let context_payload_digest = context.component_payload_digest.clone();
            let mut recorded = BTreeSet::new();
            for usage in &memory_usage {
                if usage.scope.is_none() || !recorded.insert(usage.record_id.clone()) {
                    continue;
                }
                let usage_id = format!(
                    "usage:auto:{}:{}:{}",
                    session_id,
                    turn_id.as_str(),
                    usage.record_id
                );
                self.session.memory_runtime.apply(
                    crate::agent::memory_runtime::MemoryRuntimeCommandEnvelope {
                        operation_id: usage_id.clone(),
                        command: crate::agent::memory_runtime::MemoryRuntimeCommand::RecordUsage {
                            entry: crate::agent::memory_runtime::MemoryUsageLedgerEntry {
                                usage_id,
                                memory_id: usage.record_id.clone(),
                                session_id: session_id.clone(),
                                turn_id: turn_id.as_str().to_string(),
                                used_at_epoch_ms,
                                context_payload_digest: context_payload_digest.clone(),
                                citation_ids: Vec::new(),
                            },
                        },
                    },
                )?;
            }
        }
        for usage in &mut memory_usage {
            if let Some(stats) = self
                .session
                .memory_runtime
                .usage_stats
                .get(&usage.record_id)
            {
                usage.usage_count = stats.usage_count;
                usage.last_used_at_epoch_ms = stats.last_used_at_epoch_ms;
            }
        }
        events.push(RuntimeEvent::TurnContextCaptured {
            turn_id: turn_id.clone(),
            model: context.configuration.model.clone(),
            tool_count: context.capability_projection.enabled.len(),
            stable_prefix_digest: context.runtime_snapshot.stable_prefix_digest.clone(),
            prompt_fragment_ids: context.runtime_snapshot.prompt_fragment_ids.clone(),
            context_item_count: context.items.len(),
            context_component_count: context.components.len(),
            context_payload_digest: context.component_payload_digest.clone(),
            context_window_tokens: context.configuration.context_window.context_window_tokens,
            prompt_token_limit: context.configuration.context_window.prompt_token_limit(),
            host_id: context.host_id,
            host_catalog_revision: context.host_catalog_revision,
            host_availability_epoch: context.host_availability_epoch,
            capability_projection: context.capability_projection.clone(),
            memory_usage,
        });
        if self.session.execution.active_turn.as_ref().is_some_and(|turn| turn.user_delegation_call.is_some()) {
            self.schedule_tools(&turn_id, events, effects)
        } else if input_origin == AgentInputOrigin::Maintenance {
            Ok(())
        } else {
            self.request_model(&turn_id, events, effects)
        }
    }

    pub(super) fn complete_turn(
        &mut self,
        turn_id: &TurnId,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        self.reconcile_interrupted_code_calls()?;
        if self.code_controls_pending(turn_id, effects) {
            self.session.execution.active_turn.as_mut().ok_or("no active turn")?
                .code_completion_deferred = true;
            return Ok(());
        }
        let turn = self.session.execution.active_turn.as_mut().ok_or("no active turn")?;
        if &turn.turn_id != turn_id { return Err("turn id does not match the active turn".into()); }
        if turn.code_cells.values().any(|cell| !cell.is_terminal()) {
            turn.code_completion_deferred = true;
            self.advance_deferred_code_completion(events, effects)?;
            return Ok(());
        }
        if turn.code_cells.values().any(|cell| !cell.is_settled()) {
            turn.code_completion_deferred = true;
            turn.status = AgentTurnStatus::AwaitingTools;
            return Ok(());
        }
        if self.session.execution.user_input_waits.values().any(|wait| wait.is_open() && wait.turn_id == turn_id.as_str()) {
            self.session.execution.active_turn.as_mut().ok_or("no active turn")?.status = AgentTurnStatus::AwaitingInput;
            return Ok(());
        }
        {
            let turn = self
                .session
                .execution
                .active_turn
                .as_ref()
                .ok_or_else(|| "no active turn".to_string())?;
            if &turn.turn_id != turn_id {
                return Err("turn id does not match the active turn".to_string());
            }
        }
        let completion = nola_kernel::plan_turn_completion(
            self.session.execution.input_queue.has_current_turn_input(),
            self.session.execution.input_queue.has_next_turn_input(),
        );
        if completion == nola_domain::TurnCompletionAction::ContinueCurrent {
            return self.request_model(turn_id, events, effects);
        }
        if self.session.execution.permissions.pending.iter().any(|request| request.turn_id.as_str() == turn_id.as_str()) {
            self.session.execution.active_turn.as_mut().ok_or("no active turn")?.status = AgentTurnStatus::AwaitingPermission;
            return Ok(());
        }
        // Resolve provider aggregate IDs onto the canonical streamed Parts
        // before the result binds its answer references. Terminal publication
        // closes these Parts in finish_transition's existing boundary pass.
        self.project_events(events)?;
        self.decide_requirement_result(turn_id, events)?;
        if let Some(turn) = self.session.execution.active_turn.as_mut() {
            turn.status = AgentTurnStatus::Completed;
        }
        self.enqueue_automatic_memory_extraction(turn_id)?;
        self.session.seal_completed_turn_model_context(turn_id);
        events.push(RuntimeEvent::TurnCompleted {
            session_id: self.session.session_id.clone(),
            turn_id: turn_id.clone(),
            outcome: self
                .session
                .execution
                .active_turn
                .as_ref()
                .and_then(|turn| turn.completion_outcome),
        });
        Ok(())
    }

    pub(super) fn promote_queued_input(
        &mut self, admission_id: &str, previous_turn_id: &str, successor_turn_id: String,
        turn_context_components: Vec<crate::context_engine::ContextComponent>,
        events: &mut Vec<RuntimeEvent>, effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let previous = self.session.execution.active_turn.as_ref()
            .ok_or("successor admission requires a predecessor Turn")?;
        if previous.turn_id.as_str() != previous_turn_id || !previous.status.is_terminal()
            || !self.session.host.pending_effects.is_quiescent()
            || successor_turn_id.trim().is_empty() || successor_turn_id == previous_turn_id {
            return Err("successor admission requires the exact settled predecessor Turn".into());
        }
        let batch = self.session.execution.input_queue.take_successor(admission_id)?;
        self.start_turn(batch.admission.origin, batch.items.clone(), vec![batch], Vec::new(),
            crate::context_engine::ContextPlatform::Shared, turn_context_components, Some(successor_turn_id),
            None, None, events, effects)
    }

    fn enqueue_automatic_memory_extraction(&mut self, turn_id: &TurnId) -> Result<(), String> {
        let policy = &self.session.configuration.memory_policy;
        if !policy.long_term_enabled || !policy.automatic_extraction_enabled {
            return Ok(());
        }
        // A successful explicit memory mutation is already an authoritative,
        // transactional update for this Turn. Scheduling a second model pass
        // over the same evidence would create duplicate or competing records.
        // This gate is based on the normalized tool result, never user text.
        if current_turn_has_explicit_memory_mutation(&self.session.conversation.transcript) {
            return Ok(());
        }

        let extraction_items = current_turn_memory_extraction_items(&self.session, turn_id);
        let extraction_transcript = extraction_items
            .iter()
            .map(|item| super::super::memory_worker::project_memory_worker_transcript_item(item))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if extraction_transcript.is_empty() {
            return Ok(());
        }
        let transcript_digest =
            super::super::memory_worker::memory_worker_transcript_digest(&extraction_transcript)?;
        let source_watermark = format!(
            "session:{}:transcript:{}",
            self.session.session_id.as_str(),
            transcript_digest
        );
        let job_id = format!("memory-extract-{}", &transcript_digest[..24]);
        if self.session.memory_runtime.jobs.contains_key(&job_id) {
            return Ok(());
        }

        let item_ids = extraction_transcript
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>();
        let envelope = crate::agent::memory_runtime::MemoryRuntimeCommandEnvelope {
            operation_id: format!("enqueue-{job_id}"),
            command: crate::agent::memory_runtime::MemoryRuntimeCommand::CreateExtraction {
                job_id,
                input_watermark: source_watermark.clone(),
                sources: vec![crate::agent::memory_runtime::MemorySourceSnapshot {
                    session_id: self.session.session_id.as_str().to_string(),
                    turn_id: Some(turn_id.as_str().to_string()),
                    item_ids,
                    transcript_digest,
                    source_watermark,
                    // Core owns deterministic state transitions, not wall-clock acquisition.
                    // The Host supplies the real clock when it claims the queued job.
                    captured_at_epoch_ms: 0,
                }],
                now_epoch_ms: 0,
            },
        };
        self.session.memory_runtime.apply(envelope)?;
        Ok(())
    }

    pub(super) fn cancel_with_identity(&mut self, cancellation_id: String, reason: Option<String>, events: &mut Vec<RuntimeEvent>, effects: &mut Vec<AgentEffect>) -> Result<(), String> {
        if cancellation_id.trim().is_empty() { return Err("cancellation identity is required".into()); }
        if let Some(turn_id) = self.session.execution.active_turn.as_ref().filter(|turn| !turn.status.is_terminal()).map(|turn| turn.turn_id.clone()) {
            self.session.execution.requirements.cancellation_id = Some(cancellation_id.clone());
            if self.session.execution.requirements.scope.is_some() {
                self.record_result_fact(&turn_id, nola_domain::core_requirements::CoreRuntimeDecisionFactKind::Cancellation {
                    cancellation_id, reason: reason.clone().unwrap_or_else(|| "cancelled by explicit command".into()),
                }, events);
            }
        }
        self.cancel(reason, events, effects)
    }

    pub(super) fn cancel(
        &mut self,
        reason: Option<String>,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let reason = reason
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "cancelled by user".to_string());
        let active_turn = self
            .session
            .execution
            .active_turn
            .as_ref()
            .ok_or_else(|| "no active turn to cancel".to_string())?;
        if active_turn.status == AgentTurnStatus::Cancelling {
            return Err("the active turn is already cancelling".to_string());
        }
        let host_catalog = active_turn
            .context
            .capability_projection
            .frozen_host_catalog();
        let blocking_calls = crate::agent::tool_scheduler::ToolExecutionRuntime::new(
            &active_turn.tools,
            &host_catalog,
        )
        .blocking_interrupts();
        if !blocking_calls.is_empty() {
            return Err(format!(
                "turn interruption is blocked by non-cancellable running tool call(s): {}",
                blocking_calls
                    .iter()
                    .map(|call_id| call_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let pending_effects = self.session.host.pending_effects.iter().collect::<Vec<_>>();
        let facts = pending_effects
            .iter()
            .map(|effect| nola_domain::CancellableEffectFact {
                belongs_to_turn: effect.turn_id.as_deref() == Some(active_turn.turn_id.as_str()),
                is_cancellation_request: effect
                    .request
                    .as_ref()
                    .and_then(|value| {
                        serde_json::from_value::<HostRequestEnvelope>(value.clone()).ok()
                    })
                    .is_some_and(|request| {
                        matches!(
                            request.payload,
                            HostRequest::Cancel { .. } | HostRequest::CancelTask { .. }
                        )
                    }),
                is_cancelling: effect.state == PendingHostEffectState::Cancelling,
            })
            .collect::<Vec<_>>();
        let cancellation = nola_kernel::plan_turn_cancellation(
            &facts,
            active_turn.tools.iter().map(|tool| tool.status),
        );
        let pending_effect_ids = cancellation
            .effect_indices
            .iter()
            .map(|index| pending_effects[*index].effect_id.clone())
            .collect::<Vec<_>>();
        let cell_cancellation_targets = active_turn.code_cells.values()
            .filter(|cell| !cell.is_terminal())
            .map(|cell| {
                let targets = cancellation.effect_indices.iter().filter_map(|index| {
                    let effect = pending_effects[*index];
                    let owned_call = effect.call_id.as_deref()
                        .is_some_and(|id| cell.owns_execution_call(id));
                    let owned_receipt = effect.validated_control().ok().is_some_and(|control| {
                        matches!(&control.payload.policy,
                            nola_domain::host_control::HostResponsePolicy::ResolveCodeTool { cell: owner, .. }
                                | nola_domain::host_control::HostResponsePolicy::PollCodeCell { cell: owner }
                            if owner == cell.identity())
                    });
                    (owned_call || owned_receipt).then(|| effect.effect_id.clone())
                }).collect::<Vec<_>>();
                (cell.identity().cell_id.clone(), targets)
            }).collect::<std::collections::BTreeMap<_, _>>();
        let cancellable_tools = cancellation
            .tool_indices
            .iter()
            .map(|index| {
                let tool = &active_turn.tools[*index];
                (tool.call_id.clone(), tool.name.clone())
            })
            .collect::<Vec<_>>();
        let owner_turn_id = active_turn.turn_id.clone();
        let cancelled_results = cancellable_tools
            .iter()
            .map(|(call_id, name)| AgentItem::ToolResult {
                id: ItemId::from_string(self.session.allocate_id("tool-result")),
                call_id: call_id.clone(),
                name: name.clone(),
                // Cancellation is an execution fact, never an authorization
                // denial. Keep its cause in model context as well as Trace.
                output: serde_json::json!({
                    "status": "cancelled",
                    "reason": reason,
                    "permission_decision": null,
                    "instruction": "This call was interrupted, not denied. Preserve earlier completed results; cancellation does not revoke existing grants."
                }),
                is_error: true,
                full_output_ref: None,
                artifacts: Vec::new(),
            })
            .collect::<Vec<_>>();
        // Promoted background work no longer owns a pending spawn effect.
        // Cancel only this Turn's tasks; unrelated prior-Turn work is retained.
        let task_ids = self.session.task_projection.tasks.values()
            .filter(|task| task.owner_turn_id == owner_turn_id && !task.state.is_terminal() && !task.cancel_requested)
            .map(|task| task.id.clone()).collect::<Vec<_>>();
        let mut task_cancellations = 0;
        for task_id in task_ids {
            for (task_projection_revision, task) in self.session.task_projection.request_cancellation_tree(&task_id)? {
                events.push(RuntimeEvent::TaskUpdated { task_projection_revision, task: task.clone() });
                effects.push(AgentEffect::CancelTask { turn_id: task.owner_turn_id, task_id: task.id });
                task_cancellations += 1;
            }
        }
        let turn = self
            .session
            .execution
            .active_turn
            .as_mut()
            .ok_or_else(|| "no active turn to cancel".to_string())?;
        for index in cancellation.tool_indices {
            turn.tools[index].status = ToolCallStatus::Cancelled;
        }
        let waits_for_host_cancellation = !pending_effect_ids.is_empty() || task_cancellations > 0;
        for cell in turn.code_cells.values_mut().filter(|cell| !cell.is_terminal()) {
            let targets = cell_cancellation_targets.get(&cell.identity().cell_id)
                .ok_or("missing code cell cancellation plan")?;
            cell.track_cancellation(&cell.identity().clone(), targets)?;
        }
        if !waits_for_host_cancellation { turn.confirm_code_cells_cancelled()?; }
        effects.extend(pending_effect_ids.into_iter().map(|target_effect_id| {
            AgentEffect::CancelHostEffect {
                turn_id: turn.turn_id.clone(),
                target_effect_id,
            }
        }));
        turn.status = if waits_for_host_cancellation {
            AgentTurnStatus::Cancelling
        } else {
            AgentTurnStatus::Cancelled
        };
        turn.cancellation_reason = waits_for_host_cancellation.then_some(reason.clone());
        let turn_id = turn.turn_id.clone();
        self.withdraw_user_input_waits(&reason, events);
        let admission_ids = self.session.execution.input_queue.withdraw_current(turn_id.as_str(), &reason);
        if !admission_ids.is_empty() {
            events.push(RuntimeEvent::InputWithdrawn { turn_id: turn_id.clone(), admission_ids, reason: reason.clone() });
        }
        self.session
            .execution
            .permissions
            .interrupt_turn(&turn_id, &reason);
        if !waits_for_host_cancellation {
            events.push(RuntimeEvent::TurnCancelled {
                session_id: self.session.session_id.clone(),
                turn_id,
                reason,
            });
        }
        self.session.conversation.context_window.cancel_compaction();
        self.session.record_items(cancelled_results);
        Ok(())
    }

    pub(super) fn require_turn(
        &self,
        turn_id: &TurnId,
        expected: AgentTurnStatus,
    ) -> Result<&AgentTurnState, String> {
        let turn = self
            .session
            .execution
            .active_turn
            .as_ref()
            .ok_or_else(|| "no active turn".to_string())?;
        if &turn.turn_id != turn_id {
            return Err("turn id does not match the active turn".to_string());
        }
        if turn.status != expected {
            return Err(format!(
                "turn is in {:?}, expected {:?}",
                turn.status, expected
            ));
        }
        Ok(turn)
    }

    pub(super) fn require_turn_mut(
        &mut self,
        turn_id: &TurnId,
        expected: AgentTurnStatus,
    ) -> Result<&mut AgentTurnState, String> {
        let turn = self
            .session
            .execution
            .active_turn
            .as_mut()
            .ok_or_else(|| "no active turn".to_string())?;
        if &turn.turn_id != turn_id {
            return Err("turn id does not match the active turn".to_string());
        }
        if turn.status != expected {
            return Err(format!(
                "turn is in {:?}, expected {:?}",
                turn.status, expected
            ));
        }
        Ok(turn)
    }

    pub(super) fn emit_context_update(&mut self, events: &mut Vec<RuntimeEvent>) {
        let context_usage = self.session.conversation.context_window.usage_snapshot();
        let prompt_cache_measurement = self.session.conversation.context_window.prompt_cache_measurement.clone();
        let measurement = self.session.conversation.context_window.measurement.clone();
        let prompt_cache_diagnostic = self
            .session
            .conversation
            .context_window
            .cumulative_usage
            .prompt_cache_diagnostic
            .clone().filter(|_| prompt_cache_measurement.is_some());
        let fingerprint = crate::agent::session::AgentContextUsageFingerprint {
            measurement: measurement.clone(),
            prompt_cache_measurement: prompt_cache_measurement.clone(),
            cache_usage: context_usage.cache_usage,
            used_tokens: context_usage.used_tokens,
            context_window_tokens: context_usage.context_window_tokens,
            usage_is_estimated: context_usage.usage_is_estimated,
            cached_input_tokens: context_usage.cached_input_tokens,
            cached_input_tokens_reported: context_usage.cached_input_tokens_reported,
            cache_write_input_tokens: context_usage.cache_write_input_tokens,
            cache_write_input_tokens_reported: context_usage.cache_write_input_tokens_reported,
            prompt_cache_diagnostic: prompt_cache_diagnostic.clone(),
        };
        if self.session.journal.last_context_usage_fingerprint.as_ref() == Some(&fingerprint) {
            return;
        }
        self.session.journal.last_context_usage_fingerprint = Some(fingerprint.clone());
        events.push(RuntimeEvent::ContextUpdated {
            measurement,
            prompt_cache_measurement,
            used_tokens: fingerprint.used_tokens,
            context_window_tokens: fingerprint.context_window_tokens,
            usage_percent: context_usage.usage_percent,
            usage_is_estimated: fingerprint.usage_is_estimated,
            context_usage,
            prompt_cache_diagnostic,
        });
    }
}

fn current_turn_has_explicit_memory_mutation(
    transcript: &[crate::agent::protocol::AgentItem],
) -> bool {
    let current_turn_start = transcript
        .iter()
        .rposition(|item| item.is_user_input())
        .unwrap_or(0);

    transcript[current_turn_start..].iter().any(|item| {
        matches!(
            item,
            crate::agent::protocol::AgentItem::ToolResult {
                name,
                is_error: false,
                ..
            } if matches!(name.as_str(), "memory_save" | "memory_delete")
        )
    })
}

fn current_turn_memory_extraction_items<'a>(
    session: &'a AgentThreadState,
    turn_id: &TurnId,
) -> Vec<&'a AgentItem> {
    session
        .conversation
        .transcript
        .iter()
        .filter(|item| {
            session
                .item_ownership
                .get(item.id().as_str())
                .is_some_and(|owner| owner.turn_id == turn_id.as_str())
        })
        .collect()
}

#[cfg(test)]
mod automatic_memory_extraction_tests {
    use super::{current_turn_has_explicit_memory_mutation, current_turn_memory_extraction_items};
    use crate::agent::protocol::{AgentItem, CallId, ItemId, SessionId, TurnId};
    use crate::agent::{
        AgentSessionConfiguration, AgentThreadState, AgentTurnIdentity, ContextWindowConfig,
    };
    use serde_json::json;

    fn memory_result(id: &str, is_error: bool) -> AgentItem {
        AgentItem::ToolResult {
            id: ItemId::from_string(format!("result-{id}")),
            call_id: CallId::from_string(format!("call-{id}")),
            name: "memory_save".to_string(),
            output: json!({"saved": !is_error}),
            is_error,
            full_output_ref: None,
            artifacts: Vec::new(),
        }
    }

    #[test]
    fn successful_explicit_write_suppresses_same_turn_extraction() {
        let transcript = vec![
            AgentItem::user_message(
                ItemId::from_string("user-current".to_string()),
                "Please remember this preference.",
            ),
            memory_result("current", false),
        ];
        assert!(current_turn_has_explicit_memory_mutation(&transcript));
    }

    #[test]
    fn failed_explicit_write_does_not_suppress_extraction() {
        let transcript = vec![
            AgentItem::user_message(
                ItemId::from_string("user-current".to_string()),
                "Please remember this preference.",
            ),
            memory_result("current", true),
        ];
        assert!(!current_turn_has_explicit_memory_mutation(&transcript));
    }

    #[test]
    fn earlier_turn_write_does_not_suppress_the_current_turn() {
        let transcript = vec![
            AgentItem::user_message(
                ItemId::from_string("user-earlier".to_string()),
                "Remember the earlier preference.",
            ),
            memory_result("earlier", false),
            AgentItem::user_message(
                ItemId::from_string("user-current".to_string()),
                "This turn contains different durable evidence.",
            ),
            AgentItem::assistant_message(
                ItemId::from_string("assistant-current".to_string()),
                "Acknowledged.",
            ),
        ];
        assert!(!current_turn_has_explicit_memory_mutation(&transcript));
    }

    #[test]
    fn automatic_extraction_freezes_only_the_completed_turn_items() {
        let mut session = AgentThreadState::new(
            SessionId::from_string("session".to_string()),
            AgentSessionConfiguration {
                model: None,
                max_model_iterations: 0,
                max_strategy_recovery_attempts: 3,
                tools: Vec::new(),
                context_window: ContextWindowConfig::default(),
                memory_policy: crate::memory::MemoryPolicy::default(),
                turn_identity: AgentTurnIdentity::default(),
            },
        );
        let earlier = TurnId::from_string("turn-earlier".to_string());
        let current = TurnId::from_string("turn-current".to_string());
        session.record_turn_items(
            &earlier,
            [
                AgentItem::user_message(
                    ItemId::from_string("user-earlier".to_string()),
                    "My old preference was verbose output.",
                ),
                AgentItem::assistant_message(
                    ItemId::from_string("assistant-earlier".to_string()),
                    "Stored the old preference.",
                ),
            ],
        );
        session.record_turn_items(
            &current,
            [
                AgentItem::user_message(
                    ItemId::from_string("user-current".to_string()),
                    "I now prefer concise output.",
                ),
                AgentItem::assistant_message(
                    ItemId::from_string("assistant-current".to_string()),
                    "Acknowledged.",
                ),
            ],
        );

        let item_ids = current_turn_memory_extraction_items(&session, &current)
            .into_iter()
            .map(|item| item.id().as_str())
            .collect::<Vec<_>>();
        assert_eq!(item_ids, vec!["user-current", "assistant-current"]);
    }
}
