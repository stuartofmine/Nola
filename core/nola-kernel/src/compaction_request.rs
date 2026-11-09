// SPDX-License-Identifier: Apache-2.0
//! Compaction admission and continuation policy; no execution or persistence.

use nola_domain::{
    CompactionReason, CompactionRequestFacts, CompactionRequestRejection,
    EmptyCompactionArchiveAction,
};

/// Returns the admitted consecutive-maintenance count without mutating rejected
/// state. Core rearms it after fresh business-generation progress in a long Run.
pub fn admit_compaction_request(
    facts: CompactionRequestFacts,
) -> Result<Option<u8>, CompactionRequestRejection> {
    if facts.compaction_in_progress {
        return Err(CompactionRequestRejection::AlreadyActive);
    }
    if facts.context_is_empty {
        return Err(CompactionRequestRejection::EmptyContext);
    }
    if facts.reason == CompactionReason::Manual {
        return Ok(facts.automatic_compaction_count);
    }
    match facts.automatic_compaction_count {
        Some(count) if count >= 2 => Err(CompactionRequestRejection::AutomaticLimitReached),
        Some(count) => Ok(Some(count.saturating_add(1))),
        None => Ok(None),
    }
}

pub fn compaction_request_target(
    reason: CompactionReason,
    configured_target: usize,
    estimated_used_tokens: usize,
) -> usize {
    if matches!(
        reason,
        CompactionReason::Manual | CompactionReason::ContextLimitRecovery
    ) {
        // A provider overflow is authoritative evidence that the configured
        // target is too large for this request, even when the configured or
        // probed context window claimed otherwise. Force measurable progress
        // from the rejected working set instead of planning against the same
        // oversized target and failing to find an archive.
        configured_target
            .min(estimated_used_tokens.saturating_mul(60) / 100)
            .max(1)
    } else {
        configured_target
    }
}

/// The full-request working target is soft. A large mandatory prefix must not
/// turn its last few tokens into the budget for the entire history. Reserve a
/// quarter of the existing history for the replacement working set (recent
/// complete transactions plus summary), bounded by actual hard input capacity.
/// Normal targets with enough history space remain unchanged.
pub fn compaction_history_target(
    preferred_target: usize,
    hard_history_limit: usize,
    history_tokens: usize,
) -> usize {
    preferred_target.max(history_tokens.div_ceil(4)).min(hard_history_limit)
}

/// Soft-threshold maintenance must reclaim enough history to justify another
/// model round. Hard-budget recovery and explicit manual requests are exempt.
/// The summary budget is an upper bound; include the checkpoint framing cost.
pub fn defer_low_gain_compaction(
    reason: CompactionReason,
    archive_tokens: usize,
    summary_budget: usize,
    full_request_tokens: usize,
    full_prompt_limit: usize,
) -> bool {
    reason == CompactionReason::ThresholdReached
        && full_request_tokens < full_prompt_limit
        && archive_tokens.saturating_sub(summary_budget.saturating_add(4)) < 512
}

