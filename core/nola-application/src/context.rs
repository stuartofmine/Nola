// SPDX-License-Identifier: Apache-2.0
//! Application-owned model-context aggregate.
//!
//! A checkpoint describes the exact bounded working set used by a model
//! attempt. Components preserve source authorization, freshness, content
//! location, and provenance without conflating context with transcript or
//! long-term memory.

use nola_store_port::context::{self as repository, ContextRepository};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextComponentKind {
    ConversationSummary,
    TranscriptTail,
    DataSource,
    Memory,
    SkillResource,
    UserAttachment,
    SystemInstruction,
}

impl ContextComponentKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ConversationSummary => "conversation_summary",
            Self::TranscriptTail => "transcript_tail",
            Self::DataSource => "data_source",
            Self::Memory => "memory",
            Self::SkillResource => "skill_resource",
            Self::UserAttachment => "user_attachment",
            Self::SystemInstruction => "system_instruction",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "conversation_summary" => Ok(Self::ConversationSummary),
            "transcript_tail" => Ok(Self::TranscriptTail),
            "data_source" => Ok(Self::DataSource),
            "memory" => Ok(Self::Memory),
            "skill_resource" => Ok(Self::SkillResource),
            "user_attachment" => Ok(Self::UserAttachment),
            "system_instruction" => Ok(Self::SystemInstruction),
            _ => Err(format!("unknown context component kind: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextAuthorization {
    NotDetermined,
    Authorized,
    Denied,
    Restricted,
    Unavailable,
    NotRequired,
}

impl ContextAuthorization {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotDetermined => "not_determined",
            Self::Authorized => "authorized",
            Self::Denied => "denied",
            Self::Restricted => "restricted",
            Self::Unavailable => "unavailable",
            Self::NotRequired => "not_required",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "not_determined" => Ok(Self::NotDetermined),
            "authorized" => Ok(Self::Authorized),
            "denied" => Ok(Self::Denied),
            "restricted" => Ok(Self::Restricted),
            "unavailable" => Ok(Self::Unavailable),
            "not_required" => Ok(Self::NotRequired),
            _ => Err(format!("unknown context authorization: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextFreshness {
    Fresh,
    Stale,
    Unknown,
}

impl ContextFreshness {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "fresh" => Ok(Self::Fresh),
            "stale" => Ok(Self::Stale),
            "unknown" => Ok(Self::Unknown),
            _ => Err(format!("unknown context freshness: {value}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextContentReference {
    InlinePreview { text: String, truncated: bool },
    Artifact { artifact_id: String },
    Blob { blob_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextProvenance {
    pub provider_id: String,
    pub collection_method: String,
    pub source_revision: Option<String>,
    pub disclosed_to_model: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextTokenUsage {
    pub estimated: i64,
    pub measured: Option<i64>,
    pub window: i64,
    pub prompt_limit: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCheckpoint {
    pub id: String,
    pub conversation_id: String,
    pub run_id: Option<String>,
    pub model_profile_id: String,
    pub policy_version: String,
    pub covered_through_message_id: Option<String>,
    pub covered_through_sequence: Option<i64>,
    pub summary_part_id: Option<String>,
    pub retained_tail_start_message_id: Option<String>,
    pub token_usage: ContextTokenUsage,
    pub payload_digest: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextComponent {
    pub id: String,
    pub checkpoint_id: String,
    pub kind: ContextComponentKind,
    pub source_id: Option<String>,
    pub source_uri: Option<String>,
    pub title: Option<String>,
    pub mime_type: Option<String>,
    pub authorization: ContextAuthorization,
    pub observed_at_ms: Option<i64>,
    pub maximum_age_minutes: Option<i64>,
    pub freshness: ContextFreshness,
    pub content_reference: ContextContentReference,
    pub content_digest: String,
    pub estimated_tokens: i64,
    pub provenance: ContextProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitContextCheckpoint {
    pub checkpoint: ContextCheckpoint,
    pub components: Vec<ContextComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextAggregate {
    pub checkpoint: ContextCheckpoint,
    pub components: Vec<ContextComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextCommand {
    PrepareMaintenance { source: repository::ContextCompactionSource, input: nola_agent_engine::context_engine::ContextPrepareInput },
    CommitMaintenance { input: ContextMaintenanceCommit },
    ReplaceSummary { source: repository::ContextCompactionSource, summary: String, committed_at_ms: i64 },
    PrepareCompaction { conversation_id: String },
    CommitCheckpoint { input: CommitContextCheckpoint },
    CountCheckpoints { conversation_id: String },
    LoadCheckpoint { checkpoint_id: String },
}

impl ContextCommand {
    pub(crate) fn conversation_id(&self) -> Option<&str> {
        match self {
            Self::PrepareMaintenance { source, .. } => Some(&source.conversation_id),
            Self::CommitMaintenance { input } => Some(&input.source.conversation_id),
            Self::ReplaceSummary { source, .. } => Some(&source.conversation_id),
            Self::PrepareCompaction { conversation_id } => Some(conversation_id),
            Self::CommitCheckpoint { input } => Some(&input.checkpoint.conversation_id),
            Self::CountCheckpoints { conversation_id } => Some(conversation_id),
            Self::LoadCheckpoint { .. } => None,
        }
    }
}

pub(crate) fn execute_context_command<R: ContextRepository + ?Sized>(
    store: &mut R,
    command: ContextCommand,
) -> Result<Value, String> {
    match command {
        ContextCommand::PrepareMaintenance { source, input } => {
            if input.summary_result.is_some() { return Err("maintenance preparation cannot contain a candidate".into()); }
            if store.prepare_context_compaction(&source.conversation_id).map_err(|e| e.to_string())? != source {
                return Err("context changed while collecting maintenance input".into());
            }
            let output = nola_agent_engine::context_engine::prepare_context(&input);
            if output.action == nola_agent_engine::context_engine::ContextPrepareAction::CannotFit {
                return Err(format!("context maintenance cannot fit: {}", output.reason));
            }
            let instructions = output.archive_request.as_ref().map(|archive|
                nola_kernel::compaction_maintenance::compaction_instructions(archive.target_summary_tokens));
            Ok(serde_json::json!({"source": source, "input": input, "output": output, "instructions": instructions}))
        }
        ContextCommand::CommitMaintenance { input } => commit_context_maintenance(store, input),
        ContextCommand::ReplaceSummary { source, summary, committed_at_ms } => {
            store.commit_context_compaction(&repository::CommitContextCompactionInput {
                validated_source: source.clone(), source, summary, entry: Value::Null, committed_at_ms,
            }).map_err(|e| e.to_string())?;
            Ok(Value::Bool(true))
        }
        ContextCommand::PrepareCompaction { conversation_id } => serde_json::to_value(
            store.prepare_context_compaction(&conversation_id).map_err(|e| e.to_string())?
        ).map_err(|e| e.to_string()),
        ContextCommand::CommitCheckpoint { input } => {
            let store_input = to_store_commit(input)?;
            store
                .commit_context_checkpoint(&store_input)
                .map_err(|error| error.to_string())?;
            let aggregate = store
                .load_context_checkpoint(&store_input.checkpoint.id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "committed context checkpoint is missing".to_string())?;
            serde_json::to_value(from_store_aggregate(aggregate)?)
                .map_err(|error| error.to_string())
        }
        ContextCommand::CountCheckpoints { conversation_id } => {
            let count = store
                .count_context_checkpoints(&conversation_id)
                .map_err(|error| error.to_string())?;
            serde_json::to_value(count).map_err(|error| error.to_string())
        }
        ContextCommand::LoadCheckpoint { checkpoint_id } => {
            let aggregate = store
                .load_context_checkpoint(&checkpoint_id)
                .map_err(|error| error.to_string())?
                .map(from_store_aggregate)
                .transpose()?;
            serde_json::to_value(aggregate).map_err(|error| error.to_string())
        }
    }
}

fn to_store_commit(
    input: CommitContextCheckpoint,
) -> Result<repository::CommitContextCheckpointInput, String> {
    let checkpoint = input.checkpoint;
    let components = input
        .components
        .into_iter()
        .map(|component| {
            Ok(repository::ContextComponentInput {
                id: component.id,
                checkpoint_id: component.checkpoint_id,
                kind: component.kind.as_str().to_string(),
                source_id: component.source_id,
                source_uri: component.source_uri,
                title: component.title,
                mime_type: component.mime_type,
                authorization: component.authorization.as_str().to_string(),
                observed_at_ms: component.observed_at_ms,
                maximum_age_minutes: component.maximum_age_minutes,
                freshness: component.freshness.as_str().to_string(),
                content_reference_json: serde_json::to_string(&component.content_reference)
                    .map_err(|error| format!("encode context content reference failed: {error}"))?,
                content_digest: component.content_digest,
                estimated_tokens: component.estimated_tokens,
                provenance_json: serde_json::to_string(&component.provenance)
                    .map_err(|error| format!("encode context provenance failed: {error}"))?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(repository::CommitContextCheckpointInput {
        checkpoint: repository::ContextCheckpointInput {
            id: checkpoint.id,
            conversation_id: checkpoint.conversation_id,
            run_id: checkpoint.run_id,
            model_profile_id: checkpoint.model_profile_id,
            policy_version: checkpoint.policy_version,
            covered_through_message_id: checkpoint.covered_through_message_id,
            covered_through_sequence: checkpoint.covered_through_sequence,
            summary_part_id: checkpoint.summary_part_id,
            retained_tail_start_message_id: checkpoint.retained_tail_start_message_id,
            estimated_tokens: checkpoint.token_usage.estimated,
            measured_tokens: checkpoint.token_usage.measured,
            context_window_tokens: checkpoint.token_usage.window,
            prompt_limit_tokens: checkpoint.token_usage.prompt_limit,
            payload_digest: checkpoint.payload_digest,
            created_at_ms: checkpoint.created_at_ms,
        },
        components,
    })
}

fn from_store_aggregate(
    aggregate: repository::ContextAggregateRecord,
) -> Result<ContextAggregate, String> {
    let checkpoint = aggregate.checkpoint;
    Ok(ContextAggregate {
        checkpoint: ContextCheckpoint {
            id: checkpoint.id,
            conversation_id: checkpoint.conversation_id,
            run_id: checkpoint.run_id,
            model_profile_id: checkpoint.model_profile_id,
            policy_version: checkpoint.policy_version,
            covered_through_message_id: checkpoint.covered_through_message_id,
            covered_through_sequence: checkpoint.covered_through_sequence,
            summary_part_id: checkpoint.summary_part_id,
            retained_tail_start_message_id: checkpoint.retained_tail_start_message_id,
            token_usage: ContextTokenUsage {
                estimated: checkpoint.estimated_tokens,
                measured: checkpoint.measured_tokens,
                window: checkpoint.context_window_tokens,
                prompt_limit: checkpoint.prompt_limit_tokens,
            },
            payload_digest: checkpoint.payload_digest,
            created_at_ms: checkpoint.created_at_ms,
        },
        components: aggregate
            .components
            .into_iter()
            .map(|component| {
                Ok(ContextComponent {
                    id: component.id,
                    checkpoint_id: component.checkpoint_id,
                    kind: ContextComponentKind::parse(&component.kind)?,
                    source_id: component.source_id,
                    source_uri: component.source_uri,
                    title: component.title,
                    mime_type: component.mime_type,
                    authorization: ContextAuthorization::parse(&component.authorization)?,
                    observed_at_ms: component.observed_at_ms,
                    maximum_age_minutes: component.maximum_age_minutes,
                    freshness: ContextFreshness::parse(&component.freshness)?,
                    content_reference: serde_json::from_str(&component.content_reference_json)
                        .map_err(|error| {
                            format!("decode context content reference failed: {error}")
                        })?,
                    content_digest: component.content_digest,
                    estimated_tokens: component.estimated_tokens,
                    provenance: serde_json::from_str(&component.provenance_json)
                        .map_err(|error| format!("decode context provenance failed: {error}"))?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FixtureRepository(Option<repository::ContextAggregateRecord>);

    impl ContextRepository for FixtureRepository {
        type Error = &'static str;
        fn prepare_context_compaction(&self, _: &str) -> Result<repository::ContextCompactionSource, Self::Error> { Err("not a compaction fixture") }
        fn commit_context_compaction(&mut self, _: &repository::CommitContextCompactionInput) -> Result<(), Self::Error> { Err("not a compaction fixture") }
        fn commit_context_checkpoint(
            &mut self,
            input: &repository::CommitContextCheckpointInput,
        ) -> Result<(), Self::Error> {
            self.0 = Some(repository::ContextAggregateRecord {
                checkpoint: input.checkpoint.clone(),
                components: input.components.clone(),
            });
            Ok(())
        }
        fn load_context_checkpoint(
            &self,
            id: &str,
        ) -> Result<Option<repository::ContextAggregateRecord>, Self::Error> {
            Ok(self
                .0
                .as_ref()
                .filter(|value| value.checkpoint.id == id)
                .cloned())
        }
        fn count_context_checkpoints(&self, conversation: &str) -> Result<u64, Self::Error> {
            Ok(self
                .0
                .iter()
                .filter(|value| value.checkpoint.conversation_id == conversation)
                .count() as u64)
        }
    }

    #[test]
    fn context_commands_round_trip_through_non_sqlite_repository() {
        let input = CommitContextCheckpoint {
            checkpoint: ContextCheckpoint {
                id: "cp".into(),
                conversation_id: "c".into(),
                run_id: None,
                model_profile_id: "model".into(),
                policy_version: "v2".into(),
                covered_through_message_id: None,
                covered_through_sequence: Some(5),
                summary_part_id: None,
                retained_tail_start_message_id: None,
                token_usage: ContextTokenUsage {
                    estimated: 42,
                    measured: Some(40),
                    window: 64000,
                    prompt_limit: 60000,
                },
                payload_digest: "fixture".into(),
                created_at_ms: 7,
            },
            components: vec![],
        };
        let mut store = FixtureRepository::default();
        let port: &mut dyn ContextRepository<Error = &'static str> = &mut store;
        let saved = execute_context_command(
            port,
            ContextCommand::CommitCheckpoint {
                input: input.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            saved["checkpoint"],
            serde_json::to_value(input.checkpoint).unwrap()
        );
        assert_eq!(
            execute_context_command(
                port,
                ContextCommand::LoadCheckpoint {
                    checkpoint_id: "cp".into()
                }
            )
            .unwrap(),
            saved
        );
        assert_eq!(
            execute_context_command(
                port,
                ContextCommand::CountCheckpoints {
                    conversation_id: "other".into()
                }
            )
            .unwrap(),
            serde_json::json!(0)
        );
        assert_eq!(
            execute_context_command(
                port,
                ContextCommand::LoadCheckpoint {
                    checkpoint_id: "missing".into()
                }
            )
            .unwrap(),
            Value::Null
        );
    }

    #[test]
    fn context_contract_keeps_source_state_and_content_reference_typed() {
        let component = ContextComponent {
            id: "component-1".into(),
            checkpoint_id: "checkpoint-1".into(),
            kind: ContextComponentKind::DataSource,
            source_id: Some("health".into()),
            source_uri: Some("healthkit://daily-summary".into()),
            title: Some("Daily health".into()),
            mime_type: Some("application/json".into()),
            authorization: ContextAuthorization::Authorized,
            observed_at_ms: Some(1_000),
            maximum_age_minutes: Some(60),
            freshness: ContextFreshness::Fresh,
            content_reference: ContextContentReference::InlinePreview {
                text: "{\"steps\":1200}".into(),
                truncated: false,
            },
            content_digest: "digest".into(),
            estimated_tokens: 8,
            provenance: ContextProvenance {
                provider_id: "mac-health".into(),
                collection_method: "healthkit_query".into(),
                source_revision: None,
                disclosed_to_model: true,
            },
        };
        let value = serde_json::to_value(component).unwrap();
        assert_eq!(value["authorization"], "authorized");
        assert_eq!(value["freshness"], "fresh");
        assert_eq!(value["content_reference"]["type"], "inline_preview");
    }
}

/// A source-bound maintenance transaction. Clients gather platform components and
/// execute model requests; Application alone validates and commits the result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextMaintenanceCommit {
    pub source: repository::ContextCompactionSource,
    pub validated_source: repository::ContextCompactionSource,
    pub original_input: nola_agent_engine::context_engine::ContextPrepareInput,
    pub current_input: nola_agent_engine::context_engine::ContextPrepareInput,
    pub summary: String,
    pub committed_at_ms: i64,
}

fn commit_context_maintenance<R: ContextRepository + ?Sized>(store: &mut R, input: ContextMaintenanceCommit) -> Result<Value, String> {
    use nola_agent_engine::context_engine::{prepare_context, ContextPrepareAction, ContextPrepareMode, ContextSummaryResult};
    if input.original_input.summary_result.is_some() || input.current_input.summary_result.is_some() {
        return Err("maintenance inputs cannot carry an already applied candidate".into());
    }
    let prepared = prepare_context(&input.original_input);
    let archive = prepared.archive_request.ok_or("maintenance no longer requires a summary")?;
    nola_kernel::compaction_plan::validate_summary(&input.summary, archive.target_summary_tokens).map_err(str::to_owned)?;
    let mut candidate = input.original_input;
    candidate.summary_result = Some(ContextSummaryResult {
        archive_id: archive.archive_id.clone(), archive_digest: archive.archive_digest.clone(), content: input.summary.clone(),
    });
    let committed = prepare_context(&candidate);
    if committed.action != ContextPrepareAction::Ready { return Err(format!("maintenance candidate rejected: {}", committed.reason)); }
    let archived = archive.message_ids.iter().collect::<std::collections::HashSet<_>>();
    let mut current = input.current_input;
    current.budget = candidate.budget.clone();
    current.messages.retain(|message| message.get("id").and_then(Value::as_str).is_none_or(|id| !archived.iter().any(|archived| archived.as_str() == id)));
    current.existing_summary = Some(input.summary.clone());
    current.mode = ContextPrepareMode::Normal;
    let revalidated = prepare_context(&current);
    if revalidated.action != ContextPrepareAction::Ready { return Err(format!("context changed during maintenance: {}", revalidated.reason)); }
    let created_at = chrono::DateTime::from_timestamp_millis(input.committed_at_ms).ok_or("invalid maintenance timestamp")?.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let entry = serde_json::json!({"id": uuid::Uuid::new_v4().to_string(), "createdAt": created_at,
        "summary": input.summary, "rawExcerpt": "", "archivedMessageIDs": archive.message_ids,
        "archiveID": archive.archive_id, "archiveDigest": archive.archive_digest, "payloadDigest": committed.payload_digest});
    store.commit_context_compaction(&repository::CommitContextCompactionInput {
        source: input.source, validated_source: input.validated_source, summary: input.summary,
        entry, committed_at_ms: input.committed_at_ms,
    }).map_err(|e| e.to_string())?;
    serde_json::to_value(revalidated).map_err(|e| e.to_string())
}

#[cfg(test)]
mod maintenance_tests {
    use super::*;
    use nola_agent_engine::context_engine::{ContextPrepareInput, ContextPrepareMode, ContextBudgetSpec};
    struct Store { source: repository::ContextCompactionSource, committed: Option<repository::CommitContextCompactionInput> }
    impl ContextRepository for Store {
        type Error = &'static str;
        fn prepare_context_compaction(&self, _: &str) -> Result<repository::ContextCompactionSource, Self::Error> { Ok(self.source.clone()) }
        fn commit_context_compaction(&mut self, input: &repository::CommitContextCompactionInput) -> Result<(), Self::Error> {
            if input.validated_source != self.source || input.source.context_fingerprint != self.source.context_fingerprint { return Err("changed source"); }
            self.committed = Some(input.clone()); Ok(())
        }
        fn commit_context_checkpoint(&mut self, _: &repository::CommitContextCheckpointInput) -> Result<(), Self::Error> { unreachable!() }
        fn load_context_checkpoint(&self, _: &str) -> Result<Option<repository::ContextAggregateRecord>, Self::Error> { unreachable!() }
        fn count_context_checkpoints(&self, _: &str) -> Result<u64, Self::Error> { unreachable!() }
    }
    fn fixture() -> (Store, ContextPrepareInput) {
        (Store { source: repository::ContextCompactionSource { conversation_id: "c".into(), message_count: 3,
            source_fingerprint: "source".into(), context_fingerprint: "context".into() }, committed: None },
        ContextPrepareInput { mode: ContextPrepareMode::ForceSummary,
            messages: vec![serde_json::json!({"id":"old-user","role":"user","content":"evidence ".repeat(1200)}),
                serde_json::json!({"id":"old-answer","role":"assistant","content":"observed ".repeat(1200)}),
                serde_json::json!({"id":"latest","role":"user","content":"Continue the work"})],
            budget: ContextBudgetSpec { prompt_token_limit: Some(8000), token_threshold: Some(1000),
                keep_recent_messages: 1, min_messages_before_compaction: 0, ..Default::default() }, ..Default::default() })
    }
    fn candidate(store: &Store, input: ContextPrepareInput) -> ContextMaintenanceCommit {
        ContextMaintenanceCommit { source: store.source.clone(), validated_source: store.source.clone(),
            original_input: input.clone(), current_input: input, summary: "Observed prior work; continue the latest request.".into(), committed_at_ms: 1778000000000 }
    }
    #[test] fn maintenance_commits_summary_and_lineage_together() {
        let (mut store, input) = fixture();
        let commit = candidate(&store, input);
        let output = commit_context_maintenance(&mut store, commit);
        assert!(output.is_ok(), "{output:?}");
        let committed = store.committed.unwrap();
        assert_eq!(committed.entry["summary"], committed.summary);
        assert!(committed.entry["archivedMessageIDs"].as_array().unwrap().iter().all(|id| id != "latest"));
    }
    #[test] fn changed_source_and_tail_pressure_leave_original_context_untouched() {
        let (mut store, input) = fixture();
        let mut commit = candidate(&store, input.clone());
        commit.validated_source.source_fingerprint = "different".into();
        assert!(commit_context_maintenance(&mut store, commit).is_err());
        assert!(store.committed.is_none());
        let mut commit = candidate(&store, input);
        commit.current_input.messages.push(serde_json::json!({"id":"new","role":"user","content":"x".repeat(80000)}));
        assert!(commit_context_maintenance(&mut store, commit).is_err());
        assert!(store.committed.is_none());
    }
    #[test] fn preparation_rejects_a_snapshot_changed_during_platform_collection() {
        let (mut store, input) = fixture(); let mut source = store.source.clone(); source.context_fingerprint = "old".into();
        assert!(execute_context_command(&mut store, ContextCommand::PrepareMaintenance { source, input }).is_err());
        assert!(store.committed.is_none());
    }
}
