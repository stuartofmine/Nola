// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns context accounting and applies Kernel compaction transitions.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::protocol::{AgentItem, TokenUsage};

pub use nola_domain::UsageOrigin;

pub use nola_domain::CompactionReason;

/// Core-owned continuation state for an in-flight compaction. The Host sees
/// only the selected archive and can return only a summary. The plan retains
/// an immutable working-set witness, not a second mutable copy of its tail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingCompactionPlan {
    pub handoff: bool,
    pub reason: CompactionReason,
    pub working_set_item_count: usize,
    pub working_set_fingerprint: String,
    pub source_fingerprint: String,
    pub source_item_ids: Vec<super::protocol::ItemId>,
    pub original_token_count: usize,
    pub replaced_item_count: usize,
    pub summary_token_limit: usize,
}

/// Bounded, deterministic identity of the Core-planned archive. Serializing
/// into a digest avoids retaining an additional JSON-sized allocation.
pub fn compaction_source_fingerprint(items: &[AgentItem]) -> Result<String, String> {
    struct DigestWriter(Sha256);
    impl std::io::Write for DigestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = DigestWriter(Sha256::new());
    serde_json::to_writer(&mut writer, items)
        .map_err(|error| format!("identify compaction archive failed: {error}"))?;
    Ok(format!("{:x}", writer.0.finalize()))
}

pub use nola_domain::CompactionRetryState;

impl PendingCompactionPlan {
    /// Only append-only growth may be merged into an in-flight plan. Changes
    /// to the captured prefix invalidate the candidate; neither source history
    /// nor the current working set is changed on rejection.
    pub fn retained_items<'a>(&self, current: &'a [AgentItem]) -> Result<Vec<&'a AgentItem>, String> {
        let prefix = current.get(..self.working_set_item_count)
            .filter(|prefix| !prefix.is_empty())
            .ok_or_else(|| "compaction working-set witness is missing or truncated".to_string())?;
        if compaction_source_fingerprint(prefix)? != self.working_set_fingerprint {
            return Err("compaction working-set prefix changed; original context retained".into());
        }
        let selected: std::collections::HashSet<_> = self.source_item_ids.iter().collect();
        if selected.is_empty() || selected.len() != self.source_item_ids.len()
            || self.replaced_item_count != selected.len()
        {
            return Err("compaction archive identity is invalid".into());
        }
        let mut all_ids = std::collections::HashSet::new();
        if current.iter().any(|item| !all_ids.insert(item.id())) {
            return Err("compaction working set contains duplicate item identities".into());
        }
        let archive = prefix.iter().filter(|item| selected.contains(item.id()))
            .cloned().collect::<Vec<_>>();
        if archive.iter().map(AgentItem::id).ne(self.source_item_ids.iter())
            || compaction_source_fingerprint(&archive)? != self.source_fingerprint
        {
            return Err("compaction archive does not match its original sources".into());
        }
        // Append-only inputs remain in their original order and role. They are
        // not included in the summary's source lineage or replaced-item count.
        Ok(current.iter().filter(|item| !selected.contains(item.id())).collect())
    }

    pub fn validate_summary_for_prompt(
        &self, summary: &str, prompt_limit: usize, current: &[AgentItem],
    ) -> Result<(), String> {
        self.validate_summary(summary)?;
        let replacement = summary.chars().count().saturating_add(3) / 4 + 4;
        let retained = self.retained_items(current)?.iter()
            .fold(0usize, |total, item| total.saturating_add(item.estimated_tokens()));
        if retained.saturating_add(replacement) > prompt_limit {
            return Err("compacted context exceeds the current prompt budget; original context retained".into());
        }
        Ok(())
    }

    pub fn validate_summary(&self, summary: &str) -> Result<(), String> {
        nola_kernel::compaction_plan::validate_summary(summary, self.summary_token_limit)
            .map_err(str::to_string)?;
        let replacement_tokens = summary.chars().count().saturating_add(3) / 4 + 4;
        if replacement_tokens >= self.original_token_count {
            return Err("compaction summary does not reduce the archived context".into());
        }
        Ok(())
    }
}

pub use nola_domain::ContextUsageSnapshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWindowConfig {
    #[serde(default = "default_context_window_tokens")]
    pub context_window_tokens: usize,
    #[serde(default = "default_reserved_output_tokens")]
    pub reserved_output_tokens: usize,
    #[serde(default)]
    pub prompt_token_limit: Option<usize>,
    #[serde(default)]
    pub auto_compact_tokens: Option<usize>,
    #[serde(default)]
    pub compaction_target_tokens: Option<usize>,
    #[serde(default = "default_auto_compact_percent")]
    pub auto_compact_percent: u8,
    #[serde(default = "default_compaction_target_percent")]
    pub compaction_target_percent: u8,
}

