// SPDX-License-Identifier: Apache-2.0
//! Shared product use cases and transaction coordination.
//!
//! Platform adapters call this layer instead of assembling Store mutations,
//! Core checkpoints and Host acknowledgements independently.

use nola_agent_engine::AgentReducerPort;
mod activity;
pub use activity::{ActivityQuery,ActivityCursor};
mod today_facts;
pub use today_facts::{load_today_facts, TodayFactsQuery, TodayFactsSnapshot};
mod task_cancellation_control;
mod application_command;
pub mod automation_occurrence;
#[cfg(test)]
#[path = "../tests/support/model_identity.rs"]
mod model_fixture;
mod host_execution_state;
mod permission_policy;
mod user_input_wait;
#[cfg(test)]
mod user_input_wait_tests;
#[cfg(test)]
mod execution_evidence_tests;
pub use user_input_wait::{ConversationWaitResponse, RespondToConversationWait, WaitConflictReason, WaitResumePolicy};
pub use nola_agent_engine::agent::{HostExecutionState, HostPendingEffect};
mod agent_task_admission;
pub use nola_domain::agent_task_admission::{AgentTaskAdmission, RuntimeAgentTaskAuthority};
mod task_ownership;
pub use task_ownership::{ClaimAgentTaskFair, RecoverExpiredAgentTask};
mod canonical_part_projection;
mod public_part_semantics;
mod stored_public_part;
use canonical_part_projection::{
    part_upsert, CanonicalPartProjection, CanonicalPartProjectionPlanner,
};
mod event_diagnostics;
mod execution_trace_journal;
#[cfg(test)]
mod terminal_event_provenance_tests;
mod initial_run_events;
use event_diagnostics::event_payload_for_persistence;

fn committed_event_payload_for_persistence(
    envelope: &CoreEventEnvelope<AgentEvent>,
) -> Result<Value, serde_json::Error> {
    let mut payload = match &envelope.event {
        AgentEvent::InputApplied { turn_id, applications, .. } => {
            serde_json::json!({"type":"input_applied","turn_id":turn_id,"applications":applications})
        }
        event => event_payload_for_persistence(event)?,
    };
    let requires_source_frontier = matches!(&envelope.event,
        AgentEvent::TurnCompleted { .. }
        | AgentEvent::TurnCancelled { .. }
        | AgentEvent::TurnFailed { .. }
        | AgentEvent::AssistantSegmentStarted { .. }
        | AgentEvent::AssistantEffectBound { .. }
        | AgentEvent::AssistantActivityBound { .. }
        | AgentEvent::InputAccepted { .. }
        | AgentEvent::InputApplied { .. }
        | AgentEvent::InputWithdrawn { .. }
        | AgentEvent::RequirementScopeChanged { .. }
        | AgentEvent::RequirementAssessed { .. }
        | AgentEvent::RunResultDecided { .. }
        | AgentEvent::RuntimeDecisionFactObserved { .. }
        | AgentEvent::ExecutionEvidenceObserved { .. }
        | AgentEvent::TaskUpdated { .. }
        | AgentEvent::UserInputWaitChanged { .. }
        | AgentEvent::PermissionSettled { .. });
    // The run-event sequence is a Store ordering, not the Core cursor. Preserve
    // the actual decision frontier even after the session starts another Turn.
    let is_recovery_accounting = matches!(&envelope.event,
        AgentEvent::EngineeringLoopUpdated { state, .. }
            if state.no_progress_recovery.reason().is_some());
    if requires_source_frontier || is_recovery_accounting {
        payload.as_object_mut().expect("Core event is a tagged object").insert(
            "core_envelope".into(),
            serde_json::to_value(envelope.source())?,
        );
    }
    Ok(payload)
}
#[cfg(test)]
use event_diagnostics::{bounded_event_json, bounded_event_text};
mod assistant_journal_changes;
mod assistant_journal_planner;
mod assistant_journal_publication;
mod assistant_journal_producer;
mod assistant_journal_read;
pub use assistant_journal_read::{AssistantJournalReadResult, AssistantJournalRecord,
    AssistantJournalSnapshotHandle, AssistantJournalTextRange, AssistantJournalPartPage,
    AssistantJournalPartChange, AssistantJournalPartChangePage,
    AssistantJournalManifestPage, AssistantJournalChunk, AssistantJournalReceiveResult, StageAssistantJournalContent};
mod assistant_publication_transaction;
mod assistant_owner;
mod assistant_synthetic_semantics;
mod assistant_turn;
mod assistant_turn_binding;
mod command_effect;
mod delegated_invocation;
mod delegated_permission;
mod effect_delivery;
#[cfg(test)]
mod delivery_witness_contract_tests;
mod host_replay;
#[cfg(test)]
mod event_preview_tests;
mod maintenance_turn;
mod model_attempt;
mod unknown_effect_recovery;
pub use unknown_effect_recovery::{ResolveUnknownEffectRecovery, UnknownEffectRecoveryChoice, UnknownEffectRecoveryReply};
pub use effect_delivery::{
    PrepareRunEffectRecovery, PreparedRunEffectRecovery,
    claim_committed_host_requests, heartbeat_effect_delivery,
    load_runtime_effect_work_schedule, ClaimCommittedHostRequests,
    HeartbeatEffectDelivery, LoadRuntimeEffectWorkSchedule, RuntimeEffectWorkReason,
    RuntimeEffectWorkRunHead, RuntimeEffectWorkSchedule,
};
pub use maintenance_turn::{
    begin_maintenance_turn, build_memory_worker_plan, query_claimable_memory_jobs,
    AcceptedMaintenanceTurn, BeginMaintenanceTurn, BuildMemoryWorkerPlan, QueryClaimableMemoryJobs,
};
mod content_upload;
pub use content_upload::MAXIMUM_UPLOAD_BYTES as MAXIMUM_CONTENT_UPLOAD_BYTES;
mod conversation_recall;
mod transcript_export;
pub use application_command::{
    GoalSelectedModel,
    execute_application_command, ApplicationCommand, ApplicationCommandExecutor,
    PermissionProjectionReadScope,
};
pub use assistant_turn::{begin_assistant_turn, promote_queued_assistant_turn, AcceptedAssistantTurn,
    BeginAssistantTurn, PromoteQueuedAssistantTurn, BeginQueuedUserTurn, begin_queued_user_turn};
pub use transcript_export::load_compaction_lineage;
pub mod cancellation_delivery;
mod cancellation_not_started;
mod command_receipt;
mod run_cancellation;
pub use command_receipt::{
    ApplicationCommandExecutionError, ApplicationCommandReceipt, ApplicationCommandReceiptOutcome,
    ExecuteReceivedCommand,
};
pub use run_cancellation::{
    request_run_cancellation, RequestRunCancellation, RunCancellationDisposition,
    RunCancellationReceipt,
};
mod projection_feed;
pub use projection_feed::{
    ApplicationConversationListProjectionEvent, ApplicationConversationListProjectionSnapshot,
    ApplicationConversationListProjectionSuffix, ApplicationConversationProjectionSnapshot,
    ApplicationProjectionEvent, ApplicationProjectionScope, ApplicationProjectionSuffix,
};
mod projection_journal;
mod runtime;
pub use runtime::{ApplicationRuntime, response_has_incremental_projection};
mod agent_command;
mod task_terminal;
mod task_guidance;
pub use task_guidance::TaskGuidanceApplyResult;
pub use task_terminal::TaskTerminalReconciliationEnqueued;
pub use nola_domain::agent_task_admission::TaskTerminalReconciliationApplied;
mod runtime_supervisor;
mod session_projection;
pub use agent_command::{AgentCommand, AgentCommandHead, AgentCommandKind, AgentCommandLease};
pub use session_projection::{
    load_session_projection, SessionProjection,
    SessionProjectionSource, SessionSkillProjection,
};
mod context;
pub use context::{
    CommitContextCheckpoint, ContextAggregate, ContextAuthorization, ContextCheckpoint,
    ContextCommand, ContextComponent, ContextComponentKind, ContextContentReference,
    ContextFreshness, ContextProvenance, ContextTokenUsage,
};
mod replica_sync;
use nola_protocol::RemoteSyncPage;
mod product_setting;
pub mod model_selection;
pub use product_setting::ProductSetting;
mod operation_result;
pub use operation_result::{
    ConversationBranchCreation, ConversationCreation, ConversationProjectionChange,
    TranscriptImportResult, TransitionCommit, TurnOpened,
};
mod trace;
pub use trace::{TraceRecord, TraceRun};
mod skill;
pub use skill::{Skill, SkillCommand, SkillInput, SkillSource};
mod skill_catalog;
pub use skill_catalog::{
    delete_user_skill, load_skill_catalog, register_skill, set_skill_enabled,
    SkillCatalogProjection,
};
mod mcp;
mod plugin;
pub use plugin::PluginCommand;
pub use mcp::{McpCommand, McpServer};
mod data_source;
pub use data_source::{
    ConversationDataSourceOverride, DataSource, DataSourceAuthorization,
    DataSourceAuthorizationState, DataSourceCommand, DataSourceConfiguration,
    DataSourceContentReference, DataSourceContextSelection, DataSourceDraft,
    DataSourceFreshnessPolicy, DataSourceKind, DataSourceLifecycle, DataSourceLocationKind,
    DataSourceReadResult, DataSourceReadState, DataSourceReadStatus, DataSourceSelectionReason,
    DataSourceUsage, DataSourceValidation, DataSourceValidationResult, DataSourceValidationStatus,
    UpdateDataSourceConfiguration,
};
mod document_source;
pub use document_source::{DocumentSource, DocumentSourceCommand, DocumentSourceSummary};
mod memory;
pub use memory::{
    MemoryCandidate, MemoryCommand, MemoryCommitIntent, MemoryRecallQuery, MemoryRecord,
    MemoryUsageStats,
};
mod model;
mod model_connection;
pub use model_connection::remote_profile_parameters;
mod model_generation;
mod run_generation_intent;
pub use model::{
    validate_model_reasoning, ChatOutputTokenField, ModelCommand, ModelExecutionCapabilities,
    ModelProfile, ModelRuntime, ModelRuntimePhase,
};
pub use model_generation::{
    model_reasoning_default_selection, resolve_model_reasoning, resolve_model_reasoning_request,
    resolve_model_reasoning_tier,
};
pub use run_generation_intent::{RunGenerationIntent, RunGenerationOverrides};
mod conversation_plan;
pub use conversation_plan::{
    ConversationPlan, ConversationPlanApprovalStatus, ConversationPlanCommand,
    ConversationPlanDiff, ConversationPlanRevision, ConversationPlanSopIdentity,
    ConversationPlanStep, ConversationPlanStepChange, ConversationPlanStepEdit,
    ConversationPlanStepStatus, ConversationPlanTerminationReason,
    ConversationPlanVerificationStatus,
};
pub mod owned_task_cancellation;
mod runtime_task;
pub use nola_store_port::runtime_task::{
    RuntimeTaskNotificationAcknowledgement, RuntimeTaskNotificationDelivery,
    CancelRuntimeTaskPermissionSuspension, ResumeRuntimeTaskAfterPermissionFair,
    RuntimeTaskAttempt, RuntimeTaskCommandReconciliation, RuntimeTaskLeaseWitness,
    RuntimeTaskRunBinding,
    RuntimeTaskTerminalReceipt, RuntimeTaskTerminalSource,
    RuntimeTaskGuidance,
    RuntimeTaskPermissionSuspension, RuntimeTaskPermissionResume, SuspendRuntimeTaskForCommandReconciliation,
    SuspendRuntimeTaskForPermission,
};
pub use runtime_task::{
    AcknowledgeRuntimeTaskResult,
    PermissionContinuationGrantSelector, RecoverRuntimeTaskPermissionContinuationFair,
    RuntimeTaskPermissionContinuationGrant,
    CreateRuntimeTask, RuntimeTask, RuntimeTaskCommand, RuntimeTaskEvent, RuntimeTaskPriority,
    RuntimeTaskProgressAck, RuntimeTaskFinalObservation, RuntimeTaskResourceBudget, RuntimeTaskResourceUsage,
    RuntimeTaskScheduling,
};
mod sop_runtime;
pub use sop_runtime::{SopRunCommand, SopRunProjection};
mod resume_snapshot;
pub use resume_snapshot::{
    acknowledge_resume_snapshot, load_resume_snapshot, AcknowledgeResumeSnapshot, ResumeAction,
    ResumeMailboxCursor, ResumeSnapshot, SupervisorReconciliationReport,
    SupervisorSideEffectDisposition, SupervisorSideEffectReconciliation, SupervisorTaskDisposition,
    SupervisorTaskReconciliation,
};
mod work_step_transaction;
pub use work_step_transaction::{
    WorkStepTransaction, WorkStepTransactionCommand,
    WorkStepTransactionPhase, WorkStepTransactionPreparation,
};
mod automation;
pub use automation::{
    AutomationCommand, AutomationDefinition, AutomationDefinitionCommand, AutomationRun,
    AutomationRunCommand, AutomationRunStatus,
};
pub use nola_domain::remote_submission::{PrepareRemoteSubmissionInput, RemoteSubmissionIdentity,
    RemoteSubmissionIntentView, SourceSubmissionReceipt, RegisteredSourceSubmission};
mod run_recovery_inspection;
pub use run_recovery_inspection::inspect_run_recovery;
pub mod cancellation_process_observation;
mod run_recovery_authority;
pub use run_recovery_authority::{validate_run_recovery_authority, RunRecoveryAuthority, RunRecoveryAuthorityReceipt};
mod run_invocation_recovery;
pub use run_invocation_recovery::{list_recoverable_runs, load_exact_run_invocation, ExactRunInvocation};
mod run_recovery;

use nola_domain::{MessagePartInput, PartMutation, TransitionMessageInput};
use nola_agent_engine::agent::host::{
    HostRequest, HostRequestEnvelope, HostResponse, HostResponseEnvelope,
};
#[cfg(test)]
use nola_agent_engine::agent::parts::ToolPartState;
use nola_agent_engine::agent::parts::{
    AgentPart, ArtifactPart, ArtifactReference, PartOwnership, PartUpdate,
};
use nola_agent_engine::agent::protocol::{
    AgentCoreConfig, AgentEvent, AgentItem, AgentRole, AgentSubmission,
    CallId, ItemId,
};
use nola_agent_engine::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_domain::core_event::{CoreEventEnvelope};
use nola_agent_engine::agent::{AgentCore, AgentItemOwnership, AgentThreadState};
use nola_agent_engine::agent_trace::AgentTraceEvent;
use nola_store_port::conversation_revert::SaveCheckpointInput;
use nola_store_port::effect_delivery::EffectDeliveryRepository;
use nola_store_port::memory::StoredMemoryUsageEntry;
use nola_store_port::run_effect::{
    CommitTransitionInput, HostEventInput, OutboxEffectInput, RunEffectRepository, RunEventInput,
};
use sha2::{Digest, Sha256};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::borrow::Borrow;

fn external_outbox_requests<R: RunEffectRepository + ?Sized>(
    store: &R,
    requests: &[HostRequestEnvelope],
) -> Result<Vec<OutboxEffectInput>, String> {
    requests
        .iter()
        .map(|request| {
            let effect_kind = match &request.payload {
                HostRequest::ModelStream { .. } => "model_stream",
                HostRequest::DelegatedAgent { .. } => "delegated_agent",
                HostRequest::ExecuteTool { .. } => "execute_tool",
                HostRequest::StartCodeCell { .. } => "start_code_cell",
                HostRequest::ResolveCodeTool { .. } => "resolve_code_tool",
                HostRequest::ObserveCodeCell { .. } => "observe_code_cell",
                HostRequest::ReadCodeCellHistory { .. } => "read_code_cell_history",
                HostRequest::PollCodeCell { .. } => "poll_code_cell",
                HostRequest::RequestPermission { .. } => "request_permission",
                HostRequest::ResolveDelegatedPermission { .. } => "resolve_delegated_permission",
                HostRequest::FetchContext { .. } => "fetch_context",
                HostRequest::ResolvePermissionResources { .. } => "resolve_permission_resources",
                HostRequest::Compact { .. } => "compact",
                HostRequest::Cancel { .. } => "cancel",
                HostRequest::CancelTask { .. } => "cancel_task",
                HostRequest::RefreshCapabilityCatalog { .. } => "refresh_capability_catalog",
                HostRequest::RunLifecycleHook { .. } => "run_lifecycle_hook",
            };
            // The outbox owns executable effect identity, not stream
            // progress. The checkpoint journal is the single durable source
            // of the accepted response cursor; Application composes both
            // projections when a Host asks for work.
            let identity = store.outbox_effect_identity(&request.effect_id.0)
                .map_err(|error| format!("read bounded outbox identity failed: {error}"))?;
            let write = if let Some(previous) = identity {
                if previous.effect_id != request.effect_id.0 || previous.effect_kind != effect_kind
                    || previous.status != "pending" || previous.binding.session_id != request.session_id
                    || previous.binding.turn_id != request.turn_id
                {
                    return Err("rebound Host request changed its immutable owner or kind".into());
                }
                let mut binding = nola_store_port::host_replay::CoreHostBindingOverlay::from_request(request);
                binding.resume_after_sequence = None;
                // A Core-generated continuation can still precede its ledger
                // target. Keep the already committed, exact occurrence proof;
                // never clear or replace it by an absent observation.
                if let nola_store_port::host_replay::CoreHostPayloadBinding::DelegatedPermissionDelivery { delivery } = &mut binding.payload_binding {
                    if delivery.is_none() {
                        if let nola_store_port::host_replay::CoreHostPayloadBinding::DelegatedPermissionDelivery { delivery: admitted } = &previous.binding.payload_binding {
                            *delivery = admitted.clone();
                        }
                    }
                }
                if let nola_store_port::host_replay::CoreHostPayloadBinding::CancellationNotStarted { proof } = &mut binding.payload_binding {
                    if proof.is_none() {
                        if let nola_store_port::host_replay::CoreHostPayloadBinding::CancellationNotStarted { proof: admitted } = &previous.binding.payload_binding {
                            *proof = admitted.clone();
                        }
                    }
                }
                nola_store_port::run_effect::OutboxEffectWrite::Rebind {
                    expected_request_id: previous.request_id,
                    expected_attempt_id: previous.binding.attempt_id,
                    binding,
                }
            } else {
                let mut durable_request = request.clone();
                durable_request.resume_after_sequence = None;
                nola_store_port::run_effect::OutboxEffectWrite::Declare {
                    payload_json: serde_json::to_string(&durable_request)
                        .map_err(|error| format!("encode new immutable Host effect failed: {error}"))?,
                }
            };
            Ok(OutboxEffectInput {
                effect_id: request.effect_id.0.clone(),
                model_attempt_id: request.attempt_id.clone(),
                delegated_invocation_id: request.delegated_invocation_id.clone(),
                request_id: request.request_id.clone(),
                effect_kind: effect_kind.into(),
                write,
            })
        })
        .collect()
}

