// SPDX-License-Identifier: Apache-2.0
//! Canonical local product store for Nola.
//!
//! This crate owns durable product facts. Agent Core checkpoints and Agent Host
//! journals are recovery inputs, not alternate conversation databases.

use rusqlite::{params, Connection, OptionalExtension, Row};
use sha2::{Digest, Sha256};

mod canonical_part_mutation;
use canonical_part_mutation::{apply_part_mutation, part_mutation_id};
mod canonical_stream_read;
mod canonical_message_ownership;
mod canonical_stream_store;
#[cfg(test)]
mod canonical_stream_transition_tests;
#[cfg(test)]
mod canonical_stream_performance_tests;
#[cfg(test)]
mod canonical_stream_consumer_tests;

mod connection;
pub mod commit_diagnostics;
pub mod work_archive;
mod work_archive_worker;
pub use work_archive_worker::ArchivePublicationStatus;
mod contracts;
mod activity_store;
mod automation_facts_port;
mod room_presentation_port;
mod conversation_revert;
mod delegated_invocations;
mod delegated_permissions;
mod effect_deliveries;
mod cancellation_delivery;
mod mcp_port;
mod plugin_port;
pub use plugin_port::SqlitePluginRepository;
pub use mcp_port::SqliteMcpRepository;
mod model_attempts;
mod auxiliary_invocation;
mod prepared_host_tool_plan;
mod model_part_ownership;
mod model_port;
pub use model_port::SqliteModelRepository;
pub use nola_store_port::{
    automation::{AutomationRepository, AutomationRepositoryProvider},
    context::{ContextRepository, ContextRepositoryProvider},
    conversation_profile::{ConversationProfileRepository, ConversationWriteRepositoryProvider},
    data_source::{DataSourceRepository, DataSourceRepositoryProvider},
    document_source::{DocumentSourceRepository, DocumentSourceRepositoryProvider},
    mcp::{McpRepository, McpRepositoryProvider},
    model::{ModelRepository, ModelRepositoryProvider},
    product_setting::{ProductSettingRepository, ProductSettingRepositoryProvider},
    protocol_journal::{
        CommandProtocolRepositoryProvider, CommandReceiptRepository, CommandTransactionRepository,
    },
    replica_sync::{ReplicaSyncRepository, ReplicaSyncRepositoryProvider},
    skill::{SkillRepository, SkillRepositoryProvider},
    workspace_ownership::{WorkspaceOwnershipRepository, WorkspaceOwnershipRepositoryProvider},
};
mod permission_transport;
mod permission_source;
mod cancellation_not_started;
mod protocol_journal;
mod run_recovery;
mod run_decision;
mod run_recovery_inspection;
mod exact_run_invocation;
mod skill_port;
pub use skill_port::SqliteSkillRepository;
mod skill_selection;
mod sop_run_port;
pub use delegated_invocations::StoredDelegatedRequestParts;
mod active_run_store;
mod agent_command_store;
mod remote_submission;
#[cfg(test)]
mod remote_submission_tests;
mod archive_restore_sync;
mod automation_store;
mod bounded_message;
mod canonical_body;
mod tool_source;
#[cfg(test)]
mod tool_source_tests;
mod canonical_text_replica;
mod checkpoint_store;
mod context_checkpoint_store;
mod context_port;
pub use context_port::SqliteContextRepository;
mod conversation_append_store;
mod conversation_append_sync;
mod conversation_deletion;
mod conversation_metadata_sync;
mod conversation_plan_store;
mod conversation_profile;
mod conversation_resource;
mod conversation_projection_store;
mod conversation_read_repository;
mod conversation_read_store;
mod conversation_summary_store;
mod conversation_transition_store;
mod conversation_transition_sync;
mod conversation_write_port;
pub use conversation_write_port::SqliteConversationWriteRepository;
mod data_source_store;
mod effect_outbox_store;
mod outbox_payload;
#[cfg(test)]
#[path = "../tests/support/outbox.rs"]
mod outbox_fixture;
pub mod historical_part_projection;
mod mcp_store;
mod memory_port;
mod memory_store;
mod model_store;
mod model_connection_store;
mod permission_grant_store;
mod presentation_input_store;
mod product_setting_store;
mod edited_resend;
mod published_part_body;
mod canonical_field_reader;
mod remote_session_port;
mod remote_session_store;
mod run_effect_port;
#[cfg(test)]
mod host_replay_budget_tests;
mod runtime_task_claim_store;
mod agent_task_admission_store;
mod runtime_task_lifecycle_store;
mod runtime_task_shell_delivery_store;
mod terminal_input_store;
mod runtime_task_user_input_wait;
mod runtime_task_permission_recovery;
mod runtime_task_run_binding;
mod runtime_task_terminal;
mod runtime_task_guidance;
mod runtime_task_permission_store;
mod runtime_task_port;
mod schema_store;
pub mod semantic_json_projection;
pub mod semantic_nested_draft;
pub mod semantic_part_draft;
mod session_run_lease_store;
mod skill_store;
mod sop_run_store;
pub mod structured_json_validation;
mod sync_mutation_store;
mod sync_outbox_read;
mod trace_journal_store;
pub mod transcript_export;
pub mod transcript_publication;
mod transcript_range;
mod canonical_text_disclosure;
mod transcript_read_port;
mod transcript_search;
mod transcript_index;
mod turn_admission_store;
mod agent_room_admission;
mod successor_admission;
mod initial_run_events;
mod input_facts;
mod user_input_wait;
mod execution_evidence;
mod code_cell;
mod run_input_frontier;
mod run_requirements;
mod assistant_turn_ownership;
mod assistant_journal_planning;
#[cfg(test)]
mod assistant_journal_planning_tests;
mod work_step_transaction_store;
mod workspace_checkpoint_store;
mod workspace_ownership_port;
pub use sync_outbox_read::MAX_PENDING_SYNC_JSON_BYTES;
pub use workspace_ownership_port::SqliteWorkspaceOwnershipRepository;
mod read_snapshot;
#[cfg(test)]
mod sync_outbox_dependency_tests;
use conversation_read_repository::{
    load_conversation_projection, load_conversation_projection_page,
    load_conversation_projection_with_window, ConversationMessageWindow,
};
pub use transcript_range::StoredTranscriptRange;
mod canonical_text_search;
pub mod canonical_text_store;
pub mod content_upload;
pub mod content_upload_port;
pub use canonical_text_search::CanonicalTextMatch;
pub use connection::{NolaStore, StoreError, StoreResult};
pub use contracts::*;
use memory_store::{
    apply_memory_usage_entries, replace_memory_projection, upsert_memory_projection,
    validate_memory_candidate_identity, validate_memory_record, MemorySyncProjection,
};
pub use model_part_ownership::StoredModelRequestParts;
pub use protocol_journal::*;
pub use skill_selection::SkillInstructionSelection;
pub use transcript_search::StoredTranscriptMatch;