pub fn empty_compaction_archive_action(reason: CompactionReason) -> EmptyCompactionArchiveAction {
    match reason {
        CompactionReason::Manual => EmptyCompactionArchiveAction::CompleteWithoutSummary,
        CompactionReason::ThresholdReached => EmptyCompactionArchiveAction::ResumeModel,
        CompactionReason::ContextLimitRecovery => EmptyCompactionArchiveAction::Reject,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_automatic_attempts_are_bounded_and_manual_requests_do_not_consume_them() {
        let mut facts = CompactionRequestFacts {
            reason: CompactionReason::ThresholdReached,
            compaction_in_progress: false,
            context_is_empty: false,
            automatic_compaction_count: Some(0),
        };
        facts.automatic_compaction_count = admit_compaction_request(facts).unwrap();
        assert_eq!(facts.automatic_compaction_count, Some(1));
        facts.reason = CompactionReason::ContextLimitRecovery;
        facts.automatic_compaction_count = admit_compaction_request(facts).unwrap();
        assert_eq!(facts.automatic_compaction_count, Some(2));
        assert_eq!(
            admit_compaction_request(facts),
            Err(CompactionRequestRejection::AutomaticLimitReached)
        );
        facts.reason = CompactionReason::Manual;
        assert_eq!(admit_compaction_request(facts), Ok(Some(2)));
        facts.automatic_compaction_count = None;
        assert_eq!(admit_compaction_request(facts), Ok(None));
        facts.context_is_empty = true;
        assert_eq!(
            admit_compaction_request(facts),
            Err(CompactionRequestRejection::EmptyContext)
        );
        facts.compaction_in_progress = true;
        assert_eq!(
            admit_compaction_request(facts),
            Err(CompactionRequestRejection::AlreadyActive)
        );
    }

    #[test]
    fn low_gain_threshold_work_defers_without_disabling_hard_recovery() {
        for (archive, summary) in [(459, 200), (440, 180), (153, 70)] {
            assert!(defer_low_gain_compaction(CompactionReason::ThresholdReached, archive, summary, 30000, 54068));
            assert!(!defer_low_gain_compaction(CompactionReason::ThresholdReached, archive, summary, 54068, 54068));
            assert!(!defer_low_gain_compaction(CompactionReason::ContextLimitRecovery, archive, summary, 30000, 54068));
            assert!(!defer_low_gain_compaction(CompactionReason::Manual, archive, summary, 30000, 54068));
        }
        assert!(!defer_low_gain_compaction(CompactionReason::ThresholdReached, 1028, 512, 30000, 54068));
        assert!(defer_low_gain_compaction(CompactionReason::ThresholdReached, 1027, 512, 30000, 54068));
        // A constrained short summary can make a smaller archive worthwhile.
        assert!(!defer_low_gain_compaction(CompactionReason::ThresholdReached, 800, 100, 30000, 54068));
    }

    #[test]
    fn history_allocation_preserves_working_set_without_exceeding_capacity() {
        assert_eq!(compaction_history_target(620, 700_000, 33_376), 8_344);
        assert_eq!(compaction_history_target(57_600, 128_000, 96_000), 57_600);
        // Small model windows and growing mandatory prefixes still win.
        assert_eq!(compaction_history_target(20, 700, 20_000), 700);
        assert_eq!(compaction_history_target(0, 0, 20_000), 0);
        // Repeated maintenance applies to the current working set, not all
        // lifetime transcript tokens; a small archive cannot inflate a target.
        assert_eq!(compaction_history_target(0, 10_000, 2_001), 501);
    }

    #[test]
    fn explicit_and_overflow_targets_preserve_request_reason() {
        assert_eq!(
            compaction_request_target(CompactionReason::Manual, 900, 1000),
            600
        );
        assert_eq!(
            compaction_request_target(CompactionReason::Manual, 100, 1000),
            100
        );
        assert_eq!(
            compaction_request_target(CompactionReason::Manual, 900, 0),
            1
        );
        assert_eq!(
            compaction_request_target(CompactionReason::ThresholdReached, 900, 1000),
            900
        );
        assert_eq!(
            compaction_request_target(CompactionReason::ContextLimitRecovery, 900, 1000),
            600
        );
        assert_eq!(
            compaction_request_target(CompactionReason::ContextLimitRecovery, 100, 1000),
            100
        );
        assert_eq!(
            empty_compaction_archive_action(CompactionReason::ThresholdReached),
            EmptyCompactionArchiveAction::ResumeModel
        );
        assert_eq!(
            empty_compaction_archive_action(CompactionReason::Manual),
            EmptyCompactionArchiveAction::CompleteWithoutSummary
        );
        assert_eq!(
            empty_compaction_archive_action(CompactionReason::ContextLimitRecovery),
            EmptyCompactionArchiveAction::Reject
        );
    }
}
