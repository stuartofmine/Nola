// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Plans and resumes proactive context compaction at explicit boundaries.
// - Boundary: Private deterministic reducer layer; public clients use AgentCore.

use super::*;

impl AgentRuntime {
    pub(super) fn request_compaction(
        &mut self,
        reason: CompactionReason,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        self.request_compaction_with_history_budget(reason, None, events, effects)
    }

    pub(super) fn request_compaction_with_history_budget(
        &mut self,
        reason: CompactionReason,
        history_budget: Option<(usize, usize)>,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let full_request_estimate = self.session.conversation.context_window.used_tokens;
        let history_tokens = self.session.conversation.model_context.iter()
            .fold(0usize, |sum, item| sum.saturating_add(item.estimated_tokens()));
        let history_trigger_budget = self.session.conversation.context_window.history_budget_for_model_limit(
            self.session.conversation.context_window.config.automatic_compaction_threshold(), history_tokens,
        );
        let next_automatic_count =
            nola_kernel::admit_compaction_request(nola_domain::CompactionRequestFacts {
                reason,
                compaction_in_progress: self
                    .session
                    .conversation
                    .context_window
                    .compaction_in_progress,
                context_is_empty: self.session.conversation.model_context.is_empty(),
                automatic_compaction_count: self
                    .session
                    .execution
                    .active_turn
                    .as_ref()
                    .map(|turn| turn.automatic_compaction_count),
            })
            .map_err(|error| {
                match error {
                    nola_domain::CompactionRequestRejection::AlreadyActive => {
                        "context compaction is already active"
                    }
                    nola_domain::CompactionRequestRejection::EmptyContext => {
                        "cannot compact an empty context"
                    }
                    nola_domain::CompactionRequestRejection::AutomaticLimitReached => {
                        "automatic context compaction made insufficient progress"
                    }
                }
                .to_string()
            })?;
        if let (Some(turn), Some(count)) = (
            self.session.execution.active_turn.as_mut(),
            next_automatic_count,
        ) {
            turn.automatic_compaction_count = count;
        }
        let context_config = &self
            .session
            .execution
            .active_turn
            .as_ref()
            .map(|turn| &turn.context.configuration.context_window)
            .unwrap_or(&self.session.configuration.context_window);
        let (configured_target_tokens, prompt_limit) = history_budget.unwrap_or_else(|| (
            self.session.conversation.context_window.history_budget_for_model_limit(
                context_config.compaction_target_tokens(), history_tokens,
            ),
            self.session.conversation.context_window.history_budget_for_model_limit(
                context_config.prompt_token_limit(), history_tokens,
            ),
        ));
        let full_prompt_limit = context_config.prompt_token_limit();
        // The first request targets half of the complete input, after reserving
        // immutable instructions, tool schemas and calibrated provider framing.
        let half_history_target = self.session.conversation.context_window.history_budget_for_model_limit(
            full_request_estimate / 2, history_tokens,
        );
        let configured_target_tokens = nola_kernel::compaction_request_target(
            reason, configured_target_tokens, history_tokens,
        );
        let mut target_tokens = if half_history_target > 0 {
            if configured_target_tokens == 0 { half_history_target.min(prompt_limit) }
            else { configured_target_tokens.min(half_history_target).min(prompt_limit) }
        } else {
            // An unreachable complete-input goal must not become a one-token
            // contract. Make the best bounded reduction without deleting input.
            nola_kernel::compaction_history_target(configured_target_tokens, prompt_limit, history_tokens)
        };
        let preferred = plan_compaction(&self.session.conversation.model_context, target_tokens);
        let preferred_retained = preferred.kept_items.iter()
            .fold(0usize, |sum,item| sum.saturating_add(item.estimated_tokens()));
        // Select the broader archive BEFORE any paid summary request when the
        // recent tail / old checkpoints would consume the half-context target.
        let handoff = half_history_target == 0
            || preferred_retained.saturating_add(4) >= target_tokens;
        let plan = if handoff {
            super::super::compaction_planner::plan_context_handoff(&self.session.conversation.model_context)
        } else { preferred };
        let retained_tokens = plan.kept_items.iter()
            .fold(0usize, |sum, item| sum.saturating_add(item.estimated_tokens()));
        if handoff && retained_tokens.saturating_add(4) >= target_tokens {
            // Even the minimum safe retained context can exceed the 50% goal.
            // Keep a useful bounded summary budget rather than forcing one token.
            target_tokens = nola_kernel::compaction_history_target(
                configured_target_tokens, prompt_limit, history_tokens,
            );
        }
        // If even an empty summary cannot bring this request below the existing
        // automatic trigger, this prefix cannot solve its pressure. In
        // particular, mandatory schemas plus the current transaction must not
        // force a tiny previous transaction into an impossible summary budget.
        // Do not defer hard-limit recovery. Admission already consumed this
        // turn's automatic count, so the unchanged projection will not loop.
        if reason == CompactionReason::ThresholdReached
            && history_budget.is_some()
            && retained_tokens >= history_trigger_budget
            && full_request_estimate < full_prompt_limit
        {
            return self.request_model_after_compaction(events, effects);
        }
        let archive_tokens = plan.items_to_summarize.iter().fold(0usize, |total, item| {
            total.saturating_add(item.estimated_tokens())
        });
        // A threshold describes the entire window, not the reclaimable prefix.
        // Do not spend a model round summarizing a greeting while a large
        // indivisible tail dominates the context. Hard overflow recovery keeps
        // its separate projection/error path.
        if reason == CompactionReason::ThresholdReached
            && full_request_estimate < full_prompt_limit && archive_tokens < 64 {
            return self.request_model_after_compaction(events, effects);
        }
        let summary_budget = if plan.items_to_summarize.is_empty() {
            None
        } else {
            match plan.summary_token_budget(target_tokens, prompt_limit, handoff) {
                Ok(budget) => Some(budget),
                Err(error) => {
                    if history_budget.is_some() {
                        if let Some(turn_id) = self.session.execution.active_turn.as_ref().map(|turn| turn.turn_id.clone()) {
                            return self.block_context_budget(&turn_id, full_request_estimate, full_prompt_limit, events);
                        }
                    }
                    return Err(error);
                }
            }
        };
        if summary_budget.is_some_and(|budget| nola_kernel::defer_low_gain_compaction(
            reason, archive_tokens, budget, full_request_estimate, full_prompt_limit,
        )) {
            return self.request_model_after_compaction(events, effects);
        }
        let items_to_summarize = plan.items_to_summarize;
        if items_to_summarize.is_empty() {
            if reason == CompactionReason::ContextLimitRecovery {
                if let Some(recovery) =
                    super::super::compaction_planner::project_oversized_atomic_item(
                        &self.session.conversation.model_context,
                        target_tokens,
                    )
                {
                    let source_item_ids = recovery
                        .source_items
                        .iter()
                        .map(|item| item.id().clone())
                        .collect::<Vec<_>>();
                    let source_fingerprint =
                        super::super::context_window::compaction_source_fingerprint(
                            &recovery.source_items,
                        )?;
                    let original_token_count =
                        recovery.source_items.iter().fold(0usize, |total, item| {
                            total.saturating_add(item.estimated_tokens())
                        });
                    let replaced_item_count = recovery.source_items.len();
                    let source_item_count = source_item_ids.len();
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
                    let summary = format!(
                        "Provider context overflow recovery uses a bounded view of canonical source item(s): {}. Exact bodies remain in the canonical transcript; fingerprint: {}.",
                        source_item_ids
                            .iter()
                            .map(|id| id.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        source_fingerprint,
                    );
                    let compaction_item = AgentItem::Compaction {
                        id: compaction_item_id.clone(),
                        summary: summary.clone(),
                        replaced_item_count,
                        source_item_ids,
                        source_fingerprint: source_fingerprint.clone(),
                        original_token_count,
                    };
                    let previous_working_tokens = self
                        .session
                        .conversation
                        .model_context
                        .iter()
                        .fold(0usize, |total, item| {
                            total.saturating_add(item.estimated_tokens())
                        });
                    let mut model_context = vec![compaction_item.clone()];
                    model_context.extend(recovery.model_items);
                    let recovered_working_tokens =
                        model_context.iter().fold(0usize, |total, item| {
                            total.saturating_add(item.estimated_tokens())
                        });
                    if recovered_working_tokens >= previous_working_tokens
                        || recovered_working_tokens > prompt_limit
                    {
                        return Err(format!(
                            "oversized context recovery made no safe progress: {previous_working_tokens} -> {recovered_working_tokens} tokens (prompt limit {prompt_limit})"
                        ));
                    }
                    self.session.item_ownership.insert(
                        compaction_item_key.clone(),
                        super::super::session::AgentItemOwnership {
                            source: Default::default(),
                            turn_id: compaction_turn_id,
                            message_id: compaction_message_id,
                        },
                    );
                    // Only the replaceable model working set receives bounded
                    // source projections. Canonical source items in transcript
                    // remain byte-for-byte intact and addressable by item ID.
                    self.session
                        .conversation
                        .transcript
                        .push(compaction_item.clone());
                    self.session.conversation.model_context = model_context;
                    self.session
                        .conversation
                        .context_window
                        .refresh_estimate(&self.session.conversation.model_context);
                    events.push(RuntimeEvent::CompactionCompleted {
                        summary,
                        replaced_item_count,
                        remaining_item_count: self.session.conversation.model_context.len(),
                        reason: Some(CompactionReason::ContextLimitRecovery),
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
                            turn_id,
                            item: compaction_item,
                        });
                    }
                    return self.request_model_after_compaction(events, effects);
                }
            }
            // A large instruction or unresolved transaction can leave no safe
            // archive. Continue automatic work without inventing a summary;
            // hard overflow recovery above handles bounded source projections.
            match nola_kernel::empty_compaction_archive_action(reason) {
                nola_domain::EmptyCompactionArchiveAction::ResumeModel => {
                    return self.request_model_after_compaction(events, effects);
                }
                nola_domain::EmptyCompactionArchiveAction::CompleteWithoutSummary => {
                    events.push(RuntimeEvent::CompactionCompleted {
                        summary: String::new(),
                        replaced_item_count: 0,
                        remaining_item_count: self.session.conversation.model_context.len(),
                        reason: Some(reason),
                        compaction_item_id: None,
                        source_fingerprint: None,
                        source_item_count: 0,
                        original_token_count: 0,
                    });
                    return Ok(());
                }
                nola_domain::EmptyCompactionArchiveAction::Reject => {
                    return Err("context has no summarizable items".to_string());
                }
            }
        }
        let source_item_ids = items_to_summarize
            .iter()
            .map(|item| item.id().clone())
            .collect::<Vec<_>>();
        let original_token_count = items_to_summarize.iter().fold(0usize, |total, item| {
            total.saturating_add(item.estimated_tokens())
        });
        let source_fingerprint =
            super::super::context_window::compaction_source_fingerprint(&items_to_summarize)?;
        let source_item_count = source_item_ids.len();
        self.session.conversation.context_window.begin_compaction(
            super::super::context_window::PendingCompactionPlan {
                handoff,
                reason,
                working_set_item_count: self.session.conversation.model_context.len(),
                working_set_fingerprint: super::super::context_window::compaction_source_fingerprint(
                    &self.session.conversation.model_context,
                )?,
                source_fingerprint: source_fingerprint.clone(),
                source_item_ids,
                original_token_count,
                replaced_item_count: items_to_summarize.len(),
                summary_token_limit: summary_budget
                    .expect("non-empty archive has a summary budget"),
            },
        )?;
        if let Some(turn) = self
            .session
            .execution
            .active_turn
            .as_mut()
            .filter(|turn| !turn.status.is_terminal())
        {
            turn.status = AgentTurnStatus::Compacting;
        }
        events.push(RuntimeEvent::CompactionRequested {
            reason,
            item_count: self.session.conversation.model_context.len(),
            estimated_tokens: full_request_estimate,
            source_fingerprint,
            source_item_count,
            original_token_count,
        });
        effects.push(AgentEffect::CompactContext {
            handoff,
            session_id: self.session.session_id.clone(),
            items_to_summarize,
            target_tokens: summary_budget.expect("non-empty archive has a summary budget"),
        });
        Ok(())
    }