pub const STORE_SCHEMA_EPOCH: i64 = 8;
pub const STORE_SCHEMA_VERSION: i64 = 83;
mod assistant_journal;
mod run_message_admission;
mod committed_run_context_usage;
const STORE_APPLICATION_ID: i64 = 0x4e4f4c41;

const RUNTIME_TASK_COLUMNS: &str = "id,kind,status,title,project_root,owner_session_id,parent_task_id,parent_tool_call_id,child_session_id,agent_type,prompt,progress_summary,last_activity,result_summary,error,output_path,unread,cancel_requested,cancel_requested_at_ms,worker_id,ownership_token,lease_until_ms,heartbeat_at_ms,retry_count,retry_at_ms,last_error,input_watermark,completion_watermark,created_at_ms,started_at_ms,ended_at_ms,updated_at_ms,metadata_json";
const PERMISSION_REQUEST_COLUMNS: &str = "request_id,task_id,child_session_id,capability,input_json,scope_rule,status,decision,custom_response,created_at_ms,resolved_at_ms,core_permission_request_id,permission_instructions_json,closure_json";
const PERMISSION_REQUEST_QUALIFIED_COLUMNS: &str = "p.request_id,p.task_id,p.child_session_id,p.capability,p.input_json,p.scope_rule,p.status,p.decision,p.custom_response,p.created_at_ms,p.resolved_at_ms,p.core_permission_request_id,p.permission_instructions_json,p.closure_json";

const AUTOMATION_COLUMNS: &str = "id,name,schedule_format,enabled,revision,metadata_json,created_at_ms,updated_at_ms,deleted_at_ms,execution_spec_json";
const AUTOMATION_RUN_COLUMNS: &str = "id,automation_id,occurrence_key,trigger,status,attempt,
    worker_id,lease_token,lease_until_ms,scheduled_for_ms,started_at_ms,finished_at_ms,
    next_retry_at_ms,result_json,error_json,created_at_ms,updated_at_ms,definition_revision";
const DATA_SOURCE_COLUMNS: &str = "id,name,kind,location,enabled,access_policy,validation_contract_json,capabilities_json,privacy,metadata_json,revision,created_at_ms,updated_at_ms,deleted_at_ms";
const DATA_SOURCE_USAGE_COLUMNS: &str = "id,source_id,source_title,consumer,purpose,conversation_id,disclosed_to_cloud,metadata_json,used_at_ms";
const MODEL_PROFILE_COLUMNS: &str = "id,display_name,provider,model_name,runtime,source_json,parameters_json,selected,enabled,revision,created_at_ms,updated_at_ms,deleted_at_ms";
const MODEL_RUNTIME_COLUMNS: &str = "host_id,profile_id,phase,endpoint_json,process_id,error,started_at_ms,stopped_at_ms,updated_at_ms";

fn model_profile_row(row: &Row<'_>) -> rusqlite::Result<StoredModelProfile> {
    Ok(StoredModelProfile {
        id: row.get(0)?,
        display_name: row.get(1)?,
        provider: row.get(2)?,
        model_name: row.get(3)?,
        runtime: row.get(4)?,
        source_json: row.get(5)?,
        parameters_json: row.get(6)?,
        selected: row.get(7)?,
        enabled: row.get(8)?,
        revision: row.get(9)?,
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
        deleted_at_ms: row.get(12)?,
    })
}

fn model_runtime_row(row: &Row<'_>) -> rusqlite::Result<StoredModelRuntime> {
    Ok(StoredModelRuntime {
        host_id: row.get(0)?,
        profile_id: row.get(1)?,
        phase: row.get(2)?,
        endpoint_json: row.get(3)?,
        process_id: row.get(4)?,
        error: row.get(5)?,
        started_at_ms: row.get(6)?,
        stopped_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
    })
}
const MCP_SERVER_COLUMNS: &str =
    "id,display_name,enabled,configuration_json,revision,created_at_ms,updated_at_ms,deleted_at_ms";

fn runtime_task_row(row: &Row<'_>) -> rusqlite::Result<StoredRuntimeTask> {
    Ok(StoredRuntimeTask {
        id: row.get(0)?,
        kind: row.get(1)?,
        status: row.get(2)?,
        title: row.get(3)?,
        project_root: row.get(4)?,
        owner_session_id: row.get(5)?,
        parent_task_id: row.get(6)?,
        parent_tool_call_id: row.get(7)?,
        child_session_id: row.get(8)?,
        agent_type: row.get(9)?,
        prompt: row.get(10)?,
        progress_summary: row.get(11)?,
        last_activity: row.get(12)?,
        result_summary: row.get(13)?,
        error: row.get(14)?,
        output_path: row.get(15)?,
        unread: row.get(16)?,
        cancel_requested: row.get(17)?,
        cancel_requested_at_ms: row.get(18)?,
        worker_id: row.get(19)?,
        ownership_token: row.get(20)?,
        lease_until_ms: row.get(21)?,
        heartbeat_at_ms: row.get(22)?,
        retry_count: row.get(23)?,
        retry_at_ms: row.get(24)?,
        last_error: row.get(25)?,
        input_watermark: row.get(26)?,
        completion_watermark: row.get(27)?,
        created_at_ms: row.get(28)?,
        started_at_ms: row.get(29)?,
        ended_at_ms: row.get(30)?,
        updated_at_ms: row.get(31)?,
        metadata_json: row.get(32)?,
    })
}

fn runtime_task_event_row(row: &Row<'_>) -> rusqlite::Result<StoredRuntimeTaskEvent> {
    Ok(StoredRuntimeTaskEvent {
        id: row.get(0)?,
        task_id: row.get(1)?,
        kind: row.get(2)?,
        status: row.get(3)?,
        summary: row.get(4)?,
        detail_json: row.get(5)?,
        created_at_ms: row.get(6)?,
    })
}

