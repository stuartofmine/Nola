// SPDX-License-Identifier: Apache-2.0
// Internal layer of the deterministic agent runtime. Public clients use AgentRuntime.

use super::*;
use crate::agent::host::{HostCapabilityOrigin, HostToolOutcome};
use crate::agent::runtime_event::ItemDiscardDisposition;

// Long public Turn/call identities must not overflow canonical Part ownership.
// Preserve existing short IDs so restoration keeps finding already-published Parts.
pub(super) fn bounded_tool_part_identity(identity: String) -> String {
    if identity.len() <= 128 { return identity; }
    use sha2::{Digest, Sha256};
    format!("tool:sha256:{:x}", Sha256::digest(identity.as_bytes()))
}

fn projected_tool_identity(
    session: &AgentThreadState,
    capability_id: &str,
) -> (String, HostCapabilityOrigin) {
    // A capability discovered during the Turn (for example an activated
    // Skill or MCP tool) belongs to that Turn's frozen catalog. The mutable
    // session catalog intentionally does not acquire transient capabilities,
    // so consulting it first loses the Host-declared title and provenance in
    // the durable Part even though execution used the correct descriptor.
    if let Some(descriptor) = session.execution.active_turn.as_ref().and_then(|turn| {
        turn.context
            .capability_projection
            .frozen_host_catalog()
            .resolve_available(capability_id)
            .cloned()
    }) {
        return (descriptor.title, descriptor.input_normalization.origin);
    }
    session
        .host
        .catalog
        .resolve_available(capability_id)
        .map(|descriptor| {
            (
                descriptor.title.clone(),
                descriptor.input_normalization.origin.clone(),
            )
        })
        .unwrap_or_else(|| (capability_id.to_string(), HostCapabilityOrigin::default()))
}
use crate::agent::parts::{
    CitationPart, CitationRange, PartOwnership, SourceEvidence, SourceLocation, SourcePart,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
struct SourceCandidate {
    title: String,
    uri: String,
    mime_type: Option<String>,
    location: Option<SourceLocation>,
    range: Option<CitationRange>,
}

fn discarded_process_part(
    part: Option<&AgentPart>,
    disposition: ItemDiscardDisposition,
) -> Option<(PartOwnership, String)> {
    if disposition != ItemDiscardDisposition::Process {
        return None;
    }
    match part {
        Some(AgentPart::Text(part)) if !part.text.trim().is_empty() => {
            Some((part.ownership.clone(), part.text.trim().to_string()))
        }
        _ => None,
    }
}

fn provisional_process_reasoning_part(
    item_id: &ItemId,
    ownership: PartOwnership,
    detail: String,
) -> AgentPart {
    AgentPart::Reasoning(ReasoningPart {
        id: format!("{}:process", item_id.as_str()),
        ownership,
        title: "thinking".to_string(),
        detail,
        // ModelCompleted owns the authoritative completed reasoning item. Keep
        // this converted provider preamble open so that completion extends the
        // same Part instead of creating a second copy beside it.
        is_complete: false,
    })
}

fn reasoning_completion_matches(part: &ReasoningPart, turn_id: &TurnId, content: &str) -> bool {
    part.ownership.turn_id == turn_id.as_str()
        && (part.detail == content
            || part.detail.is_empty()
            || (!part.is_complete && content.starts_with(&part.detail)))
}

fn reasoning_item_has_visible_content(summary: &str, content: &str) -> bool {
    !summary.trim().is_empty() || !content.trim().is_empty()
}

impl AgentRuntime {
    /// A completed response closes its reasoning even if its provider aggregate
    /// did not match a provisional preamble. Tool execution and the next model
    /// request belong to subsequent phases, not to this still-spinning document.
    pub(super) fn complete_model_reasoning_parts(
        &mut self,
        turn_id: &TurnId,
        events: &mut Vec<RuntimeEvent>,
    ) -> Result<(), String> {
        let ids = self.session.conversation.projection.order.iter().filter_map(|id| {
            match self.session.conversation.projection.parts.get(id) {
                Some(AgentPart::Reasoning(part))
                    if part.ownership.turn_id == turn_id.as_str() && !part.is_complete => Some(id.clone()),
                _ => None,
            }
        }).collect::<Vec<_>>();
        for part_id in ids {
            let mut updates = Vec::new();
            self.apply_part(PartUpdate::Completed { part_id }, &mut updates)?;
            for (update, part) in updates {
                events.push(RuntimeEvent::PartUpdated {
                    turn_id: turn_id.clone(),
                    projection_revision: self.session.conversation.projection.revision,
                    update, part,
                });
            }
        }
        Ok(())
    }

    fn close_empty_text_parts_before_tool(
        &mut self,
        turn_id: &TurnId,
        updates: &mut Vec<(PartUpdate, AgentPart)>,
    ) -> Result<(), String> {
        let part_ids = self
            .session
            .conversation
            .projection
            .order
            .iter()
            .filter_map(
                |part_id| match self.session.conversation.projection.parts.get(part_id) {
                    Some(AgentPart::Text(part))
                        if part.ownership.turn_id == turn_id.as_str()
                            && !part.is_complete
                            && part.text.is_empty() =>
                    {
                        Some(part_id.clone())
                    }
                    _ => None,
                },
            )
            .collect::<Vec<_>>();
        for part_id in part_ids {
            self.apply_part(PartUpdate::Completed { part_id }, updates)?;
        }
        Ok(())
    }

    /// Closes every visible nonterminal Part owned by a terminal Turn.
    ///
    /// This is the projection-side terminal invariant: a cancelled or failed
    /// Turn cannot leave a text stream, reasoning stream, or tool call in a
    /// running state even when the Host response races the cancellation.
    pub(super) fn terminate_turn_parts(
        &mut self,
        turn_id: &TurnId,
        tool_state: ToolPartState,
        events: &mut Vec<RuntimeEvent>,
    ) -> Result<(), String> {
        let part_ids = self
            .session
            .conversation
            .projection
            .order
            .iter()
            .filter_map(|part_id| {
                self.session
                    .conversation
                    .projection
                    .parts
                    .get(part_id)
                    .filter(|part| part.turn_id() == turn_id.as_str() && !part.is_terminal())
                    .map(|_| part_id.clone())
            })
            .collect::<Vec<_>>();

        for part_id in part_ids {
            let unknown_outcome = tool_state == ToolPartState::Completed
                && matches!(self.session.conversation.projection.parts.get(&part_id), Some(AgentPart::Tool(part))
                    if self.session.host.pending_effects.iter().any(|effect|
                        effect.turn_id.as_deref() == Some(turn_id.as_str())
                            && effect.call_id.as_deref() == Some(part.call_id.as_str())));
            let update = match self.session.conversation.projection.parts.get(&part_id) {
                Some(AgentPart::Tool(_)) => PartUpdate::ToolState {
                    part_id: part_id.clone(),
                    // A terminal Turn is not a receipt for an outstanding Host
                    // operation. Close its presentation without asserting success.
                    state: if unknown_outcome {
                        ToolPartState::Sealed
                    } else { tool_state },
                    title: None,
                    summary: if unknown_outcome {
                        Some("Execution outcome unknown; presentation sealed without a completion receipt.".into())
                    } else { None },
                    output_preview: None,
                    structured_output: None,
                    full_output_ref: None,
                    artifacts: Vec::new(),
                    timestamp_ms: Some(self.observed_at_ms),
                },
                Some(AgentPart::Text(_) | AgentPart::Reasoning(_)) => PartUpdate::Completed {
                    part_id: part_id.clone(),
                },
                Some(
                    AgentPart::Source(_)
                    | AgentPart::Citation(_)
                    | AgentPart::Artifact(_)
                    | AgentPart::Step(_)
                    | AgentPart::Compaction(_),
                ) => continue,
                None => continue,
            };
            let mut updates = Vec::new();
            self.apply_part(update, &mut updates)?;
            for (update, part) in updates {
                events.push(RuntimeEvent::PartUpdated {
                    turn_id: turn_id.clone(),
                    projection_revision: self.session.conversation.projection.revision,
                    update,
                    part,
                });
            }
        }
        Ok(())
    }

    pub(super) fn project_events(&mut self, events: &mut Vec<RuntimeEvent>) -> Result<(), String> {
        let source_events = std::mem::take(events);
        // Host completions retain their richer outcome (including cancellation).
        // Core control tools have only ItemCompleted/ToolResult; they must not
        // remain pending until turn closure fabricates a successful completion.
        let host_completions: std::collections::BTreeSet<_> = source_events.iter().filter_map(|event|
            match event { RuntimeEvent::ToolCompleted { call_id, .. } => Some(call_id.as_str().to_owned()), _ => None }).collect();
        let mut visible_events = Vec::new();
        for event in source_events {
            let terminal = match &event {
                RuntimeEvent::TurnCompleted { turn_id, .. } => {
                    Some((turn_id.clone(), ToolPartState::Completed))
                }
                RuntimeEvent::TurnFailed { turn_id, .. } => {
                    Some((turn_id.clone(), ToolPartState::Failed))
                }
                RuntimeEvent::TurnCancelled { turn_id, .. } => {
                    Some((turn_id.clone(), ToolPartState::Cancelled))
                }
                _ => None,
            };
            if let Some((turn_id, tool_state)) = terminal {
                // Terminal lifecycle events are reduced only after every
                // preceding Item event in this transition has reached the
                // canonical Part projection. Closing earlier snapshots can
                // otherwise race a terminal provider aggregate and overwrite
                // an append-only stream with stale empty content.
                self.terminate_turn_parts(&turn_id, tool_state, &mut visible_events)?;
                visible_events.push(event);
                continue;
            }
            let activity = match &event {
                RuntimeEvent::TaskUpdated { task, .. } => Some(("task", task.id.as_str(), task.owner_turn_id.as_str())),
                RuntimeEvent::SkillRuntimeUpdated { update: crate::agent::capability::SkillRuntimeEvent::Started { run }, .. } =>
                    Some(("skill", run.run_id.as_str(), run.owner.parent_turn_id.as_str())),
                _ => None,
            };
            if let Some((kind,id,turn)) = activity {
                let key = format!("{kind}:{id}");
                if !self.session.conversation.projection.activity_message_owners.contains_key(&key) {
                    let owner = self.session.conversation.projection.active_message_segments.get(turn).cloned()
                        .unwrap_or_else(|| PartOwnership::for_turn(turn).message_id);
                    self.session.conversation.projection.activity_message_owners.insert(key,owner.clone());
                    visible_events.push(RuntimeEvent::AssistantActivityBound {
                        turn_id:TurnId::from_string(turn.into()), assistant_message_id:owner,
                        activity_id:id.into(), activity_kind:kind.into(),
                    });
                }
            }
            let mut updates = Vec::new();
            match &event {
                RuntimeEvent::AssistantSegmentStarted { turn_id, assistant_message_id, .. } => {
                    self.session.conversation.projection.active_message_segments
                        .insert(turn_id.as_str().into(), assistant_message_id.clone());
                }
                RuntimeEvent::TurnContextCaptured {
                    turn_id,
                    memory_usage,
                    ..
                } => {
                    for usage in memory_usage {
                        let part_id =
                            format!("{}:memory-source:{}", turn_id.as_str(), usage.component_id);
                        if self
                            .session
                            .conversation
                            .projection
                            .parts
                            .contains_key(&part_id)
                        {
                            continue;
                        }
                        let title = usage
                            .title
                            .as_deref()
                            .map(str::trim)
                            .filter(|title| !title.is_empty())
                            .or_else(|| {
                                usage
                                    .source
                                    .as_deref()
                                    .map(str::trim)
                                    .filter(|source| !source.is_empty())
                            })
                            .unwrap_or("Long-term memory")
                            .to_string();
                        self.apply_part(
                            PartUpdate::Created {
                                part: AgentPart::Source(SourcePart {
                                    id: part_id,
                                    ownership: PartOwnership::for_turn(turn_id.as_str()),
                                    source_id: format!("memory:{}", usage.record_id),
                                    title,
                                    uri: format!("nola-memory://record/{}", usage.record_id),
                                    mime_type: Some("application/x-nola-memory".to_string()),
                                    evidence: Some(SourceEvidence::Memory {
                                        usage: usage.clone(),
                                    }),
                                }),
                            },
                            &mut updates,
                        )?;
                    }
                }
                RuntimeEvent::ItemStarted {
                    turn_id,
                    item_id,
                    kind,
                } => {
                    let id = item_id.as_str().to_string();
                    if self.session.conversation.projection.parts.contains_key(&id) {
                        continue;
                    }
                    let part = match kind {
                        AgentItemDeltaKind::AssistantText => AgentPart::Text(TextPart {
                            review: self.pending_text_review(),
                            id,
                            ownership: PartOwnership::for_turn(turn_id.as_str()),
                            text: String::new(),
                            is_complete: false,
                        }),
                        _ => AgentPart::Reasoning(ReasoningPart {
                            id,
                            ownership: PartOwnership::for_turn(turn_id.as_str()),
                            title: "reasoning".to_string(),
                            detail: String::new(),
                            is_complete: false,
                        }),
                    };
                    self.apply_part(PartUpdate::Created { part }, &mut updates)?;
                }
                RuntimeEvent::ItemDelta {
                    turn_id,
                    item_id,
                    kind,
                    delta,
                    presentation_committed,
                    ..
                } => {
                    if !self
                        .session
                        .conversation
                        .projection
                        .parts
                        .contains_key(item_id.as_str())
                    {
                        let part = match kind {
                            AgentItemDeltaKind::AssistantText => AgentPart::Text(TextPart {
                                review: self.pending_text_review(),
                                id: item_id.as_str().to_string(),
                                ownership: PartOwnership::for_turn(turn_id.as_str()),
                                text: String::new(),
                                is_complete: false,
                            }),
                            _ => AgentPart::Reasoning(ReasoningPart {
                                id: item_id.as_str().to_string(),
                                ownership: PartOwnership::for_turn(turn_id.as_str()),
                                title: "reasoning".to_string(),
                                detail: String::new(),
                                is_complete: false,
                            }),
                        };
                        self.apply_part(PartUpdate::Created { part }, &mut updates)?;
                    }
                    let update = match kind {
                        AgentItemDeltaKind::AssistantText if *presentation_committed => {
                            PartUpdate::TextCommittedDelta {
                                part_id: item_id.as_str().to_string(),
                                delta: delta.clone(),
                            }
                        }
                        AgentItemDeltaKind::AssistantText => PartUpdate::TextDelta {
                            part_id: item_id.as_str().to_string(),
                            delta: delta.clone(),
                        },
                        AgentItemDeltaKind::ReasoningSummary => PartUpdate::ReasoningDelta {
                            part_id: item_id.as_str().to_string(),
                            title: None,
                            detail_delta: delta.clone(),
                        },
                        AgentItemDeltaKind::ReasoningContent => PartUpdate::ReasoningDelta {
                            part_id: item_id.as_str().to_string(),
                            title: None,
                            detail_delta: delta.clone(),
                        },
                    };
                    self.apply_part(update, &mut updates)?;
                }
                RuntimeEvent::ItemDiscarded {
                    item_id,
                    disposition,
                    ..
                } => {
                    let existing_part = self
                        .session
                        .conversation
                        .projection
                        .parts
                        .get(item_id.as_str());
                    let provisional_process = discarded_process_part(existing_part, *disposition);
                    let discard_update = existing_part.map(|part| match (part, disposition) {
                        (AgentPart::Text(part), _) if part.review == Some(super::super::parts::TextReviewState::Pending) => PartUpdate::TextReviewChanged {
                            part_id: item_id.as_str().to_string(),
                            state: super::super::parts::TextReviewState::Interrupted,
                        },
                        (AgentPart::Text(part), _) if part.review.is_some() => PartUpdate::Completed {
                            part_id: item_id.as_str().to_string(),
                        },
                        (AgentPart::Text(_), _) => PartUpdate::TextReplaced {
                            part_id: item_id.as_str().to_string(),
                            text: String::new(),
                            is_complete: true,
                        },
                        (AgentPart::Reasoning(_), ItemDiscardDisposition::Hidden) => {
                            PartUpdate::ReasoningReplaced {
                                part_id: item_id.as_str().to_string(),
                                title: String::new(),
                                detail: String::new(),
                                is_complete: true,
                            }
                        }
                        (AgentPart::Tool(_), ItemDiscardDisposition::Hidden) => {
                            PartUpdate::ToolInputReplaced {
                                part_id: item_id.as_str().to_string(),
                                input: Value::Null,
                                raw_input: String::new(),
                                state: ToolPartState::Cancelled,
                            }
                        }
                        _ => PartUpdate::Completed {
                            part_id: item_id.as_str().to_string(),
                        },
                    });
                    if let Some((ownership, detail)) = provisional_process.as_ref() {
                        let process_part_id = format!("{}:process", item_id.as_str());
                        if !self
                            .session
                            .conversation
                            .projection
                            .parts
                            .contains_key(&process_part_id)
                        {
                            self.apply_part(
                                PartUpdate::Created {
                                    part: provisional_process_reasoning_part(
                                        item_id,
                                        ownership.clone(),
                                        detail.clone(),
                                    ),
                                },
                                &mut updates,
                            )?;
                        }
                    }
                    if let Some(update) = discard_update {
                        self.apply_part(update, &mut updates)?;
                    }
                }
                RuntimeEvent::ItemCompleted { turn_id, item } => {
                    if let AgentItem::ToolResult { call_id, name, output, is_error, full_output_ref, artifacts, .. } = item {
                        if !host_completions.contains(call_id.as_str()) {
                            let part_id = self.latest_tool_part_id(call_id)
                                .unwrap_or_else(|| Self::tool_part_id(turn_id, call_id));
                            self.complete_tool_part(turn_id, &part_id, call_id.as_str(), name, output,
                                if *is_error { ToolPartState::Failed } else { ToolPartState::Completed },
                                full_output_ref.clone(), artifacts, &mut updates)?;
                        }
                    } else {
                        self.project_completed_item(turn_id, item, &mut updates)?;
                    }
                }
                RuntimeEvent::ToolInputDelta {
                    turn_id,
                    part_id,
                    call_id,
                    capability_id,
                    arguments_delta,
                } => {
                    self.close_empty_text_parts_before_tool(turn_id, &mut updates)?;
                    let (tool_title, tool_origin) =
                        projected_tool_identity(&self.session, capability_id);
                    if !self
                        .session
                        .conversation
                        .projection
                        .parts
                        .contains_key(part_id)
                    {
                        self.apply_part(
                            PartUpdate::Created {
                                part: AgentPart::Tool(ToolPart {
                                    execution_source: self.session.tool_record_source(Some(turn_id), call_id.as_str()),
                                    id: part_id.clone(),
                                    ownership: PartOwnership::for_turn(turn_id.as_str()),
                                    call_id: call_id.as_str().to_string(),
                                    capability_id: capability_id.clone(),
                                    title: tool_title,
                                    summary: None,
                                    origin: tool_origin,
                                    state: ToolPartState::Pending,
                                    input: Value::Null,
                                    raw_input: String::new(),
                                    permission: None,
                                    output_preview: None,
                                    structured_output: None,
                                    full_output_ref: None,
                                    artifacts: Vec::new(),
                                    started_at_ms: None,
                                    finished_at_ms: None,
                                }),
                            },
                            &mut updates,
                        )?;
                    }
                    self.apply_part(
                        PartUpdate::ToolInputDelta {
                            part_id: part_id.clone(),
                            delta: arguments_delta.clone(),
                        },
                        &mut updates,
                    )?;
                }
                RuntimeEvent::PermissionRequested {
                    turn_id,
                    request_id,
                    call_id,
                    tool_name,
                    arguments,
                    permission_domain,
                    rationale,
                } => {
                    let part_id = self
                        .latest_tool_part_id(call_id)
                        .unwrap_or_else(|| Self::tool_part_id(turn_id, call_id));
                    let permission = ToolPermissionPart {
                        request_id: request_id.as_str().to_string(),
                        permission_domain: permission_domain.clone(),
                        rationale: rationale.clone(),
                        decision: None,
                    };
                    let (tool_title, tool_origin) =
                        projected_tool_identity(&self.session, tool_name);
                    if !self
                        .session
                        .conversation
                        .projection
                        .parts
                        .contains_key(&part_id)
                    {
                        self.apply_part(
                            PartUpdate::Created {
                                part: AgentPart::Tool(ToolPart {
                                    execution_source: self.session.tool_record_source(Some(turn_id), call_id.as_str()),
                                    id: part_id.clone(),
                                    ownership: PartOwnership::for_turn(turn_id.as_str()),
                                    call_id: call_id.as_str().to_string(),
                                    capability_id: tool_name.clone(),
                                    title: tool_title,
                                    summary: None,
                                    origin: tool_origin,
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
                            },
                            &mut updates,
                        )?;
                    } else {
                        self.apply_part(
                            PartUpdate::ToolInputResolved {
                                part_id: part_id.clone(),
                                input: arguments.clone(),
                            },
                            &mut updates,
                        )?;
                    }
                    // A nested call may first appear at its permission gate,
                    // without streamed model arguments. Publish the same
                    // permission transition as an already-created tool so
                    // Host transports receive its canonical challenge trace.
                    self.apply_part(
                        PartUpdate::ToolPermission {
                            part_id,
                            permission,
                        },
                        &mut updates,
                    )?;
                }
                RuntimeEvent::PermissionResolved {
                    turn_id,
                    request_id,
                    call_id,
                    decision,
                } => {
                    let receipt = self.session.execution.permissions.receipts.iter()
                        .find(|receipt| receipt.request_id == request_id.as_str())
                        .filter(|receipt| receipt.turn_id == turn_id.as_str() && receipt.call_id == call_id.as_str())
                        .cloned().ok_or("permission settlement lacks its original Core receipt")?;
                    visible_events.push(RuntimeEvent::PermissionSettled { receipt });
                    let part_id = self
                        .latest_tool_part_id(call_id)
                        .unwrap_or_else(|| Self::tool_part_id(turn_id, call_id));
                    let current = self
                        .session
                        .conversation
                        .projection
                        .parts
                        .get(&part_id)
                        .and_then(|part| match part {
                            AgentPart::Tool(tool) => tool.permission.clone(),
                            _ => None,
                        })
                        .ok_or_else(|| {
                            format!(
                                "permission {} has no projected Tool Part",
                                request_id.as_str()
                            )
                        })?;
                    self.apply_part(
                        PartUpdate::ToolPermission {
                            part_id,
                            permission: ToolPermissionPart {
                                request_id: request_id.as_str().to_string(),
                                permission_domain: current.permission_domain,
                                rationale: current.rationale,
                                decision: Some(decision.as_str().to_string()),
                            },
                        },
                        &mut updates,
                    )?;
                }
                RuntimeEvent::ToolStarted {
                    turn_id,
                    call_id,
                    tool_name,
                    arguments,
                } => {
                    self.close_empty_text_parts_before_tool(turn_id, &mut updates)?;
                    let part_id = self
                        .latest_tool_part_id(call_id)
                        .unwrap_or_else(|| Self::tool_part_id(turn_id, call_id));
                    self.ensure_tool_part(
                        turn_id,
                        &part_id,
                        call_id.as_str(),
                        tool_name,
                        Some(arguments),
                        ToolPartState::Running,
                        &mut updates,
                    )?;
                }
                RuntimeEvent::UserInputWaitChanged { wait } if wait.is_open() && wait.originating_call_id.is_some() => {
                    let turn_id = TurnId::from_string(wait.turn_id.clone());
                    let call_id = CallId::from_string(wait.originating_call_id.clone().expect("tool wait"));
                    let part_id = self.latest_tool_part_id(&call_id).unwrap_or_else(|| Self::tool_part_id(&turn_id, &call_id));
                    let arguments = serde_json::to_value(&wait.question_set).map_err(|error| error.to_string())?;
                    self.ensure_tool_part(&turn_id, &part_id, call_id.as_str(), "request_user_input",
                        Some(&arguments), ToolPartState::WaitingForInput, &mut updates)?;
                }
                RuntimeEvent::ToolCallProgress {
                    turn_id,
                    call_id,
                    title,
                    detail,
                    delta,
                } => {
                    let part_id = self
                        .latest_tool_part_id(call_id)
                        .unwrap_or_else(|| Self::tool_part_id(turn_id, call_id));
                    if !self
                        .session
                        .conversation
                        .projection
                        .parts
                        .contains_key(&part_id)
                    {
                        self.ensure_tool_part(
                            turn_id,
                            &part_id,
                            call_id.as_str(),
                            title.as_deref().unwrap_or("tool"),
                            None,
                            ToolPartState::Running,
                            &mut updates,
                        )?;
                    }
                    self.apply_part(
                        PartUpdate::ToolProgress {
                            part_id,
                            title: title.clone(),
                            detail: detail.clone(),
                            output_delta: delta.clone(),
                        },
                        &mut updates,
                    )?;
                }
                RuntimeEvent::ToolCompleted {
                    turn_id,
                    call_id,
                    tool_name,
                    output,
                    outcome,
                    full_output_ref,
                    artifacts,
                } => {
                    let part_id = self
                        .latest_tool_part_id(call_id)
                        .unwrap_or_else(|| Self::tool_part_id(turn_id, call_id));
                    self.complete_tool_part(
                        turn_id,
                        &part_id,
                        call_id.as_str(),
                        tool_name,
                        output,
                        match outcome {
                            HostToolOutcome::Succeeded | HostToolOutcome::Deferred => ToolPartState::Completed,
                            HostToolOutcome::Failed => ToolPartState::Failed,
                            HostToolOutcome::Cancelled => ToolPartState::Cancelled,
                        },
                        full_output_ref.clone(),
                        artifacts,
                        &mut updates,
                    )?;
                }
                RuntimeEvent::TextReviewChanged { item_id, state, .. } => {
                    self.apply_part(PartUpdate::TextReviewChanged {
                        part_id: item_id.as_str().to_string(), state: *state,
                    }, &mut updates)?;
                }
                _ => {}
            }
            if updates.is_empty() {
                if !Self::is_internal_part_event(&event) {
                    visible_events.push(event);
                }
                continue;
            }
            let turn_id = match &event {
                RuntimeEvent::UserInputWaitChanged { wait } => Some(TurnId::from_string(wait.turn_id.clone())),
                _ => Self::runtime_event_turn_id(&event).cloned(),
            }
                .ok_or_else(|| "Part update is missing its owning Turn".to_string())?;
            for (update, part) in updates {
                visible_events.push(RuntimeEvent::PartUpdated {
                    turn_id: turn_id.clone(),
                    projection_revision: self.session.conversation.projection.revision,
                    update,
                    part,
                });
            }
            if matches!(event, RuntimeEvent::UserInputWaitChanged { .. }) { visible_events.push(event); }
        }
        *events = visible_events;
        Ok(())
    }

    fn is_internal_part_event(event: &RuntimeEvent) -> bool {
        matches!(
            event,
            RuntimeEvent::ItemStarted { .. }
                | RuntimeEvent::TextReviewChanged { .. }
                | RuntimeEvent::ItemDelta { .. }
                | RuntimeEvent::ItemDiscarded { .. }
                | RuntimeEvent::ItemCompleted { .. }
                | RuntimeEvent::ToolInputDelta { .. }
                | RuntimeEvent::PermissionRequested { .. }
                | RuntimeEvent::PermissionResolved { .. }
                | RuntimeEvent::ToolStarted { .. }
                | RuntimeEvent::ToolCallProgress { .. }
                | RuntimeEvent::ToolCompleted { .. }
        )
    }

    fn runtime_event_turn_id(event: &RuntimeEvent) -> Option<&TurnId> {
        match event {
            RuntimeEvent::PermissionSettled { .. } => None,
            RuntimeEvent::UserInputWaitChanged { .. } => None,
            RuntimeEvent::AssistantSegmentStarted { turn_id, .. }
            | RuntimeEvent::AssistantEffectBound { turn_id, .. }
            | RuntimeEvent::AssistantActivityBound { turn_id, .. }
            | RuntimeEvent::TurnStarted { turn_id, .. }
            | RuntimeEvent::TurnContextCaptured { turn_id, .. }
            | RuntimeEvent::InputQueued { turn_id, .. }
            | RuntimeEvent::InputApplied { turn_id, .. }
            | RuntimeEvent::InputWithdrawn { turn_id, .. }
            | RuntimeEvent::PartUpdated { turn_id, .. }
            | RuntimeEvent::ItemStarted { turn_id, .. }
            | RuntimeEvent::TextReviewChanged { turn_id, .. }
            | RuntimeEvent::ItemDelta { turn_id, .. }
            | RuntimeEvent::ItemDiscarded { turn_id, .. }
            | RuntimeEvent::ItemCompleted { turn_id, .. }
            | RuntimeEvent::ToolInputDelta { turn_id, .. }
            | RuntimeEvent::PermissionRequested { turn_id, .. }
            | RuntimeEvent::PermissionResolved { turn_id, .. }
            | RuntimeEvent::ToolStarted { turn_id, .. }
            | RuntimeEvent::ToolCallProgress { turn_id, .. }
            | RuntimeEvent::ToolCompleted { turn_id, .. }
            | RuntimeEvent::CapabilitiesActivated { turn_id, .. }
            | RuntimeEvent::EngineeringLoopUpdated { turn_id, .. }
            | RuntimeEvent::ExecutionEvidenceObserved { turn_id, .. }
            | RuntimeEvent::RuntimeDecisionFactObserved { turn_id, .. }
            | RuntimeEvent::RequirementScopeChanged { turn_id, .. }
            | RuntimeEvent::RequirementAssessed { turn_id, .. }
            | RuntimeEvent::RunResultDecided { turn_id, .. }
            | RuntimeEvent::TurnCompleted { turn_id, .. }
            | RuntimeEvent::TurnCancelled { turn_id, .. }
            | RuntimeEvent::TurnFailed { turn_id, .. } => Some(turn_id),
            RuntimeEvent::TaskUpdated { task, .. } => Some(&task.owner_turn_id),
            RuntimeEvent::SkillRuntimeUpdated { turn_id, .. } => turn_id.as_ref(),
            RuntimeEvent::HostProgress { turn_id, .. } => turn_id.as_ref(),
            RuntimeEvent::LifecycleHookUpdated { turn_id, .. } => turn_id.as_ref(),
            RuntimeEvent::SessionStarted { .. }
            | RuntimeEvent::InputAccepted { .. }
            | RuntimeEvent::GoalUpdated { .. }
            | RuntimeEvent::MemoryUpdated { .. }
            | RuntimeEvent::HostCatalogUpdated { .. }
            | RuntimeEvent::HostLeaseRenewed { .. }
            | RuntimeEvent::SessionConfigurationUpdated { .. }
            | RuntimeEvent::HostDisconnected { .. }
            | RuntimeEvent::ContextUpdated { .. }
            | RuntimeEvent::CompactionRequested { .. }
            | RuntimeEvent::CompactionCompleted { .. }
            | RuntimeEvent::CompactionFailed { .. } => None,
        }
    }

    pub(super) fn project_completed_item(
        &mut self,
        turn_id: &TurnId,
        item: &AgentItem,
        updates: &mut Vec<(PartUpdate, AgentPart)>,
    ) -> Result<(), String> {
        match item {
            AgentItem::Message {
                id,
                role: AgentRole::Assistant,
                content,
            } => {
                let provider_part_id = id.as_str().to_string();
                // Streaming uses Core's stable text_part_id while compatible
                // providers may return a different message id at completion.
                // They still represent one assistant document for this Turn.
                // Reconcile the completion into the open owned text part
                // instead of creating a second durable body.
                let part_id = self
                    .session
                    .conversation
                    .projection
                    .order
                    .iter()
                    .rev()
                    .find_map(|candidate| {
                        match self.session.conversation.projection.parts.get(candidate) {
                            Some(AgentPart::Text(part))
                                if part.ownership.turn_id == turn_id.as_str()
                                    && part.review.is_none()
                                    && !part.is_complete
                                    && content.starts_with(&part.text) =>
                            {
                                Some(candidate.clone())
                            }
                            _ => None,
                        }
                    })
                    .unwrap_or(provider_part_id.clone());
                // A review commits the already displayed candidate, not a new
                // document selected by prefix similarity.
                let part_id = if matches!(self.session.conversation.projection.parts.get(&provider_part_id),
                    Some(AgentPart::Text(part)) if part.review.is_some()) {
                    provider_part_id
                } else { part_id };
                if !self
                    .session
                    .conversation
                    .projection
                    .parts
                    .contains_key(&part_id)
                {
                    self.apply_part(
                        PartUpdate::Created {
                            part: AgentPart::Text(TextPart {
                                review: None,
                                id: part_id,
                                ownership: PartOwnership::for_turn(turn_id.as_str()),
                                text: content.clone(),
                                is_complete: true,
                            }),
                        },
                        updates,
                    )
                } else {
                    if let Some(AgentPart::Text(part)) =
                        self.session.conversation.projection.parts.get(&part_id)
                    {
                        if part.text.is_empty() && part.is_complete {
                            self.apply_part(
                                PartUpdate::TextReplaced {
                                    part_id: part_id.clone(),
                                    text: content.clone(),
                                    is_complete: true,
                                },
                                updates,
                            )?;
                        } else if content.starts_with(&part.text) {
                            let suffix = content[part.text.len()..].to_string();
                            if !suffix.is_empty() {
                                self.apply_part(
                                    PartUpdate::TextDelta {
                                        part_id: part_id.clone(),
                                        delta: suffix,
                                    },
                                    updates,
                                )?;
                            }
                        }
                    }
                    self.apply_part(PartUpdate::Completed { part_id }, updates)
                }
            }
            AgentItem::Reasoning {
                id,
                summary,
                content,
                ..
            } => {
                // An opaque provider continuation is a canonical model fact,
                // not an empty user-facing thinking card.
                if !reasoning_item_has_visible_content(summary, content) {
                    return Ok(());
                }
                let provider_part_id = id.as_str().to_string();
                // Providers commonly return a completion item with their own
                // id after streaming through Core's stable reasoning_part_id.
                // Both values describe one reasoning document for this Turn.
                // Reconcile them just like assistant text; otherwise clients
                // receive the same thought once as a delta and again as a new
                // completed Part.
                let part_id = self
                    .session
                    .conversation
                    .projection
                    .order
                    .iter()
                    .rev()
                    .find_map(|candidate| {
                        match self.session.conversation.projection.parts.get(candidate) {
                            Some(AgentPart::Reasoning(part))
                                if reasoning_completion_matches(part, turn_id, content) =>
                            {
                                Some(candidate.clone())
                            }
                            _ => None,
                        }
                    })
                    .unwrap_or(provider_part_id);
                if !self
                    .session
                    .conversation
                    .projection
                    .parts
                    .contains_key(&part_id)
                {
                    self.apply_part(
                        PartUpdate::Created {
                            part: AgentPart::Reasoning(ReasoningPart {
                                id: part_id,
                                ownership: PartOwnership::for_turn(turn_id.as_str()),
                                title: summary.clone(),
                                detail: content.clone(),
                                is_complete: true,
                            }),
                        },
                        updates,
                    )
                } else {
                    if let Some(AgentPart::Reasoning(part)) =
                        self.session.conversation.projection.parts.get(&part_id)
                    {
                        if content.starts_with(&part.detail) {
                            let suffix = content[part.detail.len()..].to_string();
                            let completed_title = (!summary.is_empty() && summary != &part.title)
                                .then(|| summary.clone());
                            if !suffix.is_empty() || completed_title.is_some() {
                                self.apply_part(
                                    PartUpdate::ReasoningDelta {
                                        part_id: part_id.clone(),
                                        title: completed_title,
                                        detail_delta: suffix,
                                    },
                                    updates,
                                )?;
                            }
                        }
                    }
                    self.apply_part(PartUpdate::Completed { part_id }, updates)
                }
            }
            AgentItem::Compaction {
                id,
                summary,
                source_item_ids,
                source_fingerprint,
                original_token_count,
                ..
            } => self.apply_part(
                PartUpdate::Created {
                    part: AgentPart::Compaction(CompactionPart {
                        id: id.as_str().to_string(),
                        ownership: PartOwnership::for_turn(turn_id.as_str()),
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
                },
                updates,
            ),
            AgentItem::ToolCall {
                id,
                call_id,
                name,
                arguments,
            } => {
                let part_id = self
                    .active_tool_part_id(call_id)
                    .unwrap_or_else(|| bounded_tool_part_identity(format!("tool:{}", id.as_str())));
                self.ensure_tool_part(
                    turn_id,
                    &part_id,
                    call_id.as_str(),
                    name,
                    Some(arguments),
                    ToolPartState::Pending,
                    updates,
                )
            }
            _ => Ok(()),
        }
    }

    pub(super) fn ensure_tool_part(
        &mut self,
        turn_id: &TurnId,
        part_id: &str,
        call_id: &str,
        tool_name: &str,
        input: Option<&Value>,
        state: ToolPartState,
        updates: &mut Vec<(PartUpdate, AgentPart)>,
    ) -> Result<(), String> {
        let (tool_title, tool_origin) = projected_tool_identity(&self.session, tool_name);
        if !self
            .session
            .conversation
            .projection
            .parts
            .contains_key(part_id)
        {
            return self.apply_part(
                PartUpdate::Created {
                    part: AgentPart::Tool(ToolPart {
                        execution_source: self.session.tool_record_source(Some(turn_id), call_id),
                        id: part_id.to_string(),
                        ownership: PartOwnership::for_turn(turn_id.as_str()),
                        call_id: call_id.to_string(),
                        capability_id: tool_name.to_string(),
                        title: tool_title.clone(),
                        summary: None,
                        origin: tool_origin,
                        state,
                        input: input.cloned().unwrap_or(Value::Null),
                        raw_input: String::new(),
                        permission: None,
                        output_preview: None,
                        structured_output: None,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                        started_at_ms: matches!(state, ToolPartState::Running).then_some(self.observed_at_ms),
                        finished_at_ms: state.is_terminal().then_some(self.observed_at_ms),
                    }),
                },
                updates,
            );
        }
        if let Some(input) = input {
            self.apply_part(
                PartUpdate::ToolInputResolved {
                    part_id: part_id.to_string(),
                    input: input.clone(),
                },
                updates,
            )?;
        }
        self.apply_part(
            PartUpdate::ToolState {
                part_id: part_id.to_string(),
                state,
                title: Some(tool_title),
                summary: None,
                output_preview: None,
                structured_output: None,
                full_output_ref: None,
                artifacts: Vec::new(),
                timestamp_ms: Some(self.observed_at_ms),
            },
            updates,
        )
    }

    pub(super) fn complete_tool_part(
        &mut self,
        turn_id: &TurnId,
        part_id: &str,
        call_id: &str,
        tool_name: &str,
        output: &Value,
        state: ToolPartState,
        full_output_ref: Option<String>,
        artifacts: &[HostArtifact],
        updates: &mut Vec<(PartUpdate, AgentPart)>,
    ) -> Result<(), String> {
        let output_summary = self.tool_output_summary(output);
        // Presentation is separate from the evidence supplied to the model.
        let output_preview = Some(output.get("_nola_display")
            .and_then(Value::as_str).filter(|text| !text.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.tool_output_preview(tool_name, output).0));
        let input = self
            .session
            .conversation
            .projection
            .parts
            .get(part_id)
            .and_then(|part| match part {
                AgentPart::Tool(tool) => Some(tool.input.clone()),
                _ => None,
            })
            .unwrap_or(Value::Null);
        let source_candidates = Self::source_candidates(tool_name, &input, output, artifacts);
        let artifacts = artifacts
            .iter()
            .map(|artifact| ArtifactReference {
                uri: artifact.uri.clone(),
                name: artifact.name.clone(),
                mime_type: artifact.mime_type.clone(),
                operation: artifact.operation.clone(),
            })
            .collect::<Vec<_>>();
        let (tool_title, tool_origin) = projected_tool_identity(&self.session, tool_name);
        if !self
            .session
            .conversation
            .projection
            .parts
            .contains_key(part_id)
        {
            self.apply_part(
                PartUpdate::Created {
                    part: AgentPart::Tool(ToolPart {
                        execution_source: self.session.tool_record_source(Some(turn_id), call_id),
                        id: part_id.to_string(),
                        ownership: PartOwnership::for_turn(turn_id.as_str()),
                        call_id: call_id.to_string(),
                        capability_id: tool_name.to_string(),
                        title: tool_title.clone(),
                        summary: output_summary.clone(),
                        origin: tool_origin,
                        state,
                        input: Value::Null,
                        raw_input: String::new(),
                        permission: None,
                        output_preview,
                        structured_output: Some(output.clone()),
                        full_output_ref,
                        artifacts,
                        started_at_ms: None,
                        finished_at_ms: Some(self.observed_at_ms),
                    }),
                },
                updates,
            )?;
        } else {
            self.apply_part(
                PartUpdate::ToolState {
                    part_id: part_id.to_string(),
                    state,
                    title: Some(tool_title),
                    summary: output_summary,
                    output_preview,
                    structured_output: Some(output.clone()),
                    full_output_ref,
                    artifacts,
                    timestamp_ms: Some(self.observed_at_ms),
                },
                updates,
            )?;
        }
        self.project_sources(turn_id, call_id, source_candidates, updates)
    }

    fn project_sources(
        &mut self,
        turn_id: &TurnId,
        call_id: &str,
        candidates: Vec<SourceCandidate>,
        updates: &mut Vec<(PartUpdate, AgentPart)>,
    ) -> Result<(), String> {
        for candidate in candidates {
            let source_id = Self::stable_evidence_id("source", &[candidate.uri.as_str()]);
            // `source_id` is the stable evidence identity and may legitimately
            // recur in many messages. A persisted Part id, however, is owned
            // by exactly one assistant Message. Scope that storage identity to
            // the Turn instead of reusing the evidence id as a global Part id.
            let source_part_id =
                Self::stable_evidence_id("source", &[turn_id.as_str(), source_id.as_str()]);
            if !self
                .session
                .conversation
                .projection
                .parts
                .contains_key(&source_part_id)
            {
                self.apply_part(
                    PartUpdate::Created {
                        part: AgentPart::Source(SourcePart {
                            id: source_part_id,
                            ownership: PartOwnership::for_turn(turn_id.as_str()),
                            source_id: source_id.clone(),
                            title: candidate.title.clone(),
                            uri: candidate.uri.clone(),
                            mime_type: candidate.mime_type.clone(),
                            evidence: None,
                        }),
                    },
                    updates,
                )?;
            }
            let location_json = serde_json::to_string(&candidate.location)
                .map_err(|error| format!("serialize Source location: {error}"))?;
            let range_json = serde_json::to_string(&candidate.range)
                .map_err(|error| format!("serialize Citation range: {error}"))?;
            let citation_id = Self::stable_evidence_id(
                "citation",
                &[
                    source_id.as_str(),
                    call_id,
                    location_json.as_str(),
                    range_json.as_str(),
                ],
            );
            let citation_part_id =
                Self::stable_evidence_id("citation", &[turn_id.as_str(), citation_id.as_str()]);
            if self
                .session
                .conversation
                .projection
                .parts
                .contains_key(&citation_part_id)
            {
                continue;
            }
            self.apply_part(
                PartUpdate::Created {
                    part: AgentPart::Citation(CitationPart {
                        id: citation_part_id,
                        ownership: PartOwnership::for_turn(turn_id.as_str()),
                        citation_id,
                        source_id,
                        title: candidate.title,
                        uri: candidate.uri,
                        mime_type: candidate.mime_type,
                        location: candidate.location,
                        range: candidate.range,
                        origin_tool_call_id: call_id.to_string(),
                    }),
                },
                updates,
            )?;
        }
        Ok(())
    }

    fn stable_evidence_id(prefix: &str, fields: &[&str]) -> String {
        let mut digest = Sha256::new();
        for field in fields {
            digest.update((field.len() as u64).to_be_bytes());
            digest.update(field.as_bytes());
        }
        let encoded = hex::encode(digest.finalize());
        format!("{prefix}_{}", &encoded[..24])
    }

    fn source_candidates(
        tool_name: &str,
        _input: &Value,
        output: &Value,
        artifacts: &[HostArtifact],
    ) -> Vec<SourceCandidate> {
        // Reply-level sources are evidence that the user may want to revisit,
        // not an inventory of every resource touched while doing the work.
        // Local files remain attached to their Tool Part through artifacts and
        // structured output. Projecting reads as citations duplicates those
        // files below the assistant message on every client.
        let is_source_tool = matches!(tool_name, "web_search" | "web_fetch" | "fetch_url");
        let mut candidates = if is_source_tool {
            artifacts
                .iter()
                .filter(|artifact| !artifact.uri.trim().is_empty())
                .map(|artifact| SourceCandidate {
                    title: if artifact.name.trim().is_empty() {
                        Self::source_title(&artifact.uri)
                    } else {
                        artifact.name.clone()
                    },
                    uri: artifact.uri.trim().to_string(),
                    mime_type: artifact.mime_type.clone(),
                    location: None,
                    range: None,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        match tool_name {
            "web_search" | "web_fetch" | "fetch_url" => {
                for value in Self::source_result_values(output) {
                    let Some(uri) = Self::string_field(value, &["url", "uri", "link"]) else {
                        continue;
                    };
                    candidates.push(SourceCandidate {
                        title: Self::string_field(value, &["title", "name"])
                            .unwrap_or_else(|| Self::source_title(&uri)),
                        mime_type: Self::string_field(value, &["mime_type", "mimeType"]),
                        uri,
                        location: Self::source_location(value),
                        range: Self::citation_range(value),
                    });
                }
            }
            _ => {}
        }

        for candidate in &mut candidates {
            candidate.uri = Self::normalize_source_uri(&candidate.uri);
        }
        candidates.retain(|candidate| !candidate.uri.is_empty());
        candidates.sort_by(|left, right| {
            left.uri
                .cmp(&right.uri)
                .then_with(|| left.title.cmp(&right.title))
        });
        candidates.dedup_by(|left, right| {
            left.uri == right.uri && left.location == right.location && left.range == right.range
        });
        candidates
    }

    fn source_result_values(output: &Value) -> Vec<&Value> {
        if let Some(values) = output.as_array() {
            return values.iter().collect();
        }
        for key in ["results", "sources", "items", "data"] {
            if let Some(values) = output.get(key).and_then(Value::as_array) {
                return values.iter().collect();
            }
        }
        output.as_object().map(|_| vec![output]).unwrap_or_default()
    }

    fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
        keys.iter().find_map(|key| {
            value
                .get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        })
    }

    fn source_location(value: &Value) -> Option<SourceLocation> {
        let location = SourceLocation {
            page: value.get("page").and_then(Value::as_u64),
            line_start: value
                .get("line_start")
                .or_else(|| value.get("start_line"))
                .and_then(Value::as_u64),
            line_end: value.get("line_end").and_then(Value::as_u64),
            section: Self::string_field(value, &["section", "heading"]),
        };
        (location != SourceLocation::default()).then_some(location)
    }

    fn citation_range(value: &Value) -> Option<CitationRange> {
        let range = value.get("range")?;
        let start = range.get("start").and_then(Value::as_u64)?;
        let end = range.get("end").and_then(Value::as_u64)?;
        (end >= start).then_some(CitationRange { start, end })
    }

    fn source_title(uri: &str) -> String {
        uri.trim_end_matches('/')
            .rsplit(['/', '\\'])
            .next()
            .filter(|title| !title.is_empty())
            .unwrap_or(uri)
            .to_string()
    }

    fn normalize_source_uri(uri: &str) -> String {
        let mut normalized = uri.trim().to_string();
        if normalized.starts_with("http://") || normalized.starts_with("https://") {
            if let Some(fragment) = normalized.find('#') {
                normalized.truncate(fragment);
            }
            while normalized.ends_with('/') {
                let scheme_end = normalized.find("://").map(|index| index + 3).unwrap_or(0);
                if normalized[scheme_end..].contains('/') {
                    normalized.pop();
                } else {
                    break;
                }
            }
        }
        normalized
    }

    fn active_tool_part_id(&self, call_id: &CallId) -> Option<String> {
        self.session
            .conversation
            .projection
            .order
            .iter()
            .rev()
            .find_map(
                |part_id| match self.session.conversation.projection.parts.get(part_id) {
                    Some(AgentPart::Tool(tool))
                        if tool.call_id == call_id.as_str() && !tool.state.is_terminal() =>
                    {
                        Some(part_id.clone())
                    }
                    _ => None,
                },
            )
    }

    fn latest_tool_part_id(&self, call_id: &CallId) -> Option<String> {
        self.active_tool_part_id(call_id).or_else(|| {
            self.session
                .conversation
                .projection
                .order
                .iter()
                .rev()
                .find_map(
                    |part_id| match self.session.conversation.projection.parts.get(part_id) {
                        Some(AgentPart::Tool(tool)) if tool.call_id == call_id.as_str() => {
                            Some(part_id.clone())
                        }
                        _ => None,
                    },
                )
        })
    }

    fn tool_part_id(turn_id: &TurnId, call_id: &CallId) -> String {
        bounded_tool_part_identity(format!("tool:{}:{}", turn_id.as_str(), call_id.as_str()))
    }

    fn pending_text_review(&self) -> Option<crate::agent::parts::TextReviewState> {
        self.session.execution.active_turn.as_ref().filter(|turn|
            turn.final_answer_audit_required
                && !turn.user_denied_permission
                && turn.context.configuration.turn_identity.supports_reviewed_text_parts
        ).map(|_| crate::agent::parts::TextReviewState::Pending)
    }

    pub(super) fn apply_part(
        &mut self,
        update: PartUpdate,
        updates: &mut Vec<(PartUpdate, AgentPart)>,
    ) -> Result<(), String> {
        let update = match update {
            PartUpdate::Created { part } => {
                let projection = &self.session.conversation.projection;
                let owner = projection.parts.get(part.id()).map(|existing| existing.message_id())
                    .or_else(|| projection.active_message_segments.get(part.turn_id()).map(String::as_str));
                let part = match owner { Some(owner) => part.with_message_id(owner), None => part };
                PartUpdate::Created { part }
            }
            other => other,
        };
        let part_id = update.part_id().to_string();
        let changed = self
            .session
            .conversation
            .projection
            .apply(update.clone())
            .map_err(|error| format!("invalid message part transition: {error:?}"))?;
        if changed {
            let part = self
                .session
                .conversation
                .projection
                .parts
                .get(&part_id)
                .cloned()
                .ok_or_else(|| format!("projected Part {part_id} is missing"))?;
            updates.push((update, part));
        }
        Ok(())
    }
}

#[cfg(test)]
mod discarded_item_projection_tests {
    use super::*;

    #[test]
    fn long_tool_part_ownership_is_bounded_stable_and_distinct() {
        let turn = TurnId::from_string(format!("user-delegation:{}:workspace", "a".repeat(36)));
        let call = CallId::from_string("b".repeat(64));
        let part = AgentRuntime::tool_part_id(&turn, &call);
        assert!(part.len() <= 128);
        assert_eq!(part, AgentRuntime::tool_part_id(&turn, &call));
        assert_ne!(part, AgentRuntime::tool_part_id(&turn, &CallId::from_string("c".repeat(64))));
        assert_eq!(bounded_tool_part_identity("tool:turn:call".into()), "tool:turn:call");
        nola_domain::CanonicalBodyReference::for_field("body".into(), "conversation".into(),
            "message".into(), part, nola_domain::PartBodyField::ContentJson, 1, 1).unwrap();
    }

    #[test]
    fn core_tool_rejection_is_visible_and_host_cancellation_keeps_its_outcome() {
        for host_cancelled in [false, true] {
            let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
                turn_identity: crate::agent::model_capability::test_host_identity(),
                session_id: Some(SessionId::from_string("control-rejection".into())),
                ..Default::default()
            }).unwrap();
            let turn = TurnId::from_string("turn".into());
            let call = CallId::from_string("call".into());
            let output = json!({"error":"invalid_task_scope: unexpected fields"});
            let mut events = vec![
                RuntimeEvent::ItemCompleted {turn_id:turn.clone(),item:AgentItem::ToolCall {
                    id:ItemId::from_string("call-item".into()),call_id:call.clone(),name:"task_scope_update".into(),arguments:json!({})}},
                RuntimeEvent::ItemCompleted {turn_id:turn.clone(),item:AgentItem::ToolResult {
                    id:ItemId::from_string("result-item".into()),call_id:call.clone(),name:"task_scope_update".into(),
                    output:output.clone(),is_error:true,full_output_ref:None,artifacts:vec![]}},
            ];
            if host_cancelled {
                events.push(RuntimeEvent::ToolCompleted {turn_id:turn.clone(),call_id:call.clone(),tool_name:"task_scope_update".into(),
                    output:output.clone(),outcome:HostToolOutcome::Cancelled,full_output_ref:None,artifacts:vec![]});
            }
            runtime.project_events(&mut events).unwrap();
            runtime.terminate_turn_parts(&turn,ToolPartState::Completed,&mut events).unwrap();
            let part_id = runtime.latest_tool_part_id(&call).unwrap();
            let AgentPart::Tool(part) = &runtime.session.conversation.projection.parts[&part_id] else { panic!("tool part") };
            assert_eq!(part.state, if host_cancelled {ToolPartState::Cancelled} else {ToolPartState::Failed});
            assert_eq!(part.structured_output.as_ref(),Some(&output));
        }
    }

    #[test]
    fn later_answer_does_not_reuse_closed_empty_tool_round_text() {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("closed-text-regression".into())),
            ..Default::default()
        }).unwrap();
        let turn = TurnId::from_string("turn-1".into());
        runtime.apply_part(PartUpdate::Created {
            part: AgentPart::Text(TextPart {
                id: "closed-tool-round".into(),
                ownership: PartOwnership::for_turn(turn.as_str()),
                review: None,
                text: String::new(),
                is_complete: true,
            }),
        }, &mut Vec::new()).unwrap();
        let mut updates = Vec::new();
        runtime.project_completed_item(&turn, &AgentItem::Message {
            id: ItemId::from_string("new-answer".into()),
            role: AgentRole::Assistant,
            content: "Delivered and verified.".into(),
        }, &mut updates).unwrap();
        assert!(matches!(&updates[0].0, PartUpdate::Created { part }
            if part.id() == "new-answer"));
        let AgentPart::Text(old) = &runtime.session.conversation.projection.parts["closed-tool-round"]
            else { panic!("text") };
        assert!(old.text.is_empty());
        assert!(old.is_complete);
    }

    #[test]
    fn tool_timestamps_use_admitted_observation_clock_and_survive_replay() {
        fn project() -> AgentRuntime {
            let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
                turn_identity: crate::agent::model_capability::test_host_identity(),
                session_id: Some(SessionId::from_string("tool-clock-session".into())),
                ..Default::default()
            }).unwrap();
            let turn = TurnId::from_string("timed-turn".into());
            let mut updates = Vec::new();
            runtime.observed_at_ms = 100;
            runtime.ensure_tool_part(&turn, "timed-tool", "call", "bash", None,
                ToolPartState::Pending, &mut updates).unwrap();
            runtime.observed_at_ms = 200;
            runtime.ensure_tool_part(&turn, "timed-tool", "call", "bash", None,
                ToolPartState::Running, &mut updates).unwrap();
            runtime.observed_at_ms = 250;
            runtime.ensure_tool_part(&turn, "timed-tool", "call", "bash", None,
                ToolPartState::Running, &mut updates).unwrap();
            runtime.observed_at_ms = 400;
            runtime.complete_tool_part(&turn, "timed-tool", "call", "bash", &serde_json::json!({"text":"ok"}),
                ToolPartState::Completed, None, &[], &mut updates).unwrap();
            let AgentPart::Tool(part) = &runtime.session.conversation.projection.parts["timed-tool"] else { panic!("tool") };
            assert_eq!(part.started_at_ms, Some(200));
            assert_eq!(part.finished_at_ms, Some(400));
            runtime
        }
        let original = project();
        let replay = project();
        assert_eq!(serde_json::to_value(&original.session.conversation.projection).unwrap(),
            serde_json::to_value(&replay.session.conversation.projection).unwrap());
    }

    #[test]
    fn tool_terminal_without_start_records_finish_but_never_invents_start() {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("tool-terminal-clock-session".into())),
            ..Default::default()
        }).unwrap();
        let turn = TurnId::from_string("cancelled-turn".into());
        let mut updates = Vec::new();
        runtime.observed_at_ms = 100;
        runtime.ensure_tool_part(&turn, "pending", "call", "bash", None,
            ToolPartState::WaitingForPermission, &mut updates).unwrap();
        runtime.observed_at_ms = 200;
        runtime.terminate_turn_parts(&turn, ToolPartState::Cancelled, &mut Vec::new()).unwrap();
        let AgentPart::Tool(part) = &runtime.session.conversation.projection.parts["pending"] else { panic!("tool") };
        assert_eq!(part.started_at_ms, None);
        assert_eq!(part.finished_at_ms, Some(200));
        runtime.observed_at_ms = 300;
        runtime.complete_tool_part(&turn, "late-result", "late-call", "bash", &serde_json::json!({"text":"late"}),
            ToolPartState::Completed, None, &[], &mut updates).unwrap();
        let AgentPart::Tool(part) = &runtime.session.conversation.projection.parts["late-result"] else { panic!("tool") };
        assert_eq!(part.started_at_ms, None);
        assert_eq!(part.finished_at_ms, Some(300));
    }

    fn provisional_text(text: &str) -> AgentPart {
        AgentPart::Text(TextPart {
            review: None,
            id: "part-1".to_string(),
            ownership: PartOwnership::for_turn("turn-1"),
            text: text.to_string(),
            is_complete: false,
        })
    }

    #[test]
    fn process_discard_preserves_trimmed_text_as_structured_process() {
        let part = provisional_text("  inspecting the repository  ");
        let (ownership, detail) =
            discarded_process_part(Some(&part), ItemDiscardDisposition::Process)
                .expect("tool-round process should remain visible as structured reasoning");

        assert_eq!(ownership, PartOwnership::for_turn("turn-1"));
        assert_eq!(detail, "inspecting the repository");
    }

    #[test]
    fn hidden_discard_never_exposes_retry_or_failure_text() {
        let part = provisional_text("internal retry text");

        assert_eq!(
            discarded_process_part(Some(&part), ItemDiscardDisposition::Hidden),
            None
        );
    }

    #[test]
    fn empty_process_discard_does_not_create_a_reasoning_part() {
        let part = provisional_text("   \n  ");

        assert_eq!(
            discarded_process_part(Some(&part), ItemDiscardDisposition::Process),
            None
        );
    }

    #[test]
    fn opaque_only_reasoning_does_not_create_a_visible_part() {
        assert!(!reasoning_item_has_visible_content("", ""));
        assert!(!reasoning_item_has_visible_content("  ", "\n"));
        assert!(reasoning_item_has_visible_content("Summary", ""));
        assert!(reasoning_item_has_visible_content("", "Visible reasoning"));
    }

    #[test]
    fn completed_process_reasoning_matches_identical_provider_completion() {
        let part = ReasoningPart {
            id: "assistant-1:process".to_string(),
            ownership: PartOwnership::for_turn("turn-1"),
            title: "thinking".to_string(),
            detail: "Inspecting the requested source.".to_string(),
            is_complete: true,
        };

        assert!(reasoning_completion_matches(
            &part,
            &TurnId::from_string("turn-1".to_string()),
            "Inspecting the requested source."
        ));
        assert!(!reasoning_completion_matches(
            &part,
            &TurnId::from_string("turn-2".to_string()),
            "Inspecting the requested source."
        ));
    }

    #[test]
    fn provisional_process_reasoning_stays_open_and_matches_extended_completion() {
        let item_id = ItemId::from_string("assistant-1".to_string());
        let AgentPart::Reasoning(part) = provisional_process_reasoning_part(
            &item_id,
            PartOwnership::for_turn("turn-1"),
            "Inspecting the requested source".to_string(),
        ) else {
            panic!("process conversion must create reasoning");
        };

        assert!(!part.is_complete);
        assert!(reasoning_completion_matches(
            &part,
            &TurnId::from_string("turn-1".to_string()),
            "Inspecting the requested source and its official deadline."
        ));
    }
}

