// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Diagnoses prompt-cache identity stability and provider cache evidence.
// - Interfaces: Pure Rust API plus a versioned JSON C ABI for platform Hosts.
// - Security: Accepts digests and bounded metadata only; prompt content is never recorded.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const PROMPT_CACHE_SCHEMA_VERSION: u32 = 1;
pub const PROMPT_CACHE_ABI_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PromptCacheControlIdentity {
    pub strategy: String,
    pub ttl_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PromptCacheIdentity {
    pub provider: String,
    pub model: String,
    pub stable_prefix_sha256: String,
    pub tool_manifest_sha256: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stable_fragment_sha256: BTreeMap<String, String>,
    pub cache_control: Option<PromptCacheControlIdentity>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PromptCacheProviderUsage {
    pub input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_write_input_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCacheDiagnosticInput {
    pub schema_version: u32,
    /// Whether explicit request cache controls are enabled. Providers may still
    /// cache automatically; this is not evidence about server-side availability.
    pub enabled: bool,
    pub current: PromptCacheIdentity,
    pub previous: Option<PromptCacheIdentity>,
    #[serde(default)]
    pub usage: PromptCacheProviderUsage,
}

pub use nola_domain::{PromptCacheDiagnostic, PromptCacheDiagnosticReason, PromptCacheDiagnosticStatus,
    PromptCacheFragmentChange, PromptCacheFragmentChangeKind};

pub fn diagnose_prompt_cache(input: &PromptCacheDiagnosticInput) -> PromptCacheDiagnostic {
    let mut reasons = validate_input(input);
    let valid = reasons.is_empty();
    let identity_sha256 = valid.then(|| digest_identity(&input.current));

    let identity_stable = valid && input
        .previous
        .as_ref()
        .is_some_and(|previous| previous == &input.current);
    let fragment_changes = input
        .previous
        .as_ref()
        .map(|previous| fragment_changes(previous, &input.current))
        .unwrap_or_default();

    if valid {
        if let Some(previous) = input.previous.as_ref() {
            append_identity_changes(previous, &input.current, &mut reasons);
            if identity_stable {
                reasons.push(PromptCacheDiagnosticReason::IdentityStable);
            }
        } else {
            reasons.push(PromptCacheDiagnosticReason::NoPreviousIdentity);
        }
    }

    // Observed provider accounting outranks request configuration and identity
    // heuristics. Invalid identity still carries validation reasons below.
    let status = if input.usage.cache_read_input_tokens.unwrap_or(0) > 0 {
        reasons.push(PromptCacheDiagnosticReason::ProviderReportedCacheRead);
        PromptCacheDiagnosticStatus::Hit
    } else if input.usage.cache_write_input_tokens.unwrap_or(0) > 0 {
        reasons.push(PromptCacheDiagnosticReason::ProviderReportedCacheWrite);
        PromptCacheDiagnosticStatus::Write
    } else if matches!(input.usage.cache_read_input_tokens, Some(0)) {
        // Read reuse is independently reported. OpenAI-compatible providers
        // need not expose cache-write accounting to confirm zero reused tokens.
        reasons.push(PromptCacheDiagnosticReason::ProviderReportedCacheMiss);
        PromptCacheDiagnosticStatus::Miss
    } else if input.usage.cache_read_input_tokens.is_some()
        || input.usage.cache_write_input_tokens.is_some()
    {
        reasons.push(PromptCacheDiagnosticReason::ProviderUsageIncomplete);
        PromptCacheDiagnosticStatus::Unknown
    } else if !valid {
        PromptCacheDiagnosticStatus::InvalidIdentity
    } else if !input.enabled {
        reasons.push(PromptCacheDiagnosticReason::ProviderUsageIncomplete);
        PromptCacheDiagnosticStatus::Unknown
    } else if input.previous.is_none() {
        PromptCacheDiagnosticStatus::FirstRequest
    } else if identity_stable {
        PromptCacheDiagnosticStatus::StableCandidate
    } else {
        PromptCacheDiagnosticStatus::Unknown
    };

    PromptCacheDiagnostic {
        cache_usage: None,
        schema_version: PROMPT_CACHE_SCHEMA_VERSION,
        status,
        identity_stable,
        identity_sha256,
        reasons,
        fragment_changes,
        cache_read_input_tokens: input.usage.cache_read_input_tokens,
        cache_write_input_tokens: input.usage.cache_write_input_tokens,
    }
}

fn validate_input(input: &PromptCacheDiagnosticInput) -> Vec<PromptCacheDiagnosticReason> {
    let mut reasons = Vec::new();
    if input.schema_version != PROMPT_CACHE_SCHEMA_VERSION {
        reasons.push(PromptCacheDiagnosticReason::InvalidSchemaVersion);
    }
    if !is_bounded_identifier(&input.current.provider) {
        reasons.push(PromptCacheDiagnosticReason::InvalidProvider);
    }
    if !is_bounded_identifier(&input.current.model) {
        reasons.push(PromptCacheDiagnosticReason::InvalidModel);
    }
    if !is_sha256(&input.current.stable_prefix_sha256) {
        reasons.push(PromptCacheDiagnosticReason::InvalidStablePrefixDigest);
    }
    if !is_sha256(&input.current.tool_manifest_sha256) {
        reasons.push(PromptCacheDiagnosticReason::InvalidToolManifestDigest);
    }
    if input
        .current
        .stable_fragment_sha256
        .iter()
        .any(|(fragment_id, digest)| !is_bounded_identifier(fragment_id) || !is_sha256(digest))
    {
        reasons.push(PromptCacheDiagnosticReason::InvalidStableFragmentIdentity);
    }
    if input.current.cache_control.as_ref().is_some_and(|control| {
        !is_bounded_identifier(&control.strategy)
            || control
                .ttl_seconds
                .is_some_and(|ttl| ttl == 0 || ttl > 86_400)
    }) {
        reasons.push(PromptCacheDiagnosticReason::InvalidCacheControl);
    }
    reasons
}

fn fragment_changes(
    previous: &PromptCacheIdentity,
    current: &PromptCacheIdentity,
) -> Vec<PromptCacheFragmentChange> {
    previous
        .stable_fragment_sha256
        .keys()
        .chain(current.stable_fragment_sha256.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|fragment_id| {
            let kind = match (
                previous.stable_fragment_sha256.get(&fragment_id),
                current.stable_fragment_sha256.get(&fragment_id),
            ) {
                (None, Some(_)) => PromptCacheFragmentChangeKind::Added,
                (Some(_), None) => PromptCacheFragmentChangeKind::Removed,
                (Some(previous), Some(current)) if previous != current => {
                    PromptCacheFragmentChangeKind::Changed
                }
                _ => return None,
            };
            Some(PromptCacheFragmentChange { fragment_id, kind })
        })
        .collect()
}

fn append_identity_changes(
    previous: &PromptCacheIdentity,
    current: &PromptCacheIdentity,
    reasons: &mut Vec<PromptCacheDiagnosticReason>,
) {
    if previous.provider != current.provider {
        reasons.push(PromptCacheDiagnosticReason::ProviderChanged);
    }
    if previous.model != current.model {
        reasons.push(PromptCacheDiagnosticReason::ModelChanged);
    }
    if previous.stable_prefix_sha256 != current.stable_prefix_sha256 {
        reasons.push(PromptCacheDiagnosticReason::StablePrefixChanged);
    }
    if previous.tool_manifest_sha256 != current.tool_manifest_sha256 {
        reasons.push(PromptCacheDiagnosticReason::ToolManifestChanged);
    }
    if previous.cache_control != current.cache_control {
        reasons.push(PromptCacheDiagnosticReason::CacheControlChanged);
    }
}

fn digest_identity(identity: &PromptCacheIdentity) -> String {
    let encoded = serde_json::to_vec(identity).unwrap_or_default();
    hex::encode(Sha256::digest(encoded))
}

fn is_bounded_identifier(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value.len() <= 160 && !value.chars().any(char::is_control)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}





#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> PromptCacheIdentity {
        PromptCacheIdentity {
            provider: "openai-compatible".into(),
            model: "example-model".into(),
            stable_prefix_sha256: "a".repeat(64),
            tool_manifest_sha256: "b".repeat(64),
            stable_fragment_sha256: BTreeMap::from([
                ("product-policy".into(), "c".repeat(64)),
                ("host-policy".into(), "d".repeat(64)),
            ]),
            cache_control: Some(PromptCacheControlIdentity {
                strategy: "ephemeral".into(),
                ttl_seconds: Some(3_600),
            }),
        }
    }

    fn input() -> PromptCacheDiagnosticInput {
        PromptCacheDiagnosticInput {
            schema_version: PROMPT_CACHE_SCHEMA_VERSION,
            enabled: true,
            current: identity(),
            previous: None,
            usage: PromptCacheProviderUsage::default(),
        }
    }

    #[test]
    fn first_request_does_not_claim_a_miss() {
        let result = diagnose_prompt_cache(&input());
        assert_eq!(result.status, PromptCacheDiagnosticStatus::FirstRequest);
        assert!(result
            .reasons
            .contains(&PromptCacheDiagnosticReason::NoPreviousIdentity));
    }

    #[test]
    fn stable_identity_without_provider_evidence_is_only_a_candidate() {
        let mut input = input();
        input.previous = Some(input.current.clone());
        let result = diagnose_prompt_cache(&input);
        assert_eq!(result.status, PromptCacheDiagnosticStatus::StableCandidate);
        assert!(result.identity_stable);
    }

    #[test]
    fn provider_cache_read_is_a_confirmed_hit() {
        let mut input = input();
        input.usage.cache_read_input_tokens = Some(1_024);
        let result = diagnose_prompt_cache(&input);
        assert_eq!(result.status, PromptCacheDiagnosticStatus::Hit);
    }

    #[test]
    fn explicit_zero_read_and_write_are_a_confirmed_miss() {
        let mut input = input();
        input.usage.cache_read_input_tokens = Some(0);
        input.usage.cache_write_input_tokens = Some(0);
        let result = diagnose_prompt_cache(&input);
        assert_eq!(result.status, PromptCacheDiagnosticStatus::Miss);
    }

    #[test]
    fn automatic_provider_usage_overrides_disabled_explicit_controls() {
        for (read, write, expected) in [
            (Some(15_450), None, PromptCacheDiagnosticStatus::Hit),
            (None, Some(1_024), PromptCacheDiagnosticStatus::Write),
            (Some(0), Some(0), PromptCacheDiagnosticStatus::Miss),
            (Some(0), None, PromptCacheDiagnosticStatus::Miss),
            (None, None, PromptCacheDiagnosticStatus::Unknown),
        ] {
            let mut input = input();
            input.enabled = false;
            input.usage.input_tokens = Some(15_485);
            input.usage.cache_read_input_tokens = read;
            input.usage.cache_write_input_tokens = write;
            let result = diagnose_prompt_cache(&input);
            assert_eq!(result.status, expected);
            assert_eq!(result.cache_read_input_tokens, read);
            assert_eq!(result.cache_write_input_tokens, write);
        }
    }

    #[test]
    fn read_miss_does_not_require_write_reporting_or_invent_a_write_count() {
        for previous in [None, Some(identity())] {
            let mut input = input();
            input.previous = previous;
            input.usage.input_tokens = Some(70_014);
            input.usage.cache_read_input_tokens = Some(0);
            let result = diagnose_prompt_cache(&input);
            assert_eq!(result.status, PromptCacheDiagnosticStatus::Miss);
            assert_eq!(result.cache_write_input_tokens, None);
            assert!(result.reasons.contains(&PromptCacheDiagnosticReason::ProviderReportedCacheMiss));
            assert!(!result.reasons.contains(&PromptCacheDiagnosticReason::ProviderUsageIncomplete));
        }
        let mut input = input();
        input.usage.cache_write_input_tokens = Some(0);
        assert_eq!(diagnose_prompt_cache(&input).status, PromptCacheDiagnosticStatus::Unknown,
            "A reported zero write count says nothing about cache reads");
        input.usage.cache_write_input_tokens = Some(100);
        assert_eq!(diagnose_prompt_cache(&input).status, PromptCacheDiagnosticStatus::Write);
        assert_eq!(diagnose_prompt_cache(&input).cache_read_input_tokens, None);
    }

    #[test]
    fn provider_usage_is_not_erased_by_invalid_local_identity() {
        let mut input = input();
        input.current.stable_prefix_sha256 = "invalid".into();
        input.usage.cache_read_input_tokens = Some(1_024);
        let result = diagnose_prompt_cache(&input);
        assert_eq!(result.status, PromptCacheDiagnosticStatus::Hit);
        assert!(result.identity_sha256.is_none());
        assert!(result
            .reasons
            .contains(&PromptCacheDiagnosticReason::InvalidStablePrefixDigest));
        assert!(result
            .reasons
            .contains(&PromptCacheDiagnosticReason::ProviderReportedCacheRead));
        input.previous = Some(input.current.clone());
        assert!(!diagnose_prompt_cache(&input).identity_stable,
            "two equally invalid identities do not prove cache prefix stability");
    }

    #[test]
    fn identity_changes_are_explained_without_prompt_content() {
        let mut input = input();
        let mut previous = input.current.clone();
        previous.model = "previous-model".into();
        previous.tool_manifest_sha256 = "c".repeat(64);
        previous
            .stable_fragment_sha256
            .insert("host-policy".into(), "e".repeat(64));
        input.previous = Some(previous);
        let result = diagnose_prompt_cache(&input);
        assert_eq!(result.status, PromptCacheDiagnosticStatus::Unknown);
        assert!(result
            .reasons
            .contains(&PromptCacheDiagnosticReason::ModelChanged));
        assert!(result
            .reasons
            .contains(&PromptCacheDiagnosticReason::ToolManifestChanged));
        assert_eq!(result.fragment_changes.len(), 1);
        assert_eq!(result.fragment_changes[0].fragment_id, "host-policy");
        assert_eq!(
            result.fragment_changes[0].kind,
            PromptCacheFragmentChangeKind::Changed
        );
    }

    #[test]
    fn malformed_digest_is_rejected() {
        let mut input = input();
        input.current.stable_prefix_sha256 = "prompt text".into();
        let result = diagnose_prompt_cache(&input);
        assert_eq!(result.status, PromptCacheDiagnosticStatus::InvalidIdentity);
        assert!(result.identity_sha256.is_none());
    }
}