fn permission_request_row(row: &Row<'_>) -> rusqlite::Result<StoredPermissionRequest> {
    Ok(StoredPermissionRequest {
        closure: row.get::<_, Option<String>>(13)?.map(|raw| serde_json::from_str(&raw))
            .transpose().map_err(|error| rusqlite::Error::FromSqlConversionFailure(13, rusqlite::types::Type::Text, Box::new(error)))?,
        request_id: row.get(0)?,
        task_id: row.get(1)?,
        child_session_id: row.get(2)?,
        capability: row.get(3)?,
        input_json: row.get(4)?,
        scope_rule: row.get(5)?,
        status: row.get(6)?,
        decision: row.get(7)?,
        custom_response: row.get(8)?,
        created_at_ms: row.get(9)?,
        resolved_at_ms: row.get(10)?,
        core_permission_request_id: row.get(11)?,
        permission_instructions_json: row.get(12)?,
    })
}

fn automation_definition_row(row: &Row<'_>) -> rusqlite::Result<StoredAutomationDefinition> {
    Ok(StoredAutomationDefinition {
        execution_spec: serde_json::from_str(&row.get::<_, String>(9)?).map_err(|error| rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(error)))?,
        id: row.get(0)?,
        name: row.get(1)?,
        schedule_format: row.get(2)?,
        enabled: row.get(3)?,
        revision: row.get(4)?,
        metadata_json: row.get(5)?,
        created_at_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
        deleted_at_ms: row.get(8)?,
    })
}

fn automation_run_row(row: &Row<'_>) -> rusqlite::Result<StoredAutomationRun> {
    Ok(StoredAutomationRun {
        definition_revision: row.get(17)?,
        id: row.get(0)?,
        automation_id: row.get(1)?,
        occurrence_key: row.get(2)?,
        trigger: row.get(3)?,
        status: row.get(4)?,
        attempt: row.get(5)?,
        worker_id: row.get(6)?,
        lease_token: row.get(7)?,
        lease_until_ms: row.get(8)?,
        scheduled_for_ms: row.get(9)?,
        started_at_ms: row.get(10)?,
        finished_at_ms: row.get(11)?,
        next_retry_at_ms: row.get(12)?,
        result_json: row.get(13)?,
        error_json: row.get(14)?,
        created_at_ms: row.get(15)?,
        updated_at_ms: row.get(16)?,
    })
}

fn sop_run_row(row: &Row<'_>) -> rusqlite::Result<StoredSopRun> {
    Ok(StoredSopRun {
        run_id: row.get(0)?,
        sop_id: row.get(1)?,
        sop_version: row.get(2)?,
        owner_session_id: row.get(3)?,
        project_root: row.get(4)?,
        status: row.get(5)?,
        definition_json: row.get(6)?,
        record_json: row.get(7)?,
        revision: row.get(8)?,
        created_at_ms: row.get(9)?,
        updated_at_ms: row.get(10)?,
    })
}

fn data_source_row(row: &Row<'_>) -> rusqlite::Result<StoredDataSourceRecord> {
    Ok(StoredDataSourceRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        location: row.get(3)?,
        enabled: row.get(4)?,
        access_policy: row.get(5)?,
        validation_contract_json: row.get(6)?,
        capabilities_json: row.get(7)?,
        privacy: row.get(8)?,
        metadata_json: row.get(9)?,
        revision: row.get(10)?,
        created_at_ms: row.get(11)?,
        updated_at_ms: row.get(12)?,
        deleted_at_ms: row.get(13)?,
    })
}

fn data_source_usage_row(row: &Row<'_>) -> rusqlite::Result<StoredDataSourceUsage> {
    Ok(StoredDataSourceUsage {
        id: row.get(0)?,
        source_id: row.get(1)?,
        source_title: row.get(2)?,
        consumer: row.get(3)?,
        purpose: row.get(4)?,
        conversation_id: row.get(5)?,
        disclosed_to_cloud: row.get(6)?,
        metadata_json: row.get(7)?,
        used_at_ms: row.get(8)?,
    })
}

fn mcp_server_row(row: &Row<'_>) -> rusqlite::Result<StoredMcpServerRecord> {
    Ok(StoredMcpServerRecord {
        id: row.get(0)?,
        display_name: row.get(1)?,
        enabled: row.get(2)?,
        configuration_json: row.get(3)?,
        revision: row.get(4)?,
        created_at_ms: row.get(5)?,
        updated_at_ms: row.get(6)?,
        deleted_at_ms: row.get(7)?,
    })
}

fn validate_automation_definition(definition: &StoredAutomationDefinition) -> StoreResult<()> {
    definition.execution_spec.validate().map_err(StoreError::Invariant)?;
    require_id("automation.id", &definition.id)?;
    if definition.name.trim().is_empty() {
        return Err(StoreError::Invariant(
            "automation name must not be empty".into(),
        ));
    }
    require_id("automation.schedule_format", &definition.schedule_format)?;
    validate_json("automation.metadata_json", &definition.metadata_json)
}

fn validate_sop_run(run: &StoredSopRun) -> StoreResult<()> {
    require_id("run_id", &run.run_id)?;
    require_id("sop_id", &run.sop_id)?;
    if run.sop_version <= 0 || run.revision <= 0 {
        return Err(StoreError::Invariant(
            "SOP version and revision must be positive".into(),
        ));
    }
    if run.project_root.trim().is_empty() {
        return Err(StoreError::Invariant(
            "SOP project_root must not be empty".into(),
        ));
    }
    if !matches!(
        run.status.as_str(),
        "queued"
            | "running"
            | "waiting_permission"
            | "waiting_user"
            | "waiting_external"
            | "paused"
            | "verifying"
            | "compensating"
            | "completed"
            | "failed"
            | "cancelled"
    ) {
        return Err(StoreError::Invariant("invalid SOP run status".into()));
    }
    validate_json("definition_json", &run.definition_json)?;
    validate_json("record_json", &run.record_json)?;
    if run.updated_at_ms < run.created_at_ms {
        return Err(StoreError::Invariant(
            "SOP updated_at_ms precedes created_at_ms".into(),
        ));
    }
    Ok(())
}

