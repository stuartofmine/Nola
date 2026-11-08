// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the platform-neutral durable runtime contract for memory extraction,
//   consolidation, citations, and usage accounting.
// - Boundary: Core owns validation and state transitions; Hosts own LLM calls and persistence I/O.

use crate::memory::{MemoryQualityAssessment, MemoryRecord, MemoryUsage};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_RETRY_DELAY_MS: i64 = 2_000;
const MAX_RETRY_DELAY_MS: i64 = 30_000;
const DEFAULT_CONSOLIDATION_MINIMUM: usize = 8;
const DEFAULT_CONSOLIDATION_LIMIT: usize = 40;

/// Identifies the semantic input, not merely the set of database identities.
/// Ordering is normalized so a ranking change alone does not create new work.
pub fn consolidation_input_digest(records: &[MemoryRecord]) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    struct HashWriter(Sha256);
    impl std::io::Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut ordered = records.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    if ordered.is_empty() || ordered.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err("memory consolidation input requires non-empty unique records".into());
    }
    for record in &ordered {
        record.validate()?;
    }
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, &ordered)
        .map_err(|error| format!("encode memory consolidation input failed: {error}"))?;
    Ok(format!("sha256:{:x}", writer.0.finalize()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryJobKind {
    Extraction,
    Consolidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryJobState {
    Queued,
    Running,
    RetryWaiting,
    Completed,
    Failed,
    Cancelled,
}

impl MemoryJobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySourceSnapshot {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_ids: Vec<String>,
    pub transcript_digest: String,
    pub source_watermark: String,
    pub captured_at_epoch_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCitation {
    pub citation_id: String,
    pub memory_id: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedMemory {
    pub record: MemoryRecord,
    pub quality: MemoryQualityAssessment,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<MemoryCitation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryExtractionOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<ExtractedMemory>,
    pub rollout_summary: String,
    pub output_digest: String,
    #[serde(default)]
    pub redacted_secret_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryConsolidationOutput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<MemoryRecord>,
    pub summary: String,
    pub output_digest: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<MemoryCitation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded_memory_ids: Vec<String>,
    /// One semantic quality assessment for every output record, keyed by the
    /// record id. Core rejects missing, extra, or weak assessments.
    pub quality_by_memory_id: BTreeMap<String, MemoryQualityAssessment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemoryJobOutput {
    Extraction(MemoryExtractionOutput),
    Consolidation(MemoryConsolidationOutput),
}

impl MemoryJobOutput {
    fn kind(&self) -> MemoryJobKind {
        match self {
            Self::Extraction(_) => MemoryJobKind::Extraction,
            Self::Consolidation(_) => MemoryJobKind::Consolidation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryJobHandle {
    pub id: String,
    pub kind: MemoryJobKind,
    pub state: MemoryJobState,
    pub created_at_epoch_ms: i64,
    pub updated_at_epoch_ms: i64,
    pub input_watermark: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_watermark: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<MemorySourceSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_memory_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ownership_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_until_epoch_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_epoch_ms: Option<i64>,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at_epoch_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<MemoryJobOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryUsageLedgerEntry {
    pub usage_id: String,
    pub memory_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub used_at_epoch_ms: i64,
    pub context_payload_digest: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citation_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryUsageStats {
    pub usage_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at_epoch_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemoryRuntimeEvent {
    JobCreated {
        job: MemoryJobHandle,
    },
    JobClaimed {
        job: MemoryJobHandle,
    },
    JobHeartbeat {
        job: MemoryJobHandle,
    },
    JobRetryScheduled {
        job: MemoryJobHandle,
    },
    JobCompleted {
        job: MemoryJobHandle,
    },
    JobFailed {
        job: MemoryJobHandle,
    },
    JobCancelled {
        job: MemoryJobHandle,
    },
    UsageRecorded {
        entry: MemoryUsageLedgerEntry,
        stats: MemoryUsageStats,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemoryRuntimeCommand {
    CreateExtraction {
        job_id: String,
        input_watermark: String,
        sources: Vec<MemorySourceSnapshot>,
        now_epoch_ms: i64,
    },
    CreateConsolidation {
        job_id: String,
        input_watermark: String,
        input_digest: String,
        selected_memory_ids: Vec<String>,
        now_epoch_ms: i64,
    },
    Claim {
        job_id: String,
        worker_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
    },
    Heartbeat {
        job_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
    },
    ScheduleRetry {
        job_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_ms: Option<i64>,
        max_retry_count: u32,
        error: String,
    },
    Complete {
        job_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        completion_watermark: String,
        output: MemoryJobOutput,
    },
    Cancel {
        job_id: String,
        now_epoch_ms: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    SweepExpiredLeases {
        now_epoch_ms: i64,
        max_retry_count: u32,
    },
    RecordUsage {
        entry: MemoryUsageLedgerEntry,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRuntimeCommandEnvelope {
    pub operation_id: String,
    pub command: MemoryRuntimeCommand,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryRuntimeProjection {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub jobs: BTreeMap<String, MemoryJobHandle>,
    #[serde(default)]
    pub job_order: Vec<String>,
    #[serde(default)]
    pub usage_ledger: Vec<MemoryUsageLedgerEntry>,
    #[serde(default)]
    pub usage_stats: BTreeMap<String, MemoryUsageStats>,
    #[serde(default)]
    processed_operations: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    processed_operation_digests: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRuntimeApplyResult {
    pub revision: u64,
    pub duplicate: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<MemoryRuntimeEvent>,
}

impl MemoryRuntimeProjection {
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty() && self.usage_ledger.is_empty()
    }

    pub fn job(&self, job_id: &str) -> Option<&MemoryJobHandle> {
        self.jobs.get(job_id)
    }

    /// Returns work that a Host may claim at the supplied clock boundary.
    /// Readiness is a Core policy decision; Hosts must not duplicate state or
    /// retry-deadline interpretation in platform-specific code.
    pub fn claimable_jobs(&self, now_epoch_ms: i64) -> Vec<MemoryJobHandle> {
        self.job_order
            .iter()
            .filter_map(|job_id| self.jobs.get(job_id))
            .filter(|job| match job.state {
                MemoryJobState::Queued => true,
                MemoryJobState::RetryWaiting => job
                    .retry_at_epoch_ms
                    .is_some_and(|retry_at| retry_at <= now_epoch_ms),
                MemoryJobState::Running
                | MemoryJobState::Completed
                | MemoryJobState::Failed
                | MemoryJobState::Cancelled => false,
            })
            .cloned()
            .collect()
    }

    /// Selects the live records for the next consolidation pass. Core owns
    /// this policy so every Host gets the same single-flight, supersession,
    /// and usage-aware behavior without reimplementing lifecycle rules.
    pub fn consolidation_candidates(&self, records: &[MemoryRecord]) -> Vec<MemoryRecord> {
        self.consolidation_candidates_with_usage(records, &BTreeMap::new())
    }

    pub fn consolidation_candidates_with_usage(
        &self,
        records: &[MemoryRecord],
        durable_usage_counts: &BTreeMap<String, u64>,
    ) -> Vec<MemoryRecord> {
        if self
            .jobs
            .values()
            .any(|job| job.kind == MemoryJobKind::Consolidation && !job.state.is_terminal())
        {
            return Vec::new();
        }
        let superseded = records
            .iter()
            .flat_map(|record| {
                record
                    .supersedes
                    .iter()
                    .map(|memory_id| (record.owner.clone(), record.namespace.clone(), record.scope, memory_id.clone()))
            })
            .collect::<BTreeSet<_>>();
        let mut owner_groups =
            BTreeMap::<(nola_domain::memory_owner::MemoryOwner, crate::memory::MemoryNamespace), Vec<&MemoryRecord>>::new();
        for record in records.iter().filter(|record| {
            record.validate().is_ok() && record.scope == crate::memory::MemoryScope::Global
                && !superseded.contains(&(
                    record.owner.clone(),
                    record.namespace.clone(),
                    record.scope,
                    record.id.clone(),
                ))
        }) {
            owner_groups
                .entry((record.owner.clone(), record.namespace.clone()))
                .or_default()
                .push(record);
        }
        let Some(largest_group_size) = owner_groups.values().map(Vec::len).max() else {
            return Vec::new();
        };
        if largest_group_size < DEFAULT_CONSOLIDATION_MINIMUM {
            return Vec::new();
        }
        // One worker job owns one concrete owner and namespace. Choose the
        // largest group and use owner/namespace ordering to break ties.
        let mut candidates = owner_groups
            .into_iter()
            .find_map(|(_, records)| (records.len() == largest_group_size).then_some(records))
            .expect("largest memory owner group exists");
        candidates.sort_by(|left, right| {
            let usage_count = |id: &str| {
                durable_usage_counts.get(id).copied().unwrap_or_else(|| {
                    self.usage_stats
                        .get(id)
                        .map(|stats| stats.usage_count)
                        .unwrap_or_default()
                })
            };
            let left_usage = usage_count(&left.id);
            let right_usage = usage_count(&right.id);
            right_usage
                .cmp(&left_usage)
                .then_with(|| right.importance.total_cmp(&left.importance))
                .then_with(|| right.confidence.total_cmp(&left.confidence))
                .then_with(|| left.id.cmp(&right.id))
        });
        candidates.truncate(DEFAULT_CONSOLIDATION_LIMIT);
        candidates.into_iter().cloned().collect()
    }

    pub fn has_processed_operation(&self, operation_id: &str) -> bool {
        self.processed_operations.contains(operation_id)
    }

    /// Validates worker output against the immutable job input without
    /// changing lifecycle state. Hosts use this before durable I/O so an
    /// invalid model response can never be committed ahead of Core.
    pub fn validate_job_output(
        &self,
        job_id: &str,
        output: &MemoryJobOutput,
    ) -> Result<(), String> {
        let job = self
            .job(job_id)
            .ok_or_else(|| format!("unknown memory job {job_id}"))?;
        if output.kind() != job.kind {
            return Err("memory job output kind does not match the claimed job".to_string());
        }
        validate_memory_job_output(job, output)
    }

    pub fn apply(
        &mut self,
        envelope: MemoryRuntimeCommandEnvelope,
    ) -> Result<MemoryRuntimeApplyResult, String> {
        let operation_id = envelope.operation_id.trim();
        if operation_id.is_empty() {
            return Err("memory runtime operation id must not be empty".to_string());
        }
        let command_digest = memory_runtime_command_digest(&envelope.command)?;
        if self.processed_operations.contains(operation_id) {
            match self.processed_operation_digests.get(operation_id) {
                Some(processed_digest) if processed_digest == &command_digest => {}
                Some(_) => {
                    return Err(format!(
                        "memory runtime operation {operation_id} was replayed with a different command"
                    ));
                }
                None => {
                    return Err(format!(
                        "memory runtime operation {operation_id} replay identity is unavailable"
                    ));
                }
            }
            return Ok(MemoryRuntimeApplyResult {
                revision: self.revision,
                duplicate: true,
                events: Vec::new(),
            });
        }

        let events = match envelope.command {
            MemoryRuntimeCommand::CreateExtraction {
                job_id,
                input_watermark,
                sources,
                now_epoch_ms,
            } => vec![self.create_extraction(job_id, input_watermark, sources, now_epoch_ms)?],
            MemoryRuntimeCommand::CreateConsolidation {
                job_id,
                input_watermark,
                input_digest,
                selected_memory_ids,
                now_epoch_ms,
            } => vec![self.create_consolidation(
                job_id,
                input_watermark,
                input_digest,
                selected_memory_ids,
                now_epoch_ms,
            )?],
            MemoryRuntimeCommand::Claim {
                job_id,
                worker_id,
                ownership_token,
                now_epoch_ms,
                lease_until_epoch_ms,
            } => vec![self.claim(
                &job_id,
                worker_id,
                ownership_token,
                now_epoch_ms,
                lease_until_epoch_ms,
            )?],
            MemoryRuntimeCommand::Heartbeat {
                job_id,
                ownership_token,
                now_epoch_ms,
                lease_until_epoch_ms,
            } => vec![self.heartbeat(
                &job_id,
                &ownership_token,
                now_epoch_ms,
                lease_until_epoch_ms,
            )?],
            MemoryRuntimeCommand::ScheduleRetry {
                job_id,
                ownership_token,
                now_epoch_ms,
                retry_after_ms,
                max_retry_count,
                error,
            } => vec![self.schedule_retry(
                &job_id,
                &ownership_token,
                now_epoch_ms,
                retry_after_ms,
                max_retry_count,
                error,
            )?],
            MemoryRuntimeCommand::Complete {
                job_id,
                ownership_token,
                now_epoch_ms,
                completion_watermark,
                output,
            } => vec![self.complete(
                &job_id,
                &ownership_token,
                now_epoch_ms,
                completion_watermark,
                output,
            )?],
            MemoryRuntimeCommand::Cancel {
                job_id,
                now_epoch_ms,
                reason,
            } => vec![self.cancel(&job_id, now_epoch_ms, reason)?],
            MemoryRuntimeCommand::SweepExpiredLeases {
                now_epoch_ms,
                max_retry_count,
            } => self.sweep_expired(now_epoch_ms, max_retry_count),
            MemoryRuntimeCommand::RecordUsage { entry } => vec![self.record_usage(entry)?],
        };

        self.processed_operations.insert(operation_id.to_string());
        self.processed_operation_digests
            .insert(operation_id.to_string(), command_digest);
        self.revision = self.revision.saturating_add(events.len() as u64);
        Ok(MemoryRuntimeApplyResult {
            revision: self.revision,
            duplicate: false,
            events,
        })
    }

    fn create_extraction(
        &mut self,
        job_id: String,
        input_watermark: String,
        sources: Vec<MemorySourceSnapshot>,
        now_epoch_ms: i64,
    ) -> Result<MemoryRuntimeEvent, String> {
        validate_identifier("memory job id", &job_id)?;
        validate_identifier("memory input watermark", &input_watermark)?;
        if sources.is_empty() {
            return Err("memory extraction requires at least one source snapshot".to_string());
        }
        if sources.iter().any(|source| {
            source.session_id.trim().is_empty()
                || source.transcript_digest.trim().is_empty()
                || source.source_watermark.trim().is_empty()
        }) {
            return Err("memory extraction source snapshots must identify their session, digest, and watermark".to_string());
        }
        self.ensure_new_job(&job_id)?;
        let job = MemoryJobHandle {
            id: job_id.clone(),
            kind: MemoryJobKind::Extraction,
            state: MemoryJobState::Queued,
            created_at_epoch_ms: now_epoch_ms,
            updated_at_epoch_ms: now_epoch_ms,
            input_watermark,
            completion_watermark: None,
            sources,
            selected_memory_ids: Vec::new(),
            input_digest: None,
            worker_id: None,
            ownership_token: None,
            lease_until_epoch_ms: None,
            last_heartbeat_epoch_ms: None,
            attempt_count: 0,
            retry_count: 0,
            retry_at_epoch_ms: None,
            last_error: None,
            output: None,
        };
        self.jobs.insert(job_id.clone(), job.clone());
        self.job_order.push(job_id);
        Ok(MemoryRuntimeEvent::JobCreated { job })
    }

    fn create_consolidation(
        &mut self,
        job_id: String,
        input_watermark: String,
        input_digest: String,
        selected_memory_ids: Vec<String>,
        now_epoch_ms: i64,
    ) -> Result<MemoryRuntimeEvent, String> {
        validate_identifier("memory job id", &job_id)?;
        validate_identifier("memory input watermark", &input_watermark)?;
        validate_identifier("memory consolidation input digest", &input_digest)?;
        if selected_memory_ids.is_empty() {
            return Err("memory consolidation requires selected memory ids".to_string());
        }
        if selected_memory_ids.iter().any(|id| id.trim().is_empty())
            || selected_memory_ids.iter().collect::<BTreeSet<_>>().len()
                != selected_memory_ids.len()
        {
            return Err(
                "memory consolidation selected ids must be non-empty and unique".to_string(),
            );
        }
        if self
            .jobs
            .values()
            .any(|job| job.kind == MemoryJobKind::Consolidation && !job.state.is_terminal())
        {
            return Err("another memory consolidation job is already active".to_string());
        }
        self.ensure_new_job(&job_id)?;
        let job = MemoryJobHandle {
            id: job_id.clone(),
            kind: MemoryJobKind::Consolidation,
            state: MemoryJobState::Queued,
            created_at_epoch_ms: now_epoch_ms,
            updated_at_epoch_ms: now_epoch_ms,
            input_watermark,
            completion_watermark: None,
            sources: Vec::new(),
            selected_memory_ids,
            input_digest: Some(input_digest),
            worker_id: None,
            ownership_token: None,
            lease_until_epoch_ms: None,
            last_heartbeat_epoch_ms: None,
            attempt_count: 0,
            retry_count: 0,
            retry_at_epoch_ms: None,
            last_error: None,
            output: None,
        };
        self.jobs.insert(job_id.clone(), job.clone());
        self.job_order.push(job_id);
        Ok(MemoryRuntimeEvent::JobCreated { job })
    }

    fn claim(
        &mut self,
        job_id: &str,
        worker_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
    ) -> Result<MemoryRuntimeEvent, String> {
        validate_identifier("memory worker id", &worker_id)?;
        validate_identifier("memory ownership token", &ownership_token)?;
        if lease_until_epoch_ms <= now_epoch_ms {
            return Err("memory job lease must expire in the future".to_string());
        }
        let job = self.require_job_mut(job_id)?;
        if !matches!(
            job.state,
            MemoryJobState::Queued | MemoryJobState::RetryWaiting
        ) {
            return Err(format!(
                "memory job {job_id} cannot be claimed from {:?}",
                job.state
            ));
        }
        if job
            .retry_at_epoch_ms
            .is_some_and(|retry_at| retry_at > now_epoch_ms)
        {
            return Err(format!(
                "memory job {job_id} is waiting for its retry deadline"
            ));
        }
        job.state = MemoryJobState::Running;
        job.updated_at_epoch_ms = now_epoch_ms;
        job.worker_id = Some(worker_id);
        job.ownership_token = Some(ownership_token);
        job.lease_until_epoch_ms = Some(lease_until_epoch_ms);
        job.last_heartbeat_epoch_ms = Some(now_epoch_ms);
        job.retry_at_epoch_ms = None;
        job.attempt_count = job.attempt_count.saturating_add(1);
        job.last_error = None;
        Ok(MemoryRuntimeEvent::JobClaimed { job: job.clone() })
    }

    fn heartbeat(
        &mut self,
        job_id: &str,
        ownership_token: &str,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
    ) -> Result<MemoryRuntimeEvent, String> {
        let job = self.require_owned_running_job(job_id, ownership_token)?;
        if lease_until_epoch_ms <= now_epoch_ms {
            return Err("memory job heartbeat lease must expire in the future".to_string());
        }
        if job
            .last_heartbeat_epoch_ms
            .is_some_and(|last_heartbeat| now_epoch_ms < last_heartbeat)
        {
            return Err("memory job heartbeat time must be monotonic".to_string());
        }
        if job
            .lease_until_epoch_ms
            .is_some_and(|current_lease| lease_until_epoch_ms < current_lease)
        {
            return Err("memory job heartbeat cannot shorten its active lease".to_string());
        }
        job.updated_at_epoch_ms = now_epoch_ms;
        job.last_heartbeat_epoch_ms = Some(now_epoch_ms);
        job.lease_until_epoch_ms = Some(lease_until_epoch_ms);
        Ok(MemoryRuntimeEvent::JobHeartbeat { job: job.clone() })
    }

    #[allow(clippy::too_many_arguments)]
    fn schedule_retry(
        &mut self,
        job_id: &str,
        ownership_token: &str,
        now_epoch_ms: i64,
        retry_after_ms: Option<i64>,
        max_retry_count: u32,
        error: String,
    ) -> Result<MemoryRuntimeEvent, String> {
        if error.trim().is_empty() {
            return Err("memory retry error must not be empty".to_string());
        }
        let job = self.require_owned_running_job(job_id, ownership_token)?;
        job.updated_at_epoch_ms = now_epoch_ms;
        job.last_error = Some(error);
        job.worker_id = None;
        job.ownership_token = None;
        job.lease_until_epoch_ms = None;
        job.last_heartbeat_epoch_ms = None;
        if job.retry_count >= max_retry_count {
            job.state = MemoryJobState::Failed;
            job.retry_at_epoch_ms = None;
            return Ok(MemoryRuntimeEvent::JobFailed { job: job.clone() });
        }
        let delay = retry_after_ms
            .filter(|delay| *delay >= 0)
            .unwrap_or_else(|| retry_delay_ms(job.retry_count));
        job.retry_count = job.retry_count.saturating_add(1);
        job.state = MemoryJobState::RetryWaiting;
        job.retry_at_epoch_ms = Some(now_epoch_ms.saturating_add(delay));
        Ok(MemoryRuntimeEvent::JobRetryScheduled { job: job.clone() })
    }

    fn complete(
        &mut self,
        job_id: &str,
        ownership_token: &str,
        now_epoch_ms: i64,
        completion_watermark: String,
        output: MemoryJobOutput,
    ) -> Result<MemoryRuntimeEvent, String> {
        validate_identifier("memory completion watermark", &completion_watermark)?;
        let job = self.require_owned_running_job(job_id, ownership_token)?;
        if output.kind() != job.kind {
            return Err("memory job output kind does not match the claimed job".to_string());
        }
        if completion_watermark != job.input_watermark {
            return Err("memory completion watermark does not attest the frozen input".to_string());
        }
        validate_memory_job_output(job, &output)?;
        job.state = MemoryJobState::Completed;
        job.updated_at_epoch_ms = now_epoch_ms;
        job.completion_watermark = Some(completion_watermark);
        job.output = Some(output);
        job.worker_id = None;
        job.ownership_token = None;
        job.lease_until_epoch_ms = None;
        job.last_heartbeat_epoch_ms = None;
        job.retry_at_epoch_ms = None;
        job.last_error = None;
        Ok(MemoryRuntimeEvent::JobCompleted { job: job.clone() })
    }

    fn cancel(
        &mut self,
        job_id: &str,
        now_epoch_ms: i64,
        reason: Option<String>,
    ) -> Result<MemoryRuntimeEvent, String> {
        let job = self.require_job_mut(job_id)?;
        if job.state.is_terminal() {
            return Err(format!("memory job {job_id} is already terminal"));
        }
        job.state = MemoryJobState::Cancelled;
        job.updated_at_epoch_ms = now_epoch_ms;
        job.last_error = reason.filter(|value| !value.trim().is_empty());
        job.worker_id = None;
        job.ownership_token = None;
        job.lease_until_epoch_ms = None;
        job.last_heartbeat_epoch_ms = None;
        job.retry_at_epoch_ms = None;
        Ok(MemoryRuntimeEvent::JobCancelled { job: job.clone() })
    }

    fn sweep_expired(
        &mut self,
        now_epoch_ms: i64,
        max_retry_count: u32,
    ) -> Vec<MemoryRuntimeEvent> {
        let expired = self
            .job_order
            .iter()
            .filter_map(|job_id| {
                self.jobs.get(job_id).and_then(|job| {
                    (job.state == MemoryJobState::Running
                        && job
                            .lease_until_epoch_ms
                            .is_some_and(|lease_until| lease_until <= now_epoch_ms))
                    .then(|| job_id.clone())
                })
            })
            .collect::<Vec<_>>();
        let mut events = Vec::with_capacity(expired.len());
        for job_id in expired {
            let Some(job) = self.jobs.get_mut(&job_id) else {
                continue;
            };
            job.updated_at_epoch_ms = now_epoch_ms;
            job.last_error = Some("worker lease expired".to_string());
            job.worker_id = None;
            job.ownership_token = None;
            job.lease_until_epoch_ms = None;
            job.last_heartbeat_epoch_ms = None;
            if job.retry_count >= max_retry_count {
                job.state = MemoryJobState::Failed;
                job.retry_at_epoch_ms = None;
                events.push(MemoryRuntimeEvent::JobFailed { job: job.clone() });
            } else {
                let delay = retry_delay_ms(job.retry_count);
                job.retry_count = job.retry_count.saturating_add(1);
                job.state = MemoryJobState::RetryWaiting;
                job.retry_at_epoch_ms = Some(now_epoch_ms.saturating_add(delay));
                events.push(MemoryRuntimeEvent::JobRetryScheduled { job: job.clone() });
            }
        }
        events
    }

    fn record_usage(
        &mut self,
        entry: MemoryUsageLedgerEntry,
    ) -> Result<MemoryRuntimeEvent, String> {
        validate_identifier("memory usage id", &entry.usage_id)?;
        validate_identifier("memory usage memory id", &entry.memory_id)?;
        validate_identifier("memory usage session id", &entry.session_id)?;
        validate_identifier("memory usage turn id", &entry.turn_id)?;
        validate_identifier(
            "memory usage context payload digest",
            &entry.context_payload_digest,
        )?;
        if self
            .usage_ledger
            .iter()
            .any(|existing| existing.usage_id == entry.usage_id)
        {
            return Err(format!("memory usage {} already exists", entry.usage_id));
        }
        let stats = self.usage_stats.entry(entry.memory_id.clone()).or_default();
        stats.usage_count = stats.usage_count.saturating_add(1);
        stats.last_used_at_epoch_ms = Some(
            stats
                .last_used_at_epoch_ms
                .map(|current| current.max(entry.used_at_epoch_ms))
                .unwrap_or(entry.used_at_epoch_ms),
        );
        let stats = stats.clone();
        self.usage_ledger.push(entry.clone());
        Ok(MemoryRuntimeEvent::UsageRecorded { entry, stats })
    }

    fn ensure_new_job(&self, job_id: &str) -> Result<(), String> {
        if self.jobs.contains_key(job_id) {
            Err(format!("memory job {job_id} already exists"))
        } else {
            Ok(())
        }
    }

    fn require_job_mut(&mut self, job_id: &str) -> Result<&mut MemoryJobHandle, String> {
        self.jobs
            .get_mut(job_id)
            .ok_or_else(|| format!("unknown memory job {job_id}"))
    }

    fn require_owned_running_job(
        &mut self,
        job_id: &str,
        ownership_token: &str,
    ) -> Result<&mut MemoryJobHandle, String> {
        let job = self.require_job_mut(job_id)?;
        if job.state != MemoryJobState::Running {
            return Err(format!("memory job {job_id} is not running"));
        }
        if job.ownership_token.as_deref() != Some(ownership_token) {
            return Err(format!(
                "memory job {job_id} ownership token does not match"
            ));
        }
        Ok(job)
    }

    pub fn usage_records(&self) -> impl Iterator<Item = &MemoryUsageLedgerEntry> {
        self.usage_ledger.iter()
    }

    pub fn selected_usage_records<'a>(
        &'a self,
        selected: &'a [MemoryUsage],
    ) -> impl Iterator<Item = &'a MemoryUsageLedgerEntry> {
        let selected_ids = selected
            .iter()
            .map(|usage| usage.record_id.as_str())
            .collect::<BTreeSet<_>>();
        self.usage_ledger
            .iter()
            .filter(move |entry| selected_ids.contains(entry.memory_id.as_str()))
    }
}

fn memory_runtime_command_digest(command: &MemoryRuntimeCommand) -> Result<String, String> {
    use sha2::{Digest, Sha256};

    let encoded = serde_json::to_vec(command)
        .map_err(|error| format!("encode memory runtime operation identity failed: {error}"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

fn validate_memory_job_output(
    job: &MemoryJobHandle,
    output: &MemoryJobOutput,
) -> Result<(), String> {
    match output {
        MemoryJobOutput::Extraction(output) => {
            validate_identifier("memory extraction rollout summary", &output.rollout_summary)?;
            validate_identifier("memory extraction output digest", &output.output_digest)?;
            let mut record_ids = BTreeSet::new();
            let mut semantic_identities = BTreeSet::new();
            let mut citation_ids = BTreeSet::new();
            for extracted in &output.memories {
                extracted
                    .record
                    .validate_automatic_long_term_candidate(&extracted.quality)?;
                if !record_ids.insert(extracted.record.id.as_str()) {
                    return Err(
                        "memory extraction output contains duplicate record ids".to_string()
                    );
                }
                if !semantic_identities.insert(memory_deduplication_identity(&extracted.record)?) {
                    return Err(
                        "memory extraction output contains duplicate semantic identities"
                            .to_string(),
                    );
                }
                if extracted.citations.is_empty() {
                    return Err(format!(
                        "automatic memory {} requires source citations",
                        extracted.record.id
                    ));
                }
                let source = matching_frozen_source(&extracted.record, &job.sources)?;
                for citation in &extracted.citations {
                    validate_citation(citation, &extracted.record.id, source)?;
                    if !citation_ids.insert(citation.citation_id.as_str()) {
                        return Err(
                            "memory extraction output contains duplicate citation ids".to_string()
                        );
                    }
                }
            }
            Ok(())
        }
        MemoryJobOutput::Consolidation(output) => {
            validate_identifier("memory consolidation summary", &output.summary)?;
            validate_identifier("memory consolidation output digest", &output.output_digest)?;
            let selected = job
                .selected_memory_ids
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            let superseded = output
                .superseded_memory_ids
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            if superseded.len() != output.superseded_memory_ids.len()
                || !superseded.is_subset(&selected)
            {
                return Err(
                    "memory consolidation may only supersede unique selected records".to_string(),
                );
            }

            let mut output_ids = BTreeSet::new();
            let mut semantic_identities = BTreeSet::new();
            let mut record_supersedes = BTreeSet::new();
            for record in &output.records {
                let quality = output.quality_by_memory_id.get(&record.id).ok_or_else(|| {
                    format!(
                        "memory consolidation record {} has no quality assessment",
                        record.id
                    )
                })?;
                record.validate_automatic_long_term_candidate(quality)?;
                if !output_ids.insert(record.id.as_str()) {
                    return Err(
                        "memory consolidation output contains duplicate record ids".to_string()
                    );
                }
                if !semantic_identities.insert(memory_deduplication_identity(record)?) {
                    return Err(
                        "memory consolidation output contains duplicate semantic identities"
                            .to_string(),
                    );
                }
                if record
                    .supersedes
                    .iter()
                    .any(|id| !selected.contains(id.as_str()))
                {
                    return Err(format!(
                        "memory consolidation record {} supersedes an unselected record",
                        record.id
                    ));
                }
                record_supersedes.extend(record.supersedes.iter().map(String::as_str));
            }
            if record_supersedes != superseded {
                return Err(
                    "memory consolidation superseded ids must exactly match output record lineage"
                        .to_string(),
                );
            }
            if output.quality_by_memory_id.len() != output_ids.len()
                || output
                    .quality_by_memory_id
                    .keys()
                    .any(|id| !output_ids.contains(id.as_str()))
            {
                return Err(
                    "memory consolidation quality assessments must exactly match output records"
                        .to_string(),
                );
            }
            if output
                .citations
                .iter()
                .any(|citation| !output_ids.contains(citation.memory_id.as_str()))
            {
                return Err(
                    "memory consolidation citation references an unknown output record".to_string(),
                );
            }
            Ok(())
        }
    }
}

/// Matches the durable Store deduplication boundary without assigning a new
/// identity. Worker output is checked before Core records its IDs, because the
/// atomic checkpoint writer must preserve those IDs and cannot safely merge
/// two accepted records after the transition.
fn memory_deduplication_identity(record: &MemoryRecord) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&(
        &record.owner,
        &record.namespace,
        record.scope,
        record.kind,
        &record.content,
    ))
    .map_err(|error| format!("encode automatic memory identity failed: {error}"))
}

fn matching_frozen_source<'a>(
    record: &MemoryRecord,
    sources: &'a [MemorySourceSnapshot],
) -> Result<&'a MemorySourceSnapshot, String> {
    let session_id = record.provenance.session_id.as_deref().ok_or_else(|| {
        format!(
            "automatic memory {} must identify its source session",
            record.id
        )
    })?;
    if record.provenance.item_ids.is_empty() {
        return Err(format!(
            "automatic memory {} must identify source items",
            record.id
        ));
    }
    sources
        .iter()
        .find(|source| {
            source.session_id == session_id
                && record
                    .provenance
                    .turn_id
                    .as_ref()
                    .is_none_or(|turn_id| source.turn_id.as_ref() == Some(turn_id))
                && record
                    .provenance
                    .item_ids
                    .iter()
                    .all(|item_id| source.item_ids.contains(item_id))
        })
        .ok_or_else(|| {
            format!(
                "automatic memory {} provenance is outside the frozen extraction input",
                record.id
            )
        })
}

fn validate_citation(
    citation: &MemoryCitation,
    memory_id: &str,
    source: &MemorySourceSnapshot,
) -> Result<(), String> {
    validate_identifier("memory citation id", &citation.citation_id)?;
    validate_identifier("memory citation URI", &citation.uri)?;
    if citation.memory_id != memory_id {
        return Err("memory citation references a different record".to_string());
    }
    if citation
        .session_id
        .as_deref()
        .is_some_and(|id| id != source.session_id)
        || citation
            .turn_id
            .as_ref()
            .is_some_and(|id| source.turn_id.as_ref() != Some(id))
        || citation
            .item_ids
            .iter()
            .any(|item_id| !source.item_ids.contains(item_id))
    {
        return Err("memory citation is outside the frozen extraction input".to_string());
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(())
    }
}

fn retry_delay_ms(retry_count: u32) -> i64 {
    let exponent = retry_count.min(8);
    DEFAULT_RETRY_DELAY_MS
        .saturating_mul(1_i64 << exponent)
        .min(MAX_RETRY_DELAY_MS)
}
