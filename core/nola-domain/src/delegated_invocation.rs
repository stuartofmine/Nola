// SPDX-License-Identifier: Apache-2.0
//! Durable identity for an external agent invocation, independent of its opaque model sampling.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedInvocationState {
    Prepared,
    Running,
    WaitingPermission,
    Interrupted,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

impl DelegatedInvocationState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedInvocationRecord {
    pub id: String,
    pub run_id: String,
    pub effect_id: String,
    pub backend_id: String,
    pub backend_session_id: Option<String>,
    pub text_part_id: String,
    pub reasoning_part_id: String,
    pub state: DelegatedInvocationState,
    pub failure: Option<String>,
    pub usage: serde_json::Value,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedInvocationMutation {
    /// None inserts a new identity; Some performs a revision compare-and-swap.
    pub expected_revision: Option<i64>,
    pub record: DelegatedInvocationRecord,
}

/// A provider-reported cumulative snapshot, never a delta to add on replay.
/// Session totals may include earlier invocations; absent counters are unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalUsageSnapshot {
    pub backend_id: String,
    pub scope_id: String,
    pub basis: ExternalUsageBasis,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalUsageBasis { SessionCumulative, InvocationCumulative, LastModelResponse }
impl ExternalUsageSnapshot {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.backend_id.is_empty() || self.scope_id.is_empty()
            || self.backend_id.len()>128 || self.scope_id.len()>512
            || self.backend_id.chars().any(char::is_control) || self.scope_id.chars().any(char::is_control)
            || [&self.provider_id,&self.model_id].iter().filter_map(|value|value.as_deref()).any(|value|value.is_empty() || value.len()>512 || value.chars().any(char::is_control))
            || self.cache_read_tokens.zip(self.input_tokens).is_some_and(|(cache,input)|cache>input)
            || self.cache_write_tokens.zip(self.input_tokens).is_some_and(|(cache,input)|cache>input) {
            return Err("invalid external usage identity or counters");
        }
        Ok(())
    }
}
