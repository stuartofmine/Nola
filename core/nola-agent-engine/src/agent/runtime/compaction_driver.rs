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
        self.session
            .conversation
            .context_window
            .rebase_estimate(&self.session.conversation.model_context);
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
        let target_tokens = nola_kernel::compaction_request_target(
            reason,
            configured_target_tokens,
            self.session.conversation.context_window.used_tokens,
        );
        let target_tokens = nola_kernel::compaction_history_target(
            target_tokens, prompt_limit, history_tokens,
        );
        let plan = plan_compaction(&self.session.conversation.model_context, target_tokens);
        let retained_tokens = plan.kept_items.iter()
            .fold(0usize, |sum, item| sum.saturating_add(item.estimated_tokens()));
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
        if reason == CompactionReason::ThresholdReached && archive_tokens < 64 {
            return self.request_model_after_compaction(events, effects);
        }
        let summary_budget = if plan.items_to_summarize.is_empty() {
            None
        } else {
            match plan.summary_token_budget(target_tokens, prompt_limit) {
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
            estimated_tokens: self.session.conversation.context_window.used_tokens,
            source_fingerprint,
            source_item_count,
            original_token_count,
        });
        effects.push(AgentEffect::CompactContext {
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