impl Default for ContextWindowConfig {
    fn default() -> Self {
        Self::from_budget_facts(nola_domain::ContextBudgetFacts::default())
    }
}

fn default_context_window_tokens() -> usize {
    nola_domain::ContextBudgetFacts::default().context_window_tokens
}
fn default_reserved_output_tokens() -> usize {
    nola_domain::ContextBudgetFacts::default().reserved_output_tokens
}
fn default_auto_compact_percent() -> u8 {
    nola_domain::ContextBudgetFacts::default().auto_compact_percent
}
fn default_compaction_target_percent() -> u8 {
    nola_domain::ContextBudgetFacts::default().compaction_target_percent
}

impl ContextWindowConfig {
    pub fn budget_facts(&self) -> nola_domain::ContextBudgetFacts {
        nola_domain::ContextBudgetFacts {
            context_window_tokens: self.context_window_tokens,
            reserved_output_tokens: self.reserved_output_tokens,
            prompt_token_limit: self.prompt_token_limit,
            auto_compact_tokens: self.auto_compact_tokens,
            compaction_target_tokens: self.compaction_target_tokens,
            auto_compact_percent: self.auto_compact_percent,
            compaction_target_percent: self.compaction_target_percent,
        }
    }
    pub fn normalized(self) -> Self {
        Self::from_budget_facts(nola_kernel::normalize_context_budget(self.budget_facts()))
    }
    fn from_budget_facts(normalized: nola_domain::ContextBudgetFacts) -> Self {
        Self {
            context_window_tokens: normalized.context_window_tokens,
            reserved_output_tokens: normalized.reserved_output_tokens,
            prompt_token_limit: normalized.prompt_token_limit,
            auto_compact_tokens: normalized.auto_compact_tokens,
            compaction_target_tokens: normalized.compaction_target_tokens,
            auto_compact_percent: normalized.auto_compact_percent,
            compaction_target_percent: normalized.compaction_target_percent,
        }
    }
    pub fn prompt_token_limit(&self) -> usize {
        nola_kernel::context_prompt_token_limit(&self.budget_facts())
    }
    pub fn automatic_compaction_threshold(&self) -> usize {
        nola_kernel::context_automatic_compaction_threshold(&self.budget_facts())
    }
    pub fn compaction_target_tokens(&self) -> usize {
        nola_kernel::context_compaction_target_tokens(&self.budget_facts())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextWindowState {
    pub config: ContextWindowConfig,
    pub used_tokens: usize,
    #[serde(default)]
    model_estimate_tokens: usize,
    #[serde(default)]
    calibration_input_tokens: usize,
    #[serde(default)]
    calibration_estimate_tokens: usize,
    pub usage_origin: UsageOrigin,
    /// Current budget estimate/provider observation and last diagnostic have
    /// independent request owners. A new estimate cannot inherit an old source.
    pub measurement: Option<nola_domain::ContextUsageMeasurement>,
    pub prompt_cache_measurement: Option<nola_domain::ContextUsageMeasurement>,
    pub cumulative_usage: TokenUsage,
    pub compaction_generation: u64,
    pub compaction_in_progress: bool,
    #[serde(default)]
    pub pending_compaction: Option<PendingCompactionPlan>,
    pub consecutive_compaction_failures: u8,
    pub automatic_compaction_blocked: bool,
    pub compaction_retry: CompactionRetryState,
}

impl ContextWindowState {
    /// Cancellation discards only the pending projection, not original history
    /// or retry allowance. It is not a failed summarization attempt.
    pub fn cancel_compaction(&mut self) {
        self.pending_compaction = None;
        self.compaction_in_progress = false;
    }

    pub fn new(config: ContextWindowConfig) -> Self {
        Self {
            config: config.normalized(),
            used_tokens: 0,
            model_estimate_tokens: 0,
            calibration_input_tokens: 0,
            calibration_estimate_tokens: 0,
            usage_origin: UsageOrigin::Estimated,
            measurement: None,
            prompt_cache_measurement: None,
            cumulative_usage: TokenUsage::default(),
            compaction_generation: 0,
            compaction_in_progress: false,
            pending_compaction: None,
            consecutive_compaction_failures: 0,
            automatic_compaction_blocked: false,
            compaction_retry: CompactionRetryState::default(),
        }
    }

    /// Sealing is not a new model projection. Keep its complete input estimate
    /// or provider observation, especially after a committed handoff. Replacing
    /// it with history alone would falsely report the fixed prefix as reclaimed.
    pub(crate) fn seal_estimate(&mut self, items: &[AgentItem]) {
        if self.model_estimate_tokens == 0 {
            self.refresh_estimate(items);
        }
    }

    pub fn refresh_estimate(&mut self, items: &[AgentItem]) {
        self.refresh_estimate_with_additional_tokens(items, 0);
    }

    /// Estimate this complete model request, not the previous provider request.
    /// Persist calibration across Host restarts and include tool schema pressure.
    /// Never calibrate from maintenance requests, whose prompt is different.
    pub fn refresh_model_estimate(&mut self, items: &[AgentItem], tool_tokens: usize) {
        self.measurement = None;
        let estimate = items.iter().fold(tool_tokens, |sum, item|
            sum.saturating_add(item.estimated_tokens()));
        self.model_estimate_tokens = estimate;
        let calibrated = if self.calibration_estimate_tokens > 0 {
            (estimate as u128).saturating_mul(self.calibration_input_tokens as u128)
                .div_ceil(self.calibration_estimate_tokens as u128)
                .min(usize::MAX as u128) as usize
        } else { estimate };
        self.used_tokens = estimate.max(calibrated);
        self.usage_origin = UsageOrigin::Estimated;
    }

    /// Translate a full-request limit back into the history estimator's units.
    /// Call immediately after compiling/estimating the current model request.
    /// Non-history instructions and schemas remain reserved during compaction.
    pub fn history_budget_for_model_limit(&self, limit: usize, history_tokens: usize) -> usize {
        let uncalibrated_limit = if self.calibration_input_tokens > self.calibration_estimate_tokens
            && self.calibration_estimate_tokens > 0
        {
            ((limit as u128) * self.calibration_estimate_tokens as u128
                / self.calibration_input_tokens as u128).min(usize::MAX as u128) as usize
        } else { limit };
        let non_history = self.model_estimate_tokens.saturating_sub(history_tokens);
        uncalibrated_limit.saturating_sub(non_history)
    }

    fn accounting_facts(&self) -> nola_domain::ContextAccounting {
        nola_domain::ContextAccounting {
            used_tokens: self.used_tokens,
            origin: self.usage_origin,
            cumulative: self.cumulative_usage.accounting_facts(),
        }
    }

    fn apply_accounting(&mut self, facts: nola_domain::ContextAccounting) {
        self.used_tokens = facts.used_tokens;
        self.usage_origin = facts.origin;
        self.cumulative_usage.apply_accounting(facts.cumulative);
    }

    pub fn refresh_estimate_with_additional_tokens(
        &mut self,
        items: &[AgentItem],
        additional_tokens: usize,
    ) {
        let facts = nola_kernel::estimate_context_usage(
            self.accounting_facts(),
            items.iter().map(AgentItem::estimated_tokens),
            additional_tokens,
            false,
        );
        // Passive transcript updates retain the provider's measured accounting
        // until a new model projection replaces it. Keep its identity too.
        if facts.origin == UsageOrigin::Estimated {
            self.measurement = None;
        }
        self.apply_accounting(facts);
    }

    /// Provider usage describes the previous request; rebasing estimates the
    /// reconciled current context without changing cumulative billing facts.
    pub fn rebase_estimate(&mut self, items: &[AgentItem]) {
        self.measurement = None;
        self.apply_accounting(nola_kernel::estimate_context_usage(
            self.accounting_facts(),
            items.iter().map(AgentItem::estimated_tokens),
            0,
            true,
        ));
    }

    pub fn record_provider_usage(&mut self, usage: &TokenUsage) {
        self.measurement = None;
        self.prompt_cache_measurement = None;
        if usage.input_tokens.is_some_and(|input| input > 0) && self.model_estimate_tokens > 0
            && !usage.cache_usage.is_some_and(|cache| cache.is_compaction)
        {
            self.calibration_input_tokens = usage.input_tokens.unwrap_or(0);
            self.calibration_estimate_tokens = self.model_estimate_tokens;
        }
        self.cumulative_usage.cache_usage = usage.cache_usage;
        self.apply_accounting(nola_kernel::record_context_provider_usage(
            self.accounting_facts(),
            usage.accounting_facts(),
        ));
        // Provider-specific metadata is retained alongside the numeric ledger.
        if let Some(identity) = usage.prompt_cache_identity.as_ref() {
            self.cumulative_usage.prompt_cache_identity = Some(identity.clone());
        }
        self.cumulative_usage.prompt_cache_diagnostic = usage.prompt_cache_diagnostic.clone();
    }

    pub fn record_provider_context_usage(
        &mut self,
        used_tokens: usize,
        context_window_tokens: usize,
    ) {
        if let Some(update) = nola_kernel::record_provider_context_budget(
            self.config.budget_facts(),
            used_tokens,
            context_window_tokens,
        ) {
            self.config = ContextWindowConfig::from_budget_facts(update.config);
            self.used_tokens = update.used_tokens;
            self.usage_origin = UsageOrigin::Provider;
        }
    }

    pub fn usage_percent(&self) -> f64 {
        nola_kernel::context_usage_percent(self.used_tokens, self.config.context_window_tokens)
    }

    pub fn usage_snapshot(&self) -> ContextUsageSnapshot {
        let mut snapshot = nola_kernel::project_context_usage(
            self.accounting_facts(),
            self.config.context_window_tokens,
            self.config.prompt_token_limit(),
        );
        snapshot.cache_usage = self.cumulative_usage.cache_usage;
        snapshot
    }

    fn lifecycle_facts(&self) -> nola_domain::CompactionLifecycleState {
        nola_domain::CompactionLifecycleState {
            generation: self.compaction_generation,
            in_progress: self.compaction_in_progress,
            consecutive_failures: self.consecutive_compaction_failures,
            automatic_blocked: self.automatic_compaction_blocked,
            retry: self.compaction_retry.clone(),
        }
    }

    fn apply_lifecycle(&mut self, next: nola_domain::CompactionLifecycleState) {
        self.compaction_generation = next.generation;
        self.compaction_in_progress = next.in_progress;
        self.consecutive_compaction_failures = next.consecutive_failures;
        self.automatic_compaction_blocked = next.automatic_blocked;
        self.compaction_retry = next.retry;
    }

    pub fn should_compact(&self) -> bool {
        nola_kernel::should_compact(
            &self.lifecycle_facts(),
            self.used_tokens,
            self.config.automatic_compaction_threshold(),
        )
    }

    pub fn begin_compaction(&mut self, plan: PendingCompactionPlan) -> Result<(), String> {
        let next = nola_kernel::begin_compaction(&self.lifecycle_facts(), &plan.source_fingerprint)
            .map_err(|error| error.to_string())?;
        self.apply_lifecycle(next);
        self.pending_compaction = Some(plan);
        Ok(())
    }

    pub fn take_pending_compaction(&mut self) -> Option<PendingCompactionPlan> {
        self.pending_compaction.take()
    }

    pub fn finish_compaction(
        &mut self, items: &[AgentItem], usage: &TokenUsage, previous_working_tokens: usize,
    ) -> bool {
        let facts = self.lifecycle_facts();
        let non_history_tokens = self.model_estimate_tokens.saturating_sub(previous_working_tokens);
        let current_working_tokens = items.iter().fold(0usize,
            |sum, item| sum.saturating_add(item.estimated_tokens()));
        // Maintenance usage belongs to a different request. Preserve ordinary
        // calibration, then estimate the replacement with its non-history cost.
        let model_estimate = std::mem::take(&mut self.model_estimate_tokens);
        self.record_provider_usage(usage);
        self.model_estimate_tokens = model_estimate;
        self.refresh_model_estimate(items, non_history_tokens);
        let transition = nola_kernel::finish_compaction(
            &facts, previous_working_tokens, current_working_tokens,
        );
        self.apply_lifecycle(transition.state);
        transition.made_progress
    }

    pub fn fail_compaction(&mut self) {
        let plan = self.pending_compaction.take();
        let next = nola_kernel::fail_compaction(
            &self.lifecycle_facts(),
            plan.as_ref().map(|plan| plan.source_fingerprint.as_str()),
        );
        self.apply_lifecycle(next);
    }
}

#[cfg(test)]
mod usage_snapshot_tests {
    use super::{ContextWindowConfig, ContextWindowState, PendingCompactionPlan, UsageOrigin};
    use crate::agent::protocol::{AgentItem, AgentRole, ItemId, TokenUsage};

    #[test]
    fn compaction_history_budget_reserves_full_request_cost_and_calibration() {
        let mut state = ContextWindowState::new(ContextWindowConfig::default());
        let history = AgentItem::user_message(ItemId::from_string("history".into()), "a".repeat(400));
        let history_tokens = history.estimated_tokens();
        state.refresh_model_estimate(&[history.clone()], 1000);
        assert_eq!(state.history_budget_for_model_limit(2000, history_tokens), 1000);
        state.record_provider_usage(&TokenUsage {
            input_tokens: Some((history_tokens + 1000) * 2), ..Default::default()
        });
        let mut restored: ContextWindowState = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        restored.refresh_model_estimate(&[history], 1000);
        assert_eq!(restored.history_budget_for_model_limit(3000, history_tokens), 500);
        assert_eq!(restored.history_budget_for_model_limit(2000, history_tokens), 0);
        assert_eq!(restored.history_budget_for_model_limit(1000, history_tokens), 0);
        // Inverting calibration rounds down so the replacement cannot consume
        // more than the calibrated full-request allowance.
        assert_eq!(restored.history_budget_for_model_limit(3001, history_tokens), 500);
    }

    #[test]
    fn next_request_growth_is_calibrated_after_restart_and_idle_rebase() {
        let mut state = ContextWindowState::new(ContextWindowConfig::default());
        state.refresh_model_estimate(&[], 1000);
        state.record_provider_usage(&TokenUsage { input_tokens: Some(2000), ..Default::default() });
        let mut state: ContextWindowState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        state.rebase_estimate(&[]);
        state.refresh_model_estimate(&[], 1600);
        assert_eq!(state.used_tokens, 3200);
        assert_eq!(state.usage_origin, UsageOrigin::Estimated);
        assert_eq!(state.cumulative_usage.input_tokens, Some(2000));
    }

    #[test]
    fn compaction_retains_full_request_pressure_and_measures_history_progress() {
        let mut state = ContextWindowState::new(ContextWindowConfig::default());
        let old = AgentItem::user_message(ItemId::from_string("old".into()), "a".repeat(400));
        let previous = old.estimated_tokens();
        state.refresh_model_estimate(&[old], 9000);
        state.record_provider_usage(&TokenUsage {
            input_tokens: Some((previous + 9000) * 2), ..Default::default()
        });
        let larger = AgentItem::user_message(ItemId::from_string("replacement".into()), "a".repeat(800));
        let current = larger.estimated_tokens();
        let maintenance = TokenUsage {
            input_tokens: Some(50_000),
            cache_usage: Some(nola_domain::PromptCacheUsage::measured(Some(50_000), Some(0), None, true)),
            ..Default::default()
        };
        assert!(!state.finish_compaction(&[larger], &maintenance, previous));
        assert!(state.automatic_compaction_blocked);
        assert_eq!(state.used_tokens, (9000 + current) * 2);
        let mut restored: ContextWindowState = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert!(restored.finish_compaction(&[], &maintenance, current));
        assert_eq!(restored.used_tokens, 18000);
        assert!(!restored.automatic_compaction_blocked);
    }

    #[test]
    fn maintenance_usage_does_not_calibrate_the_next_normal_request() {
        let mut state = ContextWindowState::new(ContextWindowConfig::default());
        state.refresh_model_estimate(&[], 1000);
        state.record_provider_usage(&TokenUsage { input_tokens: Some(2000), ..Default::default() });
        state.record_provider_usage(&TokenUsage {
            input_tokens: Some(9000),
            cache_usage: Some(nola_domain::PromptCacheUsage::measured(
                Some(9000), Some(0), None, true)),
            ..Default::default()
        });
        state.refresh_model_estimate(&[], 1600);
        assert_eq!(state.used_tokens, 3200);
        assert_eq!(state.cumulative_usage.input_tokens, Some(11000));
    }

    #[test]
    fn snapshot_is_the_single_projection_of_estimated_and_provider_usage() {
        let mut state = ContextWindowState::new(ContextWindowConfig {
            context_window_tokens: 100_000,
            reserved_output_tokens: 10_000,
            ..ContextWindowConfig::default()
        });
        state.used_tokens = 25_000;
        let estimated = state.usage_snapshot();
        assert_eq!(estimated.used_tokens, 25_000);
        assert_eq!(estimated.context_window_tokens, 100_000);
        assert_eq!(estimated.prompt_token_limit, 90_000);
        assert_eq!(estimated.usage_percent, 25.0);
        assert!(estimated.usage_is_estimated);

        state.record_provider_usage(&TokenUsage {
            input_tokens: Some(40_000),
            ..TokenUsage::default()
        });
        let provider = state.usage_snapshot();
        assert_eq!(provider.used_tokens, 40_000);
        assert_eq!(state.usage_origin, UsageOrigin::Provider);
        assert!(!provider.usage_is_estimated);
    }

    #[test]
    fn passive_transcript_refresh_preserves_provider_identity_until_next_model_projection() {
        let mut state = ContextWindowState::new(ContextWindowConfig::default());
        state.record_provider_usage(&TokenUsage {
            input_tokens: Some(40_000),
            ..TokenUsage::default()
        });
        let measurement = nola_domain::ContextUsageMeasurement {
            request_id: "measured-request".into(),
            attempt_id: Some("physical-attempt".into()),
            context_generation: Some(0),
        };
        state.measurement = Some(measurement.clone());
        state.prompt_cache_measurement = Some(measurement.clone());
        let items = vec![AgentItem::user_message(ItemId::from_string("next".into()), "next input")];

        state.refresh_estimate_with_additional_tokens(&items, 100);
        assert_eq!(state.used_tokens, 40_000);
        assert_eq!(state.usage_origin, UsageOrigin::Provider);
        assert_eq!(state.measurement, Some(measurement.clone()));
        assert_eq!(state.prompt_cache_measurement, Some(measurement.clone()));

        state.refresh_model_estimate(&items, 100);
        assert_eq!(state.usage_origin, UsageOrigin::Estimated);
        assert_eq!(state.measurement, None);
        assert_eq!(state.prompt_cache_measurement, Some(measurement.clone()));
        assert_eq!(state.cumulative_usage.input_tokens, Some(40_000));

        state.rebase_estimate(&items);
        assert_eq!(state.measurement, None);
        assert_eq!(state.prompt_cache_measurement, Some(measurement));
        assert_eq!(state.cumulative_usage.input_tokens, Some(40_000));
    }

    #[test]
    fn pending_compaction_plan_is_checkpointed_inside_core() {
        let kept = AgentItem::Message {
            id: ItemId::from_string("current-user".to_string()),
            role: AgentRole::User,
            content: "keep this exact request".to_string(),
        };
        let mut state = ContextWindowState::new(ContextWindowConfig::default());
        state
            .begin_compaction(PendingCompactionPlan {
                handoff: false,
                reason: super::CompactionReason::Manual,
                working_set_item_count: 1,
                working_set_fingerprint: super::compaction_source_fingerprint(&[kept.clone()]).unwrap(),
                source_fingerprint: "test-archive".to_string(),
                source_item_ids: vec![ItemId::from_string("archived".to_string())],
                original_token_count: 900,
                replaced_item_count: 7,
                summary_token_limit: 512,
            })
            .unwrap();

        state.pending_compaction.as_mut().unwrap().handoff = true;
        let encoded = serde_json::to_vec(&state).unwrap();
        let mut restored: ContextWindowState = serde_json::from_slice(&encoded).unwrap();
        let pending = restored.take_pending_compaction().unwrap();
        assert!(pending.handoff, "cold restore must not restart incremental maintenance");

        assert_eq!(pending.working_set_item_count, 1);
        assert_eq!(pending.working_set_fingerprint,
            super::compaction_source_fingerprint(&[kept]).unwrap());
        assert_eq!(pending.replaced_item_count, 7);
        assert_eq!(pending.summary_token_limit, 512);
        assert!(restored.compaction_in_progress);
    }

    #[test]
    fn restored_compaction_plan_rejects_expansion_even_with_a_large_old_budget() {
        let source = vec![AgentItem::user_message(ItemId::from_string("greeting".into()), "hello")];
        let plan = PendingCompactionPlan {
            handoff: false,
            reason: super::CompactionReason::ThresholdReached,
            working_set_item_count: source.len(),
            working_set_fingerprint: super::compaction_source_fingerprint(&source).unwrap(),
            source_fingerprint: super::compaction_source_fingerprint(&source).unwrap(),
            source_item_ids: vec![ItemId::from_string("greeting".into())],
            original_token_count: 19,
            replaced_item_count: 1,
            summary_token_limit: 57_600,
        };
        let restored: PendingCompactionPlan = serde_json::from_value(serde_json::to_value(plan).unwrap()).unwrap();
        assert!(restored.validate_summary(&"x".repeat(5_176)).unwrap_err().contains("does not reduce"));
        assert!(restored.validate_summary(&"x".repeat(60)).is_err()); // 15 + 4 == 19
        assert!(restored.validate_summary("Greeting exchanged.").is_ok());
        assert!(restored.validate_summary_for_prompt("Greeting exchanged.", 8, &source).is_err());
        assert!(restored.validate_summary_for_prompt("Greeting exchanged.", 9, &source).is_ok());
    }

    #[test]
    fn estimated_usage_includes_model_tool_schema_pressure() {
        let item = AgentItem::Message {
            id: ItemId::from_string("user".to_string()),
            role: AgentRole::User,
            content: "hello".to_string(),
        };
        let mut state = ContextWindowState::new(ContextWindowConfig::default());
        let item_tokens = item.estimated_tokens();

        state.refresh_estimate_with_additional_tokens(&[item], 7_250);

        assert_eq!(state.used_tokens, item_tokens + 7_250);
        assert_eq!(state.usage_origin, UsageOrigin::Estimated);
    }

    #[test]
    fn cumulative_cache_usage_never_replaces_current_context_usage() {
        let mut state = ContextWindowState::new(ContextWindowConfig {
            context_window_tokens: 1_000_000,
            reserved_output_tokens: 8_192,
            ..ContextWindowConfig::default()
        });

        for _ in 0..32 {
            state.record_provider_usage(&TokenUsage {
                cache_usage: Some(nola_domain::PromptCacheUsage::measured(Some(80_000), Some(72_000), Some(0), false)),
                input_tokens: Some(80_000),
                cached_input_tokens: Some(72_000),
                ..TokenUsage::default()
            });
        }

        let snapshot = state.usage_snapshot();
        assert_eq!(snapshot.used_tokens, 80_000);
        assert_eq!(snapshot.context_window_tokens, 1_000_000);
        assert_eq!(snapshot.usage_percent, 8.0);
        assert_eq!(snapshot.cached_input_tokens, 2_304_000);
        assert!(snapshot.cached_input_tokens_reported);
        assert_eq!(snapshot.cache_usage.unwrap().hit_rate_bps, Some(9000));
        let restored: ContextWindowState = serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert_eq!(restored.usage_snapshot().cache_usage, snapshot.cache_usage);
        state.rebase_estimate(&[]);
        assert_eq!(state.usage_snapshot().cache_usage, snapshot.cache_usage);
        state.record_provider_usage(&TokenUsage::default());
        assert!(state.usage_snapshot().cache_usage.is_none());
    }
}

#[cfg(test)]
mod lifecycle_checkpoint_tests {
    use super::*;
    use crate::agent::recovery::CheckpointReason;
    use crate::agent::{AgentCore, AgentCoreConfig};

    #[test]
    fn next_estimate_does_not_relabel_previous_provider_diagnostic() {
        let source = nola_domain::ContextUsageMeasurement {
            request_id: "request-one".into(), attempt_id: Some("physical-one".into()), context_generation: Some(2),
        };
        let mut state = ContextWindowState::new(ContextWindowConfig::default());
        state.measurement = Some(source.clone());
        state.prompt_cache_measurement = Some(source.clone());
        state.cumulative_usage.prompt_cache_diagnostic = Some(crate::prompt_cache::PromptCacheDiagnostic {
            cache_usage: None, schema_version: 1, status: crate::prompt_cache::PromptCacheDiagnosticStatus::Hit,
            identity_stable: true, identity_sha256: Some("a".repeat(64)), reasons: vec![], fragment_changes: vec![],
            cache_read_input_tokens: Some(100), cache_write_input_tokens: Some(0),
        });
        state.refresh_model_estimate(&[], 200);
        assert!(state.measurement.is_none(), "estimate must be explicitly bound to its own request");
        assert_eq!(state.prompt_cache_measurement, Some(source.clone()));
        state.measurement = Some(nola_domain::ContextUsageMeasurement {
            request_id: "request-two".into(), attempt_id: None, context_generation: Some(2),
        });
        let restored = cold(state);
        assert_eq!(restored.measurement.as_ref().unwrap().request_id, "request-two");
        assert_eq!(restored.prompt_cache_measurement, Some(source));
        assert_eq!(restored.cumulative_usage.prompt_cache_diagnostic.unwrap().cache_read_input_tokens, Some(100));
    }

    fn cold(state: ContextWindowState) -> ContextWindowState {
        let config = AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(crate::agent::SessionId::from_string(
                "compaction-cold-fixture".into(),
            )),
            ..AgentCoreConfig::default()
        };
        let core = AgentCore::try_new(config.clone()).unwrap();
        let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
        checkpoint.state.conversation.context_window = state;
        let wire = serde_json::to_vec(&checkpoint).unwrap();
        let checkpoint = serde_json::from_slice(&wire).unwrap();
        AgentCore::restore(config, checkpoint)
            .unwrap()
            .checkpoint(CheckpointReason::Manual)
            .state
            .conversation
            .context_window
    }
    fn plan(source: &str) -> PendingCompactionPlan {
        PendingCompactionPlan {
            handoff: false,
            reason: super::CompactionReason::Manual,
            working_set_item_count: 1,
            working_set_fingerprint: source.into(),
            source_fingerprint: source.into(),
            source_item_ids: vec![],
            original_token_count: 0,
            replaced_item_count: 2,
            summary_token_limit: 256,
        }
    }
    #[test]
    fn retry_limit_and_archive_identity_survive_real_core_checkpoint_restore() {
        let mut state = ContextWindowState::new(ContextWindowConfig::default());
        state.used_tokens = state.config.automatic_compaction_threshold();
        for count in 1..=3 {
            state.begin_compaction(plan("archive-original")).unwrap();
            state = cold(state);
            assert_eq!(
                state
                    .pending_compaction
                    .as_ref()
                    .unwrap()
                    .source_fingerprint,
                "archive-original"
            );
            state.fail_compaction();
            state = cold(state);
            assert_eq!(state.compaction_retry.failed_attempts, count);
            assert_eq!(state.should_compact(), count < 3);
        }
        let before = serde_json::to_value(&state).unwrap();
        assert!(state
            .begin_compaction(plan("archive-original"))
            .unwrap_err()
            .contains("three times"));
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
        state.begin_compaction(plan("archive-changed")).unwrap();
        state = cold(state);
        assert_eq!(state.compaction_retry.failed_attempts, 0);
        assert!(state.automatic_compaction_blocked);
    }
    #[test]
    fn no_progress_block_and_later_progress_keep_usage_accounting_on_restore() {
        let mut state = ContextWindowState::new(ContextWindowConfig::default());
        state.used_tokens = 0;
        state.begin_compaction(plan("archive")).unwrap();
        state.take_pending_compaction().unwrap();
        let usage = TokenUsage {
            input_tokens: Some(100),
            output_tokens: Some(20),
            total_tokens: Some(120),
            ..Default::default()
        };
        assert!(!state.finish_compaction(&[], &usage, 0));
        state = cold(state);
        assert_eq!(state.compaction_generation, 1);
        assert_eq!(state.consecutive_compaction_failures, 1);
        assert!(state.automatic_compaction_blocked);
        assert_eq!(state.cumulative_usage.total_tokens, Some(120));
        assert_eq!(state.compaction_retry, CompactionRetryState::default());
        state.used_tokens = 10;
        state.begin_compaction(plan("changed")).unwrap();
        state.take_pending_compaction().unwrap();
        assert!(state.finish_compaction(&[], &usage, 10));
        state = cold(state);
        assert!(!state.automatic_compaction_blocked);
        assert_eq!(state.consecutive_compaction_failures, 0);
        assert_eq!(state.compaction_generation, 2);
        assert_eq!(state.cumulative_usage.total_tokens, Some(240));
        assert_eq!(state.used_tokens, 0);
        assert_eq!(state.usage_origin, UsageOrigin::Estimated);
    }
}

#[cfg(test)]
mod context_budget_checkpoint_tests {
    use super::*;
    #[test]
    fn explicit_config_budget_survives_core_checkpoint_without_wire_changes() {
        let config: ContextWindowConfig = serde_json::from_value(serde_json::json!({"context_window_tokens":0,"reserved_output_tokens":0,"prompt_token_limit":0,"auto_compact_tokens":0,"compaction_target_tokens":0,"auto_compact_percent":0,"compaction_target_percent":255})).unwrap();
        assert_eq!(config.prompt_token_limit(), 0);
        let normalized = config.normalized();
        assert_eq!(normalized.prompt_token_limit(), 1);
        assert_eq!(normalized.automatic_compaction_threshold(), 1);
        assert_eq!(normalized.compaction_target_tokens(), 1);
        let core_config = crate::agent::AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(crate::agent::SessionId::from_string(
                "budget-checkpoint".into(),
            )),
            context_window: normalized.clone(),
            ..Default::default()
        };
        let core = crate::agent::AgentCore::try_new(core_config.clone()).unwrap();
        let checkpoint = core.checkpoint(crate::agent::recovery::CheckpointReason::Manual);
        let wire = serde_json::to_vec(&checkpoint).unwrap();
        let restored =
            crate::agent::AgentCore::restore(core_config, serde_json::from_slice(&wire).unwrap())
                .unwrap();
        let state = restored
            .checkpoint(crate::agent::recovery::CheckpointReason::Manual)
            .state
            .conversation
            .context_window;
        assert_eq!(
            serde_json::to_value(state.config).unwrap(),
            serde_json::to_value(normalized).unwrap()
        );
        let default: ContextWindowConfig = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(
            serde_json::to_value(default).unwrap(),
            serde_json::to_value(ContextWindowConfig::default()).unwrap()
        );
    }
}