#[cfg(test)]
mod applied_input_segment_tests {
    use super::*;
    #[test]
    fn input_boundary_preserves_existing_parts_and_freezes_activity_birth() {
        let mut runtime=AgentRuntime::try_new(AgentCoreConfig {
            session_id: Some(SessionId::from_string("segment-session".into())),
            turn_identity:crate::agent::model_capability::test_host_identity(), ..Default::default()
        }).unwrap();
        let turn=TurnId::from_string("same-turn".into());
        let task = |id:&str| crate::agent::task::TaskHandle {
            id:id.into(),owner_session_id:runtime.session.session_id.clone(),owner_turn_id:turn.clone(),
            kind:crate::agent::task::TaskKind::Agent,state:crate::agent::task::TaskState::Queued,
            progress:None,current_step:None,result_ref:None,cancel_requested:false,cancel_requested_at_epoch_ms:None,
            lineage:Default::default(),durability:Default::default(),
        };
        let first_task=task("old-task");let second_task=task("new-task");
        let mut events=vec![
            RuntimeEvent::ItemStarted {turn_id:turn.clone(),item_id:ItemId::from_string("old-text".into()),kind:AgentItemDeltaKind::AssistantText},
            RuntimeEvent::TaskUpdated {task_projection_revision:1,task:first_task.clone()},
            RuntimeEvent::InputQueued {turn_id:turn.clone(),delivery:AgentInputDelivery::CurrentTurn,item_count:1,items:vec![]},
        ];
        runtime.project_events(&mut events).unwrap();
        assert!(runtime.session.conversation.projection.active_message_segments.is_empty(),"queueing is not application");
        let mut applied=vec![
            RuntimeEvent::AssistantSegmentStarted {turn_id:turn.clone(),assistant_message_id:"segment-two".into(),applications:vec![crate::agent::InputApplication {
                admission_id:"input".into(),destination_turn_id:turn.as_str().into(),applied_sequence:2,
            }]},
            RuntimeEvent::ItemStarted {turn_id:turn.clone(),item_id:ItemId::from_string("new-text".into()),kind:AgentItemDeltaKind::AssistantText},
            RuntimeEvent::TaskUpdated {task_projection_revision:2,task:second_task},
            RuntimeEvent::TaskUpdated {task_projection_revision:3,task:first_task},
        ];
        runtime.project_events(&mut applied).unwrap();
        let projection=&runtime.session.conversation.projection;
        assert_eq!(projection.parts["old-text"].message_id(),"same-turn:assistant");
        assert_eq!(projection.parts["new-text"].message_id(),"segment-two");
        assert_eq!(projection.activity_message_owners["task:old-task"],"same-turn:assistant");
        assert_eq!(projection.activity_message_owners["task:new-task"],"segment-two");
        assert_eq!(applied.iter().filter(|event| matches!(event,RuntimeEvent::AssistantActivityBound {..})).count(),1);
        let encoded=serde_json::to_string(projection).unwrap();
        let restored:crate::agent::parts::ConversationProjection=serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.active_message_segments,projection.active_message_segments);
        assert_eq!(restored.activity_message_owners,projection.activity_message_owners);
    }
}