fn validate_automation_run(run: &StoredAutomationRun) -> StoreResult<()> {
    if run.definition_revision < 1 {
        return Err(StoreError::Invariant("automation occurrence requires a positive definition revision".into()));
    }
    require_id("automation_run.id", &run.id)?;
    require_id("automation_run.automation_id", &run.automation_id)?;
    require_id("automation_run.occurrence_key", &run.occurrence_key)?;
    require_id("automation_run.trigger", &run.trigger)?;
    require_id("automation_run.status", &run.status)?;
    if let Some(value) = &run.result_json {
        validate_json("automation_run.result_json", value)?;
    }
    if let Some(value) = &run.error_json {
        validate_json("automation_run.error_json", value)?;
    }
    Ok(())
}

fn validate_data_source(source: &StoredDataSourceRecord) -> StoreResult<()> {
    require_id("data_source.id", &source.id)?;
    require_id("data_source.name", &source.name)?;
    require_id("data_source.kind", &source.kind)?;
    require_id("data_source.location", &source.location)?;
    require_id("data_source.access_policy", &source.access_policy)?;
    validate_json(
        "data_source.validation_contract_json",
        &source.validation_contract_json,
    )?;
    validate_json("data_source.capabilities_json", &source.capabilities_json)
        .and_then(|_| validate_json("data_source.metadata_json", &source.metadata_json))
}

fn validate_permission_request(request: &StoredPermissionRequest) -> StoreResult<()> {
    if request.closure.is_some() {
        return Err(StoreError::Invariant("permission creation cannot supply lifecycle closure".into()));
    }
    permission_transport::context(request)?;
    require_id("permission_request.request_id", &request.request_id)?;
    require_id("permission_request.task_id", &request.task_id)?;
    require_id(
        "permission_request.child_session_id",
        &request.child_session_id,
    )?;
    require_id("permission_request.capability", &request.capability)?;
    require_id("permission_request.scope_rule", &request.scope_rule)?;
    validate_json("permission_request.input_json", &request.input_json)?;
    if !matches!(request.status.as_str(), "pending" | "resolved") {
        return Err(StoreError::Invariant(format!(
            "invalid permission request status: {}",
            request.status
        )));
    }
    if request.status == "pending"
        && (request.decision.is_some()
            || request.custom_response.is_some()
            || request.resolved_at_ms.is_some())
    {
        return Err(StoreError::Invariant(
            "pending permission request cannot contain a resolution".into(),
        ));
    }
    Ok(())
}

fn validate_data_source_usage(usage: &StoredDataSourceUsage) -> StoreResult<()> {
    require_id("data_source_usage.id", &usage.id)?;
    require_id("data_source_usage.source_id", &usage.source_id)?;
    require_id("data_source_usage.consumer", &usage.consumer)?;
    require_id("data_source_usage.purpose", &usage.purpose)?;
    validate_json("data_source_usage.metadata_json", &usage.metadata_json)
}

fn validate_mcp_server(server: &StoredMcpServerRecord) -> StoreResult<()> {
    require_id("mcp_server.id", &server.id)?;
    require_id("mcp_server.display_name", &server.display_name)?;
    validate_json("mcp_server.configuration_json", &server.configuration_json)
}

fn require_id(name: &str, value: &str) -> StoreResult<()> {
    if value.trim().is_empty() {
        return Err(StoreError::Invariant(format!("{name} must not be empty")));
    }
    Ok(())
}

fn require_runtime_task_claim_identity(name: &str, value: &str) -> StoreResult<()> {
    if value.trim().is_empty()
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value.chars().count() > 128
    {
        return Err(StoreError::Invariant(format!(
            "runtime task claim {name} is not a canonical identity"
        )));
    }
    Ok(())
}

fn validate_json(name: &str, value: &str) -> StoreResult<()> {
    structured_json_validation::validate_json_str(value)
        .map_err(|error| StoreError::Invariant(format!("{name} is not valid JSON: {error}")))
}

fn permission_scope_name(scope: PermissionGrantScope) -> &'static str {
    match scope {
        PermissionGrantScope::UserGlobal => "user_global",
        PermissionGrantScope::Host => "host",
        PermissionGrantScope::Conversation => "conversation",
    }
}

fn permission_decision_name(decision: StoredPermissionDecision) -> &'static str {
    match decision {
        StoredPermissionDecision::Allow => "allow",
        StoredPermissionDecision::Deny => "deny",
    }
}

fn permission_grant_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredPermissionGrant> {
    let scope: String = row.get(1)?;
    let decision: String = row.get(6)?;
    Ok(StoredPermissionGrant {
        id: row.get(0)?,
        scope: match scope.as_str() {
            "user_global" => PermissionGrantScope::UserGlobal,
            "host" => PermissionGrantScope::Host,
            "conversation" => PermissionGrantScope::Conversation,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        conversation_id: row.get(2)?,
        host_id: row.get(3)?,
        capability_pattern: row.get(4)?,
        resource_pattern: row.get(5)?,
        decision: match decision.as_str() {
            "allow" => StoredPermissionDecision::Allow,
            "deny" => StoredPermissionDecision::Deny,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        created_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
        revoked_at_ms: row.get(9)?,
    })
}

const SKILL_SELECT_SQL_BASE: &str =
    "SELECT d.id, d.source, d.current_version, d.display_name, d.summary, d.category,
            v.instructions_ref, v.instructions_digest, v.instructions_text,
            v.required_tools_json, v.default_mode, d.enabled, d.created_at_ms,
            d.updated_at_ms, d.deleted_at_ms, d.revision
     FROM skill_definitions d
     JOIN skill_versions v
       ON v.skill_id = d.id AND v.version = d.current_version";
const SKILL_SELECT_SQL: &str =
    "SELECT d.id, d.source, d.current_version, d.display_name, d.summary, d.category,
            v.instructions_ref, v.instructions_digest, v.instructions_text,
            v.required_tools_json, v.default_mode, d.enabled, d.created_at_ms,
            d.updated_at_ms, d.deleted_at_ms, d.revision
     FROM skill_definitions d
     JOIN skill_versions v
       ON v.skill_id = d.id AND v.version = d.current_version
     WHERE d.id = ?1";

fn validate_skill_input(input: &UpsertSkillInput) -> StoreResult<()> {
    require_id("skill.id", &input.id)?;
    require_id("skill.version", &input.version)?;
    require_id("skill.display_name", &input.display_name)?;
    require_id("skill.instructions_ref", &input.instructions_ref)?;
    if input.instructions_text.trim().is_empty() {
        return Err(StoreError::Invariant(
            "Skill instructions must not be empty".to_string(),
        ));
    }
    let actual_digest = format!("{:x}", Sha256::digest(input.instructions_text.as_bytes()));
    if input.instructions_digest != actual_digest {
        return Err(StoreError::Invariant(
            "Skill instructions_digest does not match instructions_text".to_string(),
        ));
    }
    if let Some(required_tools_json) = &input.required_tools_json {
        validate_json("skill.required_tools_json", required_tools_json)?;
    }
    if input
        .default_mode
        .as_deref()
        .is_some_and(|mode| !matches!(mode, "inline" | "fork"))
    {
        return Err(StoreError::Invariant(
            "Skill default_mode must be inline or fork".to_string(),
        ));
    }
    Ok(())
}

fn register_immutable_skill_version(
    connection: &Connection,
    input: &UpsertSkillInput,
) -> StoreResult<()> {
    let existing: Option<(String, String, String, Option<String>, Option<String>)> = connection.query_row(
        "SELECT instructions_ref,instructions_digest,instructions_text,required_tools_json,default_mode
         FROM skill_versions WHERE skill_id=?1 AND version=?2",
        params![input.id, input.version],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).optional()?;
    let incoming = (
        input.instructions_ref.clone(),
        input.instructions_digest.clone(),
        input.instructions_text.clone(),
        input.required_tools_json.clone(),
        input.default_mode.clone(),
    );
    if let Some(existing) = existing {
        if existing != incoming {
            return Err(StoreError::Conflict(format!(
                "Skill {} version {} is immutable",
                input.id, input.version
            )));
        }
        return Ok(());
    }
    connection.execute(
        "INSERT INTO skill_versions(skill_id,version,instructions_ref,instructions_digest,
            instructions_text,required_tools_json,default_mode,created_at_ms)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            input.id,
            input.version,
            input.instructions_ref,
            input.instructions_digest,
            input.instructions_text,
            input.required_tools_json,
            input.default_mode,
            input.created_at_ms
        ],
    )?;
    Ok(())
}

