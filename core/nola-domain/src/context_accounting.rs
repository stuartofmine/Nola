// SPDX-License-Identifier: Apache-2.0
//! Numeric context accounting facts, separate from provider metadata.
use serde::{Deserialize, Serialize};

/// Latest physical request, not context occupancy or a mean of percentages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCacheUsage {
    pub input_tokens: Option<u64>,
    pub read_tokens: Option<u64>,
    pub write_tokens: Option<u64>,
    pub hit_rate_bps: Option<u16>,
    pub is_compaction: bool,
}

/// Durable conversation totals of reported physical requests. Coverage is
/// explicit: a failed/unreported request is neither a zero nor a cache miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationCacheUsage {
    #[serde(flatten)]
    pub totals: PromptCacheUsage,
    pub reported_requests: u64,
    pub unreported_requests: u64,
}

impl ConversationCacheUsage {
    /// One physical request. Incomplete counters are a coverage gap, not a
    /// cache miss, and never enter the weighted numerator or denominator.
    pub fn from_request(request: PromptCacheUsage) -> Self {
        let reported = request
            .input_tokens
            .zip(request.read_tokens)
            .is_some_and(|(input, read)| read <= input);
        Self {
            totals: if reported {
                PromptCacheUsage::measured(
                    request.input_tokens,
                    request.read_tokens,
                    request.write_tokens,
                    false,
                )
            } else {
                PromptCacheUsage::default()
            },
            reported_requests: u64::from(reported),
            unreported_requests: u64::from(!reported),
        }
    }

    /// Disjoint physical identities must be deduplicated by the repository
    /// before merging. Overflow cannot wrap into a plausible cache percentage.
    pub fn checked_merge(self, other: Self) -> Option<Self> {
        let add = |a: Option<u64>, b: Option<u64>| a.zip(b).and_then(|(a, b)| a.checked_add(b));
        let totals = if self.reported_requests == 0 {
            other.totals
        } else if other.reported_requests == 0 {
            self.totals
        } else {
            PromptCacheUsage::measured(
                add(self.totals.input_tokens, other.totals.input_tokens),
                add(self.totals.read_tokens, other.totals.read_tokens),
                add(self.totals.write_tokens, other.totals.write_tokens),
                false,
            )
        };
        Some(Self {
            totals,
            reported_requests: self
                .reported_requests
                .checked_add(other.reported_requests)?,
            unreported_requests: self
                .unreported_requests
                .checked_add(other.unreported_requests)?,
        })
    }
}

impl PromptCacheUsage {
    pub fn from_model_attempt_usage(value: &serde_json::Value) -> Self {
        if let Some(cache) = value.get("cache_usage").filter(|value| !value.is_null()) {
            let request: Self = serde_json::from_value(cache.clone()).unwrap_or_default();
            return Self::measured(
                request.input_tokens,
                request.read_tokens,
                request.write_tokens,
                false,
            );
        }
        Self::measured(
            value
                .get("input_tokens")
                .and_then(serde_json::Value::as_u64),
            (value
                .get("cached_input_tokens_reported")
                .and_then(serde_json::Value::as_bool)
                == Some(true))
            .then(|| {
                value
                    .get("cached_input_tokens")
                    .and_then(serde_json::Value::as_u64)
            })
            .flatten(),
            (value
                .get("cache_write_input_tokens_reported")
                .and_then(serde_json::Value::as_bool)
                == Some(true))
            .then(|| {
                value
                    .get("cache_write_input_tokens")
                    .and_then(serde_json::Value::as_u64)
            })
            .flatten(),
            false,
        )
    }

    pub fn from_auxiliary_attempt_usage(value: &serde_json::Value) -> Self {
        if value.get("coverage").and_then(serde_json::Value::as_str) != Some("known") {
            return Self::default();
        }
        let normalized = &value["normalized"];
        Self::measured(
            normalized["input_tokens"].as_u64(),
            normalized["cached_input_tokens"].as_u64(),
            normalized["cache_write_input_tokens"].as_u64(),
            false,
        )
    }

    pub fn measured(
        input: Option<u64>,
        read: Option<u64>,
        write: Option<u64>,
        is_compaction: bool,
    ) -> Self {
        let hit_rate_bps = input.zip(read).and_then(|(input, read)| {
            (input > 0 && read <= input)
                .then(|| ((u128::from(read) * 10_000) / u128::from(input)) as u16)
        });
        Self {
            input_tokens: input,
            read_tokens: read,
            write_tokens: write,
            hit_rate_bps,
            is_compaction,
        }
    }
}