    pub(super) fn request_model_after_compaction(
        &mut self,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let turn_id = self
            .session
            .execution
            .active_turn
            .as_ref()
            .map(|turn| turn.turn_id.clone())
            .ok_or_else(|| "compaction completed without an active turn".to_string())?;
        self.request_model(&turn_id, events, effects)
    }
}

#[cfg(test)]
mod long_run_tests {
    use super::*;

    #[test]
    fn later_business_generations_can_compact_again_in_the_same_run() {
        let mut model = crate::agent::code_mode::capability_descriptors(&Default::default())
            .unwrap()
            .remove(0);
        model.id = "model.stream".into();
        model.route = HostCapabilityRoute::Model;
        let mut reader = model.clone();
        reader.id = "fixture_read".into();
        reader.route = HostCapabilityRoute::Tool;
        reader.exposure = HostCapabilityExposure::Visible;
        reader.permission_domain = Some("fixture.read".into());
        reader.input_normalization = Default::default();
        reader.input_schema = json!({"type":"object","properties":{"record":{"type":"integer"}},"required":["record"]});
        let mut catalog = crate::agent::host::HostCapabilityCatalog::empty("long-context-host");
        catalog.capabilities.insert(model.id.clone(), model);
        catalog.capabilities.insert(reader.id.clone(), reader);
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            session_id: Some(SessionId::from_string("long-context-session".into())),
            turn_identity: crate::agent::model_capability::test_host_identity(),
            host_catalog: Some(catalog),
            permission_rules: crate::agent::permissions::PermissionRuleSet { rules: vec![crate::agent::permissions::PermissionRule {
                capability_pattern: "fixture.read".into(), resource_pattern: "*".into(),
                decision: RuleDecision::Allow, source: crate::agent::permissions::PermissionRuleSource::UserGlobal,
            }] },
            tools: vec![json!({"type":"function","function":{"name":"fixture_read","parameters":{"type":"object","properties":{}}}})],
            context_window: crate::agent::context_window::ContextWindowConfig {
                context_window_tokens: 32_000, reserved_output_tokens: 2_000,
                auto_compact_tokens: Some(6_000), compaction_target_tokens: Some(3_000),
                ..Default::default()
            },
            ..Default::default()
        }).unwrap();
        runtime
            .dispatch(AgentCommand::SubmitInput {
                origin: Default::default(),
                items: vec![AgentItem::user_message(
                    ItemId::from_string("long-input".into()),
                    "Continue the requested long task",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("long-submit".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            })
            .unwrap();
        let turn_id = runtime
            .session
            .execution
            .active_turn
            .as_ref()
            .unwrap()
            .turn_id
            .clone();
        let mut compacted = 0;
        let mut handoffs = 0;
        for index in 0..12 {
            while runtime
                .session
                .conversation
                .context_window
                .compaction_in_progress
            {
                if !runtime.session.conversation.context_window.pending_compaction.as_ref().unwrap().handoff {
                    assert!(runtime.session.conversation.context_window.used_tokens >= 6000,
                        "pending incremental compaction must retain full request pressure");
                }
                handoffs += usize::from(runtime.session.conversation.context_window.pending_compaction.as_ref().unwrap().handoff);
                compacted += 1;
                assert!(
                    compacted <= 20,
                    "maintenance did not yield to another business generation"
                );
                runtime.dispatch(AgentCommand::CompactionCompleted {
                    summary: format!("Earlier checkpoints through generation {index}; the user task is unfinished."),
                    usage: Default::default(),
                }).unwrap();
            }
            assert_eq!(
                runtime
                    .session
                    .execution
                    .active_turn
                    .as_ref()
                    .unwrap()
                    .status,
                AgentTurnStatus::AwaitingModel,
                "generation {index}, failure={:?}, compacted={compacted}",
                runtime
                    .session
                    .execution
                    .active_turn
                    .as_ref()
                    .unwrap()
                    .failure
            );
            runtime
                .dispatch(AgentCommand::ModelCompleted {
                    turn_id: turn_id.clone(),
                    items: vec![],
                    tool_calls: vec![crate::agent::protocol::ModelToolCall {
                        call_id: CallId::from_string(format!("progress-{index}")),
                        name: "fixture_read".into(),
                        permission_scope: None,
                        permission_rationale: None,
                        arguments: json!({"record":index}),
                    }],
                    usage: Default::default(),
                    output_limited: false,
                    effect_elapsed_millis: 1,
                })
                .unwrap();
            let tool = runtime
                .session
                .execution
                .active_turn
                .as_ref()
                .unwrap()
                .tools
                .last()
                .unwrap();
            assert_eq!(
                tool.status,
                ToolCallStatus::Running,
                "generation {index}: {tool:?}"
            );
            let call_id = tool.call_id.clone();
            runtime
                .dispatch(AgentCommand::ToolCompleted {
                    turn_id: turn_id.clone(),
                    call_id,
                    capability_id: "fixture_read".into(),
                    evidence: None,
                    output: json!({"record":index,"content":"Checkpoint detail. ".repeat(160)}),
                    outcome: crate::agent::host::HostToolOutcome::Succeeded,
                    activation: None,
                    full_output_ref: None,
                    artifacts: vec![],
                    skill_invocation: None,
                    skill_runtime_update: None,
                    effect_elapsed_millis: 1,
                })
                .unwrap();
        }
        while runtime
            .session
            .conversation
            .context_window
            .compaction_in_progress
        {
            handoffs += usize::from(runtime.session.conversation.context_window.pending_compaction.as_ref().unwrap().handoff);
            compacted += 1;
            assert!(compacted <= 20);
            runtime
                .dispatch(AgentCommand::CompactionCompleted {
                    summary:
                        "Earlier checkpoint progress; continue the original unfinished user task."
                            .into(),
                    usage: Default::default(),
                })
                .unwrap();
        }
        assert!(handoffs > 0, "automatic low-yield compaction must request a handoff");
        assert!(compacted >= 2, "later tool/model progress must retain further compaction opportunities; got {compacted}");
        assert_eq!(
            runtime
                .session
                .execution
                .active_turn
                .as_ref()
                .unwrap()
                .turn_id,
            turn_id
        );
        assert_eq!(
            runtime
                .session
                .execution
                .active_turn
                .as_ref()
                .unwrap()
                .status,
            AgentTurnStatus::AwaitingModel
        );
        assert!(runtime.session.conversation.model_context.iter().any(|item|
            matches!(item, AgentItem::Message { id, role: AgentRole::User, content } if id.as_str() == "long-input" && content == "Continue the requested long task")));
    }
}
