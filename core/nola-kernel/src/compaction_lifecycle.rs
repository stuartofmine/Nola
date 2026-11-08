// SPDX-License-Identifier: Apache-2.0
//! Pure compaction admission, retry and progress policy.
use nola_domain::{
    BeginCompactionError, CompactionLifecycleState, CompactionRetryState,
    FinishedCompactionTransition,
};

pub fn should_compact(
    state: &CompactionLifecycleState,
    used_tokens: usize,
    threshold: usize,
) -> bool {
    !state.in_progress && !state.automatic_blocked && used_tokens >= threshold
}

pub fn begin_compaction(
    state: &CompactionLifecycleState,
    source_fingerprint: &str,
) -> Result<CompactionLifecycleState, BeginCompactionError> {
    if state.in_progress {
        return Err(BeginCompactionError::AlreadyActive);
    }
    let mut next = state.clone();
    if state.retry.source_fingerprint.as_deref() == Some(source_fingerprint) {
        if state.retry.failed_attempts >= 3 {
            return Err(BeginCompactionError::ArchiveRetryLimit);
        }
    } else {
        next.retry = CompactionRetryState {
            source_fingerprint: Some(source_fingerprint.into()),
            failed_attempts: 0,
        };
    }
    next.in_progress = true;
    Ok(next)
}

pub fn finish_compaction(
    state: &CompactionLifecycleState,
    previous_used_tokens: usize,
    current_used_tokens: usize,
) -> FinishedCompactionTransition {
    let mut next = state.clone();
    next.generation = state.generation.saturating_add(1);
    next.in_progress = false;
    next.retry = CompactionRetryState::default();
    let made_progress = current_used_tokens < previous_used_tokens;
    if made_progress {
        next.consecutive_failures = 0;
        next.automatic_blocked = false;
    } else {
        next.consecutive_failures = state.consecutive_failures.saturating_add(1);
        next.automatic_blocked = true;
    }
    FinishedCompactionTransition {
        state: next,
        made_progress,
    }
}

pub fn fail_compaction(
    state: &CompactionLifecycleState,
    pending_source_fingerprint: Option<&str>,
) -> CompactionLifecycleState {
    let Some(source) = pending_source_fingerprint else {
        return state.clone();
    };
    let mut next = state.clone();
    next.in_progress = false;
    next.retry.source_fingerprint = Some(source.into());
    next.retry.failed_attempts = state.retry.failed_attempts.saturating_add(1);
    next.consecutive_failures = state.consecutive_failures.saturating_add(1);
    next.automatic_blocked = next.consecutive_failures >= 3;
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_identity_limit_and_failure_saturation() {
        let mut state = CompactionLifecycleState::default();
        for attempt in 1..=3 {
            state = begin_compaction(&state, "archive").unwrap();
            assert_eq!(
                begin_compaction(&state, "changed"),
                Err(BeginCompactionError::AlreadyActive)
            );
            state = fail_compaction(&state, Some("archive"));
            assert_eq!(state.retry.failed_attempts, attempt);
            assert_eq!(state.automatic_blocked, attempt == 3);
        }
        assert_eq!(
            begin_compaction(&state, "archive"),
            Err(BeginCompactionError::ArchiveRetryLimit)
        );
        let changed = begin_compaction(&state, "changed").unwrap();
        assert_eq!(changed.retry.failed_attempts, 0);
        assert_eq!(fail_compaction(&changed, None), changed);
        let saturated = CompactionLifecycleState {
            consecutive_failures: u8::MAX,
            retry: CompactionRetryState {
                failed_attempts: u8::MAX,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            fail_compaction(&saturated, Some("archive")).consecutive_failures,
            u8::MAX
        );
    }
    #[test]
    fn progress_threshold_and_block_policy() {
        for active in [false, true] {
            for blocked in [false, true] {
                for used in [0, 9, 10, 11] {
                    let state = CompactionLifecycleState {
                        in_progress: active,
                        automatic_blocked: blocked,
                        generation: u64::MAX,
                        ..Default::default()
                    };
                    assert_eq!(
                        should_compact(&state, used, 10),
                        !active && !blocked && used >= 10
                    );
                    let finish = finish_compaction(&state, 10, used);
                    assert_eq!(finish.made_progress, used < 10);
                    assert_eq!(finish.state.automatic_blocked, used >= 10);
                    assert_eq!(finish.state.generation, u64::MAX);
                    assert!(!finish.state.in_progress);
                }
            }
        }
    }
}