#[cfg(test)]
mod cache_usage_tests {
    use super::{ConversationCacheUsage, PromptCacheUsage};
    #[test]
    fn cache_rate_distinguishes_unknown_zero_writes_and_invalid_usage() {
        let rate =
            |input, read, write| PromptCacheUsage::measured(input, read, write, false).hit_rate_bps;
        assert_eq!(rate(Some(100), Some(90), Some(10)), Some(9000));
        assert_eq!(rate(Some(100), Some(0), Some(100)), Some(0));
        assert_eq!(rate(Some(100), None, Some(100)), None);
        assert_eq!(rate(Some(0), Some(0), None), None);
        assert_eq!(rate(None, Some(90), None), None);
        assert_eq!(rate(Some(100), Some(101), None), None);
        assert_eq!(rate(Some(u64::MAX), Some(u64::MAX), None), Some(10000));
    }

    #[test]
    fn accumulated_rates_weight_tokens_and_preserve_unknown_coverage() {
        let one = |input, read| {
            ConversationCacheUsage::from_request(PromptCacheUsage::measured(
                Some(input),
                read,
                None,
                false,
            ))
        };
        let sum = one(100, Some(100))
            .checked_merge(one(900, Some(0)))
            .unwrap()
            .checked_merge(one(400, None))
            .unwrap();
        assert_eq!(sum.totals.hit_rate_bps, Some(1000));
        assert_eq!(sum.totals.input_tokens, Some(1000));
        assert_eq!((sum.reported_requests, sum.unreported_requests), (2, 1));
        assert_eq!(one(10, Some(11)).unreported_requests, 1);
        let overflow = one(u64::MAX, Some(u64::MAX))
            .checked_merge(one(1, Some(1)))
            .unwrap();
        assert_eq!(overflow.totals.hit_rate_bps, None);
        assert_eq!(overflow.totals.input_tokens, None);
    }

    #[test]
    fn auxiliary_provider_missing_cache_counters_are_not_zero() {
        let usage = serde_json::json!({"coverage":"known","normalized":{"input_tokens":500,"output_tokens":10,"cached_input_tokens":null}});
        assert_eq!(
            ConversationCacheUsage::from_request(PromptCacheUsage::from_auxiliary_attempt_usage(
                &usage
            ))
            .unreported_requests,
            1
        );
        let zero = serde_json::json!({"coverage":"known","normalized":{"input_tokens":500,"output_tokens":10,"cached_input_tokens":0}});
        assert_eq!(
            PromptCacheUsage::from_auxiliary_attempt_usage(&zero).hit_rate_bps,
            Some(0)
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageOrigin {
    Estimated,
    Provider,
}

/// Canonical, UI-safe view of one conversation's current context usage.
///
/// This is derived from `ContextWindowState`, persisted as part of the Core
/// checkpoint, and projected into traces for Hosts that restore from an event
/// stream. Hosts must not independently estimate or reinterpret these fields.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextUsageSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_usage: Option<PromptCacheUsage>,
    #[serde(default)]
    pub used_tokens: usize,
    #[serde(default)]
    pub context_window_tokens: usize,
    #[serde(default)]
    pub prompt_token_limit: usize,
    #[serde(default)]
    pub usage_percent: f64,
    #[serde(default)]
    pub usage_is_estimated: bool,
    #[serde(default)]
    pub cached_input_tokens: usize,
    #[serde(default)]
    pub cached_input_tokens_reported: bool,
    #[serde(default)]
    pub cache_write_input_tokens: usize,
    #[serde(default)]
    pub cache_write_input_tokens_reported: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenAccounting {
    pub input_tokens: Option<usize>,
    pub cached_input_tokens: Option<usize>,
    pub cache_write_input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub reasoning_tokens: Option<usize>,
    /// Only the provider's reported total; never a derived sum.
    pub total_tokens: Option<usize>,
    pub request_count: usize,
    /// Derived budget lower bound, not a provider accounting fact.
    pub known_token_lower_bound: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextAccounting {
    pub used_tokens: usize,
    pub origin: UsageOrigin,
    pub cumulative: TokenAccounting,
}
