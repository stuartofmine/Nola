// SPDX-License-Identifier: Apache-2.0
//! Pure context budget normalization and threshold policy.
use nola_domain::ContextBudgetFacts;

pub fn normalize_context_budget(mut config: ContextBudgetFacts) -> ContextBudgetFacts {
    config.context_window_tokens = config.context_window_tokens.clamp(2_000, 4_000_000);
    config.reserved_output_tokens = config
        .reserved_output_tokens
        .clamp(256, config.context_window_tokens.saturating_sub(1));
    let maximum_prompt_tokens = config
        .context_window_tokens
        .saturating_sub(config.reserved_output_tokens)
        .max(1);
    config.prompt_token_limit = config
        .prompt_token_limit
        .map(|tokens| tokens.clamp(1, maximum_prompt_tokens));
    config.auto_compact_percent = config.auto_compact_percent.clamp(50, 95);
    config.compaction_target_percent = config
        .compaction_target_percent
        .clamp(20, config.auto_compact_percent.saturating_sub(5));
    let prompt_token_limit = context_prompt_token_limit(&config);
    config.auto_compact_tokens = config
        .auto_compact_tokens
        .map(|tokens| tokens.clamp(1, prompt_token_limit));
    let automatic_threshold = context_automatic_compaction_threshold(&config);
    config.compaction_target_tokens = config
        .compaction_target_tokens
        .map(|tokens| tokens.clamp(1, automatic_threshold.saturating_sub(1).max(1)));
    config
}

pub fn context_prompt_token_limit(config: &ContextBudgetFacts) -> usize {
    config.prompt_token_limit.unwrap_or_else(|| {
        config
            .context_window_tokens
            .saturating_sub(config.reserved_output_tokens)
    })
}

pub fn context_automatic_compaction_threshold(config: &ContextBudgetFacts) -> usize {
    let configured = config.auto_compact_tokens.unwrap_or_else(|| {
        context_prompt_token_limit(&config).saturating_mul(config.auto_compact_percent as usize)
            / 100
    });
    // A percentage alone is insufficient for small windows or large output
    // reservations. Leave output plus maintenance-instruction room before a
    // full-prefix fork becomes impossible. Clamp for small supported windows.
    let maintenance_reserve = config.reserved_output_tokens.max(8_000)
        .saturating_add(2_048).min(config.context_window_tokens / 2);
    configured.min(config.context_window_tokens.saturating_sub(maintenance_reserve).max(1))
}

