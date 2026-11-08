// SPDX-License-Identifier: Apache-2.0
//! Deterministic context accounting; callers supply measured or estimated facts.
use nola_domain::{ContextAccounting, ContextUsageSnapshot, TokenAccounting, UsageOrigin};

pub fn provider_usage_budget_lower_bound(usage: &TokenAccounting) -> usize {
    usage.known_token_lower_bound.max(usage.total_tokens.or_else(|| {
        usage.input_tokens.zip(usage.output_tokens).map(|(input, output)| input.saturating_add(output))
    }).unwrap_or(0))
}

/// Aggregate complete facts only. A missing field in any request stays unknown.
pub fn accumulate_token_accounting(total: &mut TokenAccounting, usage: TokenAccounting) {
    let first = total.request_count == 0;
    let add = |old: Option<usize>, new: Option<usize>| {
        if first { new } else { old.zip(new).and_then(|(a, b)| a.checked_add(b)) }
    };
    total.input_tokens = add(total.input_tokens, usage.input_tokens);
    total.cached_input_tokens = add(total.cached_input_tokens, usage.cached_input_tokens);
    total.cache_write_input_tokens = add(total.cache_write_input_tokens, usage.cache_write_input_tokens);
    total.output_tokens = add(total.output_tokens, usage.output_tokens);
    total.reasoning_tokens = add(total.reasoning_tokens, usage.reasoning_tokens);
    total.total_tokens = add(total.total_tokens, usage.total_tokens);
    total.known_token_lower_bound = total.known_token_lower_bound.saturating_add(provider_usage_budget_lower_bound(&usage));
    total.request_count = total.request_count.saturating_add(usage.request_count.max(1));
}

pub fn record_context_provider_usage(
    mut state: ContextAccounting,
    usage: TokenAccounting,
) -> ContextAccounting {
    accumulate_token_accounting(&mut state.cumulative, usage);
    if let Some(input) = usage.input_tokens {
        state.used_tokens = input;
        state.origin = UsageOrigin::Provider;
    }
    state
}

pub fn estimate_context_usage(
    mut state: ContextAccounting,
    item_tokens: impl IntoIterator<Item = usize>,
    additional_tokens: usize,
    rebase: bool,
) -> ContextAccounting {
    if !rebase && state.origin == UsageOrigin::Provider {
        return state;
    }
    state.used_tokens = item_tokens
        .into_iter()
        .fold(additional_tokens, usize::saturating_add);
    state.origin = UsageOrigin::Estimated;
    state
}

pub fn context_usage_percent(used_tokens: usize, window_tokens: usize) -> f64 {
    if window_tokens == 0 {
        return 0.0;
    }
    (used_tokens as f64 / window_tokens as f64 * 100.0).clamp(0.0, 100.0)
}

pub fn project_context_usage(
    state: ContextAccounting,
    window_tokens: usize,
    prompt_limit: usize,
) -> ContextUsageSnapshot {
    ContextUsageSnapshot {
        cache_usage: None,
        used_tokens: state.used_tokens,
        context_window_tokens: window_tokens,
        prompt_token_limit: prompt_limit,
        usage_percent: context_usage_percent(state.used_tokens, window_tokens),
        usage_is_estimated: state.origin == UsageOrigin::Estimated,
        cached_input_tokens: state.cumulative.cached_input_tokens.unwrap_or(0),
        cached_input_tokens_reported: state.cumulative.cached_input_tokens.is_some(),
        cache_write_input_tokens: state.cumulative.cache_write_input_tokens.unwrap_or(0),
        cache_write_input_tokens_reported: state.cumulative.cache_write_input_tokens.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> ContextAccounting {
        ContextAccounting {
            used_tokens: 0,
            origin: UsageOrigin::Estimated,
            cumulative: TokenAccounting::default(),
        }
    }

    #[test]
    fn measured_current_usage_is_not_cumulative_usage_and_rebase_clears_measurement() {
        let first = TokenAccounting {
            input_tokens: Some(100),
            cached_input_tokens: Some(90),
            output_tokens: Some(10),
            reasoning_tokens: Some(5),
            ..Default::default()
        };
        let measured = record_context_provider_usage(state(), first);
        let measured = record_context_provider_usage(measured, first);
        assert_eq!(measured.used_tokens, 100);
        assert_eq!(measured.cumulative.total_tokens, None);
        assert_eq!(measured.cumulative.known_token_lower_bound, 220);
        assert_eq!(measured.cumulative.cached_input_tokens, Some(180));
        assert_eq!(estimate_context_usage(measured, [2, 3], 4, false), measured);
        let unknown = record_context_provider_usage(measured, TokenAccounting::default());
        assert_eq!(unknown.cumulative.input_tokens, None);
        assert_eq!(unknown.cumulative.request_count, 3);
        assert_eq!(unknown.cumulative.known_token_lower_bound, 220);
        let rebased = estimate_context_usage(measured, [2, 3], 4, true);
        assert_eq!(rebased.used_tokens, 9);
        assert_eq!(rebased.origin, UsageOrigin::Estimated);
        assert_eq!(rebased.cumulative, measured.cumulative);
        let view = project_context_usage(rebased, 100, 80);
        assert_eq!(view.usage_percent, 9.0);
        assert_eq!(view.cached_input_tokens, 180);
        assert!(view.usage_is_estimated);
    }

    #[test]
    fn numeric_accounting_saturates_and_preserves_reported_zero() {
        let usage = TokenAccounting {
            input_tokens: Some(usize::MAX),
            output_tokens: Some(10),
            cache_write_input_tokens: Some(0),
            ..Default::default()
        };
        let measured =
            record_context_provider_usage(record_context_provider_usage(state(), usage), usage);
        assert_eq!(measured.cumulative.total_tokens, None);
        assert_eq!(measured.cumulative.known_token_lower_bound, usize::MAX);
        assert_eq!(measured.cumulative.cache_write_input_tokens, Some(0));
        assert_eq!(
            estimate_context_usage(state(), [usize::MAX, 1], 1, false).used_tokens,
            usize::MAX
        );
        assert_eq!(context_usage_percent(usize::MAX, 0), 0.0);
        assert_eq!(context_usage_percent(100, 10), 100.0);
        assert_eq!(
            provider_usage_budget_lower_bound(&TokenAccounting {
                total_tokens: Some(100),
                input_tokens: Some(1),
                ..Default::default()
            }),
            100
        );
    }

    #[test]
    fn absent_partial_zero_and_provider_total_are_distinct() {
        let zero = record_context_provider_usage(state(), TokenAccounting {
            input_tokens: Some(0), output_tokens: Some(0), total_tokens: Some(0),
            ..Default::default()
        });
        assert_eq!(zero.origin, UsageOrigin::Provider);
        assert_eq!(estimate_context_usage(zero, [100], 0, false), zero);
        assert_eq!(zero.cumulative.total_tokens, Some(0));
        let partial = record_context_provider_usage(zero, TokenAccounting {
            cached_input_tokens: Some(5), ..Default::default()
        });
        assert_eq!(partial.cumulative.total_tokens, None);
        assert_eq!(partial.cumulative.cached_input_tokens, None);
        let measured = record_context_provider_usage(state(), TokenAccounting {
            input_tokens: Some(10), output_tokens: Some(20), reasoning_tokens: Some(15),
            total_tokens: Some(30), ..Default::default()
        });
        assert_eq!(measured.cumulative.total_tokens, Some(30));
        assert_eq!(measured.cumulative.known_token_lower_bound, 30);
    }
}