fn skill_source_name(source: SkillSourceKind) -> &'static str {
    match source {
        SkillSourceKind::BuiltIn => "built_in",
        SkillSourceKind::User => "user",
    }
}

fn skill_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSkill> {
    let source: String = row.get(1)?;
    Ok(StoredSkill {
        id: row.get(0)?,
        source: match source.as_str() {
            "built_in" => SkillSourceKind::BuiltIn,
            "user" => SkillSourceKind::User,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        version: row.get(2)?,
        display_name: row.get(3)?,
        summary: row.get(4)?,
        category: row.get(5)?,
        instructions_ref: row.get(6)?,
        instructions_digest: row.get(7)?,
        instructions_text: row.get(8)?,
        required_tools_json: row.get(9)?,
        default_mode: row.get(10)?,
        enabled: row.get::<_, i64>(11)? != 0,
        created_at_ms: row.get(12)?,
        updated_at_ms: row.get(13)?,
        deleted_at_ms: row.get(14)?,
    })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SkillSyncProjection {
    record: StoredSkill,
    revision: i64,
}

fn skill_projection_with_revision(
    transaction: &Connection,
    id: &str,
) -> StoreResult<Option<(StoredSkill, i64)>> {
    transaction
        .query_row(SKILL_SELECT_SQL, params![id], |row| {
            Ok((skill_row(row)?, row.get(15)?))
        })
        .optional()
        .map_err(StoreError::from)
}

fn replace_skill_projection(
    transaction: &Connection,
    skill: &StoredSkill,
    revision: i64,
) -> StoreResult<()> {
    let input = UpsertSkillInput {
        id: skill.id.clone(),
        source: skill.source,
        version: skill.version.clone(),
        display_name: skill.display_name.clone(),
        summary: skill.summary.clone(),
        category: skill.category.clone(),
        instructions_ref: skill.instructions_ref.clone(),
        instructions_digest: skill.instructions_digest.clone(),
        instructions_text: skill.instructions_text.clone(),
        required_tools_json: skill.required_tools_json.clone(),
        default_mode: skill.default_mode.clone(),
        enabled: skill.enabled,
        created_at_ms: skill.created_at_ms,
        updated_at_ms: skill.updated_at_ms,
    };
    validate_skill_input(&input)?;
    let existing_source: Option<String> = transaction
        .query_row(
            "SELECT source FROM skill_definitions WHERE id=?1",
            params![skill.id],
            |row| row.get(0),
        )
        .optional()?;
    if existing_source
        .as_deref()
        .is_some_and(|source| source != skill_source_name(skill.source))
    {
        return Err(StoreError::Conflict(
            "a Skill cannot change ownership source".to_string(),
        ));
    }
    let existing_version: Option<(String, String, String, Option<String>, Option<String>)> = transaction
        .query_row(
            "SELECT instructions_ref,instructions_digest,instructions_text,required_tools_json,default_mode
             FROM skill_versions WHERE skill_id=?1 AND version=?2",
            params![skill.id, skill.version],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).optional()?;
    let incoming = (
        skill.instructions_ref.clone(),
        skill.instructions_digest.clone(),
        skill.instructions_text.clone(),
        skill.required_tools_json.clone(),
        skill.default_mode.clone(),
    );
    if existing_version
        .as_ref()
        .is_some_and(|stored| stored != &incoming)
    {
        return Err(StoreError::Conflict(format!(
            "Skill {} version {} is immutable",
            skill.id, skill.version
        )));
    }
    transaction.execute(
        "INSERT INTO skill_definitions(id,source,display_name,summary,category,current_version,
            enabled,created_at_ms,updated_at_ms,deleted_at_ms,revision)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name,
            summary=excluded.summary,category=excluded.category,current_version=excluded.current_version,
            enabled=excluded.enabled,updated_at_ms=excluded.updated_at_ms,
            deleted_at_ms=excluded.deleted_at_ms,revision=excluded.revision",
        params![skill.id, skill_source_name(skill.source), skill.display_name, skill.summary,
            skill.category, skill.version, skill.enabled, skill.created_at_ms,
            skill.updated_at_ms, skill.deleted_at_ms, revision],
    )?;
    if existing_version.is_none() {
        transaction.execute(
            "INSERT INTO skill_versions(skill_id,version,instructions_ref,instructions_digest,
                instructions_text,required_tools_json,default_mode,created_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                skill.id,
                skill.version,
                skill.instructions_ref,
                skill.instructions_digest,
                skill.instructions_text,
                skill.required_tools_json,
                skill.default_mode,
                skill.created_at_ms
            ],
        )?;
    }
    Ok(())
}

