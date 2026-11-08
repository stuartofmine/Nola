// SPDX-License-Identifier: Apache-2.0
//! Content-free, persisted provider-prefix diagnostics shared by Core and clients.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheDiagnosticStatus {
    Disabled,
    InvalidIdentity,
    FirstRequest,
    Hit,
    Write,
    Miss,
    StableCandidate,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheDiagnosticReason {
    InvalidSchemaVersion,
    InvalidProvider,
    InvalidModel,
    InvalidStablePrefixDigest,
    InvalidToolManifestDigest,
    InvalidStableFragmentIdentity,
    InvalidCacheControl,
    NoPreviousIdentity,
    ProviderChanged,
    ModelChanged,
    StablePrefixChanged,
    ToolManifestChanged,
    CacheControlChanged,
    ProviderReportedCacheRead,
    ProviderReportedCacheWrite,
    ProviderReportedCacheMiss,
    ProviderUsageIncomplete,
    IdentityStable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheFragmentChangeKind {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCacheFragmentChange {
    pub fragment_id: String,
    pub kind: PromptCacheFragmentChangeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCacheDiagnostic {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_usage: Option<crate::PromptCacheUsage>,
    pub schema_version: u32,
    pub status: PromptCacheDiagnosticStatus,
    pub identity_stable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_sha256: Option<String>,
    pub reasons: Vec<PromptCacheDiagnosticReason>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragment_changes: Vec<PromptCacheFragmentChange>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_write_input_tokens: Option<u64>,
}

impl PromptCacheDiagnostic {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.identity_sha256.as_ref().is_some_and(|id| id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()))
            || self.identity_stable && self.identity_sha256.is_none()
            || self.reasons.len() > 64 || self.fragment_changes.len() > 128
            || self.fragment_changes.iter().any(|change| change.fragment_id.is_empty() || change.fragment_id.len() > 256)
        {
            return Err("invalid persisted prompt cache diagnostic".into());
        }
        if let Some(cache) = self.cache_usage {
            if cache != crate::PromptCacheUsage::measured(cache.input_tokens, cache.read_tokens, cache.write_tokens, cache.is_compaction) {
                return Err("invalid diagnostic cache usage".into());
            }
        }
        Ok(())
    }
}

/// Identity of the context request, not a synthetic UI refresh identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextUsageMeasurement {
    pub request_id: String,
    pub attempt_id: Option<String>,
    pub context_generation: Option<u64>,
}

impl ContextUsageMeasurement {
    pub fn validate(&self) -> Result<(), String> {
        if self.request_id.trim().is_empty() || self.attempt_id.as_ref().is_some_and(|id| id.trim().is_empty()) {
            return Err("context measurement has an empty request or attempt identity".into());
        }
        Ok(())
    }
}

