// SPDX-License-Identifier: Apache-2.0
//! Atomic local Run creation on the caller's repository unit of work.
use crate::memory::StoredMemoryUsageEntry;
use crate::run_effect::{OutboxEffectInput, RunEffectRepository};
use nola_domain::MessagePartInput;
use serde::{Deserialize, Serialize};

/// A real initial Core envelope. The Store allocates its independent Run
/// ledger sequence; Core cursor/revision remain immutable evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitialCoreRunEvent {
    pub core_session_id: String,
    pub core_turn_id: Option<String>,
    pub core_cursor: u64,
    pub core_revision: u64,
    pub event_kind: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeginTurnInput {
    pub initial_events: Vec<InitialCoreRunEvent>,
    pub conversation_id: String,
    pub expected_configuration_revision: Option<i64>,
    pub expected_parent_message_id: Option<String>,
    pub conversation_title: String,
    pub user_message_id: String,
    pub user_part_id: String,
    pub user_part_kind: String,
    pub user_text: String,
    #[serde(default)]
    pub user_content_json: Option<String>,
    pub additional_user_parts: Vec<MessagePartInput>,
    pub assistant_message_id: String,
    pub assistant_part_id: String,
    pub assistant_turn_bindings: Vec<nola_domain::AssistantTurnBinding>,
    /// Ordered accepted initial Core Part changes under the exact admitted
    /// assistant owner. Applied atomically before its typed Part bindings.
    pub initial_assistant_part_mutations: Vec<nola_domain::PartMutation>,
    pub run_id: String,
    pub context_checkpoint_id: Option<String>,
    pub checkpoint_revision: i64,
    pub checkpoint_event_cursor: i64,
    pub checkpoint_json: String,
    pub invocation_config_json: String,
    pub invocation_config_digest: String,
    pub outbox_effects: Vec<OutboxEffectInput>,
    #[serde(default)]
    pub model_attempt_mutations: Vec<nola_domain::ModelAttemptMutation>,
    #[serde(default)]
    pub delegated_invocation_mutations: Vec<nola_domain::DelegatedInvocationMutation>,
    #[serde(default)]
    pub delegated_permission_mutations: Vec<nola_domain::DelegatedPermissionDeliveryMutation>,
    #[serde(default)]
    pub memory_usage_entries: Vec<StoredMemoryUsageEntry>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeginTurnResult {
    pub conversation_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub run_id: String,
    pub user_ordinal: i64,
    pub assistant_ordinal: i64,
}

/// Promotes exactly one committed FIFO NextTurn admission after its predecessor
/// has sealed. Existing visible user messages retain their original ownership.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeginSuccessorTurnInput {
    pub previous_run_id: String,
    pub expected_checkpoint_revision: i64,
    pub admission_id: String,
    pub originating_submission_id: String,
    pub ensured_user_messages: Vec<nola_domain::TransitionMessageInput>,
    pub turn: BeginTurnInput,
}

/// Starts a conversation Run that was triggered by durable product state
/// rather than a new visible user message. The Run references the latest
/// visible user message and reuses its existing assistant message slot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeginInternalTurnInput {
    pub initial_events: Vec<InitialCoreRunEvent>,
    pub conversation_id: String,
    pub assistant_message_id: String,
    pub expected_previous_run_id: String,
    pub expected_checkpoint_revision: Option<i64>,
    pub assistant_part_id: String,
    pub assistant_turn_bindings: Vec<nola_domain::AssistantTurnBinding>,
    pub initial_assistant_part_mutations: Vec<nola_domain::PartMutation>,
    pub run_id: String,
    pub context_checkpoint_id: Option<String>,
    pub checkpoint_revision: i64,
    pub checkpoint_event_cursor: i64,
    pub checkpoint_json: String,
    pub invocation_config_json: String,
    pub invocation_config_digest: String,
    pub outbox_effects: Vec<OutboxEffectInput>,
    #[serde(default)]
    pub model_attempt_mutations: Vec<nola_domain::ModelAttemptMutation>,
    #[serde(default)]
    pub delegated_invocation_mutations: Vec<nola_domain::DelegatedInvocationMutation>,
    #[serde(default)]
    pub delegated_permission_mutations: Vec<nola_domain::DelegatedPermissionDeliveryMutation>,
    #[serde(default)]
    pub memory_usage_entries: Vec<StoredMemoryUsageEntry>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeginMaintenanceRunInput {
    pub initial_events: Vec<InitialCoreRunEvent>,
    pub conversation_id: String,
    pub run_id: String,
    pub context_checkpoint_id: Option<String>,
    pub checkpoint_revision: i64,
    pub checkpoint_event_cursor: i64,
    pub checkpoint_json: String,
    pub invocation_config_json: String,
    pub invocation_config_digest: String,
    pub outbox_effects: Vec<OutboxEffectInput>,
    #[serde(default)]
    pub model_attempt_mutations: Vec<nola_domain::ModelAttemptMutation>,
    #[serde(default)]
    pub delegated_invocation_mutations: Vec<nola_domain::DelegatedInvocationMutation>,
    #[serde(default)]
    pub delegated_permission_mutations: Vec<nola_domain::DelegatedPermissionDeliveryMutation>,
    #[serde(default)]
    pub memory_usage_entries: Vec<StoredMemoryUsageEntry>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeginMaintenanceRunResult {
    pub conversation_id: String,
    pub run_id: String,
}

/// Creation must use the same connection/transaction as the caller's receipt.
/// Implementations must not commit an enclosing unit of work or open a second writer.
pub trait RunCreationRepository: RunEffectRepository {
    /// Authoritative Surface state owned by this persisted context participant.
    /// A host-supplied participant or current model selection is not authority.
    fn ui_surfaces_for_context(
        &self,
        context_id: &str,
    ) -> Result<Vec<nola_domain::generative_ui::UiSurfaceState>, Self::Error>;
    /// Returns the actual visible assistant slot and its current Run selected
    /// for internal resumption. Admission must compare this identity again in
    /// the writer transaction rather than silently substituting another slot.
    fn internal_assistant_target(
        &self,
        conversation_id: &str,
    ) -> Result<Option<crate::assistant_ownership::AssistantOwner>, Self::Error>;
    fn begin_turn(&mut self, input: &BeginTurnInput) -> Result<BeginTurnResult, Self::Error>;
    fn begin_successor_turn(
        &mut self,
        input: &BeginSuccessorTurnInput,
    ) -> Result<BeginTurnResult, Self::Error>;
    fn begin_internal_turn(
        &mut self,
        input: &BeginInternalTurnInput,
    ) -> Result<BeginTurnResult, Self::Error>;
    fn begin_maintenance_run(
        &mut self,
        input: &BeginMaintenanceRunInput,
    ) -> Result<BeginMaintenanceRunResult, Self::Error>;
}