// Message closure is not Run sealing or evidence of task acceptance.
fn is_message_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled" | "interrupted")
}

fn transition_result(
    transaction: &Connection,
    transition_id: &str,
) -> StoreResult<Option<CommitTransitionResult>> {
    Ok(transaction
        .query_row(
            "SELECT checkpoint_revision, next_event_sequence
             FROM committed_transitions WHERE transition_id = ?1",
            params![transition_id],
            |row| {
                Ok(CommitTransitionResult {
                    applied: false,
                    checkpoint_revision: row.get(0)?,
                    next_event_sequence: row.get(1)?,
                })
            },
        )
        .optional()?)
}

fn insert_outbox_effects(
    transaction: &Connection,
    run_id: &str,
    effects: &[OutboxEffectInput],
    created_at_ms: i64,
) -> StoreResult<()> {
    for effect in effects {
        outbox_payload::write(transaction, run_id, effect, created_at_ms)?;
        model_attempts::bind_delivery(transaction, run_id, effect)?;
        delegated_invocations::bind_delivery(transaction, run_id, effect)?;
    }
    Ok(())
}


fn ensure_transition_message(
    transaction: &Connection,
    conversation_id: &str,
    message: &TransitionMessageInput,
) -> StoreResult<()> {
    require_id("transition.message.id", &message.id)?;
    let stored = transaction
        .query_row(
            "SELECT conversation_id, run_id, role FROM messages WHERE id = ?1",
            params![message.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let identity =
        stored.as_ref().map(
            |(conversation, run, role)| nola_kernel::CanonicalMessageIdentity {
                conversation_id: conversation,
                message_id: &message.id,
                run_id: run.as_deref(),
                role,
            },
        );
    match nola_kernel::evaluate_message_declaration(conversation_id, message, identity.as_ref()) {
        nola_kernel::MessageAdmission::Insert => {}
        nola_kernel::MessageAdmission::Existing => {
            insert_missing_transition_parts(transaction, message)?;
            return Ok(());
        }
        nola_kernel::MessageAdmission::IdentityConflict => {
            return Err(StoreError::Conflict(format!(
                "canonical Message {} ownership conflicts with committed transition",
                message.id,
            )))
        }
        decision => {
            return Err(StoreError::Invariant(format!(
                "canonical Message declaration rejected: {decision:?}"
            )))
        }
    }
    let ordinal: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM messages WHERE conversation_id = ?1",
        params![conversation_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO messages(
            id, conversation_id, run_id, role, status, ordinal,
            created_at_ms, completed_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            message.id,
            conversation_id,
            message.run_id,
            message.role,
            message.status,
            ordinal,
            message.created_at_ms,
            message.completed_at_ms
        ],
    )?;
    insert_missing_transition_parts(transaction, message)?;
    Ok(())
}

fn insert_missing_transition_parts(
    transaction: &Connection,
    message: &TransitionMessageInput,
) -> StoreResult<()> {
    for part in &message.initial_parts {
        require_id("transition.message.part.id", &part.id)?;
        if let Some(content_json) = &part.content_json {
            validate_json("transition.message.part.content_json", content_json)?;
        }
        if let Some((stored_message_id, stored_kind)) = transaction
            .query_row(
                "SELECT message_id, kind FROM message_parts WHERE id = ?1",
                params![part.id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if stored_message_id != message.id || stored_kind != part.kind {
                return Err(StoreError::Conflict(format!(
                    "canonical Part {} ownership conflicts with committed transition",
                    part.id
                )));
            }
            continue;
        }
        let ordinal: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM message_parts WHERE message_id = ?1",
            params![message.id],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO message_parts(
                id, message_id, kind, ordinal, state, content_text,
                content_json, content_digest, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                part.id,
                message.id,
                part.kind,
                ordinal,
                part.state,
                part.content_text,
                part.content_json,
                part.content_digest,
                part.created_at_ms,
                part.updated_at_ms
            ],
        )?;
    }
    Ok(())
}

const AGENT_COMMAND_COLUMNS: &str =
    "sequence,session_id,command_id,payload_json,created_at_ms,lease_owner,lease_until_ms,attempt_count";

fn agent_command_row(row: &Row<'_>) -> rusqlite::Result<StoredAgentCommand> {
    Ok(StoredAgentCommand {
        sequence: row.get(0)?,
        session_id: row.get(1)?,
        command_id: row.get(2)?,
        payload_json: row.get(3)?,
        created_at_ms: row.get(4)?,
        lease_owner: row.get(5)?,
        lease_until_ms: row.get(6)?,
        attempt_count: row.get(7)?,
    })
}

const TRACE_JOURNAL_EVENT_COLUMNS: &str =
    "event_id,trace_id,conversation_id,turn_id,sequence,kind,status,visibility,span_id,parent_span_id,emitted_at,event_json,envelope_json";
const TRACE_JOURNAL_RUN_COLUMNS: &str =
    "trace_id,run_id,conversation_id,message_id,turn_id,status,last_sequence,terminal_reason,started_at,updated_at,finished_at";

fn trace_journal_event_row(row: &Row<'_>) -> rusqlite::Result<StoredTraceJournalEvent> {
    Ok(StoredTraceJournalEvent {
        event_id: row.get(0)?,
        trace_id: row.get(1)?,
        conversation_id: row.get(2)?,
        turn_id: row.get(3)?,
        sequence: row.get(4)?,
        kind: row.get(5)?,
        status: row.get(6)?,
        visibility: row.get(7)?,
        span_id: row.get(8)?,
        parent_span_id: row.get(9)?,
        emitted_at: row.get(10)?,
        event_json: row.get(11)?,
        envelope_json: row.get(12)?,
    })
}

fn trace_journal_run_row(row: &Row<'_>) -> rusqlite::Result<StoredTraceJournalRun> {
    Ok(StoredTraceJournalRun {
        trace_id: row.get(0)?,
        run_id: row.get(1)?,
        conversation_id: row.get(2)?,
        message_id: row.get(3)?,
        turn_id: row.get(4)?,
        status: row.get(5)?,
        last_sequence: row.get(6)?,
        terminal_reason: row.get(7)?,
        started_at: row.get(8)?,
        updated_at: row.get(9)?,
        finished_at: row.get(10)?,
    })
}

const WORKSPACE_CHECKPOINT_COLUMNS: &str =
    "id,root,git_root,revision,source,label,created_at_ms,status,untracked_json";
const SESSION_RUN_LEASE_COLUMNS: &str = "session_id,owner,pid,process_start,acquired_at_ms";
const PRODUCT_SETTING_COLUMNS: &str = "key,value_json,revision,updated_at_ms";

fn workspace_checkpoint_row(row: &Row<'_>) -> rusqlite::Result<StoredWorkspaceCheckpoint> {
    let untracked_json: String = row.get(8)?;
    let untracked = serde_json::from_str(&untracked_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(StoredWorkspaceCheckpoint {
        id: row.get(0)?,
        root: row.get(1)?,
        git_root: row.get(2)?,
        revision: row.get(3)?,
        source: row.get(4)?,
        label: row.get(5)?,
        created_at_ms: row.get(6)?,
        status: row.get(7)?,
        untracked,
    })
}

fn work_step_transaction_row(row: &Row<'_>) -> rusqlite::Result<StoredWorkStepTransaction> {
    Ok(StoredWorkStepTransaction {
        id: row.get(0)?,
        work_id: row.get(1)?,
        step_id: row.get(2)?,
        tool_call_id: row.get(3)?,
        root: row.get(4)?,
        effect_kind: row.get(5)?,
        phase: row.get(6)?,
        workspace_checkpoint_id: row.get(7)?,
        request_json: row.get(8)?,
        result_json: row.get(9)?,
        result_ref: row.get(10)?,
        error: row.get(11)?,
        revision: row.get(12)?,
        created_at_ms: row.get(13)?,
        updated_at_ms: row.get(14)?,
    })
}

fn session_run_lease_row(row: &Row<'_>) -> rusqlite::Result<StoredSessionRunLease> {
    Ok(StoredSessionRunLease {
        session_id: row.get(0)?,
        owner: row.get(1)?,
        pid: u32::try_from(row.get::<_, i64>(2)?.max(0)).unwrap_or(u32::MAX),
        process_start: row.get(3)?,
        acquired_at_ms: row.get(4)?,
    })
}

fn enqueue_sync_change(
    transaction: &Connection,
    entity_type: &str,
    entity_id: &str,
    revision: i64,
    deleted: bool,
    payload_json: &str,
    created_at_ms: i64,
) -> StoreResult<()> {
    validate_json("sync.payload_json", payload_json)?;
    // Deletion owns the stable conversation identity. Pending metadata must not
    // get delivered ahead of the tombstone after its content base is removed.
    if entity_type == "conversation" && deleted {
        transaction.execute(
            "DELETE FROM sync_outbox WHERE entity_type IN ('conversation_metadata','conversation_append','conversation_transition')
             AND entity_id=?1 AND status='pending'",
            params![entity_id],
        )?;
    }
    // Full snapshots replace older content, except a base still referenced by
    // pending metadata. Each fact retains its original queue order and identity.
    if entity_type == "conversation" {
        transaction.execute(
            "DELETE FROM sync_outbox
             WHERE entity_type=?1 AND entity_id=?2 AND status='pending' AND revision<=?3
               AND NOT EXISTS (
                 SELECT 1 FROM sync_outbox AS metadata
                 WHERE metadata.entity_type IN ('conversation_metadata','conversation_append','conversation_transition')
                   AND metadata.entity_id=sync_outbox.entity_id AND metadata.status='pending'
                   AND json_extract(metadata.payload_json,'$.base_content_revision')=sync_outbox.revision
               )",
            params![entity_type, entity_id, revision],
        )?;
    }
    let operation_id = sync_operation_id(entity_type, entity_id, revision, deleted, payload_json);
    transaction.execute(
        "INSERT OR IGNORE INTO sync_outbox(
            operation_id,entity_type,entity_id,revision,deleted,payload_json,status,created_at_ms
         ) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7)",
        params![
            operation_id,
            entity_type,
            entity_id,
            revision,
            deleted,
            payload_json,
            created_at_ms
        ],
    )?;
    transaction.execute(
        "INSERT INTO sync_entity_revisions(
            entity_type,entity_id,revision,deleted,operation_id,updated_at_ms
         ) VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(entity_type,entity_id) DO UPDATE SET
            revision=MAX(sync_entity_revisions.revision,excluded.revision),
            deleted=CASE WHEN excluded.revision>=sync_entity_revisions.revision
                         THEN excluded.deleted ELSE sync_entity_revisions.deleted END,
            operation_id=CASE WHEN excluded.revision>=sync_entity_revisions.revision
                              THEN excluded.operation_id ELSE sync_entity_revisions.operation_id END,
            updated_at_ms=CASE WHEN excluded.revision>=sync_entity_revisions.revision
                               THEN excluded.updated_at_ms ELSE sync_entity_revisions.updated_at_ms END",
        params![
            entity_type,
            entity_id,
            revision,
            deleted,
            operation_id,
            created_at_ms
        ],
    )?;
    Ok(())
}