fn durable_checkpoint_projection(
    mut checkpoint: CoreCheckpoint<AgentThreadState>,
) -> Result<CoreCheckpoint<AgentThreadState>, String> {
    checkpoint.validate_state_identity()?;
    // Application owns the durable checkpoint boundary. A Host may submit an
    // active recovery checkpoint containing the frozen capability catalog,
    // but terminal history must never retain optional Tool/Skill/SOP discovery
    // payloads. Re-project typed state here instead of trusting the Host's raw
    // JSON representation; concrete calls and results remain in the transcript.
    checkpoint.state = checkpoint.state.into_checkpoint_projection();
    Ok(checkpoint)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommittedHostRequests {
    pub effect_ids: Vec<String>,
    pub requests: Vec<HostRequestEnvelope>,
}

fn invocation_config_snapshot(config: &AgentCoreConfig) -> Result<(String, String), String> {
    if let Some(binding) = &config.turn_identity.host_invocation {
        binding.validate().map_err(str::to_string)?;
    }
    let json = serde_json::to_string(config)
        .map_err(|error| format!("encode Run invocation config failed: {error}"))?;
    let digest = format!("{:x}", Sha256::digest(json.as_bytes()));
    Ok((json, digest))
}

fn load_run_invocation_config<R: RunEffectRepository + ?Sized>(
    store: &R,
    run_id: &str,
    received_digest: &str,
) -> Result<AgentCoreConfig, String> {
    let stored = store
        .load_run_invocation_config(run_id)
        .map_err(|error| format!("load Run invocation config failed: {error}"))?
        .ok_or_else(|| format!("Run {run_id} has no local invocation config"))?;
    if stored.invocation_config_digest != received_digest {
        return Err(format!(
            "Run invocation config mismatch for {run_id}: expected={}, received={received_digest}",
            stored.invocation_config_digest
        ));
    }
    let stored_digest = format!(
        "{:x}",
        Sha256::digest(stored.invocation_config_json.as_bytes())
    );
    if stored_digest != stored.invocation_config_digest {
        return Err(format!(
            "Run invocation config integrity failure for {run_id}"
        ));
    }
    serde_json::from_str(&stored.invocation_config_json)
        .map_err(|error| format!("decode Run invocation config failed: {error}"))
}

fn validate_run_invocation_config_digest<R: RunEffectRepository + ?Sized>(
    store: &R,
    run_id: &str,
    received_digest: &str,
) -> Result<(), String> {
    let stored = store
        .load_run_invocation_config(run_id)
        .map_err(|error| format!("load Run invocation config failed: {error}"))?
        .ok_or_else(|| format!("Run {run_id} has no local invocation config"))?;
    if stored.invocation_config_digest != received_digest {
        return Err(format!(
            "Run invocation config mismatch for {run_id}: expected={}, received={received_digest}",
            stored.invocation_config_digest
        ));
    }
    let stored_digest = format!(
        "{:x}",
        Sha256::digest(stored.invocation_config_json.as_bytes())
    );
    if stored_digest != stored.invocation_config_digest {
        return Err(format!(
            "Run invocation config integrity failure for {run_id}"
        ));
    }
    Ok(())
}

fn input_origin_uses_internal_turn(
    origin: nola_agent_engine::agent::AgentInputOrigin,
) -> bool {
    origin == nola_agent_engine::agent::AgentInputOrigin::GoalContinuation
}

fn unrecorded_checkpoint_usage<R: RunEffectRepository + ?Sized, S: Borrow<AgentThreadState>>(
    store: &R,
    checkpoint: &CoreCheckpoint<S>,
) -> Result<Vec<StoredMemoryUsageEntry>, String> {
    let entries = checkpoint_usage_entries(checkpoint)?;
    if entries.is_empty() { return Ok(entries); }
    store.unrecorded_memory_usage_entries(&entries)
        .map_err(|error| format!("read committed memory usage failed: {error}"))
}

fn checkpoint_usage_entries<S: Borrow<AgentThreadState>>(
    checkpoint: &CoreCheckpoint<S>,
) -> Result<Vec<StoredMemoryUsageEntry>, String> {
    checkpoint
        .state
        .borrow()
        .memory_runtime
        .usage_records()
        .map(|entry| {
            Ok(StoredMemoryUsageEntry {
                usage_id: entry.usage_id.clone(),
                memory_id: entry.memory_id.clone(),
                session_id: entry.session_id.clone(),
                turn_id: entry.turn_id.clone(),
                used_at_ms: entry.used_at_epoch_ms,
                context_payload_digest: entry.context_payload_digest.clone(),
                citation_ids_json: serde_json::to_string(&entry.citation_ids).map_err(|error| {
                    format!("encode memory usage citation identities failed: {error}")
                })?,
            })
        })
        .collect()
}

#[test]
fn memory_usage_projection_preserves_accounting_identity_and_citations() {
    let core = AgentCore::try_new(AgentCoreConfig {
        session_id: Some(nola_agent_engine::agent::protocol::SessionId::from_string("session".into())), ..crate::model_fixture::config()
    }).unwrap();
    let mut checkpoint = core.recovery_checkpoint(CheckpointReason::Manual);
    checkpoint.state.memory_runtime.usage_ledger.push(
        nola_agent_engine::agent::memory_runtime::MemoryUsageLedgerEntry {
            usage_id: "usage-1".into(), memory_id: "memory".into(), session_id: "session".into(),
            turn_id: "turn".into(), used_at_epoch_ms: 1, context_payload_digest: "digest".into(),
            citation_ids: vec!["citation".into()],
        });
    assert_eq!(checkpoint_usage_entries(&checkpoint).unwrap(), vec![StoredMemoryUsageEntry {
        usage_id: "usage-1".into(), memory_id: "memory".into(), session_id: "session".into(),
        turn_id: "turn".into(), used_at_ms: 1, context_payload_digest: "digest".into(),
        citation_ids_json: r#"["citation"]"#.into(),
    }]);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptHostEvent<S = AgentThreadState> {
    pub conversation_id: String,
    pub run_id: String,
    pub response: HostResponseEnvelope,
    pub events: Vec<CoreEventEnvelope<AgentEvent>>,
    pub successor_requests: Vec<HostRequestEnvelope>,
    pub checkpoint: CoreCheckpoint<S>,
    pub received_at_ms: i64,
}

pub fn accept_host_event<R: RunEffectRepository + EffectDeliveryRepository + repositories::AssistantPublicationRepositories + ?Sized>(
    store: &mut R,
    mut command: AcceptHostEvent,
) -> Result<TransitionCommit, String> {
    command.checkpoint = durable_checkpoint_projection(command.checkpoint)?;
    assistant_publication_transaction::with_publication_transaction(store, |store| accept_host_event_in_transaction(store, command, None, None, None))
}

fn accept_host_event_in_transaction<R: RunEffectRepository + EffectDeliveryRepository + repositories::AssistantPublicationRepositories + ?Sized, S: Borrow<AgentThreadState> + Serialize>(
    store: &mut R,
    command: AcceptHostEvent<S>,
    reduction_json: Option<String>,
    planned_model_attempt_mutations: Option<Vec<nola_domain::ModelAttemptMutation>>,
    validated_request: Option<&nola_domain::host_control::HostEffectControl>,
) -> Result<TransitionCommit, String> {
    // The reducer entry has already resolved the immutable request under this
    // same Store unit of work. Direct transition callers still prove it from
    // the durable outbox. Never re-read/decode the entire model prompt merely
    // to commit the reducer's next streamed delta.
    let request = match validated_request {
        Some(request) => std::borrow::Cow::Borrowed(request),
        None => {
            let effect = store.outbox_effect(&command.response.effect_id.0)
                .map_err(|error| format!("load accepted Host effect: {error}"))?
                .ok_or("accepted Host event has no pending effect")?;
            if effect.run_id != command.run_id {
                return Err("accepted Host effect belongs to another Run".into());
            }
            let request=serde_json::from_str::<HostRequestEnvelope>(&effect.payload_json)
                .map_err(|error| format!("decode accepted Host effect: {error}"))?;
            std::borrow::Cow::Owned(nola_domain::host_control::HostEffectControl::from_request(&request)?)
        }
    };
    let delivery_attempt = effect_delivery::validate_response_binding(
        store,
        &command.run_id,
        &request,
        &command.response,
        command.received_at_ms,
    )?;
    // Runtime heartbeats and Host responses share one durable delivery
    // timeline. Wall-clock corrections can make a later response appear a few
    // milliseconds older than its last heartbeat, so use the attempt's durable
    // floor for every timestamp committed by this response transaction.
    let received_at_ms =
        effect_delivery::logical_received_at_ms(delivery_attempt.as_ref(), command.received_at_ms);
    model_attempt::validate_response(store, &command.run_id, &request, &command.response)?;
    delegated_invocation::validate_response(store, &command.run_id, &request, &command.response)?;
    delegated_permission::validate_response(store, &command.run_id, &request, &command.response)?;
    let active_run = store
        .active_run(&command.conversation_id)
        .map_err(|error| format!("load active Run failed: {error}"))?
        .ok_or_else(|| "Host event has no active canonical Run".to_string())?;
    if active_run.run_id != command.run_id {
        return Err(format!(
            "Host event Run mismatch: active={}, received={}",
            active_run.run_id, command.run_id
        ));
    }
    let is_maintenance = active_run.kind == "maintenance";
    command.checkpoint.validate_state_identity()?;
    if command.checkpoint.session_id != request.session_id
        || command.checkpoint.state.borrow().session_id.as_str() != request.session_id
        || command.events.iter().any(|event| event.session_id != request.session_id)
        || command.successor_requests.iter().any(|successor| successor.session_id != request.session_id)
    {
        return Err("Host transition checkpoint, events or successors belong to another Core session".into());
    }
    let next_checkpoint = command.checkpoint;
    // Accounting authority is the transactional ledger, not a prior snapshot.
    // Exact replays survive memory deletion; conflicting IDs still fail closed.
    let expected_checkpoint_revision = store.checkpoint_revision(&command.conversation_id)
        .map_err(|error| format!("load Core checkpoint revision failed: {error}"))?
        .ok_or_else(|| "Host event Run has no canonical checkpoint".to_string())?;
    let memory_usage_entries = unrecorded_checkpoint_usage(store, &next_checkpoint)?;
    let next_checkpoint_revision = expected_checkpoint_revision.checked_add(1)
        .ok_or("Core checkpoint revision exhausted")?;
    let incremental = reduction_json.is_some()
        // Eligibility belongs to the resulting transition as well as the
        // incoming response kind. A progress response may close/fail a Turn;
        // terminal evidence must be captured from its current full baseline,
        // never from the pre-stream baseline beneath a replay tail.
        && checkpoint_terminal_status(&next_checkpoint).is_none()
        && store.checkpoint_replay_ready(&command.conversation_id, &command.run_id, expected_checkpoint_revision,
            reduction_json.as_ref().map_or(0, String::len))
            .map_err(|error| format!("read checkpoint replay readiness failed: {error}"))?;
    let checkpoint_write = if incremental {
        nola_store_port::run_effect::CheckpointWrite::Replay {
            delivery_witness_json: encode_delivery_witness(&next_checkpoint)?,
        }
    } else {
        nola_store_port::run_effect::CheckpointWrite::Baseline {
            checkpoint_json: serde_json::to_string(&next_checkpoint)
                .map_err(|error| format!("encode Host transition checkpoint failed: {error}"))?,
        }
    };
    let checkpoint_event_cursor = i64::try_from(next_checkpoint.last_event_cursor.sequence)
        .map_err(|_| "Core checkpoint event cursor exceeds the durable range")?;

    let host_event_id = format!(
        "host:{}:{}:{}",
        command.response.session_id, command.response.effect_id.0, command.response.sequence
    );
    let transition_id = format!("transition:{host_event_id}");
    let host_payload_json = serde_json::to_string(&command.response)
        .map_err(|error| format!("encode Host response failed: {error}"))?;

    let mut model_attempt_mutations = match planned_model_attempt_mutations {
        Some(mutations) => mutations,
        None => model_attempt::response_mutations(
            store,
            &command.run_id,
            &command.response,
            received_at_ms,
        )?,
    };
    let mut delegated_invocation_mutations = delegated_invocation::response_mutations(
        store,
        &command.run_id,
        &command.response,
        received_at_ms,
    )?;
    let mut delegated_permission_mutations = delegated_permission::response_mutations(
        store,
        &command.run_id,
        &command.response,
        received_at_ms,
    )?;
    let mut effect_delivery_attempt_mutations = effect_delivery::response_mutations(
        delivery_attempt.as_ref(),
        &command.response,
        received_at_ms,
    )?;
    effect_delivery_attempt_mutations.extend(effect_delivery::confirmed_cancellation_mutations(
        store, &command.run_id, &command.response, next_checkpoint.state.borrow(), received_at_ms,
    )?);
    delegated_invocation_mutations.extend(delegated_permission::parent_mutations(
        store,
        &command.response,
        received_at_ms,
    )?);
    let mut next_sequence = active_run.next_event_sequence;
    let mut events = Vec::with_capacity(command.events.len());
    let mut ensured_messages = Vec::new();
    let mut part_message_owners = std::collections::BTreeMap::new();
    let mut assistant_turn_bindings = assistant_turn_binding::AssistantTurnBindings::default();
    let mut part_mutations = Vec::new();
    let mut part_planner = CanonicalPartProjectionPlanner::default();
    let mut terminal_status = None;
    let mut terminal_turn_id = None;
    let mut pending_turn_birth = None;
    for envelope in &command.events {
        let payload = committed_event_payload_for_persistence(envelope)
            .map_err(|error| format!("encode Core event failed: {error}"))?;
        let event_kind = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        events.push(RunEventInput {
            event_id: format!("core:{}:{}", command.run_id, envelope.cursor.sequence),
            sequence: next_sequence,
            event_kind,
            payload_json: serde_json::to_string(&payload)
                .map_err(|error| format!("encode Core event payload failed: {error}"))?,
            created_at_ms: received_at_ms,
        });
        next_sequence += 1;
        match &envelope.event {
            AgentEvent::TurnStarted { turn_id, assistant_message_id, .. } if active_run.kind != "maintenance" => {
                flush_started_assistant_message(&mut pending_turn_birth, &mut ensured_messages,
                    &mut assistant_turn_bindings, &command.run_id, received_at_ms)?;
                pending_turn_birth = Some((turn_id.as_str(), assistant_message_id.as_str()));
            }
            AgentEvent::AssistantSegmentStarted { turn_id, assistant_message_id, .. } if active_run.kind != "maintenance" => {
                assistant_turn_bindings.record_segment(turn_id.as_str(), assistant_message_id);
                merge_transition_message(&mut ensured_messages, TransitionMessageInput {
                    id: assistant_message_id.clone(), run_id: Some(command.run_id.clone()), role: "assistant".into(),
                    status: "streaming".into(), created_at_ms: received_at_ms, completed_at_ms: None, initial_parts: Vec::new(),
                })?;
            }
            AgentEvent::AssistantActivityBound { turn_id, assistant_message_id, activity_id, activity_kind } if active_run.kind != "maintenance" => {
                let message = if assistant_message_id == &format!("{}:assistant", command.run_id) {
                    active_run.assistant_message_id.as_deref().ok_or("activity has no admitted assistant owner")?
                } else { assistant_message_id.as_str() };
                assistant_turn_bindings.record_activity(turn_id.as_str(), message, activity_id, activity_kind);
            }
            AgentEvent::AssistantEffectBound { turn_id, assistant_message_id, effect_id } if active_run.kind != "maintenance" => {
                let message = if assistant_message_id == &format!("{}:assistant", command.run_id) {
                    active_run.assistant_message_id.as_deref().ok_or("effect has no admitted assistant owner")?
                } else { assistant_message_id.as_str() };
                assistant_turn_bindings.record_effect(turn_id.as_str(), message, effect_id);
            }
            AgentEvent::InputApplied { turn_id, items, applications: _ } => {
                ensure_applied_user_messages(&mut ensured_messages, items, &command.run_id, received_at_ms)?;
                if pending_turn_birth.is_some_and(|(turn, _)| turn == turn_id.as_str()) {
                    flush_started_assistant_message(&mut pending_turn_birth, &mut ensured_messages,
                        &mut assistant_turn_bindings, &command.run_id, received_at_ms)?;
                }
            }
            AgentEvent::PartUpdated {
                turn_id,
                projection_revision,
                update,
                part,
                ..
            } if !is_maintenance => {
                flush_started_assistant_message(&mut pending_turn_birth, &mut ensured_messages,
                    &mut assistant_turn_bindings, &command.run_id, received_at_ms)?;
                let canonical_part = CanonicalPartProjection::new(
                    part,
                    &command.run_id,
                    active_run.assistant_message_id.as_deref(),
                )?;
                assistant_turn_bindings.record_part(turn_id.as_str(), &canonical_part)?;
                ensure_assistant_message_for_part(
                    &mut ensured_messages,
                    &mut part_message_owners,
                    update,
                    &canonical_part,
                    &command.run_id,
                    received_at_ms,
                )?;
                let owner = nola_domain::CanonicalStreamOwner {
                    conversation_id: command.conversation_id.clone(),
                    run_id: command.run_id.clone(),
                    message_id: canonical_part.message_id().to_string(),
                    part_id: canonical_part.part().id().to_string(),
                    field: nola_domain::PartBodyField::ContentText,
                };
                part_mutations.extend(part_planner.plan_view(
                    &owner,
                    update,
                    &canonical_part,
                    *projection_revision as i64,
                    received_at_ms,
                    |owner| {
                        store
                            .load_canonical_text_state(owner)
                            .map_err(|error| error.to_string())
                    },
                )?);
            }
            AgentEvent::TurnCompleted { turn_id, .. } => {
                terminal_status = Some("completed");
                terminal_turn_id = Some(turn_id.as_str().to_string());
            }
            AgentEvent::TurnCancelled { turn_id, .. } => {
                terminal_status = Some("cancelled");
                terminal_turn_id = Some(turn_id.as_str().to_string());
            }
            AgentEvent::TurnFailed { turn_id, .. } => {
                terminal_status = Some("failed");
                terminal_turn_id = Some(turn_id.as_str().to_string());
            }
            _ => {}
        }
    }
    flush_started_assistant_message(&mut pending_turn_birth, &mut ensured_messages,
        &mut assistant_turn_bindings, &command.run_id, received_at_ms)?;
    if !is_maintenance {
        if let Some(turn_id) = next_checkpoint
            .state
            .borrow()
            .execution
            .active_turn
            .as_ref()
            .map(|turn| turn.turn_id.as_str())
        {
            ensure_terminal_assistant_message(
                store, &assistant_turn_bindings, &command.conversation_id,
                &mut ensured_messages,
                turn_id,
                &command.run_id,
                active_run.assistant_message_id.as_deref(),
                received_at_ms,
            )?;
        }
    }
    let assistant_status_message_id = terminal_turn_id
        .as_deref()
        .or_else(|| {
            next_checkpoint
                .state
                .borrow()
                .execution
                .active_turn
                .as_ref()
                .map(|turn| turn.turn_id.as_str())
        })
        .map(|turn_id| -> Result<Option<String>, String> {
            if turn_id == command.run_id {
                Ok(active_run.assistant_message_id.clone())
            } else {
                resolve_transition_assistant_message(store, &assistant_turn_bindings,
                    &command.conversation_id, turn_id, &command.run_id,
                    active_run.assistant_message_id.as_deref()).map(Some)
            }
        }).transpose()?.flatten();
    if let (Some(status), Some(turn_id)) = (
        terminal_status.filter(|_| !is_maintenance),
        terminal_turn_id.as_deref(),
    ) {
        ensure_terminal_assistant_message(
            store, &assistant_turn_bindings, &command.conversation_id,
            &mut ensured_messages,
            turn_id,
            &command.run_id,
            active_run.assistant_message_id.as_deref(),
            received_at_ms,
        )?;
        if turn_id != command.run_id {
            // A successor Turn owns its own Parts. The initial Run placeholder
            // was already closed by the transition that promoted the successor.
        } else if let Some(begun_part_id) = active_run.assistant_part_id.as_deref() {
            let begun_part_is_closed = part_mutations.iter().any(|mutation| match mutation {
                PartMutation::Close { part_id, .. } => part_id == begun_part_id,
                _ => false,
            });
            if !begun_part_is_closed {
                let owner = nola_domain::CanonicalStreamOwner {
                    conversation_id: command.conversation_id.clone(),
                    run_id: command.run_id.clone(),
                    message_id: active_run
                        .assistant_message_id
                        .clone()
                        .ok_or("missing assistant owner")?,
                    part_id: begun_part_id.to_string(),
                    field: nola_domain::PartBodyField::ContentText,
                };
                part_mutations.extend(part_planner.close_if_stream(
                    &owner,
                    received_at_ms,
                    |owner| {
                        store
                            .load_canonical_text_state(owner)
                            .map_err(|error| error.to_string())
                    },
                )?);
                part_mutations.push(PartMutation::Close {
                    part_id: begun_part_id.to_string(),
                    state: status.to_string(),
                    content_json: None,
                    content_digest: None,
                    updated_at_ms: received_at_ms,
                });
            }
        }
    }

    let effect_id = command.response.effect_id.0.clone();
    let request_id = command.response.request_id.clone();
    let host_sequence = command.response.sequence as i64;
    let mut successor_requests = command.successor_requests;
    effect_delivery_attempt_mutations.extend(cancellation_not_started::prepare(store,&command.run_id,
        next_checkpoint_revision,received_at_ms,next_checkpoint.state.borrow(),&mut successor_requests)?);
    let mut successor_effects = external_outbox_requests(store, &successor_requests)?;
    model_attempt_mutations.extend(model_attempt::prepare_effects(
        store,
        &command.run_id,
        &mut successor_effects,
        received_at_ms,
    )?);
    delegated_invocation_mutations.extend(delegated_invocation::prepare_effects(
        store,
        &command.run_id,
        &mut successor_effects,
        received_at_ms,
    )?);
    delegated_permission::prepare_effects(
        store,
        &command.run_id,
        &mut successor_effects,
        &mut delegated_permission_mutations,
        received_at_ms,
    )?;
    model_attempt::bind_mutated_effects(&mut successor_effects, &model_attempt_mutations)?;
    model_attempt::append_events(
        &mut events,
        &mut next_sequence,
        &model_attempt_mutations,
        received_at_ms,
    )?;
    delegated_invocation::append_events(
        &mut events,
        &mut next_sequence,
        &delegated_invocation_mutations,
        received_at_ms,
    )?;
    delegated_permission::append_events(
        &mut events,
        &mut next_sequence,
        &delegated_permission_mutations,
        received_at_ms,
    )?;
    // A terminal Turn can still own committed cleanup effects, most notably
    // `host.cancel`. Turn semantics and Host-effect settlement are separate
    // protocol boundaries: close the canonical Run only after the terminal
    // checkpoint has no external successor left to acknowledge.
    if terminal_status.is_none() && successor_effects.is_empty() {
        terminal_status = checkpoint_terminal_status(&next_checkpoint);
    }
    let response_kind = host_response_kind(&command.response.payload);
    let response_sequence = command.response.sequence;
    let effect_is_settled = command.response.payload.is_terminal();
    let terminal_cleanup_pending = terminal_status.is_some() && (!successor_effects.is_empty()
        || next_checkpoint.state.borrow().host.pending_effects.iter().any(|effect| effect.target_effect_id.is_some()));
    if is_maintenance { maintenance_turn::validate_maintenance_checkpoint(&command.run_id,&next_checkpoint,&command.events)?; }
    let execution_status = checkpoint_execution_status(&next_checkpoint);
    let mut acknowledged_effect_ids = effect_is_settled.then_some(effect_id.clone()).into_iter().collect::<Vec<_>>();
    if let HostResponse::CancellationConfirmed { target_effect_id } = &command.response.payload {
        // Only the accepted physical acknowledgement, after Core removes its
        // exact target, retires that target from executable delivery.
        if !next_checkpoint.state.borrow().host.pending_effects.iter().any(|effect| effect.effect_id == target_effect_id.0) {
            acknowledged_effect_ids.push(target_effect_id.0.clone());
        }
    }
    let mutation_summary = part_mutations
        .iter()
        .map(part_mutation_summary)
        .collect::<Vec<_>>()
        .join(",");
    let input = CommitTransitionInput {
            effect_delivery_attempt_mutations,
            transition_id,
            run_id: command.run_id,
            expected_checkpoint_revision,
            checkpoint_revision: next_checkpoint_revision,
            checkpoint_event_cursor,
            checkpoint: checkpoint_write,
            submission_reduction_json: None, host_event: Some(HostEventInput {
                event_id: host_event_id,
                effect_id: effect_id.clone(),
                request_id,
                sequence: host_sequence,
                payload_json: host_payload_json,
                reduction_json,
                // Preserve the reducer's input clock. The lease/attempt
                // mutations above use their separately monotonic clock floor.
                received_at_ms: command.received_at_ms,
            }),
            events,
            ensured_messages,
            part_message_owners,
            assistant_turn_bindings: assistant_turn_bindings.into_bindings(),
            part_mutations,
            model_attempt_mutations,
            delegated_invocation_mutations,
            delegated_permission_mutations,
            outbox_effects: successor_effects,
            // The durable outbox is the Runtime's source of executable work.
            // A progress response proves delivery but does not settle the
            // effect: acknowledging it here makes the Runtime lose the
            // terminal response after the first stream delta/progress event.
            // Host-event inbox deduplication already records delivery. Remove
            // the effect from executable work only when Core classifies the
            // response as terminal.
            acknowledged_effect_ids,
            memory_usage_entries,
            run_status: terminal_status
                .filter(|_| !terminal_cleanup_pending)
                .map(|_| "sealed")
                .or(execution_status)
                .map(str::to_string),
            assistant_status: terminal_status
                .or(execution_status)
                .filter(|_| !is_maintenance)
                .map(str::to_string),
            assistant_status_message_id,
            committed_at_ms: received_at_ms,
        };
    let commit = store.commit_transition(&input)
        .map_err(|error| {
            format!(
                "commit Host transition failed: {error}; response={response_kind}; sequence={response_sequence}; part_mutations=[{mutation_summary}]"
            )
        })?;
    let mut commit: TransitionCommit = commit.into();
    if commit.applied {
        commit.assistant_publications = publish_transition_journal(store, &command.conversation_id, &next_checkpoint,
            &command.events, &input, is_maintenance)?;
    }
    Ok(commit)
}

fn publish_transition_journal<R: repositories::AssistantPublicationRepositories + ?Sized, S: Borrow<AgentThreadState>>(
    store: &mut R,
    conversation_id: &str,
    checkpoint: &CoreCheckpoint<S>,
    events: &[CoreEventEnvelope<AgentEvent>],
    input: &CommitTransitionInput,
    maintenance: bool,
) -> Result<Vec<nola_domain::assistant_journal::AssistantJournalCursor>, String> {
    let changes = assistant_journal_changes::collect_assistant_journal_changes(
        assistant_journal_changes::AssistantJournalChangeScope {
            conversation_id, core_session_id: &checkpoint.session_id,
            source_run_id: Some(&input.run_id), maintenance,
        }, events, &checkpoint.state.borrow().skill_runtime, &input.model_attempt_mutations, &input.part_mutations,
    )?;
    let publication = assistant_journal_producer::publish_changed_assistant_journal(
        store, nola_store_port::assistant_journal::AssistantSourceCommitStamp::Transition {
            transition_id: input.transition_id.clone(),
        }, conversation_id, &checkpoint.session_id, Some(&input.run_id), &changes,
    )?;
    tool_source::bind_changed(store,
        &nola_store_port::assistant_journal::AssistantSourceCommitStamp::Transition { transition_id:input.transition_id.clone() },
        conversation_id,checkpoint.state.borrow(),&changes)?;
    code_cell_publication::publish(store, conversation_id, checkpoint.state.borrow())?;
    if !maintenance && input.run_status.as_deref() == Some("sealed") {
        store.seal_assistant_run_terminal(&input.transition_id).map_err(|e|e.to_string())?
            .ok_or_else(|| "terminal transition did not produce a canonical Run seal".to_string())?;
    }
    Ok(assistant_journal_producer::publication_cursors(publication))
}

fn host_response_kind(response: &HostResponse) -> &'static str {
    match response {
        HostResponse::Accepted => "accepted",
        HostResponse::ExecutionStarted => "execution_started",
        HostResponse::ExecutionRejected { .. } => "execution_rejected",
        HostResponse::DelegatedAgentEvent { .. } => "delegated_agent_event",
        HostResponse::DelegatedExecutionInterrupted { .. } => "delegated_execution_interrupted",
        HostResponse::ModelDelta { .. } => "model_delta",
        HostResponse::ModelCommittedDelta { .. } => "model_committed_delta",
        HostResponse::ModelToolCallDelta { .. } => "model_tool_call_delta",
        HostResponse::ModelContinuation { .. } => "model_continuation",
        HostResponse::ModelAttemptStarted { .. } => "model_attempt_started",
        HostResponse::ModelUsageReported { .. } => "model_usage_reported",
        HostResponse::DelegatedPermissionClaimed { .. } => "delegated_permission_claimed",
        HostResponse::DelegatedPermissionTransportWritten { .. } => {
            "delegated_permission_transport_written"
        }
        HostResponse::DelegatedPermissionInterrupted { .. } => "delegated_permission_interrupted",
        HostResponse::ToolProgress { .. } => "tool_progress",
        HostResponse::ToolCallProgress { .. } => "tool_call_progress",
        HostResponse::CodeModeToolRequested { .. } => "code_mode_tool_requested",
        HostResponse::CodeModeOutput { .. } => "code_mode_output",
        HostResponse::CodeModeYielded { .. } => "code_mode_yielded",
        HostResponse::CodeModeObservationSuspended { .. } => "code_mode_observation_suspended",
        HostResponse::ModelCompleted { .. } => "model_completed",
        HostResponse::ModelOutputLimited { .. } => "model_output_limited",
        HostResponse::ToolCompleted { .. } => "tool_completed",
        HostResponse::PermissionResolved { .. } => "permission_resolved",
        HostResponse::PermissionTransportInterrupted { .. } => "permission_transport_interrupted",
        HostResponse::TaskPromoted { .. } => "task_promoted",
        HostResponse::TaskUpdated { .. } => "task_updated",
        HostResponse::ContextFetched { .. } => "context_fetched",
        HostResponse::PermissionResourcesResolved { .. } => "permission_resources_resolved",
        HostResponse::CapabilityCatalogRefreshed { .. } => "capability_catalog_refreshed",
        HostResponse::CompactionCompleted { .. } => "compaction_completed",
        HostResponse::Completed { .. } => "completed",
        HostResponse::Failed { .. } => "failed",
        HostResponse::CancellationConfirmed { .. } => "cancellation_confirmed",
        HostResponse::LifecycleHookUpdated { .. } => "lifecycle_hook_updated",
    }
}

fn checkpoint_terminal_status<S: Borrow<AgentThreadState>>(
    checkpoint: &CoreCheckpoint<S>,
) -> Option<&'static str> {
    checkpoint
        .state
        .borrow()
        .execution
        .active_turn
        .as_ref()
        .and_then(|turn| turn_lifecycle_projection(turn.status).terminal)
}