pub fn context_compaction_target_tokens(config: &ContextBudgetFacts) -> usize {
    config
        .compaction_target_tokens
        .unwrap_or_else(|| {
            context_prompt_token_limit(&config)
                .saturating_mul(config.compaction_target_percent as usize)
                / 100
        })
        .min(
            context_automatic_compaction_threshold(&config)
                .saturating_sub(1)
                .max(1),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn facts() -> ContextBudgetFacts {
        ContextBudgetFacts {
            context_window_tokens: 128_000,
            reserved_output_tokens: 8192,
            prompt_token_limit: None,
            auto_compact_tokens: None,
            compaction_target_tokens: None,
            auto_compact_percent: 80,
            compaction_target_percent: 45,
        }
    }
    #[test]
    fn compaction_reserves_maintenance_space_even_with_an_explicit_late_threshold() {
        let mut config = facts();
        config.context_window_tokens = 16_000;
        config.reserved_output_tokens = 1_000;
        config.auto_compact_tokens = Some(15_000);
        assert_eq!(context_automatic_compaction_threshold(&config), 8_000);
        config.context_window_tokens = 128_000;
        config.reserved_output_tokens = 16_000;
        config.auto_compact_tokens = Some(120_000);
        assert_eq!(context_automatic_compaction_threshold(&config), 109_952);
        config.auto_compact_tokens = Some(60_000);
        assert_eq!(context_automatic_compaction_threshold(&config), 60_000);
    }

    #[test]
    fn default_and_explicit_zero_preserve_distinct_contracts() {
        let normal = normalize_context_budget(facts());
        assert_eq!(context_prompt_token_limit(&normal), 119_808);
        assert_eq!(context_automatic_compaction_threshold(&normal), 95_846);
        assert_eq!(context_compaction_target_tokens(&normal), 53_913);
        let zero = ContextBudgetFacts {
            context_window_tokens: 0,
            reserved_output_tokens: 0,
            prompt_token_limit: Some(0),
            auto_compact_tokens: Some(0),
            compaction_target_tokens: Some(0),
            auto_compact_percent: 0,
            compaction_target_percent: 0,
        };
        assert_eq!(context_prompt_token_limit(&zero), 0);
        assert_eq!(context_automatic_compaction_threshold(&zero), 0);
        assert_eq!(context_compaction_target_tokens(&zero), 0);
        let normalized = normalize_context_budget(zero);
        assert_eq!(normalized.context_window_tokens, 2000);
        assert_eq!(normalized.reserved_output_tokens, 256);
        assert_eq!(normalized.prompt_token_limit, Some(1));
        assert_eq!(normalized.auto_compact_tokens, Some(1));
        assert_eq!(normalized.compaction_target_tokens, Some(1));
        assert_eq!(normalized.auto_compact_percent, 50);
        assert_eq!(normalized.compaction_target_percent, 20);
    }
    #[test]
    fn oversized_inputs_saturate_and_normalization_is_idempotent() {
        for window in [0, 1999, 2000, 128_000, 4_000_001, usize::MAX] {
            for reserved in [0, 256, 8192, usize::MAX] {
                for explicit in [None, Some(0), Some(1), Some(usize::MAX)] {
                    let value = ContextBudgetFacts {
                        context_window_tokens: window,
                        reserved_output_tokens: reserved,
                        prompt_token_limit: explicit,
                        auto_compact_tokens: explicit,
                        compaction_target_tokens: explicit,
                        auto_compact_percent: u8::MAX,
                        compaction_target_percent: u8::MAX,
                    };
                    let normalized = normalize_context_budget(value);
                    assert_eq!(normalize_context_budget(normalized), normalized);
                    assert!((2000..=4_000_000).contains(&normalized.context_window_tokens));
                    assert!(normalized.reserved_output_tokens < normalized.context_window_tokens);
                    assert!(context_prompt_token_limit(&normalized) >= 1);
                    assert_eq!(normalized.auto_compact_percent, 95);
                    assert_eq!(normalized.compaction_target_percent, 90);
                }
            }
        }
        let tiny = normalize_context_budget(ContextBudgetFacts {
            context_window_tokens: 0,
            reserved_output_tokens: usize::MAX,
            ..facts()
        });
        assert_eq!(context_prompt_token_limit(&tiny), 1);
        assert_eq!(context_automatic_compaction_threshold(&tiny), 0);
        assert_eq!(context_compaction_target_tokens(&tiny), 0);
    }
}

pub fn record_provider_context_budget(
    config: ContextBudgetFacts,
    used_tokens: usize,
    reported_window: usize,
) -> Option<nola_domain::ProviderContextBudgetUpdate> {
    if reported_window == 0 {
        return None;
    }
    let next = if config.context_window_tokens != reported_window {
        normalize_context_budget(ContextBudgetFacts {
            context_window_tokens: reported_window,
            ..config
        })
    } else {
        config
    };
    Some(nola_domain::ProviderContextBudgetUpdate {
        config: next,
        used_tokens: used_tokens.min(next.context_window_tokens),
    })
}

#[cfg(test)]
mod provider_budget_tests {
    use super::*;
    #[test]
    fn zero_report_is_ignored_and_equal_window_does_not_renormalize() {
        let config = ContextBudgetFacts {
            context_window_tokens: 100,
            reserved_output_tokens: 0,
            prompt_token_limit: Some(0),
            auto_compact_tokens: None,
            compaction_target_tokens: None,
            auto_compact_percent: 1,
            compaction_target_percent: 1,
        };
        assert_eq!(record_provider_context_budget(config, 500, 0), None);
        let same = record_provider_context_budget(config, 500, 100).unwrap();
        assert_eq!(same.config, config);
        assert_eq!(same.used_tokens, 100);
        let changed = record_provider_context_budget(config, usize::MAX, usize::MAX).unwrap();
        assert_eq!(changed.config.context_window_tokens, 4_000_000);
        assert_eq!(changed.used_tokens, 4_000_000);
        assert_eq!(changed.config.prompt_token_limit, Some(1));
    }
}