// Hash the same canonical byte sequence without allocating a second payload.
// Identity generation must remain bounded even while snapshot publication is
// being replaced by independent metadata and immutable content publications.
fn sync_operation_id(
    entity_type: &str,
    entity_id: &str,
    revision: i64,
    deleted: bool,
    payload_json: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(entity_type.as_bytes());
    digest.update(b"\0");
    digest.update(entity_id.as_bytes());
    digest.update(b"\0");
    digest.update(revision.to_string().as_bytes());
    digest.update(if deleted { b"\x001\x00" } else { b"\x000\x00" });
    digest.update(payload_json.as_bytes());
    let digest = digest.finalize();
    let operation_id = format!(
        "sync-{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    operation_id
}

#[cfg(test)]
mod sync_operation_identity_tests {
    use super::*;

    #[test]
    fn incremental_hash_preserves_retry_identity() {
        for payload in [
            "{}".to_owned(),
            r#"{"title":"\u4f60\u597d","value":"a\\nb"}"#.to_owned(),
            format!("{{\"body\":\"{}\"}}", "x".repeat(1_000_000)),
        ] {
            for revision in [1, i64::MAX] {
                for deleted in [false, true] {
                    let bytes = format!(
                        "conversation\0c\0{revision}\0{}\0{payload}",
                        u8::from(deleted)
                    );
                    let expected = format!("sync-{:x}", Sha256::digest(bytes.as_bytes()));
                    assert_eq!(
                        sync_operation_id("conversation", "c", revision, deleted, &payload),
                        expected
                    );
                }
            }
        }
    }

    #[test]
    fn each_identity_field_affects_the_operation() {
        let baseline = sync_operation_id("conversation", "c", 1, false, "{}");
        for changed in [
            sync_operation_id("conversation_metadata", "c", 1, false, "{}"),
            sync_operation_id("conversation", "other", 1, false, "{}"),
            sync_operation_id("conversation", "c", 2, false, "{}"),
            sync_operation_id("conversation", "c", 1, true, "{}"),
            sync_operation_id("conversation", "c", 1, false, "{\"a\":1}"),
        ] {
            assert_ne!(baseline, changed);
        }
    }
}

fn sync_change_row(row: &Row<'_>) -> rusqlite::Result<StoredSyncChange> {
    Ok(StoredSyncChange {
        local_sequence: row.get(0)?,
        operation_id: row.get(1)?,
        entity_type: row.get(2)?,
        entity_id: row.get(3)?,
        revision: row.get(4)?,
        deleted: row.get(5)?,
        payload_json: row.get(6)?,
        status: row.get(7)?,
        remote_cursor: row.get(8)?,
        created_at_ms: row.get(9)?,
        resolved_at_ms: row.get(10)?,
    })
}

fn product_setting_row(row: &Row<'_>) -> rusqlite::Result<StoredProductSetting> {
    Ok(StoredProductSetting {
        key: row.get(0)?,
        value_json: row.get(1)?,
        revision: row.get(2)?,
        updated_at_ms: row.get(3)?,
    })
}

fn conversation_sync_payload(
    connection: &Connection,
    conversation_id: &str,
) -> StoreResult<String> {
    let conversation = load_conversation_projection(connection, conversation_id)?
        .ok_or_else(|| StoreError::Invariant(format!("unknown conversation {conversation_id}")))?;
    Ok(serde_json::to_string(&conversation)?)
}

// Establish a durable content base in the content mutation transaction, never
// lazily during a subsequent settings change. Snapshot payloads remain a
// separate large-content publication debt until immutable Part references are
// adopted by the replica transport.
fn publish_conversation_content(
    connection: &Connection,
    id: &str,
    updated_at_ms: i64,
) -> StoreResult<()> {
    let revision: i64 = connection.query_row(
        "SELECT revision FROM conversations WHERE id=?1",
        params![id],
        |row| row.get(0),
    )?;
    let payload = conversation_sync_payload(connection, id)?;
    enqueue_sync_change(
        connection,
        "conversation",
        id,
        revision,
        false,
        &payload,
        updated_at_ms,
    )
}

fn replace_conversation_projection(
    transaction: &Connection,
    conversation: &StoredConversation,
) -> StoreResult<()> {
    require_id("conversation.id", &conversation.id)?;
    conversation_deletion::require_inactive_conversation(transaction, &conversation.id)?;
    if conversation.revision <= 0 {
        return Err(StoreError::Invariant(
            "conversation revision must be positive".into(),
        ));
    }
    if let Some(settings) = &conversation.settings_json {
        validate_json("conversation.settings_json", settings)?;
    }
    transaction.execute(
        "DELETE FROM conversations WHERE id=?1",
        params![conversation.id],
    )?;
    transaction.execute(
        "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms)
         VALUES(?1,?2,?3,?4,?5)",
        params![
            conversation.id,
            conversation.title,
            conversation.revision,
            conversation.created_at_ms,
            conversation.updated_at_ms
        ],
    )?;
    if let Some(settings) = &conversation.settings_json {
        transaction.execute(
            "INSERT INTO conversation_settings(conversation_id,settings_json,updated_at_ms)
             VALUES(?1,?2,?3)",
            params![conversation.id, settings, conversation.updated_at_ms],
        )?;
    }
    let mut previous_message_ordinal = 0;
    for message in &conversation.messages {
        require_id("conversation.message.id", &message.id)?;
        if message.ordinal <= previous_message_ordinal {
            return Err(StoreError::Invariant(
                "message ordinals must be strictly increasing".into(),
            ));
        }
        previous_message_ordinal = message.ordinal;
        transaction.execute(
            "INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms,completed_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![message.id, conversation.id, message.run_id, message.role, message.status, message.ordinal, message.created_at_ms, message.completed_at_ms],
        )?;
        let mut previous_part_ordinal = 0;
        for part in &message.parts {
            require_id("conversation.part.id", &part.id)?;
            if part.ordinal <= previous_part_ordinal {
                return Err(StoreError::Invariant(
                    "part ordinals must be strictly increasing".into(),
                ));
            }
            previous_part_ordinal = part.ordinal;
            if let Some(content) = &part.content_json {
                validate_json("conversation.part.content_json", content)?;
            }
            transaction.execute(
                "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,content_digest,created_at_ms,updated_at_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![part.id, message.id, part.kind, part.ordinal, part.state, part.content_text, part.content_json, part.content_digest, part.created_at_ms, part.updated_at_ms],
            )?;
        }
    }
    Ok(())
}
pub mod host;

pub mod transcript_preparation;

mod data_source_port;
pub use data_source_port::SqliteDataSourceRepository;

#[cfg(test)]
mod data_source_snapshot_tests;

mod document_source_port;
pub use document_source_port::SqliteDocumentSourceRepository;

#[cfg(test)]
mod document_source_write_tests;

mod document_source_store;

mod conversation_branch;

mod archive_restore;

mod replica_sync_port;
pub use replica_sync_port::SqliteReplicaSyncRepository;

mod body_binding;
mod part_derivation;
mod part_derivation_sync;
pub mod part_nested_source;
pub mod part_semantic_source;
mod protocol_journal_port;
pub use part_derivation::{PartDerivation, PartDerivationSource, PartFieldFingerprint};
pub use protocol_journal_port::SqliteCommandProtocolRepository;

mod conversation_history_port;

mod permission_port;

mod automation_port;
pub use automation_port::SqliteAutomationRepository;

mod plan_work_step_port;

mod idle_checkpoint_port;

mod agent_queue_port;

mod trace_journal_port;

mod product_setting_port;
pub use product_setting_port::SqliteProductSettingRepository;

mod resume_snapshot_port;

mod conversation_query_port;

mod store_maintenance_port;

mod archive_projection_port;
mod automation_execution_store;

mod generative_ui;

mod goal_execution;

mod expert_message;
