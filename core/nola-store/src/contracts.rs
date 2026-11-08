// SPDX-License-Identifier: Apache-2.0
//! Store-owned repository records and transaction payloads.
//!
//! These types are persistence contracts, not product presentation models.

pub use nola_domain::{MessagePartInput, PartMutation, TransitionMessageInput};
use serde::{Deserialize, Serialize};

pub use nola_store_port::run_creation::{
    BeginInternalTurnInput, BeginMaintenanceRunInput, BeginMaintenanceRunResult, BeginTurnInput,
    BeginTurnResult, BeginSuccessorTurnInput,
};

/// Starts a replicated Host turn without claiming that this process owns a
/// restorable Core checkpoint. Remote journals own replicated product facts.
pub use nola_store_port::remote_session::AdmittedReplicaTurnInput;

pub use nola_store_port::transcript_write::{
    AppendMessagesInput, AppendMessagesResult, MessageInput,
};

pub use nola_store_port::conversation_profile::CreateConversationInput;

pub use nola_store_port::conversation_profile::CreateConversationResult;

pub use nola_domain::{
    CanonicalBranchIntent as CreateConversationBranchInput,
    CanonicalBranchResult as CreateConversationBranchResult,
};

pub use nola_store_port::DeferredTextPart;

pub use nola_store_port::context::{
    CommitContextCheckpointInput, ContextAggregateRecord, ContextCheckpointInput,
    ContextComponentInput,
};

/// One durable user decision. OS privacy authorization is deliberately not
/// represented here; Hosts remain responsible for reporting that capability
/// availability separately.
pub use nola_store_port::skill::{SkillSourceKind, StoredSkill, UpsertSkillInput};

pub use nola_store_port::mcp::StoredMcpServerRecord;
pub use nola_store_port::model::{StoredModelProfile, StoredModelRuntime};

/// Durable execution plan for one conversation. The JSON payload is a
/// validated array of typed plan items produced by the Agent plan tool.

/// Canonical durable background/agent task persisted independently from any
/// client process. A worker may mutate execution ownership only through the
/// Application lease operations.
pub use nola_store_port::runtime_task::{
    RenewRuntimeTaskLease, RuntimeTaskAttempt, RuntimeTaskGuidance, RuntimeTaskLeaseWitness,
    RuntimeTaskProgressMutation, RuntimeTaskRunBinding, RuntimeTaskTerminalReceipt,
    RuntimeTaskTerminalSource, StoredRuntimeTask,
};

pub use nola_store_port::runtime_task::StoredRuntimeTaskEvent;

pub use nola_store_port::workspace_ownership::StoredWorkspaceCheckpoint;

pub use nola_store_port::workspace_ownership::StoredSessionRunLease;

/// Versioned product configuration shared by every client. The value remains
/// JSON so feature-owned schemas can evolve without creating platform stores.
pub use nola_store_port::replica_sync::{
    ApplyRemoteSyncPageInput, ApplyRemoteSyncPageResult, RemoteSyncChangeInput, StoredSyncChange,
    StoredSyncCursor,
};

pub use nola_store_port::memory::{
    StoredMemoryRecord, StoredMemoryUsageEntry, StoredMemoryUsageStats,
};

/// One durable execution occurrence of an automation definition. Definitions
/// describe intent; runs own delivery, retries, worker leases, and outcomes.
pub use nola_store_port::sop_run::StoredSopRun;

pub use nola_store_port::data_source::StoredDataSourceRecord;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredDataSourceOverride {
    pub conversation_id: String,
    pub source_id: String,
    pub enabled: bool,
    pub updated_at_ms: i64,
}

pub use nola_store_port::data_source::StoredDataSourceUsage;

pub use nola_store_port::document_source::StoredDocumentSource;

pub use nola_store_port::document_source::StoredDocumentSourceSummary;

// Shared Run persistence contracts are owned by the database-independent Port.
pub use nola_store_port::run_effect::{
    CommitTransitionInput, CommitTransitionResult, HostEventInput, OutboxEffectInput,
    RunEventInput, StoredActiveRun, StoredCheckpoint, StoredOutboxEffect,
    StoredRunInvocationConfig,
};

pub use nola_store_port::conversation_history::{
    StoredConversation, StoredConversationMessageWindow, StoredMessage, StoredPart,
};
pub use nola_store_port::conversation_revert::{
    ApplyConversationProjectionInput, ApplyConversationProjectionResult,
    CommitCheckpointProjectionInput, CommitCheckpointProjectionResult, MessageProjectionMutation,
    SaveCheckpointInput,
};

pub use nola_store_port::permission::{
    PermissionGrantScope, StoredPermissionDecision, StoredPermissionGrant, StoredPermissionRequest,
    UpsertPermissionGrantInput,
};

pub use nola_store_port::automation::{StoredAutomationDefinition, StoredAutomationRun};

pub use nola_store_port::conversation_plan::{
    StoredConversationPlan, StoredConversationPlanRevision,
};

pub use nola_store_port::work_step_transaction::StoredWorkStepTransaction;

pub use nola_store_port::agent_queue::{EnqueueAgentCommandInput, StoredAgentCommand};

pub use nola_store_port::trace_journal::{
    AppendTraceJournalEventInput, StoredTraceJournalEvent, StoredTraceJournalRun,
};

pub use nola_store_port::product_setting::StoredProductSetting;

pub use nola_store_port::resume_snapshot::{
    AcknowledgeRuntimeTaskReadsInput, RuntimeTaskReadCursor,
};

pub use nola_store_port::conversation_query::{
    ConversationSummary, StoredConversationTailSnapshot,
};
