// SPDX-License-Identifier: Apache-2.0
//! Memory records, policies and owned projection; no selection I/O or ABI.
use crate::{MemoryNamespace, MemoryProvenanceKind, MemoryScope, MemorySourceRole};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap,BTreeSet};
pub const MINIMUM_AUTOMATIC_MEMORY_CONFIDENCE:f32=0.65;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Preference,
    Identity,
    Decision,
    Project,
    Feedback,
    Reference,
    #[default]
    Other,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryTemporalMetadata {
    /// When the source statement was made. This differs from when the memory
    /// row was written or last used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentioned_at: Option<String>,
    /// When the described event occurred, if it resolves to one instant/date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_time: Option<String>,
    /// Inclusive start of a described interval, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_start: Option<String>,
    /// Inclusive end of a described interval, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_end: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MemoryRetention {
    #[default]
    Permanent,
    Session,
    UntilSuperseded,
    ExpiresAt {
        expires_at: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryPolicy {
    /// Makes cross-conversation memory tools model-visible for this session.
    #[serde(default = "default_true")]
    pub long_term_enabled: bool,
    /// Allows a Host to run extraction at a safe turn boundary. It never
    /// authorizes writes by itself; extraction remains an internal Core flow.
    #[serde(default)]
    pub automatic_extraction_enabled: bool,
    #[serde(default = "default_search_limit")]
    pub search_limit: usize,
    #[serde(default = "default_minimum_confidence")]
    pub minimum_confidence: f32,
    #[serde(default)]
    pub maximum_age_days: Option<u32>,
    /// Exact Host-authorized sensitivity labels eligible for this projection.
    /// Empty denies every labeled record; unlabeled records remain eligible.
    /// Labels are typed record metadata and are never inferred from content.
    #[serde(default)]
    pub allowed_sensitivities: Vec<String>,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        Self {
            long_term_enabled: true,
            automatic_extraction_enabled: false,
            search_limit: default_search_limit(),
            minimum_confidence: default_minimum_confidence(),
            maximum_age_days: None,
            allowed_sensitivities: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryProvenance {
    #[serde(default)]
    pub kind: MemoryProvenanceKind,
    /// Host-defined stable source identifier, URI, or import name.
    pub source: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub item_ids: Vec<String>,
    #[serde(default)]
    pub observed_at: Option<String>,
    /// Primary role responsible for the remembered statement when it can be
    /// represented without losing mixed-source evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_role: Option<MemorySourceRole>,
    /// Trusted roles of cited transcript items, keyed by item id. This keeps
    /// assistant-origin episodic facts distinct from user claims.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub source_roles: BTreeMap<String, MemorySourceRole>,
    #[serde(default, skip_serializing_if = "MemoryTemporalMetadata::is_empty")]
    pub temporal: MemoryTemporalMetadata,
}

impl MemoryTemporalMetadata {
    fn is_empty(&self) -> bool {
        self.mentioned_at.is_none()
            && self.event_time.is_none()
            && self.range_start.is_none()
            && self.range_end.is_none()
    }

    fn validate(&self) -> Result<(), String> {
        for (label, value) in [
            ("mentioned_at", self.mentioned_at.as_deref()),
            ("event_time", self.event_time.as_deref()),
            ("range_start", self.range_start.as_deref()),
            ("range_end", self.range_end.as_deref()),
        ] {
            if let Some(value) = value {
                let valid = chrono::DateTime::parse_from_rfc3339(value).is_ok()
                    || chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok();
                if !valid {
                    return Err(format!(
                        "Memory temporal {label} must be an RFC3339 timestamp or ISO date"
                    ));
                }
            }
        }
        if self.range_start.is_some() != self.range_end.is_some() {
            return Err("Memory temporal range requires both start and end".to_string());
        }
        let timestamp = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|date| date.timestamp_millis())
                .or_else(|_| {
                    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map(|date| {
                        date.and_hms_opt(0, 0, 0)
                            .expect("midnight is a valid time")
                            .and_utc()
                            .timestamp_millis()
                    })
                })
                .ok()
        };
        if let (Some(start), Some(end)) = (self.range_start.as_deref(), self.range_end.as_deref()) {
            if timestamp(start) > timestamp(end) {
                return Err("Memory temporal range start must not follow its end".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecord {
    pub id: String,
    pub owner: crate::memory_owner::MemoryOwner,
    pub namespace: MemoryNamespace,
    pub scope: MemoryScope,
    pub kind: MemoryKind,
    pub content: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_minimum_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub importance: f32,
    #[serde(default)]
    pub sensitivity: Option<String>,
    #[serde(default)]
    pub retention: MemoryRetention,
    #[serde(default)]
    pub supersedes: Vec<String>,
    pub provenance: MemoryProvenance,
}

/// Model/Host assessment attached to an automatically extracted or
/// consolidated memory. The assessment is not sufficient on its own: Core
/// also validates the record's scope, retention, provenance, and citations.
/// Keeping the semantic assessment typed makes weak memory writes observable
/// without teaching Hosts language-specific extraction heuristics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryQualityAssessment {
    pub applicable: bool,
    pub durable: bool,
    pub legible: bool,
    pub rationale: String,
}

impl MemoryQualityAssessment {
    pub fn validate(&self) -> Result<(), String> {
        if !self.applicable {
            return Err("Automatic memory candidate is not applicable to future work".to_string());
        }
        if !self.durable {
            return Err("Automatic memory candidate is not durable across sessions".to_string());
        }
        if !self.legible {
            return Err(
                "Automatic memory candidate is not legible without its source session".to_string(),
            );
        }
        if self.rationale.trim().is_empty() {
            return Err("Automatic memory quality rationale must not be empty".to_string());
        }
        Ok(())
    }
}

impl MemoryRecord {
    pub fn validate(&self) -> Result<(), String> {
        self.owner.validate_scope(&self.namespace, self.scope)?;
        if self.id.trim().is_empty() {
            return Err("Memory id must not be empty".to_string());
        }
        if self.content.trim().is_empty() {
            return Err("Memory content must not be empty".to_string());
        }
        if self.provenance.source.trim().is_empty() {
            return Err("Memory provenance source must not be empty".to_string());
        }
        self.provenance.temporal.validate()?;
        if self.provenance.source_roles.iter().any(|(item_id, _)| {
            item_id.trim().is_empty() || !self.provenance.item_ids.contains(item_id)
        }) {
            return Err(
                "Memory provenance source roles must identify cited provenance items".to_string(),
            );
        }
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err("Memory confidence must be between 0 and 1".to_string());
        }
        if !self.importance.is_finite() || !(0.0..=1.0).contains(&self.importance) {
            return Err("Memory importance must be between 0 and 1".to_string());
        }
        if matches!(&self.namespace, MemoryNamespace::Custom(name) if name.trim().is_empty()) {
            return Err("Custom memory namespace must not be empty".to_string());
        }
        if self
            .sensitivity
            .as_deref()
            .is_some_and(|sensitivity| sensitivity.trim().is_empty())
        {
            return Err("Memory sensitivity must not be empty when present".to_string());
        }
        if let MemoryRetention::ExpiresAt { expires_at } = &self.retention {
            chrono::DateTime::parse_from_rfc3339(expires_at).map_err(|_| {
                "Expiring memory retention requires an RFC3339 expires_at".to_string()
            })?;
        }
        if self
            .supersedes
            .iter()
            .any(|id| id.trim().is_empty() || id == &self.id)
        {
            return Err("Memory supersedes must contain non-empty, different ids".to_string());
        }
        let unique = self.supersedes.iter().collect::<BTreeSet<_>>();
        if unique.len() != self.supersedes.len() {
            return Err("Memory supersedes must not contain duplicates".to_string());
        }
        Ok(())
    }

    /// Applies structural gates for records produced by automatic extraction
    /// or consolidation. Explicit user-authored session notes may use narrower
    /// scopes, but automatic long-term memory must actually survive a session
    /// boundary and must not be owned by one conversation.
    pub fn validate_automatic_long_term_candidate(
        &self,
        quality: &MemoryQualityAssessment,
    ) -> Result<(), String> {
        self.validate()?;
        quality.validate()?;
        if self.scope != MemoryScope::Global {
            return Err("Automatic long-term memory must use global scope".to_string());
        }
        if self.namespace == MemoryNamespace::Conversation {
            return Err(
                "Automatic long-term memory cannot use the conversation namespace".to_string(),
            );
        }
        if self.retention == MemoryRetention::Session {
            return Err("Automatic long-term memory cannot use session retention".to_string());
        }
        if self.provenance.kind != MemoryProvenanceKind::AgentInference {
            return Err(
                "Automatic long-term memory must preserve agent-inference provenance".to_string(),
            );
        }
        if self.confidence < MINIMUM_AUTOMATIC_MEMORY_CONFIDENCE {
            return Err(format!(
                "Automatic long-term memory confidence must be at least {MINIMUM_AUTOMATIC_MEMORY_CONFIDENCE}"
            ));
        }
        let source_items = self
            .provenance
            .item_ids
            .iter()
            .map(|id| id.trim())
            .collect::<BTreeSet<_>>();
        if source_items.is_empty()
            || source_items.contains("")
            || source_items.len() != self.provenance.item_ids.len()
        {
            return Err(
                "Automatic long-term memory requires non-empty unique source item ids".to_string(),
            );
        }
        if self.supersedes.is_empty() {
            let session_id = self
                .provenance
                .session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    "Automatically extracted memory requires a source session id".to_string()
                })?;
            if self.provenance.source != format!("nola://session/{session_id}") {
                return Err(
                    "Automatically extracted memory source must match its source session"
                        .to_string(),
                );
            }
        } else {
            let superseded = self
                .supersedes
                .iter()
                .map(|id| id.as_str())
                .collect::<BTreeSet<_>>();
            if !self
                .provenance
                .source
                .starts_with("nola://memory/consolidation/")
                || superseded != source_items
            {
                return Err(
                    "Consolidated memory must cite exactly the records it supersedes".to_string(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemoryUpdate {
    Created { record: MemoryRecord },
    Updated { record: MemoryRecord },
    Deleted { memory_id: String },
}

impl MemoryUpdate {
    pub fn memory_id(&self) -> &str {
        match self {
            Self::Created { record } | Self::Updated { record } => &record.id,
            Self::Deleted { memory_id } => memory_id,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryProjection {
    pub revision: u64,
    pub order: Vec<String>,
    pub records: BTreeMap<String, MemoryRecord>,
}


impl MemoryProjection {
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn get(&self, memory_id: &str) -> Option<&MemoryRecord> {
        self.records.get(memory_id)
    }

    pub fn records_in_namespace(&self, namespace: &MemoryNamespace) -> Vec<&MemoryRecord> {
        self.order
            .iter()
            .filter_map(|id| self.records.get(id))
            .filter(|record| &record.namespace == namespace)
            .collect()
    }

    pub fn apply(&mut self, update: MemoryUpdate) -> Result<MemoryUpdate, String> {
        match &update {
            MemoryUpdate::Created { record } => {
                record.validate()?;
                if self.records.contains_key(&record.id) {
                    return Err("Memory id is already registered".to_string());
                }
                self.order.push(record.id.clone());
                self.records.insert(record.id.clone(), record.clone());
            }
            MemoryUpdate::Updated { record } => {
                record.validate()?;
                let previous = self.records.get(&record.id)
                    .ok_or("Memory update references an unknown record")?;
                if previous.owner != record.owner {
                    return Err("Memory owner is immutable for an existing identity".into());
                }
                self.records.insert(record.id.clone(), record.clone());
            }
            MemoryUpdate::Deleted { memory_id } => {
                if memory_id.trim().is_empty() {
                    return Err("Memory id must not be empty".to_string());
                }
                if self.records.remove(memory_id).is_none() {
                    return Err("Memory deletion references an unknown record".to_string());
                }
                self.order.retain(|id| id != memory_id);
            }
        }
        self.revision = self.revision.saturating_add(1);
        Ok(update)
    }
}

fn default_true() -> bool {
    true
}

fn default_search_limit() -> usize {
    8
}

fn default_minimum_confidence() -> f32 {
    0.65
}