fn checkpoint_execution_status<S: Borrow<AgentThreadState>>(
    checkpoint: &CoreCheckpoint<S>,
) -> Option<&'static str> {
    checkpoint
        .state
        .borrow()
        .execution
        .active_turn
        .as_ref()
        .and_then(|turn| turn_lifecycle_projection(turn.status).executing)
}

/// Lifecycle projection only, never evidence that the user's task succeeded.
/// Both submission and Host-response paths must use the same exhaustive map.
/// The terminal-result protocol replaces these existing Store status strings;
/// adding a new Core lifecycle must first make this match exhaustive again.
struct TurnLifecycleProjection {
    terminal: Option<&'static str>,
    executing: Option<&'static str>,
}

fn turn_lifecycle_projection(status: nola_domain::AgentTurnStatus) -> TurnLifecycleProjection {
    use nola_domain::AgentTurnStatus;
    let (terminal, executing) = match status {
        AgentTurnStatus::Idle => (None, None),
        AgentTurnStatus::AwaitingInput => (None, Some("waiting_input")),
        AgentTurnStatus::AwaitingModel | AgentTurnStatus::AwaitingTools
        | AgentTurnStatus::AwaitingPermission | AgentTurnStatus::Compacting
        | AgentTurnStatus::Cancelling => (None, Some("running")),
        AgentTurnStatus::Completed => (Some("completed"), None),
        AgentTurnStatus::Cancelled => (Some("cancelled"), None),
        AgentTurnStatus::Failed => (Some("failed"), None),
    };
    TurnLifecycleProjection { terminal, executing }
}

#[cfg(test)]
mod turn_lifecycle_projection_tests {
    use super::*;

    #[test]
    fn lifecycle_has_no_implicit_terminal_fallback() {
        use nola_domain::AgentTurnStatus::*;
        for (state, terminal, executing) in [
            (Idle, None, None),
            (AwaitingModel, None, Some("running")),
            (AwaitingTools, None, Some("running")),
            (AwaitingPermission, None, Some("running")),
            (Compacting, None, Some("running")),
            (Cancelling, None, Some("running")),
            (Completed, Some("completed"), None),
            (Cancelled, Some("cancelled"), None),
            (Failed, Some("failed"), None),
        ] {
            let projection = turn_lifecycle_projection(state);
            assert_eq!(projection.terminal, terminal, "{state:?}");
            assert_eq!(projection.executing, executing, "{state:?}");
            assert_eq!(projection.terminal.is_some(), state.is_terminal());
        }
    }
}

