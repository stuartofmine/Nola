// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines platform-neutral long-term memory policy and records.
// - Boundary: Core owns memory semantics; Hosts own storage and retrieval I/O.

pub use nola_domain::{
    MemoryNamespace, MemoryProvenanceKind, MemoryScope, MemorySourceRole, MemoryUsage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MEMORY_SELECTION_ABI_VERSION: u32 = 1;
/// A caller that omits an explicit model budget still receives a bounded
/// projection. Exact budgets and tokenizer counts can be supplied for the active model.
pub const DEFAULT_MEMORY_INPUT_TOKEN_BUDGET: u32 = 2_048;
pub use nola_domain::memory_projection::{MemoryKind, MemoryTemporalMetadata, MemoryRetention, MemoryPolicy, MemoryProvenance, MemoryRecord, MemoryQualityAssessment, MemoryUpdate, MemoryProjection, MINIMUM_AUTOMATIC_MEMORY_CONFIDENCE};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRecallMode {
    ResidentProfile,
    #[default]
    OnDemand,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTokenAccounting {
    #[default]
    HostExact,
    CoreEstimate,
}

/// Host-ranked memory candidates presented to Core for deterministic policy
/// gating. Semantic relevance remains a model/retrieval responsibility; Core
/// never infers it from language-specific keywords.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySelectionInput {
    pub policy: MemoryPolicy,
    pub records: Vec<MemoryRecord>,
    /// Resident profile is the small, safe projection included on every turn.
    /// On-demand candidates have already been ranked by a Host retrieval
    /// implementation and still pass through Core policy and budgeting.
    #[serde(default)]
    pub mode: MemoryRecallMode,
    /// Empty means every namespace is eligible. Otherwise this is the exact
    /// namespace allowlist for the Turn being prepared.
    #[serde(default)]
    pub allowed_namespaces: Vec<MemoryNamespace>,
    /// Session-retained records are excluded from cross-session recall unless
    /// the Host explicitly identifies the current session as eligible.
    #[serde(default)]
    pub include_session_retention: bool,
    /// RFC 3339 evaluation time supplied by the Host. Core has no wall clock.
    #[serde(default)]
    pub now: Option<String>,
    /// Exact active-model input budget for this projection. When present,
    /// every candidate must have a positive Host-computed token count.
    #[serde(default)]
    pub maximum_input_tokens: Option<u32>,
    /// Hosts with the active model tokenizer should submit exact counts.
    /// Other Hosts can opt into Core's deterministic conservative estimate.
    #[serde(default)]
    pub token_accounting: MemoryTokenAccounting,
    #[serde(default)]
    pub token_counts: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySelectionRejectReason {
    Disabled,
    InvalidRecord,
    NamespaceNotAllowed,
    SensitivityNotAllowed,
    BelowConfidence,
    SessionOnly,
    Expired,
    TooOld,
    InvalidTimestamp,
    Superseded,
    NotResidentProfile,
    MissingTokenCount,
    TokenBudgetExceeded,
    LimitExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySelectionReason {
    ResidentProfile,
    HostRankedOnDemand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectedMemory {
    pub record: MemoryRecord,
    /// Past snapshots that may describe mutable external state must be checked
    /// against a current source before being presented as current truth.
    pub requires_current_state_verification: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u32>,
    #[serde(default)]
    pub token_count_is_estimated: bool,
    pub selection_reason: MemorySelectionReason,
}

/// Projects only components that survived context budgeting. Hosts may attach a
/// typed `SelectedMemory`/`MemoryRecord`; untyped context components receive
/// source-aware usage evidence without parsing their content.
pub fn project_memory_usage(
    components: &[crate::context_engine::ContextComponent],
    metrics: &[crate::context_engine::ContextComponentMetrics],
) -> Vec<MemoryUsage> {
    use crate::context_engine::ContextComponentKind;

    let mut usage = Vec::new();
    for metric in metrics.iter().filter(|metric| {
        matches!(
            metric.kind,
            ContextComponentKind::Memory | ContextComponentKind::PersonalIntelligence
        )
    }) {
        for component_id in &metric.used_ids {
            let Some(component) = components
                .iter()
                .find(|component| component.kind == metric.kind && component.id == *component_id)
            else {
                continue;
            };
            let selected = serde_json::from_value::<SelectedMemory>(component.content.clone()).ok();
            let standalone_record = selected
                .is_none()
                .then(|| serde_json::from_value::<MemoryRecord>(component.content.clone()).ok())
                .flatten();
            let record = selected
                .as_ref()
                .map(|selected| &selected.record)
                .or(standalone_record.as_ref());

            let source = record
                .map(|record| record.provenance.source.trim())
                .filter(|source| !source.is_empty())
                .map(ToOwned::to_owned)
                .or_else(|| {
                    component
                        .source
                        .as_ref()
                        .and_then(|source| source.uri.as_deref())
                        .map(str::trim)
                        .filter(|source| !source.is_empty())
                        .map(ToOwned::to_owned)
                });
            usage.push(MemoryUsage {
                record_id: record
                    .map(|record| record.id.clone())
                    .unwrap_or_else(|| component_id.clone()),
                component_id: component_id.clone(),
                scope: record.map(|record| record.scope),
                namespace: record.map(|record| record.namespace.clone()),
                provenance_kind: record.map(|record| record.provenance.kind),
                source,
                source_role: record.and_then(|record| record.provenance.source_role),
                event_time: record.and_then(|record| record.provenance.temporal.event_time.clone()),
                range_start: record
                    .and_then(|record| record.provenance.temporal.range_start.clone()),
                range_end: record.and_then(|record| record.provenance.temporal.range_end.clone()),
                title: component
                    .source
                    .as_ref()
                    .and_then(|source| source.title.clone()),
                observed_at: record
                    .and_then(|record| record.provenance.observed_at.clone())
                    .or_else(|| {
                        component
                            .source
                            .as_ref()
                            .and_then(|source| source.observed_at.clone())
                    }),
                requires_current_state_verification: selected
                    .as_ref()
                    .is_some_and(|selected| selected.requires_current_state_verification),
                truncated: metric.truncated_ids.contains(component_id),
                supersedes: record
                    .map(|record| record.supersedes.clone())
                    .unwrap_or_default(),
                usage_count: 0,
                last_used_at_epoch_ms: None,
            });
        }
    }
    usage
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedMemory {
    pub memory_id: String,
    pub reason: MemorySelectionRejectReason,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemorySelectionPlan {
    pub mode: MemoryRecallMode,
    pub selected: Vec<SelectedMemory>,
    pub rejected: Vec<RejectedMemory>,
    pub used_input_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_input_tokens: Option<u32>,
    #[serde(default)]
    pub token_counts_are_estimated: bool,
    pub plan_digest: String,
}

/// Applies deterministic memory policy to already-ranked candidates.
///
/// Candidate order is preserved. Supersession is evaluated only from records
/// that pass the basic policy gates, so an expired or invalid replacement does
/// not hide an otherwise valid older memory.
pub fn select_memories(input: MemorySelectionInput) -> MemorySelectionPlan {
    let maximum_input_tokens = Some(
        input
            .maximum_input_tokens
            .unwrap_or(DEFAULT_MEMORY_INPUT_TOKEN_BUDGET),
    );
    let mode = input.mode;
    let token_accounting = if input.maximum_input_tokens.is_none() {
        MemoryTokenAccounting::CoreEstimate
    } else {
        input.token_accounting
    };
    let mut plan = MemorySelectionPlan {
        mode,
        remaining_input_tokens: maximum_input_tokens,
        token_counts_are_estimated: token_accounting == MemoryTokenAccounting::CoreEstimate,
        ..MemorySelectionPlan::default()
    };
    if !input.policy.long_term_enabled {
        plan.rejected = input
            .records
            .into_iter()
            .map(|record| RejectedMemory {
                memory_id: record.id,
                reason: MemorySelectionRejectReason::Disabled,
            })
            .collect();
        return finalize_memory_selection_plan(plan, maximum_input_tokens);
    }

    let now = match input.now.as_deref() {
        Some(value) => match chrono::DateTime::parse_from_rfc3339(value) {
            Ok(value) => Some(value),
            Err(_) => {
                plan.rejected = input
                    .records
                    .into_iter()
                    .map(|record| RejectedMemory {
                        memory_id: record.id,
                        reason: MemorySelectionRejectReason::InvalidTimestamp,
                    })
                    .collect();
                return finalize_memory_selection_plan(plan, maximum_input_tokens);
            }
        },
        None => None,
    };

    let mut eligible = Vec::new();
    for record in input.records {
        let rejection = if record.validate().is_err() {
            Some(MemorySelectionRejectReason::InvalidRecord)
        } else if mode == MemoryRecallMode::ResidentProfile && !is_resident_profile_record(&record)
        {
            Some(MemorySelectionRejectReason::NotResidentProfile)
        } else if !input.allowed_namespaces.is_empty()
            && !input.allowed_namespaces.contains(&record.namespace)
        {
            Some(MemorySelectionRejectReason::NamespaceNotAllowed)
        } else if record.sensitivity.as_deref().is_some_and(|sensitivity| {
            !input
                .policy
                .allowed_sensitivities
                .iter()
                .any(|allowed| allowed == sensitivity)
        }) {
            Some(MemorySelectionRejectReason::SensitivityNotAllowed)
        } else if record.confidence < input.policy.minimum_confidence {
            Some(MemorySelectionRejectReason::BelowConfidence)
        } else if matches!(record.retention, MemoryRetention::Session)
            && !input.include_session_retention
        {
            Some(MemorySelectionRejectReason::SessionOnly)
        } else {
            memory_time_rejection(&record, &input.policy, now.as_ref())
        };
        if let Some(reason) = rejection {
            plan.rejected.push(RejectedMemory {
                memory_id: record.id,
                reason,
            });
        } else {
            eligible.push(record);
        }
    }

    // Supersession is an ownership-scoped relation. A Host can prepare one
    // ContextPlan from several allowed namespaces, but a record from one
    // namespace/scope must never hide a fact owned by another merely by
    // naming its globally visible id.
    let superseded = eligible
        .iter()
        .flat_map(|record| {
            record
                .supersedes
                .iter()
                .cloned()
                .map(|id| (record.owner.clone(), record.namespace.clone(), record.scope, id))
        })
        .collect::<BTreeSet<_>>();
    let limit = input.policy.search_limit;
    for record in eligible {
        if superseded.contains(&(record.owner.clone(), record.namespace.clone(), record.scope, record.id.clone())) {
            plan.rejected.push(RejectedMemory {
                memory_id: record.id,
                reason: MemorySelectionRejectReason::Superseded,
            });
            continue;
        }
        if plan.selected.len() >= limit {
            plan.rejected.push(RejectedMemory {
                memory_id: record.id,
                reason: MemorySelectionRejectReason::LimitExceeded,
            });
            continue;
        }
        let (token_count, token_count_is_estimated) = match token_accounting {
            MemoryTokenAccounting::HostExact => {
                (input.token_counts.get(&record.id).copied(), false)
            }
            MemoryTokenAccounting::CoreEstimate => (Some(estimated_memory_tokens(&record)), true),
        };
        if maximum_input_tokens.is_some()
            && token_accounting == MemoryTokenAccounting::HostExact
            && token_count.unwrap_or(0) == 0
        {
            plan.rejected.push(RejectedMemory {
                memory_id: record.id,
                reason: MemorySelectionRejectReason::MissingTokenCount,
            });
            continue;
        }
        if let (Some(maximum), Some(count)) = (maximum_input_tokens, token_count) {
            if plan.used_input_tokens.saturating_add(count) > maximum {
                plan.rejected.push(RejectedMemory {
                    memory_id: record.id,
                    reason: MemorySelectionRejectReason::TokenBudgetExceeded,
                });
                continue;
            }
            plan.used_input_tokens = plan.used_input_tokens.saturating_add(count);
        }
        let requires_current_state_verification = matches!(
            record.provenance.kind,
            MemoryProvenanceKind::AgentInference
                | MemoryProvenanceKind::ToolResult
                | MemoryProvenanceKind::Imported
                | MemoryProvenanceKind::Synced
                | MemoryProvenanceKind::Other
        ) || matches!(
            record.kind,
            MemoryKind::Decision | MemoryKind::Project | MemoryKind::Reference
        );
        plan.selected.push(SelectedMemory {
            record,
            requires_current_state_verification,
            token_count,
            token_count_is_estimated,
            selection_reason: match mode {
                MemoryRecallMode::ResidentProfile => MemorySelectionReason::ResidentProfile,
                MemoryRecallMode::OnDemand => MemorySelectionReason::HostRankedOnDemand,
            },
        });
    }
    finalize_memory_selection_plan(plan, maximum_input_tokens)
}

fn is_resident_profile_record(record: &MemoryRecord) -> bool {
    record.owner == nola_domain::memory_owner::MemoryOwner::local_profile()
        && record.scope == MemoryScope::Global
        && record.namespace == MemoryNamespace::Personal
        && matches!(
            record.kind,
            MemoryKind::Identity
                | MemoryKind::Preference
                | MemoryKind::Feedback
                | MemoryKind::Reference
        )
        && matches!(
            record.provenance.kind,
            MemoryProvenanceKind::ExplicitUser | MemoryProvenanceKind::AgentInference
        )
}

fn estimated_memory_tokens(record: &MemoryRecord) -> u32 {
    let value = serde_json::to_value(record).unwrap_or_default();
    let estimated = crate::context_engine::estimate_json_value_tokens(&value);
    let conservative = estimated.saturating_mul(6).saturating_add(4) / 5;
    conservative.max(1).min(u32::MAX as usize) as u32
}

fn finalize_memory_selection_plan(
    mut plan: MemorySelectionPlan,
    maximum_input_tokens: Option<u32>,
) -> MemorySelectionPlan {
    plan.remaining_input_tokens =
        maximum_input_tokens.map(|maximum| maximum.saturating_sub(plan.used_input_tokens));
    let digest_payload = serde_json::json!({
        "mode": plan.mode,
        "selected": plan.selected.iter().map(|entry| serde_json::json!({
            "id": entry.record.id,
            "token_count": entry.token_count,
            "token_count_is_estimated": entry.token_count_is_estimated,
            "reason": entry.selection_reason,
            "verify": entry.requires_current_state_verification,
        })).collect::<Vec<_>>(),
        "rejected": plan.rejected,
        "used_input_tokens": plan.used_input_tokens,
        "remaining_input_tokens": plan.remaining_input_tokens,
        "token_counts_are_estimated": plan.token_counts_are_estimated,
    });
    let encoded = serde_json::to_vec(&digest_payload).unwrap_or_default();
    plan.plan_digest = format!("sha256:{:x}", Sha256::digest(encoded));
    plan
}

/// Stable, stateless JSON C ABI for platform memory selection.
/// Hosts own storage and candidate retrieval; Core only applies policy.




fn memory_time_rejection(
    record: &MemoryRecord,
    policy: &MemoryPolicy,
    now: Option<&chrono::DateTime<chrono::FixedOffset>>,
) -> Option<MemorySelectionRejectReason> {
    let Some(now) = now else { return None };
    if let MemoryRetention::ExpiresAt { expires_at } = &record.retention {
        let Ok(expires_at) = chrono::DateTime::parse_from_rfc3339(expires_at) else {
            return Some(MemorySelectionRejectReason::InvalidTimestamp);
        };
        if expires_at <= *now {
            return Some(MemorySelectionRejectReason::Expired);
        }
    }
    let Some(maximum_age_days) = policy.maximum_age_days else {
        return None;
    };
    let Some(observed_at) = record.provenance.observed_at.as_deref() else {
        return Some(MemorySelectionRejectReason::InvalidTimestamp);
    };
    let Ok(observed_at) = chrono::DateTime::parse_from_rfc3339(observed_at) else {
        return Some(MemorySelectionRejectReason::InvalidTimestamp);
    };
    if observed_at > *now {
        return Some(MemorySelectionRejectReason::InvalidTimestamp);
    }
    let maximum_age = chrono::Duration::days(i64::from(maximum_age_days));
    (now.signed_duration_since(observed_at) > maximum_age)
        .then_some(MemorySelectionRejectReason::TooOld)
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_defaults_are_explicit_and_privacy_safe() {
        let policy: MemoryPolicy = serde_json::from_str("{}").unwrap();
        assert!(policy.long_term_enabled);
        assert!(!policy.automatic_extraction_enabled);
        assert_eq!(policy.search_limit, 8);
        assert_eq!(policy.minimum_confidence, 0.65);
        assert!(policy.allowed_sensitivities.is_empty());
    }

    #[test]
    fn record_round_trip_preserves_provenance_and_supersession() {
        let record = MemoryRecord {
            id: "memory-2".to_string(),
            owner: nola_domain::memory_owner::MemoryOwner::local_profile(),
            namespace: MemoryNamespace::Personal,
            scope: MemoryScope::Global,
            kind: MemoryKind::Preference,
            content: "Prefer concise release notes".to_string(),
            summary: None,
            tags: vec!["writing".to_string()],
            confidence: 0.9,
            importance: 0.7,
            sensitivity: None,
            retention: MemoryRetention::UntilSuperseded,
            supersedes: vec!["memory-1".to_string()],
            provenance: MemoryProvenance {
                kind: MemoryProvenanceKind::ExplicitUser,
                source: "explicit_user_statement".to_string(),
                session_id: Some("session-1".to_string()),
                turn_id: Some("turn-4".to_string()),
                item_ids: vec!["message-9".to_string()],
                observed_at: None,
                ..MemoryProvenance::default()
            },
        };
        let restored: MemoryRecord =
            serde_json::from_value(serde_json::to_value(&record).unwrap()).unwrap();
        assert_eq!(restored, record);
    }

    #[test]
    fn record_validation_rejects_incomplete_or_reversed_event_ranges() {
        let mut record = MemoryRecord {
            id: "memory-temporal".to_string(),
            owner: nola_domain::memory_owner::MemoryOwner::local_profile(),
            namespace: MemoryNamespace::Personal,
            scope: MemoryScope::Global,
            kind: MemoryKind::Reference,
            content: "A recorded event interval".to_string(),
            summary: None,
            tags: Vec::new(),
            confidence: 0.9,
            importance: 0.7,
            sensitivity: None,
            retention: MemoryRetention::Permanent,
            supersedes: Vec::new(),
            provenance: MemoryProvenance {
                source: "conversation://source".to_string(),
                temporal: MemoryTemporalMetadata {
                    range_start: Some("2026-08-02".to_string()),
                    range_end: None,
                    ..MemoryTemporalMetadata::default()
                },
                ..MemoryProvenance::default()
            },
        };
        assert!(record.validate().is_err());

        record.provenance.temporal.range_end = Some("2026-08-01".to_string());
        assert!(record.validate().is_err());

        record.provenance.temporal.range_end = Some("2026-08-03".to_string());
        assert!(record.validate().is_ok());
    }

    #[test]
    fn usage_evidence_round_trip_preserves_history_and_supersession() {
        let usage = MemoryUsage {
            record_id: "memory-2".to_string(),
            component_id: "context-memory-2".to_string(),
            scope: Some(MemoryScope::Global),
            namespace: Some(MemoryNamespace::Workspace),
            provenance_kind: Some(MemoryProvenanceKind::ExplicitUser),
            source: Some("conversation://session-1/message-9".to_string()),
            title: Some("Workspace preference".to_string()),
            observed_at: Some("2026-08-06T00:00:00Z".to_string()),
            source_role: Some(MemorySourceRole::User),
            event_time: None,
            range_start: None,
            range_end: None,
            requires_current_state_verification: true,
            truncated: false,
            supersedes: vec!["memory-1".to_string()],
            usage_count: 4,
            last_used_at_epoch_ms: Some(1_786_000_000_000),
        };

        let restored: MemoryUsage =
            serde_json::from_value(serde_json::to_value(&usage).unwrap()).unwrap();

        assert_eq!(restored, usage);
    }

    #[test]
    fn automatic_long_term_candidate_requires_semantic_and_structural_quality() {
        let mut record = MemoryRecord {
            id: "memory-1".to_string(),
            owner: nola_domain::memory_owner::MemoryOwner::local_profile(),
            namespace: MemoryNamespace::Personal,
            scope: MemoryScope::Global,
            kind: MemoryKind::Feedback,
            content: "Prefer real end-to-end evidence for routing changes.".to_string(),
            summary: Some("Use real E2E evidence for routing changes".to_string()),
            tags: vec!["testing".to_string()],
            confidence: 0.95,
            importance: 0.9,
            sensitivity: None,
            retention: MemoryRetention::UntilSuperseded,
            supersedes: Vec::new(),
            provenance: MemoryProvenance {
                kind: MemoryProvenanceKind::AgentInference,
                source: "nola://session/session-1".to_string(),
                session_id: Some("session-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_ids: vec!["message-1".to_string()],
                observed_at: None,
                ..MemoryProvenance::default()
            },
        };
        let quality = MemoryQualityAssessment {
            applicable: true,
            durable: true,
            legible: true,
            rationale: "The user repeatedly applies this testing rule across changes.".to_string(),
        };
        assert!(record
            .validate_automatic_long_term_candidate(&quality)
            .is_ok());

        record.provenance.kind = MemoryProvenanceKind::ExplicitUser;
        assert!(record
            .validate_automatic_long_term_candidate(&quality)
            .unwrap_err()
            .contains("agent-inference provenance"));
        record.provenance.kind = MemoryProvenanceKind::AgentInference;

        record.confidence = 0.4;
        assert!(record
            .validate_automatic_long_term_candidate(&quality)
            .unwrap_err()
            .contains("confidence"));
        record.confidence = 0.95;

        record.provenance.item_ids.clear();
        assert!(record
            .validate_automatic_long_term_candidate(&quality)
            .unwrap_err()
            .contains("source item ids"));
        record.provenance.item_ids.push("message-1".to_string());

        record.retention = MemoryRetention::Session;
        assert!(record
            .validate_automatic_long_term_candidate(&quality)
            .is_err());

        let weak_quality = MemoryQualityAssessment {
            applicable: true,
            durable: false,
            legible: true,
            rationale: "Only describes the current turn.".to_string(),
        };
        assert!(record
            .validate_automatic_long_term_candidate(&weak_quality)
            .is_err());
    }

    #[test]
    fn projection_enforces_typed_crud_and_namespace_order() {
        let mut projection = MemoryProjection::default();
        let record = MemoryRecord {
            id: "memory-1".to_string(),
            owner: nola_domain::memory_owner::MemoryOwner::Workspace { profile_id: "local".into(), workspace_id: "workspace-fixture".into() },
            namespace: MemoryNamespace::Workspace,
            scope: MemoryScope::Global,
            kind: MemoryKind::Project,
            content: "Use the Core contract".to_string(),
            summary: None,
            tags: Vec::new(),
            confidence: 0.9,
            importance: 0.8,
            sensitivity: None,
            retention: MemoryRetention::Permanent,
            supersedes: Vec::new(),
            provenance: MemoryProvenance {
                kind: MemoryProvenanceKind::ExplicitUser,
                source: "message-1".to_string(),
                session_id: Some("session-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_ids: vec!["message-1".to_string()],
                observed_at: None,
                ..MemoryProvenance::default()
            },
        };
        projection
            .apply(MemoryUpdate::Created {
                record: record.clone(),
            })
            .unwrap();
        assert_eq!(projection.revision, 1);
        assert_eq!(
            projection.records_in_namespace(&MemoryNamespace::Workspace),
            vec![&record]
        );
        assert!(projection
            .apply(MemoryUpdate::Created {
                record: record.clone()
            })
            .is_err());
        projection
            .apply(MemoryUpdate::Deleted {
                memory_id: record.id.clone(),
            })
            .unwrap();
        assert!(projection.is_empty());
        assert_eq!(projection.revision, 2);
    }
}
