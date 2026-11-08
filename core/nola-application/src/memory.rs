// SPDX-License-Identifier: Apache-2.0
//! Application-owned long-term-memory aggregate.
//!
//! Core owns semantic memory types and policy. Application owns validation,
//! durable lifecycle, deduplication, and query orchestration. Store JSON
//! columns never cross this boundary.

use nola_agent_engine::agent::{
    select_memories, MemoryKind, MemoryNamespace, MemoryPolicy, MemoryProvenance,
    MemoryQualityAssessment, MemoryRecallMode, MemoryRecord as SemanticMemoryRecord,
    MemoryRetention, MemoryScope, MemorySelectionInput, MemoryTokenAccounting,
};
use nola_store_port::memory::{MemoryReadRepository, MemoryRepository, StoredMemoryRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecord {
    pub record: SemanticMemoryRecord,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub deleted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoryUsageStats {
    pub memory_id: String,
    pub usage_count: u64,
    pub last_used_at_ms: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCommitIntent {
    ExplicitUser,
    ValidatedAutomatic,
    Imported,
    Synced,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MemoryCandidate {
    pub record: SemanticMemoryRecord,
    pub quality: Option<MemoryQualityAssessment>,
    pub proposed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecallQuery {
    pub binding: nola_domain::memory_owner::MemoryExecutionBinding,
    pub namespace: MemoryNamespace,
    pub scope: MemoryScope,
    pub query: Option<String>,
    pub kind: Option<MemoryKind>,
    pub now_iso: String,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MemoryCommand {
    Replace {
        expected: Vec<MemoryRecord>,
        candidates: Vec<MemoryCandidate>,
        intent: MemoryCommitIntent,
        deleted_at_ms: i64,
    },
    Commit {
        input: MemoryCandidate,
        intent: MemoryCommitIntent,
    },
    List {
        limit: usize,
    },
    Get {
        id: String,
    },
    GetMany {
        ids: Vec<String>,
    },
    UsageStats {
        ids: Vec<String>,
    },
    Recall {
        input: MemoryRecallQuery,
    },
    Delete {
        id: String,
        deleted_at_ms: i64,
    },
}

pub(crate) fn execute_memory_command<R: MemoryRepository + ?Sized>(
    store: &mut R,
    command: MemoryCommand,
) -> Result<Value, String> {
    match command {
        MemoryCommand::Replace {
            mut expected,
            candidates,
            intent,
            deleted_at_ms,
        } => {
            if !store.has_write_unit_of_work() {
                return Err("memory replacement requires durable command admission".into());
            }
            const LIMIT: usize = 1_000;
            if expected.len() > LIMIT || candidates.len() > LIMIT {
                return Err("memory replacement exceeds the bounded profile size".into());
            }
            let mut current = store
                .list_memory_records(LIMIT + 1)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(from_store_record)
                .collect::<Result<Vec<_>, _>>()?;
            current.sort_by(|a, b| a.record.id.cmp(&b.record.id));
            expected.sort_by(|a, b| a.record.id.cmp(&b.record.id));
            if current != expected {
                return Err(
                    "memory_snapshot_conflict: reload the canonical profile before saving".into(),
                );
            }
            let mut ids = std::collections::BTreeSet::new();
            for candidate in &candidates {
                validate_candidate(candidate, intent)?;
                if !ids.insert(candidate.record.id.clone()) {
                    return Err("memory replacement contains duplicate candidate IDs".into());
                }
            }
            let mut saved = Vec::new();
            let mut retained = std::collections::BTreeSet::new();
            for candidate in candidates {
                // Retaining a record is not a new observation or use. Preserve all
                // lifecycle metadata instead of rebuilding it from a candidate.
                if let Ok(index) =
                    current.binary_search_by(|item| item.record.id.cmp(&candidate.record.id))
                {
                    if current[index].record == candidate.record {
                        retained.insert(candidate.record.id.clone());
                        saved.push(current[index].clone());
                        continue;
                    }
                }
                // Explicit replacement names the desired identities. Content
                // deduplication here can overwrite another candidate in this batch.
                let mut input = to_store_record(candidate)?;
                if let Ok(index) = current.binary_search_by(|item| item.record.id.cmp(&input.id)) {
                    input.last_used_at_ms = current[index].last_used_at_ms;
                }
                let stored = store
                    .upsert_memory_record(&input)
                    .map_err(|error| error.to_string())?;
                if retained.insert(stored.id.clone()) {
                    saved.push(from_store_record(stored)?);
                }
            }
            for record in current {
                if !retained.contains(&record.record.id) {
                    store
                        .delete_memory_record(&record.record.id, deleted_at_ms)
                        .map_err(|error| error.to_string())?;
                }
            }
            encode(saved, "memory replacement")
        }
        MemoryCommand::Commit { input, intent } => {
            validate_candidate(&input, intent)?;
            let stored_input = to_store_record(input)?;
            let saved = store
                .save_memory_record_deduplicated(&stored_input)
                .map_err(|error| format!("save deduplicated memory failed: {error}"))?;
            encode(from_store_record(saved)?, "memory")
        }
        MemoryCommand::List { limit } => {
            let records = store
                .list_memory_records(limit)
                .map_err(|error| format!("list memories failed: {error}"))?
                .into_iter()
                .map(from_store_record)
                .collect::<Result<Vec<_>, _>>()?;
            encode(records, "memory list")
        }
        MemoryCommand::Get { id } => {
            let record = store
                .memory_record(&id)
                .map_err(|error| format!("load memory failed: {error}"))?
                .map(from_store_record)
                .transpose()?;
            encode(record, "memory")
        }
        MemoryCommand::GetMany { ids } => {
            let records = store
                .memory_records_by_ids(&ids)
                .map_err(|error| format!("load memories failed: {error}"))?
                .into_iter()
                .map(from_store_record)
                .collect::<Result<Vec<_>, _>>()?;
            encode(records, "memory list")
        }
        MemoryCommand::UsageStats { ids } => encode(
            store
                .memory_usage_stats(&ids)
                .map_err(|error| format!("load memory usage stats failed: {error}"))?
                .into_iter()
                .map(|stats| MemoryUsageStats {
                    memory_id: stats.memory_id,
                    usage_count: stats.usage_count,
                    last_used_at_ms: stats.last_used_at_ms,
                })
                .collect::<Vec<_>>(),
            "memory usage stats",
        ),
        MemoryCommand::Recall { input } => {
            let owner = input.binding.owner_for(&input.namespace, input.scope)?;
            let owner_json = serde_json::to_string(&owner).map_err(|error| error.to_string())?;
            chrono::DateTime::parse_from_rfc3339(&input.now_iso)
                .map_err(|_| "memory recall requires a valid RFC3339 now_iso".to_string())?;
            let namespace_json = serde_json::to_string(&input.namespace)
                .map_err(|error| format!("encode memory namespace failed: {error}"))?;
            let kind = input.kind.map(memory_kind);
            // Store performs bounded acquisition and literal matching. Core then
            // owns the semantic safety gates so every client gets the same
            // confidence, supersession, freshness, namespace, and token policy.
            let candidates = store
                .recall_memory_records(
                    &owner_json,
                    &namespace_json,
                    memory_scope(input.scope),
                    input.query.as_deref(),
                    kind,
                    &input.now_iso,
                    50,
                )
                .map_err(|error| format!("recall memories failed: {error}"))?
                .into_iter()
                .map(from_store_record)
                .collect::<Result<Vec<_>, _>>()?;
            let records_by_id = candidates
                .iter()
                .cloned()
                .map(|record| (record.record.id.clone(), record))
                .collect::<std::collections::BTreeMap<_, _>>();
            let plan = select_memories(MemorySelectionInput {
                policy: MemoryPolicy {
                    search_limit: input.limit.clamp(1, 50),
                    // `normal` is the non-sensitive label emitted by the
                    // built-in memory writer. An explicit Recall command is
                    // already the authorized on-demand projection boundary;
                    // leaving this list empty made freshly saved ordinary
                    // records impossible to read back. Sensitive and
                    // restricted labels remain excluded unless a future
                    // caller supplies a stronger authorization contract.
                    allowed_sensitivities: vec!["normal".to_string()],
                    ..MemoryPolicy::default()
                },
                records: candidates.into_iter().map(|record| record.record).collect(),
                mode: MemoryRecallMode::OnDemand,
                allowed_namespaces: vec![input.namespace],
                // Session retention is visible only under an exact local
                // conversation/turn owner, never a global profile or workspace.
                include_session_retention: matches!(
                    owner,
                    nola_domain::memory_owner::MemoryOwner::Conversation { .. }
                        | nola_domain::memory_owner::MemoryOwner::Turn { .. }
                ),
                now: Some(input.now_iso),
                maximum_input_tokens: None,
                token_accounting: MemoryTokenAccounting::CoreEstimate,
                token_counts: std::collections::BTreeMap::new(),
            });
            let records = plan
                .selected
                .into_iter()
                // Superseding facts of another kind still participate in Core's
                // conflict suppression, but a typed recall must not expose them
                // as if they matched the caller's requested kind.
                .filter(|selected| {
                    kind.is_none_or(|expected| memory_kind(selected.record.kind) == expected)
                })
                .filter_map(|selected| records_by_id.get(&selected.record.id).cloned())
                .collect::<Vec<_>>();
            encode(records, "memory recall")
        }
        MemoryCommand::Delete { id, deleted_at_ms } => Ok(json!(store
            .delete_memory_record(&id, deleted_at_ms)
            .map_err(|error| format!("delete memory failed: {error}"))?)),
    }
}

fn validate_candidate(
    candidate: &MemoryCandidate,
    intent: MemoryCommitIntent,
) -> Result<(), String> {
    match intent {
        MemoryCommitIntent::ValidatedAutomatic => {
            if candidate.record.supersedes.is_empty()
                && (candidate.record.owner
                    != nola_domain::memory_owner::MemoryOwner::local_profile()
                    || candidate.record.namespace != MemoryNamespace::Personal)
            {
                return Err(
                    "automatic extraction requires the current personal memory profile".into(),
                );
            }
            let quality = candidate.quality.as_ref().ok_or_else(|| {
                "automatic memory candidate requires a quality assessment".to_string()
            })?;
            candidate
                .record
                .validate_automatic_long_term_candidate(quality)
        }
        MemoryCommitIntent::ExplicitUser
        | MemoryCommitIntent::Imported
        | MemoryCommitIntent::Synced => candidate.record.validate(),
    }
}

/// Revalidate the frozen semantic input; Store rechecks the observed row
/// revisions inside the checkpoint commit transaction to close the read/write gap.
pub(crate) fn idle_memory_input_preconditions<R: MemoryReadRepository + ?Sized>(
    store: &R,
    submission: &nola_agent_engine::agent::AgentSubmission,
    runtime: Option<&nola_agent_engine::agent::memory_runtime::MemoryRuntimeProjection>,
) -> Result<Vec<(String, i64)>, String> {
    use nola_agent_engine::agent::memory_runtime::{consolidation_input_digest, MemoryJobKind};
    use nola_agent_engine::agent::{AgentSubmission, MemoryRuntimeCommand};
    let (ids, expected_digest) = match submission {
        AgentSubmission::MemoryRuntime { envelope } => match &envelope.command {
            MemoryRuntimeCommand::CreateConsolidation {
                selected_memory_ids,
                input_digest,
                ..
            } => (selected_memory_ids.as_slice(), input_digest.as_str()),
            MemoryRuntimeCommand::Complete { job_id, .. } => {
                let job = runtime
                    .and_then(|runtime| runtime.job(job_id))
                    .ok_or_else(|| format!("unknown memory job {job_id}"))?;
                if job.kind != MemoryJobKind::Consolidation {
                    return Ok(Vec::new());
                }
                (
                    job.selected_memory_ids.as_slice(),
                    job.input_digest.as_deref().ok_or_else(|| {
                        "memory consolidation input digest is missing".to_string()
                    })?,
                )
            }
            _ => return Ok(Vec::new()),
        },
        AgentSubmission::MemoryWorkerCompleted { job_id, .. } => {
            let job = runtime
                .and_then(|runtime| runtime.job(job_id))
                .ok_or_else(|| format!("unknown memory job {job_id}"))?;
            if job.kind != MemoryJobKind::Consolidation {
                return Ok(Vec::new());
            }
            (
                job.selected_memory_ids.as_slice(),
                job.input_digest
                    .as_deref()
                    .ok_or_else(|| "memory consolidation input digest is missing".to_string())?,
            )
        }
        _ => return Ok(Vec::new()),
    };
    if ids.is_empty() || ids.len() > 40 {
        return Err("memory consolidation requires between 1 and 40 input records".into());
    }
    let mut records = Vec::with_capacity(ids.len());
    let mut revisions = Vec::with_capacity(ids.len());
    for id in ids {
        let (record, revision) = store
            .memory_record_with_revision(id)
            .map_err(|error| format!("load memory input failed: {error}"))?
            .ok_or_else(|| format!("memory_input_conflict: input {id} is missing or deleted"))?;
        records.push(from_store_record(record)?.record);
        revisions.push((id.clone(), revision));
    }
    let owner = &records[0];
    if records.iter().any(|record| {
        record.owner != owner.owner
            || record.namespace != owner.namespace
            || record.scope != owner.scope
    }) {
        return Err(
            "memory_owner_conflict: consolidation input crosses owners or collections".into(),
        );
    }
    let output = match submission {
        AgentSubmission::MemoryRuntime { envelope } => match &envelope.command {
            MemoryRuntimeCommand::Complete { output, .. } => Some(output),
            _ => None,
        },
        // Provider-shaped worker output is normalized by Engine from its frozen plan.
        AgentSubmission::MemoryWorkerCompleted { .. } => None,
        _ => None,
    };
    if let Some(nola_agent_engine::agent::memory_runtime::MemoryJobOutput::Consolidation(output)) =
        output
    {
        if output.records.iter().any(|record| {
            record.owner != owner.owner
                || record.namespace != owner.namespace
                || record.scope != owner.scope
        }) {
            return Err(
                "memory_owner_conflict: consolidation output differs from its frozen input owner"
                    .into(),
            );
        }
    }
    if consolidation_input_digest(&records)? != expected_digest {
        return Err("memory_input_conflict: consolidation input changed; plan a new job".into());
    }
    Ok(revisions)
}

pub(crate) fn validated_job_output_store_records(
    output: &nola_agent_engine::agent::memory_runtime::MemoryJobOutput,
    proposed_at_ms: i64,
) -> Result<Vec<StoredMemoryRecord>, String> {
    use nola_agent_engine::agent::memory_runtime::MemoryJobOutput;

    let candidates = match output {
        MemoryJobOutput::Extraction(extraction) => extraction
            .memories
            .iter()
            .map(|memory| MemoryCandidate {
                record: memory.record.clone(),
                quality: Some(memory.quality.clone()),
                proposed_at_ms,
            })
            .collect::<Vec<_>>(),
        MemoryJobOutput::Consolidation(consolidation) => {
            if consolidation.quality_by_memory_id.len() != consolidation.records.len() {
                return Err(
                    "consolidated memory quality assessments must match output records".to_string(),
                );
            }
            consolidation
                .records
                .iter()
                .map(|record| {
                    let quality = consolidation
                        .quality_by_memory_id
                        .get(&record.id)
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "consolidated memory {} has no quality assessment",
                                record.id
                            )
                        })?;
                    Ok(MemoryCandidate {
                        record: record.clone(),
                        quality: Some(quality),
                        proposed_at_ms,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?
        }
    };
    candidates
        .into_iter()
        .map(|candidate| {
            validate_candidate(&candidate, MemoryCommitIntent::ValidatedAutomatic)?;
            to_store_record(candidate)
        })
        .collect()
}

fn to_store_record(candidate: MemoryCandidate) -> Result<StoredMemoryRecord, String> {
    let record = candidate.record;
    let expires_at = match &record.retention {
        MemoryRetention::ExpiresAt { expires_at } => Some(expires_at.clone()),
        _ => None,
    };
    Ok(StoredMemoryRecord {
        id: record.id,
        owner_json: serde_json::to_string(&record.owner).map_err(|error| error.to_string())?,
        namespace_json: serde_json::to_string(&record.namespace)
            .map_err(|error| format!("encode memory namespace failed: {error}"))?,
        scope: memory_scope(record.scope).to_string(),
        kind: memory_kind(record.kind).to_string(),
        content: record.content,
        summary: record.summary,
        tags_json: serde_json::to_string(&record.tags)
            .map_err(|error| format!("encode memory tags failed: {error}"))?,
        confidence: f64::from(record.confidence),
        importance: f64::from(record.importance),
        sensitivity: record.sensitivity,
        retention_json: serde_json::to_string(&record.retention)
            .map_err(|error| format!("encode memory retention failed: {error}"))?,
        expires_at,
        supersedes_json: serde_json::to_string(&record.supersedes)
            .map_err(|error| format!("encode memory supersession failed: {error}"))?,
        provenance_json: serde_json::to_string(&record.provenance)
            .map_err(|error| format!("encode memory provenance failed: {error}"))?,
        created_at_ms: candidate.proposed_at_ms,
        updated_at_ms: candidate.proposed_at_ms,
        last_used_at_ms: None,
        deleted_at_ms: None,
    })
}

fn from_store_record(value: StoredMemoryRecord) -> Result<MemoryRecord, String> {
    let record = SemanticMemoryRecord {
        id: value.id,
        owner: serde_json::from_str(&value.owner_json).map_err(|error| error.to_string())?,
        namespace: serde_json::from_str(&value.namespace_json)
            .map_err(|error| format!("decode memory namespace failed: {error}"))?,
        scope: parse_memory_scope(&value.scope)?,
        kind: parse_memory_kind(&value.kind)?,
        content: value.content,
        summary: value.summary,
        tags: serde_json::from_str(&value.tags_json)
            .map_err(|error| format!("decode memory tags failed: {error}"))?,
        confidence: value.confidence as f32,
        importance: value.importance as f32,
        sensitivity: value.sensitivity,
        retention: serde_json::from_str(&value.retention_json)
            .map_err(|error| format!("decode memory retention failed: {error}"))?,
        supersedes: serde_json::from_str(&value.supersedes_json)
            .map_err(|error| format!("decode memory supersession failed: {error}"))?,
        provenance: serde_json::from_str::<MemoryProvenance>(&value.provenance_json)
            .map_err(|error| format!("decode memory provenance failed: {error}"))?,
    };
    record.validate()?;
    Ok(MemoryRecord {
        record,
        created_at_ms: value.created_at_ms,
        updated_at_ms: value.updated_at_ms,
        last_used_at_ms: value.last_used_at_ms,
        deleted_at_ms: value.deleted_at_ms,
    })
}

fn memory_scope(value: MemoryScope) -> &'static str {
    match value {
        MemoryScope::Turn => "turn",
        MemoryScope::Conversation => "conversation",
        MemoryScope::Global => "global",
    }
}

fn parse_memory_scope(value: &str) -> Result<MemoryScope, String> {
    match value {
        "turn" => Ok(MemoryScope::Turn),
        "conversation" => Ok(MemoryScope::Conversation),
        "global" => Ok(MemoryScope::Global),
        _ => Err(format!("unknown memory scope: {value}")),
    }
}

fn memory_kind(value: MemoryKind) -> &'static str {
    match value {
        MemoryKind::Preference => "preference",
        MemoryKind::Identity => "identity",
        MemoryKind::Decision => "decision",
        MemoryKind::Project => "project",
        MemoryKind::Feedback => "feedback",
        MemoryKind::Reference => "reference",
        MemoryKind::Other => "other",
    }
}

fn parse_memory_kind(value: &str) -> Result<MemoryKind, String> {
    match value {
        "preference" => Ok(MemoryKind::Preference),
        "identity" => Ok(MemoryKind::Identity),
        "decision" => Ok(MemoryKind::Decision),
        "project" => Ok(MemoryKind::Project),
        "feedback" => Ok(MemoryKind::Feedback),
        "reference" => Ok(MemoryKind::Reference),
        "other" => Ok(MemoryKind::Other),
        _ => Err(format!("unknown memory kind: {value}")),
    }
}

fn encode<T: Serialize>(value: T, label: &str) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| format!("encode {label} result failed: {error}"))
}