fn part_mutation_summary(mutation: &PartMutation) -> String {
    match mutation {
        PartMutation::Upsert { part_id, state, .. } => format!("upsert:{part_id}:{state}"),
        PartMutation::AppendText { part_id, .. } => format!("append:{part_id}"),
        PartMutation::Close { part_id, state, .. } => format!("close:{part_id}:{state}"),
        PartMutation::Metadata { part_id, state, .. } => format!("metadata:{part_id}:{state}"),
        PartMutation::Stream { operation, .. } => match operation {
            nola_domain::CanonicalStreamOperation::Declare { owner } => {
                format!("stream-declare:{}", owner.part_id)
            }
            nola_domain::CanonicalStreamOperation::Append { owner, .. } => {
                format!("stream-append:{}", owner.part_id)
            }
            nola_domain::CanonicalStreamOperation::Replace { owner, .. } => {
                format!("stream-replace:{}", owner.part_id)
            }
            nola_domain::CanonicalStreamOperation::Close { owner, .. } => {
                format!("stream-close:{}", owner.part_id)
            }
        },
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReduceAndAcceptHostResponse {
    pub conversation_id: String,
    pub run_id: String,
    pub invocation_config_digest: String,
    pub response: HostResponseEnvelope,
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptedHostResponse {
    pub commit: TransitionCommit,
    pub events: Vec<CoreEventEnvelope<AgentEvent>>,
    pub traces: Vec<AgentTraceEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<nola_agent_engine::agent::protocol::AgentControlSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReduceAndCommitActiveSubmission {
    pub conversation_id: String,
    pub run_id: String,
    pub transition_id: String,
    pub invocation_config_digest: String,
    pub submission: AgentSubmission,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedActiveSubmission {
    pub commit: TransitionCommit,
    pub events: Vec<CoreEventEnvelope<AgentEvent>>,
    pub traces: Vec<AgentTraceEvent>,
    pub control: nola_agent_engine::agent::protocol::AgentControlSnapshot,
}

pub fn reduce_and_commit_active_submission<R: RunEffectRepository + repositories::AssistantPublicationRepositories + ?Sized>(
    store: &mut R,
    command: ReduceAndCommitActiveSubmission,
) -> Result<AcceptedActiveSubmission, String> {
    reduce_and_commit_active_submission_with_core(store, command, None, Vec::new())
        .map(|(accepted, _)| accepted)
}

pub(crate) fn reduce_and_commit_active_submission_with_core<R: RunEffectRepository + repositories::AssistantPublicationRepositories + ?Sized>(
    store: &mut R,
    command: ReduceAndCommitActiveSubmission,
    hot_core: Option<AgentCore>,
    deferred_events: Vec<CoreEventEnvelope<AgentEvent>>,
) -> Result<(AcceptedActiveSubmission, AgentCore), String> {
    goal_execution::reject_unbound_goal_start(&command.submission)?;
    if matches!(&command.submission, AgentSubmission::ResolveUnknownEffects { .. }) {
        return Err("unknown effect recovery requires the explicit user recovery command".into());
    }
    assistant_publication_transaction::with_publication_transaction(store, |store|
        reduce_and_commit_active_submission_in_transaction(store, command, hot_core, deferred_events))
}

fn reduce_and_commit_active_submission_in_transaction<R: RunEffectRepository + repositories::AssistantPublicationRepositories + ?Sized>(
    store: &mut R,
    mut command: ReduceAndCommitActiveSubmission,
    hot_core: Option<AgentCore>,
    deferred_events: Vec<CoreEventEnvelope<AgentEvent>>,
) -> Result<(AcceptedActiveSubmission, AgentCore), String> {
    // Only this already-validated explicit user decision may end waiting
    // without physical stop evidence. It preserves Blocked, never Cancelled.
    let preserve_unknown = matches!(&command.submission, AgentSubmission::ResolveUnknownEffects {decision}
        if decision.disposition == nola_domain::unknown_effect_recovery::UnknownEffectRecoveryDisposition::KeepUnknownBlocked);
    if matches!(command.submission, AgentSubmission::PromoteQueuedInput { .. }) {
        return Err("queued successor requires independent Run admission".into());
    }
    let active_run = store
        .active_run(&command.conversation_id)
        .map_err(|error| format!("load active Run failed: {error}"))?
        .ok_or_else(|| "active submission has no active Run".to_string())?;
    if active_run.run_id != command.run_id {
        return Err(format!(
            "active submission Run mismatch: active={}, received={}",
            active_run.run_id, command.run_id
        ));
    }
    let pending_effect_ids = store.pending_outbox_effect_ids(&command.run_id)
        .map_err(|error| format!("load pending effect settlement identities failed: {error}"))?;
    let expected_revision = store.checkpoint_revision(&command.conversation_id)
        .map_err(|error| format!("load active Core checkpoint revision failed: {error}"))?
        .ok_or_else(|| "active submission has no Core checkpoint".to_string())?;
    let mut submission_bindings = Vec::new();
    let core = match hot_core {
        Some(core) => {
            validate_run_invocation_config_digest(
                store,
                &command.run_id,
                &command.invocation_config_digest,
            )?;
            core
        }
        None => {
            let committed_host_requests = load_committed_host_requests(store, &command.run_id)?;
            let stored = load_exact_run_checkpoint(store, &command.conversation_id, &command.run_id)?;
            let mut core = restore_run_recovery(store, &command.run_id,
                &command.invocation_config_digest, &stored)?;
            // Rebind current delivery facts only after replay. Applying the
            // latest outbox cursor before replay could skip historical inputs.
            for request in &committed_host_requests.requests {
                core.bind_committed_host_effect(request).map_err(|error| {
                    format!("bind committed Host effect before active submission failed: {error}")
                })?;
                submission_bindings.push(nola_store_port::host_replay::CoreHostBindingOverlay::from_request(request));
            }
            core
        }
    };
    if matches!(&command.submission, AgentSubmission::HostResponse { .. }) {
        return Err("Host responses require their delivery-validated commit path".into());
    }
    // Recovery binding and cancellation are low-frequency durable frontiers. Materialize their
    // fully replayed state once so subsequent cold readers do not repeatedly
    // replay prior bindings and hash the same immutable model input. Streaming
    // Host responses retain their separate incremental checkpoint path.
    command.submission = goal_control_intent::active_submission(command.submission);
    let materialize_control_boundary = matches!(
        &command.submission, AgentSubmission::ResumeHostEffect { .. } | AgentSubmission::Interrupt { .. }
    );
    let submission_reduction_json = serde_json::to_string(
        &nola_store_port::run_effect::SubmissionReplayReduction {
            protocol_version: 1,
            session_id: core.checkpoint_view(CheckpointReason::Manual).session_id.as_str().to_owned(),
            decision_time_ms: command.committed_at_ms,
            submission_json: serde_json::to_string(&command.submission).map_err(|error| error.to_string())?,
            bindings: submission_bindings,
        }).map_err(|error| format!("encode submission recovery facts failed: {error}"))?;
    let (core, transition) = core
        .reduce_candidate(
            command.submission,
            decision_time_ms(command.committed_at_ms)?,
        )
        .map_err(|error| format!("reduce active Core submission failed: {error}"))?;
    let checkpoint_reason = transition.durability.as_ref()
        .map_or(CheckpointReason::Manual, |intent| intent.checkpoint_reason);
    let checkpoint = core.checkpoint_view(checkpoint_reason);
    let memory_usage_entries = unrecorded_checkpoint_usage(store, &checkpoint)?;
    let incremental = !materialize_control_boundary
        && checkpoint_terminal_status(&checkpoint).is_none()
        && store.checkpoint_replay_ready(&command.conversation_id, &command.run_id,
            expected_revision, submission_reduction_json.len())
            .map_err(|error| format!("read submission replay readiness failed: {error}"))?;
    let checkpoint_write = if incremental {
        nola_store_port::run_effect::CheckpointWrite::Replay {
            delivery_witness_json: encode_delivery_witness(&checkpoint)?,
        }
    } else {
        nola_store_port::run_effect::CheckpointWrite::Baseline {
            checkpoint_json: serde_json::to_string(&checkpoint)
                .map_err(|error| format!("encode active submission checkpoint failed: {error}"))?,
        }
    };
    let mut committed_events = deferred_events;
    committed_events.extend(transition.event_envelopes.clone());
    let mut next_sequence = active_run.next_event_sequence;
    let mut events = Vec::with_capacity(committed_events.len());
    let mut ensured_messages = Vec::new();
    let mut part_message_owners = std::collections::BTreeMap::new();
    let mut assistant_turn_bindings = assistant_turn_binding::AssistantTurnBindings::default();
    let mut part_mutations = Vec::new();
    let mut part_planner = CanonicalPartProjectionPlanner::default();
    let mut terminal_status = None;
    let mut terminal_turn_id = None;
    let mut pending_turn_birth = None;
    for envelope in &committed_events {
        let payload = committed_event_payload_for_persistence(envelope)
            .map_err(|error| format!("encode active Core event failed: {error}"))?;
        events.push(RunEventInput {
            event_id: format!("core:{}:{}", command.run_id, envelope.cursor.sequence),
            sequence: next_sequence,
            event_kind: payload
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            payload_json: serde_json::to_string(&payload)
                .map_err(|error| format!("encode active Core event payload failed: {error}"))?,
            created_at_ms: command.committed_at_ms,
        });
        next_sequence += 1;
        match &envelope.event {
            AgentEvent::TurnStarted { turn_id, assistant_message_id, .. } if active_run.kind != "maintenance" => {
                flush_started_assistant_message(&mut pending_turn_birth, &mut ensured_messages,
                    &mut assistant_turn_bindings, &command.run_id, command.committed_at_ms)?;
                pending_turn_birth = Some((turn_id.as_str(), assistant_message_id.as_str()));
            }
            AgentEvent::AssistantSegmentStarted { turn_id, assistant_message_id, .. } if active_run.kind != "maintenance" => {
                assistant_turn_bindings.record_segment(turn_id.as_str(), assistant_message_id);
                merge_transition_message(&mut ensured_messages, TransitionMessageInput {
                    id: assistant_message_id.clone(), run_id: Some(command.run_id.clone()), role: "assistant".into(),
                    status: "streaming".into(), created_at_ms: command.committed_at_ms, completed_at_ms: None, initial_parts: Vec::new(),
                })?;
            }
            AgentEvent::AssistantActivityBound { turn_id, assistant_message_id, activity_id, activity_kind } if active_run.kind != "maintenance" => {
                let message = if assistant_message_id == &format!("{}:assistant", command.run_id) {
                    active_run.assistant_message_id.as_deref().ok_or("activity has no admitted assistant owner")?
                } else { assistant_message_id.as_str() };
                assistant_turn_bindings.record_activity(turn_id.as_str(), message, activity_id, activity_kind);
            }
            AgentEvent::AssistantEffectBound { turn_id, assistant_message_id, effect_id } if active_run.kind != "maintenance" => {
                let message = if assistant_message_id == &format!("{}:assistant", command.run_id) {
                    active_run.assistant_message_id.as_deref().ok_or("effect has no admitted assistant owner")?
                } else { assistant_message_id.as_str() };
                assistant_turn_bindings.record_effect(turn_id.as_str(), message, effect_id);
            }
            AgentEvent::InputApplied { turn_id, items, applications: _ } => {
                ensure_applied_user_messages(&mut ensured_messages, items, &command.run_id, command.committed_at_ms)?;
                if pending_turn_birth.is_some_and(|(turn, _)| turn == turn_id.as_str()) {
                    flush_started_assistant_message(&mut pending_turn_birth, &mut ensured_messages,
                        &mut assistant_turn_bindings, &command.run_id, command.committed_at_ms)?;
                }
            }
            AgentEvent::PartUpdated {
                turn_id,
                projection_revision,
                update,
                part,
                ..
            } => {
                flush_started_assistant_message(&mut pending_turn_birth, &mut ensured_messages,
                    &mut assistant_turn_bindings, &command.run_id, command.committed_at_ms)?;
                let canonical_part = CanonicalPartProjection::new(
                    part,
                    &command.run_id,
                    active_run.assistant_message_id.as_deref(),
                )?;
                assistant_turn_bindings.record_part(turn_id.as_str(), &canonical_part)?;
                ensure_assistant_message_for_part(
                    &mut ensured_messages,
                    &mut part_message_owners,
                    update,
                    &canonical_part,
                    &command.run_id,
                    command.committed_at_ms,
                )?;
                let owner = nola_domain::CanonicalStreamOwner {
                    conversation_id: command.conversation_id.clone(),
                    run_id: command.run_id.clone(),
                    message_id: canonical_part.message_id().to_string(),
                    part_id: canonical_part.part().id().to_string(),
                    field: nola_domain::PartBodyField::ContentText,
                };
                part_mutations.extend(part_planner.plan_view(
                    &owner,
                    update,
                    &canonical_part,
                    *projection_revision as i64,
                    command.committed_at_ms,
                    |owner| {
                        store
                            .load_canonical_text_state(owner)
                            .map_err(|error| error.to_string())
                    },
                )?);
            }
            AgentEvent::TurnCompleted { turn_id, .. } => {
                terminal_status = Some("completed");
                terminal_turn_id = Some(turn_id.as_str().to_string());
            }
            AgentEvent::TurnCancelled { turn_id, .. } => {
                terminal_status = Some("cancelled");
                terminal_turn_id = Some(turn_id.as_str().to_string());
            }
            AgentEvent::TurnFailed { turn_id, .. } => {
                terminal_status = Some("failed");
                terminal_turn_id = Some(turn_id.as_str().to_string());
            }
            _ => {}
        }
    }
    flush_started_assistant_message(&mut pending_turn_birth, &mut ensured_messages,
        &mut assistant_turn_bindings, &command.run_id, command.committed_at_ms)?;
    if terminal_status.is_none() {
        terminal_status = checkpoint_terminal_status(&checkpoint);
    }
    if active_run.kind != "maintenance" {
        if let Some(turn) = checkpoint.state.execution.active_turn.as_ref() {
            ensure_terminal_assistant_message(
                store, &assistant_turn_bindings, &command.conversation_id,
                &mut ensured_messages,
                turn.turn_id.as_str(),
                &command.run_id,
                active_run.assistant_message_id.as_deref(),
                command.committed_at_ms,
            )?;
        }
    }
    let assistant_status_message_id = terminal_turn_id
        .as_deref()
        .or_else(|| {
            checkpoint.state.execution.active_turn.as_ref()
                .map(|turn| turn.turn_id.as_str())
        })
        .map(|turn_id| -> Result<Option<String>, String> {
            if turn_id == command.run_id {
                Ok(active_run.assistant_message_id.clone())
            } else {
                resolve_transition_assistant_message(store, &assistant_turn_bindings,
                    &command.conversation_id, turn_id, &command.run_id,
                    active_run.assistant_message_id.as_deref()).map(Some)
            }
        }).transpose()?.flatten();
    if let (Some(status), Some(turn_id)) = (terminal_status, terminal_turn_id.as_deref()) {
        ensure_terminal_assistant_message(
            store, &assistant_turn_bindings, &command.conversation_id,
            &mut ensured_messages,
            turn_id,
            &command.run_id,
            active_run.assistant_message_id.as_deref(),
            command.committed_at_ms,
        )?;
        if turn_id != command.run_id {
            // Successor Turn Parts have independent canonical ownership.
        } else if let Some(begun_part_id) = active_run.assistant_part_id.as_deref() {
            let begun_part_is_closed = part_mutations.iter().any(|mutation| match mutation {
                PartMutation::Close { part_id, .. } => part_id == begun_part_id,
                _ => false,
            });
            if !begun_part_is_closed {
                let owner = nola_domain::CanonicalStreamOwner {
                    conversation_id: command.conversation_id.clone(),
                    run_id: command.run_id.clone(),
                    message_id: active_run
                        .assistant_message_id
                        .clone()
                        .ok_or("missing assistant owner")?,
                    part_id: begun_part_id.to_string(),
                    field: nola_domain::PartBodyField::ContentText,
                };
                part_mutations.extend(part_planner.close_if_stream(
                    &owner,
                    command.committed_at_ms,
                    |owner| {
                        store
                            .load_canonical_text_state(owner)
                            .map_err(|error| error.to_string())
                    },
                )?);
                part_mutations.push(PartMutation::Close {
                    part_id: begun_part_id.to_string(),
                    state: status.to_string(),
                    content_json: None,
                    content_digest: None,
                    updated_at_ms: command.committed_at_ms,
                });
            }
        }
    }
    let mut successor_requests = transition.host_requests;
    let cancellation_delivery_mutations = cancellation_not_started::prepare(store,&command.run_id,
        expected_revision.checked_add(1).ok_or("Core checkpoint revision exhausted")?,
        command.committed_at_ms,checkpoint.state.borrow(),&mut successor_requests)?;
    let mut successor_effects = external_outbox_requests(store, &successor_requests)?;
    let mut model_attempt_mutations = model_attempt::prepare_effects(
        store,
        &command.run_id,
        &mut successor_effects,
        command.committed_at_ms,
    )?;
    let mut delegated_invocation_mutations = delegated_invocation::prepare_effects(
        store,
        &command.run_id,
        &mut successor_effects,
        command.committed_at_ms,
    )?;
    let mut delegated_permission_mutations = Vec::new();
    delegated_permission::prepare_effects(
        store,
        &command.run_id,
        &mut successor_effects,
        &mut delegated_permission_mutations,
        command.committed_at_ms,
    )?;
    let canonical_pending_effect_ids = checkpoint
        .state
        .host
        .pending_effects
        .iter()
        .map(|effect| effect.effect_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let cancellation_target_ids = checkpoint.state.host.pending_effects.iter()
        .map(|effect| effect.validated_control())
        .collect::<Result<Vec<_>,_>>()?
        .into_iter().filter_map(|control| match &control.payload.policy {
            nola_domain::host_control::HostResponsePolicy::Cancel {target_effect_id,..} => Some(target_effect_id.0.clone()),
            _ => None,
        }).collect::<std::collections::HashSet<_>>();
    // Boundary submissions such as a durable Task completion can settle a
    // Host effect without consuming another Host response. The Core journal
    // is authoritative for that lifecycle transition, while the Application
    // outbox is authoritative for executable delivery. Reconcile both in the
    // same transaction so a settled promoted effect cannot be replayed and
    // accidentally complete its original tool call a second time.
    let settled_effect_ids = if terminal_status.is_some() {
        // A terminal Turn makes every previously executable effect obsolete.
        // Cancellation cannot reach this branch until its target and control
        // effects have both been acknowledged and removed from the checkpoint.
        pending_effect_ids
    } else {
        pending_effect_ids
            .into_iter()
            .filter(|effect_id| {
                !canonical_pending_effect_ids.contains(effect_id.as_str())
                    || cancellation_target_ids.contains(effect_id.as_str())
            })
            .collect::<Vec<_>>()
    };
    let terminal_cleanup_pending = terminal_status.is_some() && !preserve_unknown && (!successor_effects.is_empty()
        || checkpoint.state.host.pending_effects.iter().any(|effect| effect.target_effect_id.is_some()));
    if active_run.kind=="maintenance" { maintenance_turn::validate_maintenance_checkpoint(&command.run_id,&checkpoint,&committed_events)?; }
    let execution_status = checkpoint_execution_status(&checkpoint);
    model_attempt_mutations.extend(model_attempt::settle_effects(
        store,
        &settled_effect_ids,
        command.committed_at_ms,
    )?);
    delegated_invocation_mutations.extend(delegated_invocation::settle_effects(
        store,
        &settled_effect_ids,
        command.committed_at_ms,
    )?);
    delegated_permission_mutations.extend(delegated_permission::settle_effects(
        store,
        &settled_effect_ids,
        command.committed_at_ms,
    )?);
    model_attempt::append_events(
        &mut events,
        &mut next_sequence,
        &model_attempt_mutations,
        command.committed_at_ms,
    )?;
    delegated_invocation::append_events(
        &mut events,
        &mut next_sequence,
        &delegated_invocation_mutations,
        command.committed_at_ms,
    )?;
    delegated_permission::append_events(
        &mut events,
        &mut next_sequence,
        &delegated_permission_mutations,
        command.committed_at_ms,
    )?;
    let input = CommitTransitionInput {
            effect_delivery_attempt_mutations: cancellation_delivery_mutations,
            transition_id: command.transition_id,
            run_id: command.run_id,
            expected_checkpoint_revision: expected_revision,
            checkpoint_revision: expected_revision.checked_add(1).ok_or("Core checkpoint revision exhausted")?,
            checkpoint_event_cursor: i64::try_from(checkpoint.last_event_cursor.sequence)
                .map_err(|_| "Core checkpoint event cursor exceeds the durable range")?,
            checkpoint: checkpoint_write,
            submission_reduction_json: Some(submission_reduction_json), host_event: None,
            events,
            ensured_messages,
            part_message_owners,
            assistant_turn_bindings: assistant_turn_bindings.into_bindings(),
            part_mutations,
            model_attempt_mutations,
            delegated_invocation_mutations,
            delegated_permission_mutations,
            outbox_effects: successor_effects,
            acknowledged_effect_ids: settled_effect_ids,
            memory_usage_entries,
            run_status: terminal_status
                .filter(|_| !terminal_cleanup_pending)
                .map(|_| "sealed")
                .or(execution_status)
                .map(str::to_string),
            assistant_status: terminal_status.or(execution_status).filter(|_|active_run.kind!="maintenance").map(str::to_string),
            assistant_status_message_id,
            committed_at_ms: command.committed_at_ms,
        };
    let commit = store.commit_transition(&input)
        .map_err(|error| format!("commit active Core submission failed: {error}"))?;
    let mut commit: TransitionCommit = commit.into();
    if commit.applied {
        commit.assistant_publications = publish_transition_journal(store, &command.conversation_id, &checkpoint,
            &committed_events, &input, active_run.kind == "maintenance")?;
        execution_trace_journal::record_accepted(store, &command.conversation_id, &transition.trace_events)?;
    }
    drop(checkpoint);
    let accepted = AcceptedActiveSubmission {
        commit: commit.into(),
        events: transition.event_envelopes,
        traces: transition.trace_events,
        control: core.control_snapshot(),
    };
    Ok((accepted, core))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReduceAndCommitIdleSubmission {
    pub conversation_id: String,
    pub core_config: AgentCoreConfig,
    pub submission: AgentSubmission,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedIdleSubmission {
    pub assistant_publications: Vec<nola_domain::assistant_journal::AssistantJournalCursor>,
    pub execution: HostExecutionState,
    pub events: Vec<CoreEventEnvelope<AgentEvent>>,
    pub traces: Vec<AgentTraceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevertConversation {
    pub conversation_id: String,
    pub core_config: AgentCoreConfig,
    pub operation_id: String,
    pub through_message_id: String,
    pub keep_message: bool,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedConversationRevert {
    pub execution: HostExecutionState,
    pub projection: ConversationProjectionChange,
    pub events: Vec<CoreEventEnvelope<AgentEvent>>,
    pub traces: Vec<AgentTraceEvent>,
}

pub use nola_domain::CanonicalBranchIntent as CreateConversationBranch;

pub fn create_conversation_branch(
    store: &mut impl nola_store_port::canonical_branch::CanonicalBranchRepository,
    command: CreateConversationBranch,
) -> Result<ConversationBranchCreation, String> {
    store
        .create_conversation_branch(&command)
        .map_err(|error| format!("commit canonical branch failed: {error}"))
}

fn durable_conversation_items<
    S: nola_store_port::historical_part_projection::HistoricalPartProjectionRepository + ?Sized,
>(
    store: &S,
    conversation: &nola_store_port::conversation_history::StoredConversation,
    boundary_message_id: Option<&str>,
) -> Result<(Vec<AgentItem>, BTreeMap<String, AgentItemOwnership>, String), String> {
    use historical_part_projection::context_item_id;
    let mut items = Vec::new();
    let mut semantic_budget = 32768usize;
    // Read once within the shared selection budget, then resolve cross-Part
    // references independently of original Part ordering.
    let historical = conversation
        .messages
        .iter()
        .map(|message| {
            message
                .parts
                .iter()
                .map(|part| {
                    historical_part_projection::read(
                        store,
                        conversation,
                        message,
                        part,
                        &mut semantic_budget,
                    )
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut historical_tools=BTreeMap::new();
    for semantic in historical.iter().flatten().filter_map(Option::as_ref) {
        if semantic.canonical_kind=="tool" && semantic.status==nola_domain::HistoricalProjectionStatus::Supported {
            historical_tools.insert(semantic.owner.part_id().to_owned(),tool_source::historical_items(
                store,&semantic.owner,conversation.revision,&mut semantic_budget)?);
        }
    }
    let references = historical_context_references::HistoricalReferenceIndex::new(
        historical.iter().flatten().filter_map(Option::as_ref),&historical_tools,
    );
    let mut ownership = BTreeMap::new();
    let mut boundary_item_id = None;
    for (index, message) in conversation.messages.iter().enumerate() {
        let turn_id = message
            .run_id
            .clone()
            .or_else(|| {
                conversation
                    .messages
                    .get(index + 1)
                    .filter(|next| next.role == "assistant")
                    .and_then(|next| next.run_id.clone())
            })
            .unwrap_or_else(|| format!("{}:durable-turn:{}", conversation.id, message.ordinal));
        let item_ownership = AgentItemOwnership {
            source: Default::default(),
            turn_id,
            message_id: message.id.clone(),
        };
        let mut message_items = Vec::new();
        let mut item_sources = BTreeMap::new();
        for (part, semantic) in message.parts.iter().zip(&historical[index]) {
            // This is an explicit canonical kind, never a guess based on user JSON.
            // Action envelopes have their own admission budget and must not be silently
            // truncated by the generic historical display-field selection budget.
            if part.kind == "ui_action" {
                let raw = part.content_json.as_deref().ok_or("UI action canonical body is unavailable")?;
                let nola_domain::MessagePartView::UiAction { surface, envelope, summary, .. } =
                    serde_json::from_str(raw).map_err(|error| format!("Invalid UI action Part: {error}"))?
                    else { return Err("UI action Part kind mismatch".into()); };
                let action = nola_domain::generative_ui::UiActionContent { surface, envelope, summary };
                action.validate().map_err(|error| error.to_string())?;
                message_items.push(AgentItem::UiAction {
                    id: ItemId::from_string(context_item_id(&conversation.id, &message.id, Some(&part.id), "part")), action,
                });
                continue;
            }
            use historical_part_projection::text;
            use nola_domain::{HistoricalPartSchema, HistoricalProjectionStatus};
            let Some(semantic) = semantic else {
                if !part.content_text.is_empty() {
                    message_items.push(AgentItem::Message {
                        id: ItemId::from_string(context_item_id(
                            &conversation.id,
                            &message.id,
                            Some(&part.id),
                            "part",
                        )),
                        role: durable_role(&message.role).unwrap_or(AgentRole::Assistant),
                        content: part.content_text.clone(),
                    });
                }
                continue;
            };
            let before = message_items.len();
            if semantic.status == HistoricalProjectionStatus::Supported {
                let agent = semantic.schema == Some(HistoricalPartSchema::AgentPart);
                match semantic.canonical_kind.as_str() {
                    "text" => if let Some(content) = text(&semantic, if agent { "text" } else { "markdown" }) {
                        message_items.push(AgentItem::Message { id: ItemId::from_string(context_item_id(&conversation.id, &message.id, Some(&part.id), "part")),
                            role: durable_role(&message.role).unwrap_or(AgentRole::Assistant), content: content.into() });
                    },
                    "reasoning" => if let Some(content) = text(&semantic, if agent { "detail" } else { "summary" }) {
                        message_items.push(AgentItem::Reasoning { id: ItemId::from_string(context_item_id(&conversation.id, &message.id, Some(&part.id), "part")),
                            summary: text(&semantic, "title").unwrap_or_default().into(), content: content.into(), provider_continuation: None });
                    },
                    "tool" => {
                        let items=historical_tools.remove(&part.id).unwrap_or_default();
                        let original_call=text(semantic,if agent {"call_id"}else{"tool_call_id"});
                        let exact=original_call.and_then(|call|references.call(semantic,call)).is_some_and(|mapped|
                            matches!(items.first(),Some(AgentItem::ToolCall {call_id,..}) if call_id.as_str()==mapped));
                        if exact {
                            let source = {
                                let value = historical_part_projection::value(semantic, "execution_source")
                                    .ok_or("historical tool execution source is missing or deferred")?;
                                let source: nola_domain::code_mode::CodeItemSource = serde_json::from_value(
                                    serde_json::to_value(value).map_err(|error| error.to_string())?
                                ).map_err(|error| format!("invalid historical tool execution source: {error}"))?;
                                if let nola_domain::code_mode::CodeItemSource::CodeCell { cell } = &source {
                                    cell.validate()?;
                                }
                                source
                            };
                            for item in &items {
                                item_sources.insert(item.id().as_str().to_owned(), source.clone());
                            }
                            message_items.extend(items);
                        }
                    },
                    "source" | "citation" | "attachment" | "artifact" | "task" | "step" => {
                        match references.evidence(semantic) {
                            Ok(Some(content)) => message_items.push(AgentItem::Message {
                                id: ItemId::from_string(context_item_id(&conversation.id, &message.id, Some(&part.id), "part")), role: AgentRole::Assistant, content,
                            }),
                            Err(reason) => message_items.push(AgentItem::Message {
                                id: ItemId::from_string(context_item_id(&conversation.id, &message.id, Some(&part.id), "note")), role: AgentRole::System,
                                content: format!("Historical {} Part {} has an unsupported reference ({reason}); original content remains available through conversation recall.", semantic.canonical_kind, part.id),
                            }),
                            Ok(None) => {},
                        }
                    },
                    "compaction" => if let (Some(summary), Some(source_ids), Some(source_fingerprint), Some(original_token_count)) = (
                        text(&semantic, "summary"),
                        historical_part_projection::string_array(&semantic, "retained_item_ids"),
                        historical_part_projection::sha256(&semantic, "source_fingerprint"),
                        historical_part_projection::unsigned(&semantic, "original_token_count"),
                    ) {
                        let replaced_item_count = source_ids.len();
                        message_items.push(AgentItem::Compaction {
                            id: ItemId::from_string(context_item_id(&conversation.id, &message.id, Some(&part.id), "part")),
                            summary: summary.into(),
                            replaced_item_count,
                            source_item_ids: source_ids.into_iter().map(ItemId::from_string).collect(),
                            source_fingerprint: source_fingerprint.into(),
                            original_token_count: usize::try_from(original_token_count).unwrap_or(usize::MAX),
                        });
                    },
                    _ => {}
                }
            }
            if message_items.len() == before {
                message_items.push(AgentItem::Message {
                    id: ItemId::from_string(context_item_id(&conversation.id, &message.id, Some(&part.id), "note")), role: AgentRole::System,
                    content: format!("Historical {} Part {} in message {} was omitted from model context ({:?}). Original content remains available through conversation recall; no historical tool was resumed.",
                        semantic.canonical_kind, part.id, message.id, semantic.status),
                });
            }
        }
        if message_items.is_empty() {
            if let Some(role) = durable_role(&message.role) {
                let content = message
                    .parts
                    .iter()
                    .map(|part| part.content_text.as_str())
                    .collect::<Vec<_>>()
                    .join("");
                message_items.push(AgentItem::Message {
                    id: ItemId::from_string(context_item_id(
                        &conversation.id,
                        &message.id,
                        None,
                        "message",
                    )),
                    role,
                    content,
                });
            }
        }
        for item in message_items {
            let item_id = item.id().as_str().to_string();
            let mut item_ownership = item_ownership.clone();
            if let Some(source) = item_sources.remove(&item_id) {
                item_ownership.source = source;
            }
            if ownership
                .insert(item_id.clone(), item_ownership.clone())
                .is_some()
            {
                return Err("duplicate historical context item identity".into());
            }
            if boundary_message_id == Some(message.id.as_str()) {
                boundary_item_id = Some(item_id);
            }
            items.push(item);
        }
    }
    let boundary_item_id = match boundary_message_id {
        Some(message_id) => boundary_item_id
            .ok_or_else(|| format!("unknown durable message boundary {message_id}"))?,
        None => String::new(),
    };
    Ok((items, ownership, boundary_item_id))
}

fn durable_role(role: &str) -> Option<AgentRole> {
    match role {
        "system" => Some(AgentRole::System),
        "developer" => Some(AgentRole::Developer),
        "user" => Some(AgentRole::User),
        "assistant" => Some(AgentRole::Assistant),
        _ => None,
    }
}

pub fn revert_conversation<S>(
    store: &mut S,
    command: RevertConversation,
) -> Result<AcceptedConversationRevert, String>
where
    S: nola_store_port::conversation_history::CanonicalHistoryRepository
        + nola_store_port::run_effect::RunEffectRepository
        + nola_store_port::conversation_revert::ConversationRevertRepository
        + nola_store_port::trace_journal::TraceJournalRepository
        + nola_store_port::protocol_journal::CommandProtocolRepositoryProvider
        + ?Sized,
{
    assistant_publication_transaction::with_publication_transaction(store, |store|
        revert_conversation_in_transaction(store, command))
}

fn revert_conversation_in_transaction<S>(
    store: &mut S,
    command: RevertConversation,
) -> Result<AcceptedConversationRevert, String>
where
    S: nola_store_port::conversation_history::CanonicalHistoryRepository
        + nola_store_port::run_effect::RunEffectRepository
        + nola_store_port::conversation_revert::ConversationRevertRepository
        + nola_store_port::trace_journal::TraceJournalRepository
        + nola_store_port::protocol_journal::CommandProtocolRepositoryProvider
        + ?Sized,
{
    if command.operation_id.trim().is_empty() {
        return Err("conversation revert operation_id cannot be empty".to_string());
    }
    if command.through_message_id.trim().is_empty() {
        return Err("conversation revert boundary cannot be empty".to_string());
    }
    if store
        .active_run(&command.conversation_id)
        .map_err(|error| format!("load active Run failed: {error}"))?
        .is_some()
    {
        return Err("conversation revert cannot overlap an active Run".to_string());
    }
    let boundary = store
        .conversation_revert_boundary(
            &command.conversation_id,
            &command.through_message_id,
            command.keep_message,
        )
        .map_err(|error| format!("load canonical revert boundary failed: {error}"))?;
    let mutations = boundary
        .hidden_message_ids
        .iter()
        .enumerate()
        .map(|(index, message_id)| {
            nola_store_port::conversation_revert::MessageProjectionMutation {
                event_id: format!("{}:hide:{index}:{message_id}", command.operation_id),
                message_id: message_id.clone(),
                visible: false,
                reason: "conversation_revert".into(),
            }
        })
        .collect();
    let prompt_token_limit = command
        .core_config
        .context_window
        .clone()
        .normalized()
        .prompt_token_limit();
    let stored = load_current_checkpoint(store, &command.conversation_id)?;
    let expected_revision = stored.as_ref().map_or(0, |checkpoint| checkpoint.revision);
    let mut core = match stored {
        Some(stored) => {
            let checkpoint = decode_idle_checkpoint(&stored.checkpoint_json)?;
            AgentCore::recover(command.core_config, checkpoint)
                .map_err(|error| format!("restore Core for idle submission failed: {error}"))?
        }
        None => AgentCore::try_new(command.core_config)
            .map_err(|error| format!("initialize Core for idle submission failed: {error}"))?,
    };
    if core.control_snapshot().history_item_count == 0 {
        let before_ordinal = boundary
            .boundary_ordinal
            .checked_add(1)
            .ok_or_else(|| "conversation revert boundary ordinal overflow".to_owned())?;
        canonical_branch::restore_canonical_context_before(
            store,
            &command.conversation_id,
            &mut core,
            prompt_token_limit,
            Some(before_ordinal),
        )?;
    }
    let (core, transition) = core
        .reduce_candidate(
            AgentSubmission::RevertConversation {
                through_message_id: command.through_message_id,
                keep_message: command.keep_message,
                submission_id: Some(command.operation_id.clone()),
            },
            decision_time_ms(command.committed_at_ms)?,
        )
        .map_err(|error| format!("reduce Core conversation revert failed: {error}"))?;
    if !transition.host_requests.is_empty() {
        return Err("conversation revert attempted an external Host effect".to_string());
    }
    let execution = core.host_execution_state();
    let checkpoint = core.into_checkpoint(transition.durability.as_ref()
        .map(|intent| intent.checkpoint_reason).unwrap_or(CheckpointReason::Manual));
    let checkpoint_json = serde_json::to_string(&checkpoint)
        .map_err(|error| format!("encode idle Core checkpoint failed: {error}"))?;
    let committed = store
        .commit_checkpoint_projection(
            &nola_store_port::conversation_revert::CommitCheckpointProjectionInput {
                checkpoint: nola_store_port::conversation_revert::SaveCheckpointInput {
                    conversation_id: command.conversation_id.clone(),
                    expected_revision,
                    revision: expected_revision + 1,
                    event_cursor: checkpoint.last_event_cursor.sequence as i64,
                    checkpoint_json,
                    updated_at_ms: command.committed_at_ms,
                },
                projection:
                    nola_store_port::conversation_revert::ApplyConversationProjectionInput {
                        operation_id: command.operation_id,
                        conversation_id: command.conversation_id.clone(),
                        expected_revision: boundary.conversation_revision,
                        operation_kind: "conversation_revert".to_string(),
                        parent_message_id: boundary.parent_message_id,
                        mutations,
                        appended_messages: Vec::new(),
                        created_at_ms: command.committed_at_ms,
                    },
            },
        )
        .map_err(|error| format!("commit idle Core projection failed: {error}"))?;
    execution_trace_journal::record_accepted(store, &command.conversation_id, &transition.trace_events)?;
    Ok(AcceptedConversationRevert {
        execution,
        projection: committed.projection.into(),
        events: transition.event_envelopes,
        traces: transition.trace_events,
    })
}

/// Session controls read settled history, not a previous Run's Host binding.
fn decode_idle_checkpoint(raw: &str) -> Result<CoreCheckpoint<AgentThreadState>, String> {
    let value = serde_json::from_str(raw)
        .map_err(|error| format!("decode idle Core checkpoint JSON failed: {error}"))?;
    CoreCheckpoint::decode_history_for_new_turn(value)
}

pub fn reduce_and_commit_idle_submission<
    R: nola_store_port::memory::IdleCheckpointRepository + repositories::AssistantPublicationRepositories + ?Sized,
>(
    store: &mut R,
    command: ReduceAndCommitIdleSubmission,
) -> Result<AcceptedIdleSubmission, String> {
    goal_execution::reject_unbound_goal_start(&command.submission)?;
    assistant_publication_transaction::with_publication_transaction(store, |store|
        reduce_and_commit_idle_submission_in_transaction(store, command))
}

pub(crate) fn reduce_and_commit_idle_submission_in_transaction<R: nola_store_port::memory::IdleCheckpointRepository + repositories::AssistantPublicationRepositories + ?Sized>(
    store: &mut R,
    command: ReduceAndCommitIdleSubmission,
) -> Result<AcceptedIdleSubmission, String> {
    if matches!(&command.submission, AgentSubmission::ResolveUnknownEffects { .. }) {
        return Err("unknown effect recovery requires the exact active Run recovery command".into());
    }
    if matches!(command.submission, AgentSubmission::PromoteQueuedInput { .. }) {
        return Err("queued successor requires independent Run admission".into());
    }
    let completed_memory_job_id = match &command.submission {
        AgentSubmission::MemoryWorkerCompleted { job_id, .. } => Some(job_id.clone()),
        AgentSubmission::MemoryRuntime { envelope } => match &envelope.command {
            nola_agent_engine::agent::MemoryRuntimeCommand::Complete { job_id, .. } => {
                Some(job_id.clone())
            }
            _ => None,
        },
        _ => None,
    };
    if store
        .active_run(&command.conversation_id)
        .map_err(|error| format!("load active Run failed: {error}"))?
        .is_some()
    {
        return Err("idle Core submission cannot overlap an active Run".to_string());
    }
    let stored = load_current_checkpoint(store, &command.conversation_id)?;
    let expected_revision = stored.as_ref().map_or(0, |checkpoint| checkpoint.revision);
    let checkpoint = stored
        .map(|stored| {
            decode_idle_checkpoint(&stored.checkpoint_json)
        })
        .transpose()?;
    let memory_preconditions = crate::memory::idle_memory_input_preconditions(
        store,
        &command.submission,
        checkpoint
            .as_ref()
            .map(|checkpoint| &checkpoint.state.memory_runtime),
    )?;
    let core = match checkpoint {
        Some(checkpoint) => AgentCore::recover(command.core_config, checkpoint)
            .map_err(|error| format!("restore Core for idle submission failed: {error}"))?,
        None => AgentCore::try_new(command.core_config)
            .map_err(|error| format!("initialize Core for idle submission failed: {error}"))?,
    };
    let (core, transition) = core
        .reduce_candidate(
            command.submission,
            decision_time_ms(command.committed_at_ms)?,
        )
        .map_err(|error| format!("reduce idle Core submission failed: {error}"))?;
    if !transition.host_requests.is_empty() {
        return Err("idle Core submission attempted an external Host effect".to_string());
    }
    let execution = core.host_execution_state();
    let checkpoint = core.into_checkpoint(transition.durability.as_ref()
        .map(|intent| intent.checkpoint_reason).unwrap_or(CheckpointReason::Manual));
    let memory_usage_entries = unrecorded_checkpoint_usage(store, &checkpoint)?;
    let checkpoint_json = serde_json::to_string(&checkpoint)
        .map_err(|error| format!("encode idle Core checkpoint failed: {error}"))?;
    let memory_records = completed_memory_job_id
        .as_deref()
        .map(|job_id| {
            checkpoint
                .state
                .memory_runtime
                .job(job_id)
                .ok_or_else(|| format!("completed memory job {job_id} is missing from checkpoint"))?
                .output
                .as_ref()
                .ok_or_else(|| format!("completed memory job {job_id} has no accepted output"))
                .and_then(|output| {
                    crate::memory::validated_job_output_store_records(
                        output,
                        command.committed_at_ms,
                    )
                })
        })
        .transpose()?
        .unwrap_or_default();
    store
        .save_checkpoint_with_memories(
            &SaveCheckpointInput {
                conversation_id: command.conversation_id.clone(),
                expected_revision,
                revision: expected_revision + 1,
                event_cursor: checkpoint.last_event_cursor.sequence as i64,
                checkpoint_json,
                updated_at_ms: command.committed_at_ms,
            },
            &memory_records,
            &memory_preconditions,
            &memory_usage_entries,
        )
        .map_err(|error| format!("commit idle Core checkpoint failed: {error}"))?;
    let changes = assistant_journal_changes::collect_assistant_journal_changes(
        assistant_journal_changes::AssistantJournalChangeScope {
            conversation_id: &command.conversation_id, core_session_id: &checkpoint.session_id,
            source_run_id: None, maintenance: false,
        }, &transition.event_envelopes, &checkpoint.state.skill_runtime, &[], &[],
    )?;
    let publication = assistant_journal_producer::publish_changed_assistant_journal(
        store, nola_store_port::assistant_journal::AssistantSourceCommitStamp::IdleCheckpoint {
            conversation_id: command.conversation_id.clone(), checkpoint_revision: expected_revision + 1,
            event_cursor: checkpoint.last_event_cursor.sequence as i64,
        }, &command.conversation_id, &checkpoint.session_id, None, &changes,
    )?;
    execution_trace_journal::record_accepted(store, &command.conversation_id, &transition.trace_events)?;
    Ok(AcceptedIdleSubmission {
        assistant_publications: assistant_journal_producer::publication_cursors(publication),
        execution,
        events: transition.event_envelopes,
        traces: transition.trace_events,
    })
}

/// Restores Core from the canonical checkpoint, reduces one raw Host response,
/// and atomically commits the resulting facts before exposing them to Runtime.
pub fn reduce_and_accept_host_response<
    R: RunEffectRepository + EffectDeliveryRepository + repositories::AssistantPublicationRepositories + ?Sized,
>(
    store: &mut R,
    command: ReduceAndAcceptHostResponse,
) -> Result<AcceptedHostResponse, String> {
    assistant_publication_transaction::with_publication_transaction(store, |store| {
        if let Some(accepted) = reconcile_committed_terminal_response(store, &command)? {
            return Ok(accepted);
        }
        reduce_and_accept_host_response_with_core(
        store,
        command,
        None,
        None,
        None,
        None,
        Vec::new(),
        true,
        true,
    )
    .map(|(mut accepted, _, _)| {
        accepted.events.retain(|event| !matches!(event.event, AgentEvent::PartUpdated { .. }));
        accepted
    })
    })
}

/// Reconcile acceptance before touching executable Core state. In particular,
/// a duplicate must not take/flush a newer hot reducer's pending publication.
pub(crate) fn reconcile_committed_terminal_response<R: RunEffectRepository + ?Sized>(
    store: &R,
    command: &ReduceAndAcceptHostResponse,
) -> Result<Option<AcceptedHostResponse>, String> {
    if !command.response.payload.is_terminal() { return Ok(None); }
    let event_id = format!("host:{}:{}:{}", command.response.session_id,
        command.response.effect_id.0, command.response.sequence);
    let response_json = serde_json::to_string(&command.response).map_err(|e| e.to_string())?;
    let Some(receipt) = store.committed_terminal_host_response(&command.conversation_id, &command.run_id,
        &event_id, &response_json).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    validate_run_invocation_config_digest(store, &command.run_id, &command.invocation_config_digest)?;
    Ok(Some(AcceptedHostResponse {
        commit: TransitionCommit {
            assistant_publications: Vec::new(), applied: false,
            checkpoint_revision: receipt.checkpoint_revision, next_event_sequence: receipt.next_event_sequence,
        },
        events: Vec::new(), traces: Vec::new(), control: None,
    }))
}

fn load_exact_run_checkpoint<R: RunEffectRepository + ?Sized>(
    store: &R, conversation_id: &str, run_id: &str,
) -> Result<nola_store_port::run_effect::StoredRecoveryCheckpoint, String> {
    let recovery = store.load_recovery_checkpoint(conversation_id)
        .map_err(|error| format!("load Core recovery checkpoint failed: {error}"))?
        .ok_or("Run has no canonical Core checkpoint")?;
    if recovery.checkpoint.conversation_id != conversation_id
        || recovery.replay_base.as_ref().is_some_and(|base|
            base.run_id != run_id || base.checkpoint_revision > recovery.checkpoint.revision || base.through_ordinal < 0)
    {
        return Err("Core recovery checkpoint replay owner mismatch".into());
    }
    if recovery.replay_base.is_none() && !recovery.replay_inputs.is_empty() {
        return Err("Core recovery inputs have no Run-bound replay prefix".into());
    }
    Ok(recovery)
}

fn restore_run_recovery<R: RunEffectRepository + ?Sized>(
    store: &R, run_id: &str, invocation_config_digest: &str,
    recovery: &nola_store_port::run_effect::StoredRecoveryCheckpoint,
) -> Result<AgentCore, String> {
    restore_run_recovery_with_reason(store, run_id, invocation_config_digest, recovery)
        .map(|(core, _)| core)
}

fn restore_run_recovery_with_reason<R: RunEffectRepository + ?Sized>(
    store: &R, run_id: &str, invocation_config_digest: &str,
    recovery: &nola_store_port::run_effect::StoredRecoveryCheckpoint,
) -> Result<(AgentCore, CheckpointReason), String> {
    let (core, reason) = restore_run_recovery_baseline(store, run_id, invocation_config_digest, recovery)?;
    replay_recovery_inputs_with_reason(core, recovery, reason)
}

fn restore_run_recovery_baseline<R: RunEffectRepository + ?Sized>(
    store: &R, run_id: &str, invocation_config_digest: &str,
    recovery: &nola_store_port::run_effect::StoredRecoveryCheckpoint,
) -> Result<(AgentCore, CheckpointReason), String> {
    let checkpoint = serde_json::from_str::<CoreCheckpoint<AgentThreadState>>(
        &recovery.checkpoint.checkpoint_json,
    ).map_err(|error| format!("decode canonical Core checkpoint failed: {error}"))?;
    let mut config = load_run_invocation_config(store, run_id, invocation_config_digest)?;
    delegated_invocation::bind_stored_restore_configuration(store, &mut config, &checkpoint, run_id)?;
    let reason = checkpoint.reason;
    let core = AgentCore::recover(config, checkpoint)
        .map_err(|error| format!("restore Core recovery baseline failed: {error}"))?;
    Ok((core, reason))
}

/// One pure checkpoint materializer for execution, display and exact-digest
/// readers. Preserve every header field, including the last reducer's reason;
/// never replace it with a caller-invented Manual reason or replay twice.
pub(crate) fn materialize_replay_checkpoint_json(
    core: AgentCore,
    recovery: &nola_store_port::run_effect::StoredRecoveryCheckpoint,
    baseline_reason: CheckpointReason,
) -> Result<String, String> {
    let (core, reason) = replay_recovery_inputs_with_reason(core, recovery, baseline_reason)?;
    serde_json::to_string(&core.checkpoint_view(reason))
        .map_err(|error| format!("encode recovered checkpoint failed: {error}"))
}

/// Reconstruct archived work for inspection without importing it into a Store,
/// emitting Host effects, restoring permission grants, or starting workers.
/// The caller must first verify the archive manifest and supply current config.
pub fn inspect_archived_work(
    config: AgentCoreConfig,
    recovery: &nola_store_port::run_effect::StoredRecoveryCheckpoint,
) -> Result<serde_json::Value, String> {
    let checkpoint: CoreCheckpoint<AgentThreadState> =
        serde_json::from_str(&recovery.checkpoint.checkpoint_json).map_err(|e|e.to_string())?;
    let reason = checkpoint.reason;
    let core = AgentCore::recover(config, checkpoint)?;
    let materialized = materialize_replay_checkpoint_json(core,recovery,reason)?;
    let restored: serde_json::Value = serde_json::from_str(&materialized).map_err(|e|e.to_string())?;
    Ok(serde_json::json!({
        "conversation_id":recovery.checkpoint.conversation_id,
        "goal":restored["state"]["goal_projection"],
        "tasks":restored["state"]["task_projection"],
        "transcript":restored["state"]["conversation"]["transcript"],
        "requires_permission_revalidation":true,
        "execution_resumed":false
    }))
}

fn replay_recovery_inputs(
    core: AgentCore,
    recovery: &nola_store_port::run_effect::StoredRecoveryCheckpoint,
) -> Result<AgentCore, String> {
    replay_recovery_inputs_with_reason(core, recovery, CheckpointReason::Manual)
        .map(|(core, _)| core)
}

fn replay_recovery_inputs_with_reason(
    core: AgentCore,
    recovery: &nola_store_port::run_effect::StoredRecoveryCheckpoint,
    checkpoint_reason: CheckpointReason,
) -> Result<(AgentCore, CheckpointReason), String> {
    replay_recovery_inputs_collect(core, recovery, checkpoint_reason, None)
}

fn replay_recovery_inputs_collect(
    mut core: AgentCore,
    recovery: &nola_store_port::run_effect::StoredRecoveryCheckpoint,
    mut checkpoint_reason: CheckpointReason,
    mut events: Option<&mut Vec<CoreEventEnvelope<AgentEvent>>>,
) -> Result<(AgentCore, CheckpointReason), String> {
    if recovery.replay_base.is_none() && !recovery.replay_inputs.is_empty() {
        return Err("Core recovery inputs have no Run-bound replay prefix".into());
    }
    let mut ordinal = recovery.replay_base.as_ref().map_or(0, |base| base.through_ordinal);
    for input in &recovery.replay_inputs {
        if ordinal.checked_add(1) != Some(input.ordinal) {
            return Err("Core recovery input sequence has a gap".into());
        }
        let (submission, decision_time) = match input.kind {
            nola_store_port::run_effect::ReplayInputKind::Submission => {
                let facts: nola_store_port::run_effect::SubmissionReplayReduction =
                    serde_json::from_str(&input.reduction_json)
                    .map_err(|error| format!("decode submission recovery facts failed: {error}"))?;
                if facts.protocol_version != 1 || facts.decision_time_ms != input.received_at_ms
                    || facts.session_id != core.checkpoint_view(CheckpointReason::Manual).session_id.as_str()
                {
                    return Err("submission recovery identity or decision clock mismatch".into());
                }
                let submission: AgentSubmission = serde_json::from_str(&facts.submission_json)
                    .map_err(|error| format!("decode recorded submission failed: {error}"))?;
                if matches!(submission, AgentSubmission::HostResponse { .. }) {
                    return Err("Host response cannot bypass its recovery binding checks".into());
                }
                for binding in &facts.bindings {
                    let mut control = core.committed_host_effect_control(&binding.effect_id.0)?
                        .ok_or("submission recovery binding has no pending effect")?;
                    control.apply_binding(binding)?;
                    core.bind_committed_host_control(&control)?;
                }
                (submission, decision_time_ms(facts.decision_time_ms)?)
            }
            nola_store_port::run_effect::ReplayInputKind::HostResponse => {
        let (response, binding, decision_time) = host_replay::decode_reduction(input)?;
        if let Some(binding) = binding {
            let mut request = core.committed_host_effect_control(&response.effect_id.0)?
                .ok_or("Core recovery binding has no pending effect")?;
            request.apply_binding(&binding)?;
            core.bind_committed_host_response_control(&request)
                .map_err(|error| format!("replay committed Host binding failed: {error}"))?;
        }
        let request = core.committed_host_effect_control(&response.effect_id.0)?
            .ok_or("Core recovery response has no pending effect")?;
        if request.request_id != response.request_id
            || request.session_id != response.session_id
            // The original App/Core response gate permits a missing Turn echo
            // only for the same exact unleased in-process effect. Core fills
            // it from that pending effect; leased Hosts remain strict here.
            || (request.turn_id != response.turn_id
                && !(response.turn_id.is_none() && request.lease_id.is_none()))
            || request.host_id != response.host_id
            || request.lease_id != response.lease_id
            || request.catalog_revision != response.catalog_revision
            || request.availability_epoch != response.availability_epoch
            || request.delegated_invocation_id != response.delegated_invocation_id
            || request.attempt_id != response.attempt_id
            || request.resume_after_sequence.is_some_and(|last| response.sequence <= last)
            || !request.payload.accepts_response(&response.payload)
        {
            return Err("Core recovery response does not match its frozen request".into());
        }
                (AgentSubmission::HostResponse { response }, decision_time)
            }
        };
        // Only rebuild state: committed outbox/publication rows already own the
        // effects of this reduction. Never execute or republish them on replay.
        let (next, transition) = core.reduce_candidate(submission, decision_time)
            .map_err(|error| format!("replay committed Core input failed: {error}"))?;
        checkpoint_reason = transition.durability.as_ref()
            .map(|intent| intent.checkpoint_reason).unwrap_or(CheckpointReason::Manual);
        if let Some(events) = events.as_mut() {
            events.extend(transition.event_envelopes);
        }
        core = next;
        ordinal = input.ordinal;
    }
    for raw in &recovery.frozen_model_requests {
        let request: HostRequestEnvelope = serde_json::from_str(raw)
            .map_err(|error| format!("decode frozen recovery model input failed: {error}"))?;
        if recovery.replay_base.as_ref().map(|base| base.run_id.as_str()) != request.turn_id.as_deref() {
            return Err("frozen recovery model input belongs to another Run".into());
        }
        core.restore_committed_model_input(&request)?;
    }
    let restored = core.checkpoint_view(CheckpointReason::Manual);
    // Store CAS revision counts transactions; Core revision counts reductions.
    // A batch contains several reductions, so these revisions are not equal.
    if i64::try_from(restored.last_event_cursor.sequence).ok() != Some(recovery.checkpoint.event_cursor)
    {
        return Err("Core recovery did not reproduce the committed event cursor".into());
    }
    Ok((core, checkpoint_reason))
}

/// Restore only the accepted pre-command tail after a confirmed rollback.
/// Original normalized bindings and decision times make replay deterministic;
/// none of its effects/publications leave this function.
fn restore_hot_rollback<R: RunEffectRepository + ?Sized>(
    store: &R,
    capsule: runtime_supervisor::HotRollbackCapsule,
) -> Result<runtime_supervisor::RetainHotRun, String> {
    use nola_store_port::run_effect::{ReplayInputKind, StoredCheckpointReplayBase, StoredCoreReplayInput};
    let mut recovery = load_exact_run_checkpoint(store, &capsule.conversation_id, &capsule.run_id)?;
    if recovery.checkpoint.revision != capsule.checkpoint_revision {
        return Err("hot rollback canonical checkpoint changed; reopen required".into());
    }
    let core = restore_run_recovery(store, &capsule.run_id, &capsule.invocation_config_digest, &recovery)?;
    // The frozen inputs were materialized above. Deferred, uncommitted deltas
    // may settle them; they must not be reintroduced after that replay.
    recovery.frozen_model_requests.clear();
    recovery.replay_base = Some(StoredCheckpointReplayBase {
        run_id: capsule.run_id.clone(), checkpoint_revision: capsule.checkpoint_revision,
        through_ordinal: 0,
    });
    recovery.replay_inputs = capsule.deferred_host_events.iter().enumerate().map(|(index, input)| {
        // HostEventInput carries an effect identity, not a caller-authored Run
        // identity. Resolve ownership from the original canonical outbox row.
        let original = store.outbox_effect_identity(&input.effect_id)
            .map_err(|error| format!("load hot rollback effect owner failed: {error}"))?
            .ok_or("hot rollback input has no original Host effect")?;
        if original.run_id != capsule.run_id || original.effect_id != input.effect_id
            || original.request_id != input.request_id {
            return Err("hot rollback input belongs to another Run".to_string());
        }
        let replay = StoredCoreReplayInput {
            kind: ReplayInputKind::HostResponse,
            ordinal: i64::try_from(index + 1).map_err(|e|e.to_string())?,
            reduction_json: input.reduction_json.clone().ok_or("hot rollback missing normalized replay facts")?,
            received_at_ms: input.received_at_ms,
        };
        let (response, _, _) = host_replay::decode_reduction(&replay)?;
        if response.effect_id.0 != input.effect_id || response.request_id != input.request_id
            || i64::try_from(response.sequence).ok() != Some(input.sequence) {
            return Err("hot rollback normalized response differs from its original effect identity".into());
        }
        Ok(replay)
    }).collect::<Result<Vec<_>, String>>()?;
    recovery.checkpoint.event_cursor = i64::try_from(capsule.event_cursor).map_err(|e|e.to_string())?;
    let mut events = Vec::new();
    let (core, _) = replay_recovery_inputs_collect(core, &recovery, CheckpointReason::Manual, Some(&mut events))?;
    if core.checkpoint_view(CheckpointReason::Manual).revision != capsule.core_revision {
        return Err("hot rollback failed to reproduce the accepted Core revision".into());
    }
    Ok(runtime_supervisor::RetainHotRun {
        run_id: capsule.run_id, conversation_id: capsule.conversation_id,
        terminal: core.turn_status().is_terminal(), core,
        checkpoint_revision: capsule.checkpoint_revision,
        next_event_sequence: capsule.next_event_sequence,
        invocation_config_digest: capsule.invocation_config_digest,
        deferred_events: events, deferred_host_events: capsule.deferred_host_events,
        deferred_traces: capsule.deferred_traces,
        deferred_bytes: capsule.deferred_bytes,
    })
}

/// Cold readers must inspect the recovered frontier, never authorize work from
/// a baseline that precedes already committed inputs. Serialization here is an
/// explicit cold-read adapter, not a write on each streamed delta.
pub(crate) fn load_current_checkpoint<R: RunEffectRepository + ?Sized>(
    store: &R, conversation_id: &str,
) -> Result<Option<nola_store_port::run_effect::StoredCheckpoint>, String> {
    if let Some(cached) = store.cached_read_checkpoint(conversation_id)
        .map_err(|error| format!("load current checkpoint cache failed: {error}"))? {
        return Ok(Some(cached));
    }
    let Some(mut recovery) = store.load_recovery_checkpoint(conversation_id)
        .map_err(|error| format!("load current checkpoint failed: {error}"))? else { return Ok(None); };
    if recovery.checkpoint.conversation_id != conversation_id {
        return Err("current checkpoint conversation mismatch".into());
    }
    if !recovery.replay_inputs.is_empty() {
        let base = recovery.replay_base.as_ref().ok_or("recovery tail has no Run owner")?;
        let frozen = store.load_run_invocation_config(&base.run_id)
            .map_err(|error| error.to_string())?.ok_or("recovery tail has no frozen Run configuration")?;
        let (core, reason) = restore_run_recovery_baseline(store, &base.run_id, &frozen.invocation_config_digest, &recovery)?;
        recovery.checkpoint.checkpoint_json = materialize_replay_checkpoint_json(core, &recovery, reason)?;
    }
    store.cache_read_checkpoint(&recovery.checkpoint)
        .map_err(|error| format!("cache current checkpoint failed: {error}"))?;
    Ok(Some(recovery.checkpoint))
}

pub(crate) fn checkpoint_read_override<R: RunEffectRepository + ?Sized>(
    store: &R, conversation_id: &str,
) -> Result<Option<nola_store_port::run_effect::StoredCheckpoint>, String> {
    if let Some(cached) = store.cached_read_checkpoint(conversation_id).map_err(|error| error.to_string())? {
        return Ok(Some(cached));
    }
    if !store.checkpoint_replay_pending(conversation_id).map_err(|error| error.to_string())? {
        return Ok(None);
    }
    let checkpoint = load_current_checkpoint(store, conversation_id)?
        .ok_or_else(|| "replay checkpoint disappeared during presentation read".to_string())?;
    store.cache_read_checkpoint(&checkpoint).map_err(|error| error.to_string())?;
    Ok(Some(checkpoint))
}

pub(crate) fn reduce_and_accept_host_response_with_core<
    R: RunEffectRepository + EffectDeliveryRepository + repositories::AssistantPublicationRepositories + ?Sized,
>(
    store: &mut R,
    command: ReduceAndAcceptHostResponse,
    hot_core: Option<AgentCore>,
    hot_checkpoint_revision: Option<i64>,
    hot_next_event_sequence: Option<i64>,
    hot_invocation_config_digest: Option<&str>,
    deferred_events: Vec<CoreEventEnvelope<AgentEvent>>,
    persist: bool,
    include_control: bool,
) -> Result<(AcceptedHostResponse, AgentCore, Option<HostEventInput>), String> {
    let decision_time = decision_time_ms(command.received_at_ms)?;
    let using_hot_core = hot_core.is_some();
    let stored_checkpoint = if using_hot_core {
        None
    } else {
        Some(load_exact_run_checkpoint(store, &command.conversation_id, &command.run_id)?)
    };
    let request = if let Some(core) = hot_core.as_ref() {
        core.committed_host_effect_control(&command.response.effect_id.0)?
            .ok_or_else(|| {
                format!(
                    "Host response effect {} is not bound to the active Core",
                    command.response.effect_id.0
                )
            })?
    } else {
        let stored_request = store
            .outbox_effect(&command.response.effect_id.0)
            .map_err(|error| format!("load committed Host effect failed: {error}"))?
            .ok_or_else(|| {
                format!(
                    "Host response effect {} is not in the committed outbox",
                    command.response.effect_id.0
                )
            })?;
        if stored_request.run_id != command.run_id {
            return Err(format!(
                "Host response effect {} belongs to another Run",
                command.response.effect_id.0
            ));
        }
        let request=serde_json::from_str::<HostRequestEnvelope>(&stored_request.payload_json)
            .map_err(|error| format!("decode committed Host effect failed: {error}"))?;
        nola_domain::host_control::HostEffectControl::from_request(&request)?
    };
    let mut request = request;
    // Core can retain a newly produced effect before Application prepares its
    // Attempt or a worker claims delivery. Record actual metadata changes, not
    // a response-kind whitelist: every cold replay needs the same binding that
    // the hot reducer consumed. The response cursor itself is journal-owned.
    let mut previous_binding = request.binding();
    previous_binding.resume_after_sequence = None;
    if request.request_id != command.response.request_id {
        return Err(format!(
            "Host response request mismatch: committed={}, received={}",
            request.request_id, command.response.request_id
        ));
    }

    let delivery_attempt = effect_delivery::validate_and_bind_response(
        store,
        &command.run_id,
        &mut request,
        &command.response,
        command.received_at_ms,
    )?;

    delegated_invocation::bind_response_control(store,&command.run_id,&mut request)?;
    delegated_permission::bind_response_control(store,&command.run_id,&mut request)?;
    delegated_invocation::validate_response(store, &command.run_id, &request, &command.response)?;
    delegated_permission::validate_response(store, &command.run_id, &request, &command.response)?;

    let mut core = match hot_core {
        Some(core) => {
            if hot_invocation_config_digest != Some(command.invocation_config_digest.as_str()) {
                return Err(format!(
                    "Run invocation config mismatch for {}",
                    command.run_id
                ));
            }
            core
        }
        None => {
            restore_run_recovery(store, &command.run_id, &command.invocation_config_digest,
                stored_checkpoint.as_ref().expect("cold reduction loaded its checkpoint"))?
        }
    };
    // A different Application owner may have committed cancellation since
    // this worker last observed its stream. The retired target is no longer
    // bindable in Core. ACK only this exact retired response, before normal
    // model admission can mutate its already-Cancelled Attempt. No content,
    // usage, response cursor or stop confirmation is produced by this ACK.
    if !matches!(command.response.payload,
        HostResponse::ExecutionStarted | HostResponse::ModelAttemptStarted { .. })
        && core.turn_status() == nola_domain::AgentTurnStatus::Cancelling
    {
        let retired = store
            .outbox_effect(&request.effect_id.0)
            .map_err(|e| e.to_string())?
            .is_some_and(|effect| {
                effect.run_id == command.run_id && effect.status == "acknowledged"
            });
        let committed = load_committed_host_requests(store, &command.run_id)?;
        let has_cancellation = committed.requests.iter().any(|candidate| {
            candidate.host_id == request.host_id
            && candidate.session_id == request.session_id
            && candidate.turn_id == request.turn_id
            && matches!(&candidate.payload,
            HostRequest::Cancel { target_effect_id, .. } if target_effect_id == &request.effect_id)
        });
        let target_cancelling = core.host_effect_is_cancelling(&request.effect_id.0);
        if retired && has_cancellation && target_cancelling {
            model_attempt::validate_retired_response(
                store, &command.run_id, &request, &command.response,
            )?;
            let active = store
                .active_run(&command.conversation_id)
                .map_err(|e| e.to_string())?
                .filter(|active| active.run_id == command.run_id)
                .ok_or("retired cancellation target no longer belongs to the active Run")?;
            return Ok((
                AcceptedHostResponse {
                    commit: TransitionCommit {
                        assistant_publications: Vec::new(),
                        applied: false,
                        checkpoint_revision: hot_checkpoint_revision
                            .or_else(|| stored_checkpoint.as_ref().map(|s| s.checkpoint.revision))
                            .ok_or("retired cancellation target has no checkpoint revision")?,
                        next_event_sequence: active.next_event_sequence,
                    },
                    events: Vec::new(),
                    traces: Vec::new(),
                    control: include_control.then(|| core.control_snapshot()),
                },
                core,
                None,
            ));
        }
    }
    let current_attempt =
        model_attempt::validate_response(store, &command.run_id, &request, &command.response)?;
    if let Some(attempt) = &current_attempt {
        request.attempt_id = Some(attempt.id.clone());
        request.attempt_started = attempt.started_at_ms.is_some();
    }
    let typed_sampling_admission = matches!(
        &command.response.payload,
        HostResponse::ModelAttemptStarted { .. }
    );
    let planned_model_attempt_mutations = if typed_sampling_admission {
        if !persist {
            return Err("model sampling admission requires a durable transaction".into());
        }
        // Plan the ledger admission without writing it. The same CAS mutations
        // are committed with the checkpoint below; only that transaction can
        // authorize this physical sampling. Ordinary progress titles never
        // enter this branch.
        let received_at_ms = effect_delivery::logical_received_at_ms(
            delivery_attempt.as_ref(),
            command.received_at_ms,
        );
        let mutations = model_attempt::response_mutations(
            store,
            &command.run_id,
            &command.response,
            received_at_ms,
        )?;
        let admitted = mutations
            .iter()
            .rev()
            .find(|mutation| mutation.record.effect_id == request.effect_id.0)
            .ok_or("sampling admission has no planned model Attempt")?;
        if admitted.record.run_id != command.run_id
            || admitted.record.state != nola_domain::ModelAttemptState::Running
            || admitted.record.started_at_ms.is_none()
            || command.response.attempt_id.as_deref() != Some(admitted.record.id.as_str())
        {
            return Err("sampling admission differs from its planned model Attempt".into());
        }
        request.attempt_id = Some(admitted.record.id.clone());
        request.attempt_started = admitted.record.started_at_ms.is_some();
        // Like the outbox, the frozen request owns execution identity, not a
        // second response cursor. A hot snapshot overlays that cursor from the
        // pending-effect journal; freezing it here would differ on cold bind.
        request.resume_after_sequence = None;
        Some(mutations)
    } else {
        None
    };
    let mut next_binding = request.binding();
    next_binding.resume_after_sequence = None;
    let binding = if !using_hot_core
        || previous_binding != next_binding
        || core.turn_status() == nola_domain::AgentTurnStatus::Cancelling
    {
        request.resume_after_sequence = None;
        let binding = request.binding();
        core.bind_committed_host_response_control(&request)
            .map_err(|error| format!("bind committed Host effect failed: {error}"))?;
        Some(binding)
    } else { None };
    let response_kind = host_response_kind(&command.response.payload);
    let response_effect_id = command.response.effect_id.0.clone();
    let response_sequence = command.response.sequence;
    let mut reduction_response = command.response.clone();
    if matches!(command.response.payload, HostResponse::ExecutionStarted) {
        reduction_response.payload = HostResponse::Accepted;
    }
    if let HostResponse::ExecutionRejected {
        code,
        message,
        retryable,
    } = &command.response.payload
    {
        reduction_response.payload = HostResponse::Failed {
            error: nola_agent_engine::agent::host::HostError {
                code: code.clone(),
                message: message.clone(),
                retryable: *retryable,
                http_status: None,
            },
        };
    }
    if current_attempt
        .as_ref()
        .is_some_and(|attempt| attempt.state == nola_domain::ModelAttemptState::Prepared)
    {
        if let HostResponse::ModelAttemptStarted {
            provider,
            model,
            reason,
        } = &command.response.payload
        {
            // The initial sampling has no predecessor partials to retire.
            // Earlier logical invocations in this Turn retain their content.
            reduction_response.payload = HostResponse::ToolProgress {
                title: "model_attempt_started".into(),
                detail: Some(format!("{provider} / {model}: {reason}")),
                metadata: Value::Null,
            };
        }
    }
    let reduction_json = Some(host_replay::encode_reduction(
        &reduction_response, binding, command.received_at_ms,
    )?);
    let deferred_host_event = if persist { None } else { Some(HostEventInput {
        event_id: format!("host:{}:{}:{}", command.response.session_id,
            command.response.effect_id.0, command.response.sequence),
        effect_id: command.response.effect_id.0.clone(),
        request_id: command.response.request_id.clone(),
        sequence: i64::try_from(command.response.sequence).map_err(|_| "Host sequence exceeds durable range")?,
        payload_json: serde_json::to_string(&command.response).map_err(|error| error.to_string())?,
        reduction_json: reduction_json.clone(),
        received_at_ms: command.received_at_ms,
    }) };
    let (core, transition) = core
        .reduce_candidate(AgentSubmission::HostResponse {
            response: reduction_response,
        }, decision_time)
        .map_err(|error| {
            format!(
                "reduce Host response failed: {error}; effect_id={response_effect_id}; response={response_kind}; sequence={response_sequence}"
            )
        })?;
    let checkpoint_reason = transition
        .durability
        .as_ref()
        .map(|intent| intent.checkpoint_reason)
        .unwrap_or(CheckpointReason::Manual);
    let public_events;
    let commit = if persist {
        let mut successor_requests = transition.host_requests;
        if !command.response.payload.is_terminal() {
            if let Some(existing) = successor_requests
                .iter_mut()
                .find(|candidate| candidate.effect_id == request.effect_id)
            {
                // Preserve Core's explicitly rebound identity while advancing
                // its durable stream cursor monotonically.
                existing.resume_after_sequence = Some(
                    existing.resume_after_sequence.unwrap_or(command.response.sequence)
                        .max(command.response.sequence),
                );
            } else if typed_sampling_admission {
                // Sampling changes the durable attempt binding, unlike an
                // ordinary streamed delta. Persist that exact new authority.
                let mut admitted_request=core.committed_host_request_resume_snapshot(&request.effect_id.0)?
                    .ok_or("sampling admission lost its frozen request")?;
                admitted_request.resume_after_sequence=None;
                successor_requests.push(admitted_request);
            }
            // An unchanged continuation already has its immutable request in
            // the outbox. The transaction advances the exact response cursor
            // in its delivery witness/inbox, without rewriting that request.
            // Explicitly rebound/new effects above still persist their payload.
        }
        let checkpoint = core.checkpoint_view(checkpoint_reason);
        public_events = transition.event_envelopes.iter()
            .filter(|event| !matches!(event.event, AgentEvent::PartUpdated { .. }))
            .cloned().collect();
        let mut committed_events = deferred_events;
        committed_events.extend(transition.event_envelopes);
        let commit = assistant_publication_transaction::with_publication_transaction(store, |store| {
            let conversation_id = command.conversation_id.clone();
            let commit = accept_host_event_in_transaction(store, AcceptHostEvent {
                conversation_id: command.conversation_id,
                run_id: command.run_id.clone(),
                response: command.response,
                events: committed_events,
                successor_requests,
                checkpoint,
                received_at_ms: command.received_at_ms,
            }, reduction_json, planned_model_attempt_mutations, Some(&request))?;
            if commit.applied {
                execution_trace_journal::record_accepted(store, &conversation_id, &transition.trace_events)?;
            }
            Ok(commit)
        })?;
        commit
    } else {
        if !deferred_events.is_empty() {
            return Err("transient Host response cannot receive deferred events".to_string());
        }
        // Internal transient reductions still carry their owned journal
        // planning facts until the next durable boundary; Runtime moves them
        // into its pending batch before projecting public control events.
        public_events = transition.event_envelopes;
        let next_event_sequence = match hot_next_event_sequence {
            // Presentation events remain deferred until the next durable
            // batch, so the Store-owned sequence cannot advance yet.
            Some(sequence) => sequence,
            None => {
                store
                    .active_run(&command.conversation_id)
                    .map_err(|error| format!("load transient active Run failed: {error}"))?
                    .ok_or_else(|| "transient Host response has no active Run".to_string())?
                    .next_event_sequence
            }
        };
        TransitionCommit {
            assistant_publications: Vec::new(),
            applied: false,
            checkpoint_revision: hot_checkpoint_revision
                .or_else(|| {
                    stored_checkpoint.as_ref().map(|recovery| recovery.checkpoint.revision)
                })
                .ok_or_else(|| "transient Host response has no checkpoint revision".to_string())?,
            next_event_sequence,
        }
    };
    let accepted = AcceptedHostResponse {
        commit,
        events: public_events,
        traces: transition.trace_events,
        control: include_control.then(|| core.control_snapshot()),
    };
    Ok((accepted, core, deferred_host_event))
}

#[test]
fn persisted_part_update_events_bound_repeated_tool_snapshots() {
    let part = AgentPart::Tool(nola_agent_engine::agent::parts::ToolPart {
        execution_source: Default::default(),
        id: "part".into(),
        ownership: PartOwnership::for_turn("turn"),
        call_id: "call".into(),
        capability_id: "read_file".into(),
        title: "Read file".into(),
        summary: None,
        origin: Default::default(),
        state: ToolPartState::Completed,
        input: serde_json::json!({"path":"file"}),
        raw_input: String::new(),
        permission: None,
        output_preview: Some("x".repeat(50_000)),
        structured_output: Some(serde_json::json!({"text":"y".repeat(50_000)})),
        full_output_ref: Some("/tmp/full-output".into()),
        artifacts: Vec::new(),
        started_at_ms: None,
        finished_at_ms: None,
    });
    let event = AgentEvent::PartUpdated {
        turn_id: nola_agent_engine::agent::TurnId::from_string("turn".to_string()),
        projection_revision: 1,
        update: nola_agent_engine::agent::parts::PartUpdate::Created { part: part.clone() },
        part,
    };

    let payload = event_payload_for_persistence(&event).expect("event should encode");
    let encoded = serde_json::to_string(&payload).expect("payload should encode");
    assert!(
        encoded.len() < 12_000,
        "bounded event was {} bytes",
        encoded.len()
    );
    assert_eq!(
        payload
            .pointer("/part/full_output_ref")
            .and_then(Value::as_str),
        Some("/tmp/full-output")
    );
}

#[test]
fn persisted_stream_events_do_not_grow_with_the_full_text_snapshot() {
    let text = "x".repeat(1_000_000);
    let part = AgentPart::Text(nola_agent_engine::agent::parts::TextPart {
        review: None,
        id: "answer".into(),
        ownership: PartOwnership::for_turn("turn"),
        text,
        is_complete: false,
    });
    let event = AgentEvent::PartUpdated {
        turn_id: nola_agent_engine::agent::TurnId::from_string("turn".to_string()),
        projection_revision: 1,
        update: nola_agent_engine::agent::parts::PartUpdate::TextCommittedDelta {
            part_id: "answer".into(),
            delta: "semantic phrase".into(),
        },
        part,
    };

    let payload = event_payload_for_persistence(&event).expect("event should encode");
    let encoded = serde_json::to_string(&payload).expect("payload should encode");
    assert!(
        encoded.len() < 8_000,
        "stream event retained the cumulative snapshot: {} bytes",
        encoded.len()
    );
    assert_eq!(
        payload.pointer("/update/delta").and_then(Value::as_str),
        Some("semantic phrase")
    );
}

#[test]
fn cancelled_shell_part_persists_the_command_instead_of_the_executor_name() {
    let part = AgentPart::Tool(nola_agent_engine::agent::parts::ToolPart {
        execution_source: Default::default(),
        id: "part".into(),
        ownership: PartOwnership::for_turn("turn"),
        call_id: "call".into(),
        capability_id: "bash".into(),
        title: "bash".into(),
        summary: None,
        origin: Default::default(),
        state: ToolPartState::Cancelled,
        input: serde_json::json!({
            "command": "cd /tmp/project && git status --short",
            "operation_kind": "inspect"
        }),
        raw_input: String::new(),
        permission: None,
        output_preview: None,
        structured_output: None,
        full_output_ref: None,
        artifacts: Vec::new(),
        started_at_ms: None,
        finished_at_ms: None,
    });
    let PartMutation::Upsert { content_text, .. } =
        part_upsert(&part, 1, 1).expect("persist cancelled shell part")
    else {
        panic!("expected tool upsert")
    };
    assert_eq!(content_text, "cd /tmp/project && git status --short");
}

fn ensure_applied_user_messages(
    messages: &mut Vec<TransitionMessageInput>,
    items: &[AgentItem],
    run_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    for item in items {
        if let AgentItem::UiAction { id, action } = item {
            action.validate().map_err(|error| error.to_string())?;
            let message_id = id.as_str().to_owned();
            let part = nola_domain::MessagePartView::UiAction {
                part_id: nola_domain::MessagePartId(format!("{message_id}:body")), revision: 1,
                surface: action.surface.clone(), envelope: action.envelope.clone(), summary: action.summary.clone(),
            };
            merge_transition_message(messages, TransitionMessageInput {
                id: message_id.clone(), run_id: Some(run_id.into()), role: "user".into(), status: "completed".into(),
                created_at_ms: now_ms, completed_at_ms: Some(now_ms), initial_parts: vec![MessagePartInput {
                    id: format!("{message_id}:body"), kind: "ui_action".into(), ordinal: 1, state: "completed".into(),
                    content_text: action.summary.clone(), content_json: Some(serde_json::to_string(&part).map_err(|error| error.to_string())?),
                    content_digest: Some(format!("{:x}", Sha256::digest(action.envelope.to_string().as_bytes()))),
                    created_at_ms: now_ms, updated_at_ms: now_ms,
                }],
            })?;
            continue;
        }
        let AgentItem::Message {
            id,
            role: AgentRole::User,
            content,
        } = item
        else {
            continue;
        };
        let message_id = id.as_str().to_string();
        let mut initial_parts = vec![MessagePartInput {
            id: format!("{message_id}:body"),
            kind: "text".to_string(),
            ordinal: 1,
            state: "completed".to_string(),
            content_text: content.clone(),
            content_json: None,
            content_digest: Some(format!("{:x}", Sha256::digest(content.as_bytes()))),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }];
        for attachment in items.iter().filter_map(|item| match item {
            AgentItem::Attachment {
                id,
                message_id: owner,
                uri,
                name,
                mime_type,
                ..
            } if owner.as_str() == message_id => Some((id, uri, name, mime_type)),
            _ => None,
        }) {
            let (attachment_id, uri, name, mime_type) = attachment;
            let part = AgentPart::Artifact(ArtifactPart {
                id: attachment_id.as_str().to_string(),
                ownership: PartOwnership {
                    message_id: message_id.clone(),
                    turn_id: run_id.to_string(),
                },
                artifact: ArtifactReference {
                    uri: uri.clone(),
                    name: name.clone(),
                    mime_type: mime_type.clone(),
                    operation: Some("attach".to_string()),
                },
            });
            initial_parts.push(MessagePartInput {
                id: attachment_id.as_str().to_string(),
                kind: "attachment".to_string(),
                ordinal: i64::try_from(initial_parts.len() + 1).unwrap_or(i64::MAX),
                state: "completed".to_string(),
                content_text: name.clone(),
                content_json: Some(
                    serde_json::to_string(&part)
                        .map_err(|error| format!("encode user attachment Part failed: {error}"))?,
                ),
                content_digest: None,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            });
        }
        merge_transition_message(
            messages,
            TransitionMessageInput {
                id: message_id.clone(),
                run_id: Some(run_id.to_string()),
                role: "user".to_string(),
                status: "completed".to_string(),
                created_at_ms: now_ms,
                completed_at_ms: Some(now_ms),
                initial_parts,
            },
        )?;
    }
    Ok(())
}

fn ensure_assistant_message_for_part(
    messages: &mut Vec<TransitionMessageInput>,
    part_message_owners: &mut std::collections::BTreeMap<String, String>,
    update: &PartUpdate,
    view: &CanonicalPartProjection<'_>,
    run_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    let part = view.part();
    let message_id = view.message_id().to_string();
    part_message_owners.insert(part.id().to_string(), message_id.clone());
    // Text existence/body is owned by the transition-local stream planner.
    // Seeding from every cumulative Text event would copy/hash its full prefix
    // and create a second source of truth beside the stream Declare.
    let initial_parts = if matches!(part, AgentPart::Text(_)) {
        Vec::new()
    } else {
        view.initial_parts(update, now_ms)?
    };
    merge_transition_message(
        messages,
        TransitionMessageInput {
            id: message_id,
            run_id: Some(run_id.to_string()),
            role: "assistant".to_string(),
            status: "streaming".to_string(),
            created_at_ms: now_ms,
            completed_at_ms: None,
            initial_parts,
        },
    )
}

fn flush_started_assistant_message(
    pending: &mut Option<(&str, &str)>,
    messages: &mut Vec<TransitionMessageInput>,
    bindings: &mut assistant_turn_binding::AssistantTurnBindings,
    run_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    if let Some((turn_id, message_id)) = pending.take() {
        ensure_started_assistant_message(messages, bindings, turn_id, message_id, run_id, now_ms)?;
    }
    Ok(())
}

fn ensure_started_assistant_message(
    messages: &mut Vec<TransitionMessageInput>,
    bindings: &mut assistant_turn_binding::AssistantTurnBindings,
    turn_id: &str,
    message_id: &str,
    run_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    // Initial admission already binds the Application-selected message ID.
    // Successor identity comes from Core's birth event, before any body exists.
    if turn_id == run_id {
        return Ok(());
    }
    bindings.record_turn(turn_id, message_id)?;
    merge_transition_message(messages, TransitionMessageInput {
        id: message_id.into(), run_id: Some(run_id.into()), role: "assistant".into(),
        status: "streaming".into(), created_at_ms: now_ms, completed_at_ms: None,
        initial_parts: Vec::new(),
    })
}

fn resolve_transition_assistant_message<S: nola_store_port::assistant_ownership::AssistantJournalPlanningRepository + ?Sized>(
    store: &S,
    bindings: &assistant_turn_binding::AssistantTurnBindings,
    conversation_id: &str,
    turn_id: &str,
    run_id: &str,
    initial_assistant_message_id: Option<&str>,
) -> Result<String, String> {
    if turn_id == run_id {
        return initial_assistant_message_id
            .map(str::to_owned)
            .ok_or_else(|| format!("conversation Run {run_id} has no assistant Message owner"));
    }
    let persisted = store.assistant_owner_for_core_turn(conversation_id, turn_id).map_err(|e| e.to_string())?;
    if let Some(owner) = &persisted {
        if owner.conversation_id != conversation_id || owner.run_id != run_id {
            return Err("terminal Turn has a foreign persisted Message owner".into());
        }
    }
    if let Some(message) = bindings.born_message(turn_id) {
        if persisted.as_ref().is_some_and(|owner| owner.assistant_message_id != message) {
            return Err("terminal Turn birth conflicts with persisted Message owner".into());
        }
        return Ok(message.to_owned());
    }
    persisted.map(|owner| owner.assistant_message_id)
        .ok_or_else(|| format!("terminal Turn {turn_id} has no admitted Message owner"))
}

fn ensure_terminal_assistant_message<S: nola_store_port::assistant_ownership::AssistantJournalPlanningRepository + ?Sized>(
    store: &S,
    bindings: &assistant_turn_binding::AssistantTurnBindings,
    conversation_id: &str,
    messages: &mut Vec<TransitionMessageInput>,
    turn_id: &str,
    run_id: &str,
    initial_assistant_message_id: Option<&str>,
    now_ms: i64,
) -> Result<(), String> {
    let message_id = resolve_transition_assistant_message(store, bindings, conversation_id,
        turn_id, run_id, initial_assistant_message_id)?;
    merge_transition_message(
        messages,
        TransitionMessageInput {
            id: message_id,
            run_id: Some(run_id.to_string()),
            role: "assistant".to_string(),
            status: "streaming".to_string(),
            created_at_ms: now_ms,
            completed_at_ms: None,
            initial_parts: Vec::new(),
        },
    )
}

#[cfg(test)]
mod terminal_message_owner_tests {
    use super::*;

    #[test]
    fn terminal_message_requires_explicit_birth_and_never_guesses_a_suffix() {
        let store = nola_store::NolaStore::open(":memory:").unwrap();
        let mut bindings = assistant_turn_binding::AssistantTurnBindings::default();
        assert!(resolve_transition_assistant_message(&store, &bindings,
            "conversation", "successor", "run", Some("initial")).is_err());
        bindings.record_turn("successor", "opaque-final-message").unwrap();
        let mut messages = Vec::new();
        ensure_terminal_assistant_message(&store, &bindings, "conversation", &mut messages,
            "successor", "run", Some("initial"), 10).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, "opaque-final-message");
        assert_ne!(messages[0].id, "successor:assistant");
        assert_eq!(resolve_transition_assistant_message(&store, &bindings,
            "conversation", "run", "run", Some("opaque-initial")).unwrap(), "opaque-initial");
        assert!(resolve_transition_assistant_message(&store, &bindings,
            "conversation", "run", "run", None).is_err());
    }
}

fn merge_transition_message(
    messages: &mut Vec<TransitionMessageInput>,
    mut incoming: TransitionMessageInput,
) -> Result<(), String> {
    if let Some(existing) = messages
        .iter_mut()
        .find(|message| message.id == incoming.id)
    {
        if existing.run_id != incoming.run_id || existing.role != incoming.role {
            return Err(format!(
                "transition Message {} has conflicting ownership",
                incoming.id
            ));
        }
        for part in incoming.initial_parts.drain(..) {
            if !existing
                .initial_parts
                .iter()
                .any(|current| current.id == part.id)
            {
                existing.initial_parts.push(part);
            }
        }
        return Ok(());
    }
    messages.push(incoming);
    Ok(())
}

fn initial_parts_for_transition_message(
    update: &PartUpdate,
    part: &AgentPart,
    now_ms: i64,
) -> Result<Vec<MessagePartInput>, String> {
    let should_seed = matches!(
        update,
        PartUpdate::TextDelta { .. }
            | PartUpdate::TextCommittedDelta { .. }
            | PartUpdate::Completed { .. }
    );
    if !should_seed {
        return Ok(Vec::new());
    }
    let PartMutation::Upsert {
        part_id,
        kind,
        state,
        mut content_text,
        content_json,
        content_digest,
        created_at_ms,
        updated_at_ms,
        ..
    } = part_upsert(part, 1, now_ms)?
    else {
        unreachable!("part_upsert always returns Upsert")
    };
    let mut initial_state = state;
    let mut initial_json = content_json;
    if let PartUpdate::TextDelta { delta, .. } | PartUpdate::TextCommittedDelta { delta, .. } =
        update
    {
        content_text = content_text
            .strip_suffix(delta)
            .ok_or_else(|| format!("initial Part {part_id} does not end with its committed delta"))?
            .to_string();
        initial_state = "open".to_string();
        initial_json = None;
    }
    Ok(vec![MessagePartInput {
        id: part_id,
        kind,
        ordinal: 1,
        state: initial_state,
        content_digest: content_digest
            .or_else(|| Some(format!("{:x}", Sha256::digest(content_text.as_bytes())))),
        content_text,
        content_json: initial_json,
        created_at_ms,
        updated_at_ms,
    }])
}

/// Read model for the atomic outbox/checkpoint delivery witness. Full Core
/// restoration remains a separate gate; delivery never needs transcript bodies.
fn encode_delivery_witness<S: Borrow<AgentThreadState>>(checkpoint: &CoreCheckpoint<S>) -> Result<String, String> {
    let state = checkpoint.state.borrow();
    let effects = state.host.pending_effects.iter().map(|effect| {
        let permission_request_id = effect.request.as_ref().and_then(|request| {
            (request.pointer("/payload/type").and_then(Value::as_str) == Some("request_permission"))
                .then(|| request.pointer("/payload/permission_request_id")).flatten()
        });
        (effect.effect_id.clone(), serde_json::json!({
            "effect_id":effect.effect_id, "request_id":effect.request_id,
            "permission_request_id":permission_request_id,
            "session_id":effect.session_id, "turn_id":effect.turn_id,
            "host_id":effect.host_id, "lease_id":effect.lease_id,
            "catalog_revision":effect.catalog_revision, "availability_epoch":effect.availability_epoch,
            "replay_safety":effect.replay_safety, "last_response_sequence":effect.last_response_sequence,
        }))
    }).collect::<serde_json::Map<String, Value>>();
    let permissions = state.execution.permissions.pending.iter().map(|permission| serde_json::json!({
        "request_id":permission.request_id, "continuation":permission.continuation,
        "turn_id":permission.turn_id, "call_id":permission.call_id,
    })).collect::<Vec<_>>();
    serde_json::to_string(&serde_json::json!({
        "session_id":checkpoint.session_id,
        "core_revision":checkpoint.revision,
        "state":{
            "session_id":state.session_id,
            "host":{"pending_host_effects":{"effects":effects}},
            "execution":{
                "active_turn":state.execution.active_turn.as_ref().map(|turn| serde_json::json!({
                    "turn_id":turn.turn_id,"status":turn.status,
                })),
                "permissions":{"pending":permissions}
            },
        },
    })).map_err(|error| format!("encode current delivery witness failed: {error}"))
}

#[derive(Deserialize)]
struct CommittedDeliveryCheckpoint {
    session_id: String,
    state: CommittedDeliveryState,
}
#[derive(Deserialize)]
struct CommittedDeliveryState {
    session_id: String,
    host: CommittedDeliveryHost,
    execution: CommittedDeliveryExecution,
}
#[derive(Deserialize)]
struct CommittedDeliveryExecution {
    permissions: CommittedDeliveryPermissions,
}
#[derive(Deserialize)]
struct CommittedDeliveryPermissions {
    pending: Vec<CommittedDeliveryPermission>,
}
#[derive(Deserialize)]
struct CommittedDeliveryPermission {
    request_id: String,
    #[serde(default)]
    continuation: nola_agent_engine::agent::permissions::PermissionContinuation,
}
#[derive(Deserialize)]
struct CommittedDeliveryHost {
    #[serde(rename = "pending_host_effects")]
    pending_effects: CommittedDeliveryJournal,
}
#[derive(Deserialize)]
struct CommittedDeliveryJournal {
    effects: std::collections::BTreeMap<String, CommittedDeliveryEffect>,
}
#[derive(Deserialize)]
struct CommittedDeliveryEffect {
    effect_id: String,
    request_id: String,
    #[serde(default)]
    permission_request_id: Option<String>,
    session_id: String,
    turn_id: Option<String>,
    host_id: String,
    lease_id: Option<String>,
    catalog_revision: u64,
    availability_epoch: u64,
    replay_safety: nola_agent_engine::agent::host::HostReplaySafety,
    #[serde(default)]
    last_response_sequence: Option<u64>,
}

impl CommittedDeliveryEffect {
    fn matches_request(&self, request: &HostRequestEnvelope) -> bool {
        self.effect_id == request.effect_id.0
            && self.request_id == request.request_id
            && self.session_id == request.session_id
            && self.turn_id == request.turn_id
            && self.host_id == request.host_id.0
            && self.lease_id == request.lease_id
            && self.catalog_revision == request.catalog_revision
            && self.availability_epoch == request.availability_epoch
    }
}

#[test]
fn committed_delivery_projection_preserves_exact_cursor_and_required_identity() {
    let core = AgentCore::try_new(AgentCoreConfig {
        session_id: Some(nola_agent_engine::agent::protocol::SessionId::from_string("delivery-projection".into())), ..crate::model_fixture::config()
    }).unwrap();
    let checkpoint = core.recovery_checkpoint(CheckpointReason::Manual);
    let encoded = serde_json::to_string(&checkpoint).unwrap();
    let projection: CommittedDeliveryCheckpoint = serde_json::from_str(&encoded).unwrap();
    assert_eq!(projection.session_id, checkpoint.session_id);
    assert_eq!(projection.state.session_id, checkpoint.state.session_id.as_str());
    assert!(projection.state.host.pending_effects.effects.is_empty());
    let mut permission_checkpoint: Value = serde_json::from_str(&encoded).unwrap();
    permission_checkpoint["state"]["execution"]["permissions"]["pending"] = serde_json::json!([
        {"request_id":"core-permission"},
        {"request_id":"delegated-permission", "arguments":{"must_not_be_retained":"large input"},
         "continuation":{"type":"delegated_agent", "target_effect_id":"parent-effect", "backend_id":"backend",
             "occurrence":{"occurrence_id":"occurrence", "worker_generation":"generation"}}}
    ]);
    let permissions: CommittedDeliveryCheckpoint = serde_json::from_value(permission_checkpoint).unwrap();
    let permissions = &permissions.state.execution.permissions.pending;
    assert_eq!(permissions.len(), 2);
    assert_eq!(permissions[1].request_id, "delegated-permission");
    assert!(matches!(&permissions[0].continuation,
        nola_agent_engine::agent::permissions::PermissionContinuation::CoreTool));
    assert!(matches!(&permissions[1].continuation,
        nola_agent_engine::agent::permissions::PermissionContinuation::DelegatedAgent { target_effect_id, .. }
        if target_effect_id == "parent-effect"));
    let effect: CommittedDeliveryEffect = serde_json::from_value(serde_json::json!({
        "effect_id":"effect", "replay_safety":"recoverable",
        "session_id":"session", "turn_id":"turn", "host_id":"host", "lease_id":null,
        "catalog_revision":u64::MAX, "availability_epoch":u64::MAX,
        "request_id":"request", "last_response_sequence":u64::MAX,
        "request":{"large_unrelated_model_context":"not decoded into a Host envelope"},
    })).unwrap();
    assert_eq!(effect.last_response_sequence, Some(u64::MAX));
    assert_eq!(effect.request_id, "request");
    assert_eq!(effect.effect_id, "effect");
    assert_eq!(effect.catalog_revision, u64::MAX);
    assert_eq!(effect.availability_epoch, u64::MAX);
    assert_eq!(effect.replay_safety, nola_agent_engine::agent::host::HostReplaySafety::Recoverable);
    let request_json = serde_json::json!({
        "effect_id":"effect", "request_id":"request", "session_id":"session",
        "turn_id":"turn", "host_id":"host", "lease_id":null,
        "catalog_revision":u64::MAX, "availability_epoch":u64::MAX,
        "created_at_ms":1, "deadline_ms":null,
        "payload":{"type":"cancel", "target_effect_id":"target","target_not_started":null}
    });
    let request: HostRequestEnvelope = serde_json::from_value(request_json.clone()).unwrap();
    assert!(effect.matches_request(&request));
    for (field, value) in [
        ("effect_id", serde_json::json!("other-effect")),
        ("request_id", serde_json::json!("other-request")),
        ("session_id", serde_json::json!("other-session")),
        ("turn_id", serde_json::json!("other-turn")),
        ("host_id", serde_json::json!("other-host")),
        ("lease_id", serde_json::json!("other-lease")),
        ("catalog_revision", serde_json::json!(1)),
        ("availability_epoch", serde_json::json!(1)),
    ] {
        let mut foreign = request_json.clone();
        foreign[field] = value;
        let foreign: HostRequestEnvelope = serde_json::from_value(foreign).unwrap();
        assert!(!effect.matches_request(&foreign), "{field}");
    }
    assert!(serde_json::from_value::<CommittedDeliveryEffect>(serde_json::json!({
        "last_response_sequence":1,
    })).is_err());
}

pub fn load_committed_host_requests<R: RunEffectRepository + ?Sized>(
    store: &R,
    run_id: &str,
) -> Result<CommittedHostRequests, String> {
    load_selected_committed_host_requests(store, run_id, None).map(|(requests, _)| requests)
}

fn load_selected_committed_host_requests<R: RunEffectRepository + ?Sized>(
    store: &R, run_id: &str, selected: Option<&std::collections::HashSet<String>>,
) -> Result<(CommittedHostRequests, Option<CommittedDeliveryCheckpoint>), String> {
    let delivery = store
        .load_selected_host_delivery_witness(run_id, selected)
        .map_err(|error| format!("load committed Host delivery snapshot failed: {error}"))?;
    let effects = delivery.effects;
    if effects.is_empty() {
        return Ok((CommittedHostRequests {
            effect_ids: Vec::new(),
            requests: Vec::new(),
        }, None));
    }
    let conversation_id = delivery
        .conversation_id
        .ok_or_else(|| format!("committed Host effects belong to unknown Run {run_id}"))?;
    let stored_checkpoint = delivery
        .checkpoint
        .ok_or_else(|| format!("Run {run_id} has no canonical Core checkpoint"))?;
    if stored_checkpoint.conversation_id != conversation_id {
        return Err("committed Host delivery checkpoint owner mismatch".into());
    }
    let checkpoint = serde_json::from_str::<CommittedDeliveryCheckpoint>(
        &stored_checkpoint.witness_json,
    )
    .map_err(|error| format!("decode Core checkpoint for Run {run_id} failed: {error}"))?;
    if checkpoint.state.session_id.as_str() != checkpoint.session_id {
        return Err("committed Host delivery Core checkpoint identity mismatch".into());
    }
    let mut effect_ids = Vec::with_capacity(effects.len());
    let mut requests = Vec::with_capacity(effects.len());
    for effect in effects {
        let mut request = serde_json::from_str::<HostRequestEnvelope>(&effect.payload_json)
            .map_err(|error| {
                format!(
                    "decode committed Host effect {} failed: {error}",
                    effect.effect_id
                )
            })?;
        if effect.run_id != run_id || request.session_id != checkpoint.session_id {
            return Err("committed Host delivery Run/session identity mismatch".into());
        }
        if request.effect_id.0 != effect.effect_id || request.request_id != effect.request_id {
            return Err(format!(
                "committed Host effect identity mismatch for {}",
                effect.effect_id
            ));
        }
        if request.session_id.is_empty() {
            return Err(format!(
                "committed Host effect {} has no session identity",
                effect.effect_id
            ));
        }
        let journal_effect = checkpoint
            .state
            .host
            .pending_effects
            .effects
            .get(&effect.effect_id)
            .ok_or_else(|| {
                format!(
                    "committed Host effect {} is absent from the canonical Core checkpoint",
                    effect.effect_id
                )
            })?;
        if !journal_effect.matches_request(&request) {
            return Err(format!(
                "Core checkpoint Host effect identity mismatch for {}",
                effect.effect_id
            ));
        }
        request.resume_after_sequence = journal_effect.last_response_sequence;
        model_attempt::bind_delivery(store, run_id, &mut request)?;
        delegated_invocation::bind_delivery(store, run_id, &request)?;
        delegated_permission::bind_delivery(store, run_id, &request)?;
        if !delegated_permission::should_dispatch(store, &request)? {
            continue;
        }
        effect_ids.push(effect.effect_id);
        requests.push(request);
    }
    Ok((CommittedHostRequests {
        effect_ids,
        requests,
    }, Some(checkpoint)))
}


#[cfg(test)]
mod session_projection_tests {
    use nola_agent_engine::AgentReducerPort;
    use super::{
        durable_checkpoint_projection, load_session_projection, reduce_and_commit_idle_submission,
        ReduceAndCommitIdleSubmission,
    };
    use nola_agent_engine::agent::capability::{
        SkillDescriptor, SkillExecutionContract, SkillExecutionMode, SkillRunBudget, SkillRunOwner,
        SkillRuntimeCommand, SkillRuntimeCommandEnvelope,
    };
    use nola_agent_engine::agent::context_window::ContextWindowConfig;
    use nola_agent_engine::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_domain::core_event::{EventCursor};
    use nola_agent_engine::agent::{
        AgentCore, AgentCoreConfig, AgentInvokedSkill, AgentItem, AgentRole,
        AgentSessionConfiguration, AgentSkillActivation, AgentSubmission, AgentThreadState,
        AgentTurnContext, AgentTurnState, AgentTurnStatus, CallId, ItemId,
        SessionId, TurnId,
    };
    use nola_agent_engine::context_engine::{ContextComponent, ContextComponentKind};
    use nola_store::{
        BeginTurnInput, CreateConversationInput, NolaStore,
        SaveCheckpointInput,
    };
    use tempfile::tempdir;

    #[test]
    fn session_projection_survives_real_sqlite_reopen() {
        let directory = tempdir().expect("temporary Store directory");
        let database = directory.path().join("nola-product.db");
        let conversation_id = "projection-e2e";
        let mut state = AgentThreadState::new(
            SessionId::from_string(conversation_id.to_string()),
            AgentSessionConfiguration {
                model: Some("test-model".to_string()),
                max_model_iterations: 0,
                max_strategy_recovery_attempts: 3,
                tools: Vec::new(),
                context_window: ContextWindowConfig {
                    context_window_tokens: 64_000,
                    reserved_output_tokens: 8_000,
                    ..ContextWindowConfig::default()
                },
                memory_policy: Default::default(),
                turn_identity: crate::model_fixture::identity(),
            },
        );
        state.conversation.projection.revision = 3;
        state.conversation.context_window.used_tokens = 12_345;
        state.task_projection.revision = 4;
        state.goal_projection.revision = 5;
        state.memory_projection.revision = 6;
        state.memory_runtime.revision = 7;
        state.skill_runtime.revision = 8;
        state.skill_runtime.catalog.revision = 9;
        state.execution.permissions.revision = 10;
        let checkpoint = CoreCheckpoint {
            schema_version:
                nola_agent_engine::agent::recovery::AGENT_CHECKPOINT_SCHEMA_VERSION,
            session_id: conversation_id.to_string(),
            revision: 21,
            last_event_cursor: EventCursor { sequence: 34 },
            capability_catalog_revision: 9,
            created_at_ms: 1_786_900_000_000,
            reason: CheckpointReason::Manual,
            state,
        };
        let checkpoint_json = serde_json::to_string(&checkpoint).expect("encode checkpoint");
        let round_trip: CoreCheckpoint<AgentThreadState> =
            serde_json::from_str(&checkpoint_json).expect("round-trip checkpoint");
        assert_eq!(round_trip.state.task_projection.revision, 4);
        assert_eq!(round_trip.state.execution.permissions.revision, 10);
        assert_eq!(
            round_trip.state.conversation.context_window.used_tokens,
            12_345
        );

        {
            let mut store = NolaStore::open(&database).expect("open Store");
            store
                .begin_turn(&BeginTurnInput {
                    user_part_kind: "text".into(),
                    expected_configuration_revision: None,
                    expected_parent_message_id: None,
                    initial_events: Vec::new(),
                    initial_assistant_part_mutations: vec![],
                    assistant_turn_bindings: vec![],
                    memory_usage_entries: vec![],
                    model_attempt_mutations: Vec::new(),
                    delegated_invocation_mutations: Vec::new(),
                    delegated_permission_mutations: Vec::new(),
                    conversation_id: conversation_id.to_string(),
                    conversation_title: "Projection E2E".to_string(),
                    user_message_id: "user-1".to_string(),
                    user_part_id: "user-1:body".to_string(),
                    user_text: "retain the complete transcript".to_string(),
                    user_content_json: None,
                    additional_user_parts: Vec::new(),
                    assistant_message_id: "assistant-1".to_string(),
                    assistant_part_id: "assistant-1:body".to_string(),
                    run_id: "run-1".to_string(),
                    context_checkpoint_id: None,
                    checkpoint_revision: 21,
                    checkpoint_event_cursor: 34,
                    checkpoint_json,
                    invocation_config_json: serde_json::to_string(&crate::model_fixture::config())
                        .expect("encode Core config"),
                    invocation_config_digest: "test-config-digest".to_string(),
                    outbox_effects: Vec::new(),
                    created_at_ms: 1_786_900_000_000,
                })
                .expect("persist canonical session");
        }

        let reopened = NolaStore::open(&database).expect("reopen Store");
        let projection = load_session_projection(&reopened, conversation_id)
            .expect("load projection")
            .expect("projection exists");
        assert_eq!(projection.conversation_id, conversation_id);
        assert_eq!(projection.store_revision, 21);
        assert_eq!(projection.core_revision, 21);
        assert_eq!(projection.event_cursor, 34);
        assert_eq!(projection.conversation_projection.revision, 3);
        assert_eq!(projection.task_projection.revision, 4);
        assert_eq!(projection.goal_projection.revision, 5);
        assert_eq!(projection.memory_projection.revision, 6);
        assert_eq!(projection.memory_runtime_projection.revision, 7);
        assert_eq!(projection.skill_runtime_projection.revision, 8);
        assert_eq!(projection.skill_runtime_projection.catalog_revision, 9);
        assert_eq!(projection.permission_projection.revision, 10);
        assert_eq!(projection.context_usage.used_tokens, 12_345);
        assert_eq!(projection.context_usage.context_window_tokens, 64_000);
        assert!(projection.context_usage.usage_is_estimated);
        let conversation = reopened
            .load_conversation(conversation_id)
            .expect("load transcript")
            .expect("conversation exists");
        assert_eq!(conversation.messages.len(), 2);
        assert_eq!(
            conversation.messages[0].parts[0].content_text,
            "retain the complete transcript"
        );
    }

    #[test]
    fn durable_host_checkpoint_drops_catalogs_but_keeps_actual_invocations() {
        let directory = tempdir().expect("temporary Store directory");
        let database = directory.path().join("durable-host-checkpoint.db");
        let conversation_id = "durable-host-checkpoint";
        let optional_tool = serde_json::json!({
            "type": "function",
            "function": {"name": "unused_tool", "description": "OPTIONAL_TOOL_CATALOG"}
        });
        let configuration = AgentSessionConfiguration {
            model: Some("test-model".to_string()),
            max_model_iterations: 0,
            max_strategy_recovery_attempts: 3,
            tools: vec![optional_tool.clone()],
            context_window: ContextWindowConfig::default(),
            memory_policy: Default::default(),
            turn_identity: crate::model_fixture::identity(),
        };
        let mut state = AgentThreadState::new(
            SessionId::from_string(conversation_id.to_string()),
            configuration.clone(),
        );
        state.skill_runtime.catalog.revision = 7;
        state
            .skill_runtime
            .catalog
            .builtin_tools
            .insert("OPTIONAL_SKILL_CATALOG".to_string());
        state.execution.active_turn = Some(Box::new(AgentTurnState::new(
            TurnId::from_string("run-1".to_string()),
            AgentTurnContext {
                input_origin: Default::default(),
                configuration: Box::new(configuration),
                platform: Default::default(),
                components: vec![
                    ContextComponent {
                        kind: ContextComponentKind::Skill,
                        id: "skill-catalog".to_string(),
                        content: serde_json::json!("OPTIONAL_SKILL_CATALOG"),
                        source: None,
                    },
                    ContextComponent {
                        kind: ContextComponentKind::System,
                        id: "nola-available-sops-v1".to_string(),
                        content: serde_json::json!("OPTIONAL_SOP_CATALOG"),
                        source: None,
                    },
                    ContextComponent {
                        kind: ContextComponentKind::System,
                        id: "nola-active-sop-runs-v2".to_string(),
                        content: serde_json::json!("ACTUAL_SOP_RUN"),
                        source: None,
                    },
                ],
                component_metrics: Vec::new(),
                component_payload_digest: String::new(),
                items: vec![AgentItem::Message {
                    id: ItemId::from_string(
                        "nola-context-component-system-nola-available-sops-v1".to_string(),
                    ),
                    role: AgentRole::System,
                    content: "OPTIONAL_SOP_CATALOG".to_string(),
                }],
                host_id: "host-1".to_string(),
                host_catalog_revision: 7,
                host_availability_epoch: 1,
                capability_projection: Default::default(),
                runtime_snapshot: Default::default(),
            },
        )));
        state.record_item(AgentItem::ToolCall {
            id: ItemId::from_string("actual-tool-call".to_string()),
            call_id: CallId::from_string("call-1".to_string()),
            name: "actually_called_tool".to_string(),
            arguments: serde_json::json!({"path": "README.md"}),
        });
        state.record_invoked_skill(AgentInvokedSkill {
            skill_id: "actually-invoked-skill".to_string(),
            version: "1.0.0".to_string(),
            instructions_ref: "skill://actually-invoked-skill/1.0.0".to_string(),
            instructions_digest: "actual-digest".to_string(),
            base_directory: "/skills/actually-invoked-skill".to_string(),
            arguments: serde_json::json!({"input": "ACTUAL_SKILL_RUN"}),
            instructions: "Run the selected skill.".to_string(),
            required_tools: Vec::new(),
            validation_steps: Vec::new(),
            activation: AgentSkillActivation::Turn,
        });
        let digest = "a".repeat(64);
        state
            .skill_runtime
            .apply(SkillRuntimeCommandEnvelope {
                operation_id: "skill-start:actual-run".to_string(),
                command: SkillRuntimeCommand::Start {
                    run_id: "skill:actual-run".to_string(),
                    owner: SkillRunOwner {
                        session_id: conversation_id.to_string(),
                        parent_turn_id: "run-1".to_string(),
                        child_turn_id: None,
                    },
                    descriptor: SkillDescriptor {
                        skill_id: "actual-runtime-skill".to_string(),
                        version: "1.0.0".to_string(),
                        display_name: "Actual runtime Skill".to_string(),
                        instructions_ref: format!(
                            "skill://actual-runtime-skill/1.0.0/SKILL.md#sha256={digest}"
                        ),
                        instructions_digest: digest,
                        required_tools: Vec::new(),
                        default_mode: Some(SkillExecutionMode::Inline),
                        execution_contract: SkillExecutionContract::default(),
                    },
                    mode: SkillExecutionMode::Inline,
                    budget: SkillRunBudget::default(),
                },
            })
            .expect("record actual Skill run");
        state.execution.active_turn.as_mut().unwrap().status = AgentTurnStatus::Completed;
        state.journal.revision = 2;
        state.journal.next_event_sequence = 4;
        state.host.catalog.revision = 7;
        let host_checkpoint = CoreCheckpoint {
            schema_version:
                nola_agent_engine::agent::recovery::AGENT_CHECKPOINT_SCHEMA_VERSION,
            session_id: conversation_id.to_string(),
            revision: 2,
            last_event_cursor: EventCursor { sequence: 3 },
            capability_catalog_revision: 7,
            created_at_ms: 200,
            reason: CheckpointReason::TurnCompleted,
            state,
        };
        let durable = durable_checkpoint_projection(host_checkpoint).expect("valid Host checkpoint");
        for field in 0..4 {
            let mut invalid = durable.clone();
            match field {
                0 => invalid.revision += 1,
                1 => invalid.last_event_cursor.sequence += 1,
                2 => invalid.capability_catalog_revision += 1,
                _ => invalid.schema_version += 1,
            }
            assert!(durable_checkpoint_projection(invalid).is_err(),
                "history projection cannot normalize away inconsistent header {field}");
        }

        {
            let mut store = NolaStore::open(&database).expect("open Store");
            store
                .create_conversation(&CreateConversationInput {
                    id: conversation_id.to_string(),
                    title: "Durable boundary".to_string(),
                    revision: 0,
                    settings_json: None,
                    created_at_ms: 100,
                    updated_at_ms: 100,
                })
                .expect("create conversation");
            store
                .save_checkpoint(&SaveCheckpointInput {
                    conversation_id: conversation_id.to_string(),
                    expected_revision: 0,
                    revision: 2,
                    event_cursor: 3,
                    checkpoint_json: serde_json::to_string(&durable).expect("encode checkpoint"),
                    updated_at_ms: 200,
                })
                .expect("persist durable checkpoint");
        }

        let reopened = NolaStore::open(&database).expect("reopen Store");
        let stored = reopened
            .load_checkpoint(conversation_id)
            .expect("load checkpoint")
            .expect("checkpoint exists")
            .checkpoint_json;
        assert!(!stored.contains("OPTIONAL_TOOL_CATALOG"));
        assert!(!stored.contains("OPTIONAL_SKILL_CATALOG"));
        assert!(!stored.contains("OPTIONAL_SOP_CATALOG"));
        assert!(stored.contains("actually_called_tool"));
        assert!(stored.contains("ACTUAL_SKILL_RUN"));
        assert!(stored.contains("skill:actual-run"));
        assert!(stored.contains("actual-runtime-skill"));
        assert!(stored.contains("ACTUAL_SOP_RUN"));
    }



    #[test]
    fn idle_submission_is_rejected_while_a_foreground_run_is_active() {
        let directory = tempdir().expect("temporary Store directory");
        let database = directory.path().join("idle-overlap.db");
        let conversation_id = "idle-overlap-session";
        let core = AgentCore::try_new(AgentCoreConfig {
            session_id: Some(SessionId::from_string(conversation_id.to_string())),
            ..crate::model_fixture::config()
        })
        .expect("create Core");
        let checkpoint = core.recovery_checkpoint(CheckpointReason::Manual);
        let mut store = NolaStore::open(&database).expect("open Store");
        store
            .begin_turn(&BeginTurnInput {
                user_part_kind: "text".into(),
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                initial_events: Vec::new(),
                initial_assistant_part_mutations: vec![],
                assistant_turn_bindings: vec![],
                memory_usage_entries: vec![],
                model_attempt_mutations: Vec::new(),
                delegated_invocation_mutations: Vec::new(),
                delegated_permission_mutations: Vec::new(),
                conversation_id: conversation_id.to_string(),
                conversation_title: "Idle overlap".to_string(),
                user_message_id: "user-1".to_string(),
                user_part_id: "user-1:body".to_string(),
                user_text: "foreground".to_string(),
                user_content_json: None,
                additional_user_parts: Vec::new(),
                assistant_message_id: "assistant-1".to_string(),
                assistant_part_id: "assistant-1:body".to_string(),
                run_id: "run-1".to_string(),
                context_checkpoint_id: None,
                checkpoint_revision: checkpoint.revision as i64,
                checkpoint_event_cursor: checkpoint.last_event_cursor.sequence as i64,
                checkpoint_json: serde_json::to_string(&checkpoint).expect("encode checkpoint"),
                invocation_config_json: serde_json::to_string(&AgentCoreConfig {
                    session_id: Some(SessionId::from_string(conversation_id.to_string())),
                    ..crate::model_fixture::config()
                })
                .expect("encode Core config"),
                invocation_config_digest: "test-config-digest".to_string(),
                outbox_effects: Vec::new(),
                created_at_ms: 100,
            })
            .expect("begin foreground turn");

        let error = reduce_and_commit_idle_submission(
            &mut store,
            ReduceAndCommitIdleSubmission {
                conversation_id: conversation_id.to_string(),
                core_config: AgentCoreConfig {
                    session_id: Some(SessionId::from_string(conversation_id.to_string())),
                    ..crate::model_fixture::config()
                },
                submission: AgentSubmission::RebaseConversation {
                    items: Vec::new(),
                    item_ownership: Default::default(),
                },
                committed_at_ms: 200,
            },
        )
        .expect_err("active Run must reject idle submission");
        assert!(error.contains("cannot overlap an active Run"), "{error}");
    }

    #[test]
    fn active_run_rejects_a_different_platform_invocation_config() {
        let directory = tempdir().expect("temporary directory");
        let database = directory.path().join("run-config-mismatch.db");
        let mut store = NolaStore::open(&database).expect("open Store");
        let config = crate::model_fixture::config();
        let (config_json, config_digest) =
            crate::invocation_config_snapshot(&config).expect("snapshot config");
        store
            .begin_turn(&BeginTurnInput {
                user_part_kind: "text".into(),
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                initial_events: Vec::new(),
                initial_assistant_part_mutations: vec![],
                assistant_turn_bindings: vec![],
                memory_usage_entries: vec![],
                model_attempt_mutations: Vec::new(),
                delegated_invocation_mutations: Vec::new(),
                delegated_permission_mutations: Vec::new(),
                conversation_id: "config-conversation".to_string(),
                conversation_title: "Config".to_string(),
                user_message_id: "config-user".to_string(),
                user_part_id: "config-user:text".to_string(),
                user_text: "hello".to_string(),
                user_content_json: None,
                additional_user_parts: Vec::new(),
                assistant_message_id: "config-assistant".to_string(),
                assistant_part_id: "config-assistant:text".to_string(),
                run_id: "config-run".to_string(),
                context_checkpoint_id: None,
                checkpoint_revision: 1,
                checkpoint_event_cursor: 0,
                checkpoint_json: "{}".to_string(),
                invocation_config_json: config_json,
                invocation_config_digest: config_digest.clone(),
                outbox_effects: Vec::new(),
                created_at_ms: 100,
            })
            .expect("begin Run");

        let loaded = crate::load_run_invocation_config(&store, "config-run", &config_digest)
            .expect("matching config digest");
        assert_eq!(
            serde_json::to_value(loaded).expect("encode loaded config"),
            serde_json::to_value(config).expect("encode expected config")
        );
        let changed = AgentCoreConfig {
            session_id: Some(SessionId::from_string("different-session".to_string())),
            ..crate::model_fixture::config()
        };
        let (_, changed_digest) =
            crate::invocation_config_snapshot(&changed).expect("snapshot changed config");
        let error = crate::load_run_invocation_config(&store, "config-run", &changed_digest)
            .expect_err("mismatched config digest must fail");
        assert!(error.contains("config mismatch"), "{error}");
    }
}
mod transcript_input;
use nola_protocol::ImportConversationTranscript;
pub mod conversation;
pub mod persistent_agent;
pub mod agent_room;
pub mod conversation_configuration;
pub mod conversation_projection;
pub mod conversation_read;
pub mod permission;
pub mod permission_grant;
pub mod workspace_ownership;
pub use conversation::{CreateConversation, DeleteConversationMessage, EditConversationMessage};
pub use conversation_projection::{
    load_conversation_projection_page_v1, load_conversation_projection_v1,
    load_conversation_projection_window_v1, materialize_conversation_projection_v1,
    ConversationProjectionContractError, ConversationProjectionWindowV1,
};
pub use conversation_read::{
    ActiveRun, Conversation, ConversationCheckpoint, ConversationMessage, ConversationPart,
    ConversationSummary, ConversationTailSnapshot,
};
pub use permission::{
    NewPermissionRequest, PermissionAuthorizationDecision, PermissionIntervention,
    PermissionRequest, PermissionRequestCommand, PermissionRequestResolution,
    PermissionRequestStatus, PermissionResolutionOutcome, PermissionResolutionReceipt,
};
pub use permission_grant::{
    list_effective_permission_grants, load_permission_rule_set, record_permission_grant,
    revoke_permission_grant, PermissionGrant, PermissionGrantDecision, PermissionGrantInput,
    PermissionGrantTarget,
};
pub use workspace_ownership::{SessionRunLease, WorkspaceCheckpoint};

#[cfg(test)]
mod user_attachment_projection_tests {
    use super::*;

    #[test]
    fn input_applied_persists_attachment_as_part_of_its_user_message() {
        let user_id = ItemId::from_string("user-attachment-turn".to_string());
        let items = vec![
            AgentItem::user_message(user_id.clone(), "Describe this image"),
            AgentItem::Attachment {
                id: ItemId::from_string("user-attachment-turn:attachment:1".to_string()),
                message_id: user_id,
                uri: "file:///tmp/reference.png".to_string(),
                name: "reference.png".to_string(),
                mime_type: Some("image/png".to_string()),
                byte_count: 128,
            },
        ];
        let mut messages = Vec::new();

        ensure_applied_user_messages(&mut messages, &items, "run-1", 42)
            .expect("project user input");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].initial_parts.len(), 2);
        assert_eq!(messages[0].initial_parts[1].kind, "attachment");
        let part: AgentPart = serde_json::from_str(
            messages[0].initial_parts[1]
                .content_json
                .as_deref()
                .expect("attachment payload"),
        )
        .expect("decode attachment Part");
        let AgentPart::Artifact(part) = part else {
            panic!("expected Artifact Part");
        };
        assert_eq!(part.ownership.message_id, "user-attachment-turn");
        assert_eq!(part.artifact.uri, "file:///tmp/reference.png");
        assert_eq!(part.artifact.mime_type.as_deref(), Some("image/png"));
    }
}

#[cfg(test)]
mod lifecycle_contract_tests {
    use super::input_origin_uses_internal_turn;
    use nola_agent_engine::agent::AgentInputOrigin;

    #[test]
    fn only_explicit_goal_continuation_uses_internal_turn_storage() {
        assert!(input_origin_uses_internal_turn(
            AgentInputOrigin::GoalContinuation
        ));
        assert!(!input_origin_uses_internal_turn(
            AgentInputOrigin::InteractiveUser
        ));
        assert!(!input_origin_uses_internal_turn(
            AgentInputOrigin::ScheduledAction
        ));
        assert!(!input_origin_uses_internal_turn(
            AgentInputOrigin::DelegatedAgent
        ));
    }
}

pub mod uploaded_transcript_preparation;
pub mod uploaded_transcript_publication;

#[cfg(test)]
mod data_source_port_tests;

#[cfg(test)]
mod document_repository_tests;

mod canonical_branch;
pub mod edited_resend;

pub mod archive_restore;
pub use archive_restore::stage_archive_manifest;

mod historical_context_references;
mod tool_source;
mod code_cell_publication;
mod historical_part_projection;

pub mod repositories;

fn decision_time_ms(observed_at_ms: i64) -> Result<u64, String> {
    u64::try_from(observed_at_ms)
        .map_err(|_| "decision observation time must be nonnegative".to_owned())
}

#[cfg(test)]
mod run_recovery_port_tests;
mod automation_execution;
mod auxiliary_invocation;
pub use auxiliary_invocation::execute_auxiliary_invocation;

pub mod generative_ui;

pub mod goal_execution;
pub mod goal_control_intent;

pub mod model_profile_resolution;

pub mod delegation_preparation;
pub mod mobile_delegation;

mod started_host_effect;

#[cfg(test)]
mod delegation_preparation_tests;

pub mod mobile_delegation_configuration;

pub mod expert_message;
pub mod expert_prompt_catalog;
pub mod expert_template_capture;
pub mod host_expert_template_operation;

pub mod run_process_observation;

pub mod effective_model_parameters;
