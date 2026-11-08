// SPDX-License-Identifier: Apache-2.0
use crate::assistant_journal::{AssistantDigest, AssistantJournalIdentity};
use crate::assistant_journal_operations::JournalNumber;
use crate::run_message_admission::RunMessageAdmissionCursor;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedContextUsage {
    pub measurement: Option<crate::ContextUsageMeasurement>,
    pub prompt_cache_measurement: Option<crate::ContextUsageMeasurement>,
    pub prompt_cache_diagnostic: Option<crate::PromptCacheDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_usage: Option<crate::PromptCacheUsage>,
    pub used_tokens: JournalNumber,
    pub context_window_tokens: JournalNumber,
    pub prompt_token_limit: JournalNumber,
    pub estimated: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedRunContextUsage {
    pub protocol_version: u16,
    pub identity: AssistantJournalIdentity,
    pub admission: RunMessageAdmissionCursor,
    pub event_sequence: JournalNumber,
    /// Latest committed model-attempt event, independent of context updates.
    pub attempt_sequence: JournalNumber,
    pub run_cache_usage: Option<crate::ConversationCacheUsage>,
    pub usage: CommittedContextUsage,
    pub record_digest: AssistantDigest,
}
impl CommittedRunContextUsage {
    pub fn computed_digest(&self) -> Result<AssistantDigest, String> {
        let bytes = serde_json::to_vec(&(
            self.protocol_version,
            &self.identity,
            &self.admission,
            self.event_sequence,
            self.attempt_sequence,
            &self.run_cache_usage,
            &self.usage,
        ))
        .map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        hash.update(b"NOLA-RUN-CONTEXT-USAGE-V1");
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
        AssistantDigest::parse_hex(&format!("{:x}", hash.finalize())).map_err(|e| e.to_string())
    }
    pub fn validate(&self) -> Result<(), String> {
        for identity in [&self.usage.measurement, &self.usage.prompt_cache_measurement].into_iter().flatten() {
            identity.validate()?;
        }
        if self.usage.prompt_cache_diagnostic.is_some() && self.usage.prompt_cache_measurement.is_none() {
            return Err("cache diagnostic has no source request".into());
        }
        if let Some(diagnostic) = &self.usage.prompt_cache_diagnostic { diagnostic.validate()?; }
        if let Some(run) = self.run_cache_usage {
            let cache = run.totals;
            if self.attempt_sequence.0 == 0
                || run.reported_requests.checked_add(run.unreported_requests).is_none_or(|n| n == 0)
                || cache.is_compaction
                || cache != crate::PromptCacheUsage::measured(cache.input_tokens, cache.read_tokens, cache.write_tokens, false)
                || cache.input_tokens.zip(cache.read_tokens).is_some_and(|(input, read)| read > input)
            {
                return Err("invalid cumulative Run cache usage".into());
            }
        }
        if let Some(cache) = self.usage.cache_usage {
            if cache != crate::PromptCacheUsage::measured(cache.input_tokens, cache.read_tokens, cache.write_tokens, cache.is_compaction) {
                return Err("invalid cache hit rate".into());
            }
        }
        self.identity.validate().map_err(|e| e.to_string())?;
        if self.protocol_version != 1
            || self.admission.identity != self.identity
            || self.admission.sequence.0 == 0
            || self.event_sequence.0 == 0
            || self.usage.prompt_token_limit.0 > self.usage.context_window_tokens.0
            || self.computed_digest()? != self.record_digest
        {
            return Err("invalid committed Run context usage".into());
        }
        Ok(())
    }
}
