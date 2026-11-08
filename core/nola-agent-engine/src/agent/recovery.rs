// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use nola_domain::core_event::{CoreEventEnvelope, EventCursor};

// Canonical executable-call signatures exclude authorization metadata and use
// schema-normalized arguments. Old recovery ledgers cannot be reinterpreted.
pub const AGENT_CHECKPOINT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreCheckpoint<S> {
    pub schema_version: u32,
    pub session_id: String,
    pub revision: u64,
    pub last_event_cursor: EventCursor,
    pub capability_catalog_revision: u64,
    pub created_at_ms: u64,
    pub reason: CheckpointReason,
    pub state: S,
}

impl CoreCheckpoint<super::session::AgentThreadState> {
    /// Load settled conversation history for a NEW Turn, never an execution
    /// recovery. A previous Run's Host binding is opaque historical metadata,
    /// not the successor's credentials, schema, or generation configuration.
    /// The persisted checkpoint is left untouched. Active/queued effects must
    /// use strict checkpoint deserialization and exact Run recovery instead.
    pub fn decode_history_for_new_turn(mut value: serde_json::Value) -> Result<Self, String> {
        for path in [
            "/state/configuration/turn_identity/host_invocation",
            "/state/execution/active_turn/context/configuration/turn_identity/host_invocation",
        ] {
            if let Some(binding) = value.pointer_mut(path) {
                *binding = serde_json::Value::Null;
            }
        }
        let checkpoint: Self = serde_json::from_value(value)
            .map_err(|error| format!("decode conversation history failed: {error}"))?;
        checkpoint.validate_state_identity()?;
        if !checkpoint.state.permits_new_turn_execution_configuration() {
            return Err("conversation history still owns active execution; exact Run recovery is required".into());
        }
        Ok(checkpoint)
    }
}

impl<S: std::borrow::Borrow<super::session::AgentThreadState>> CoreCheckpoint<S> {
    /// Shared admission check for both recovery and direct durable commits.
    /// Supports borrowed checkpoint views without materializing a state clone.
    pub fn validate_state_identity(&self) -> Result<(), String> {
        let state = self.state.borrow();
        if self.schema_version != AGENT_CHECKPOINT_SCHEMA_VERSION {
            return Err(format!(
                "unsupported Agent checkpoint schema {}",
                self.schema_version
            ));
        }
        if self.session_id.is_empty() || state.session_id.as_str() != self.session_id {
            return Err("Agent checkpoint session identity is inconsistent".into());
        }
        if self.revision != state.journal.revision {
            return Err("Agent checkpoint revision is inconsistent".into());
        }
        if state.journal.next_event_sequence.checked_sub(1) != Some(self.last_event_cursor.sequence)
        {
            return Err("Agent checkpoint event cursor is inconsistent".into());
        }
        if self.capability_catalog_revision != state.host.catalog.revision {
            return Err("Agent checkpoint Host catalog revision is inconsistent".into());
        }
        for (item_id, ownership) in &state.item_ownership {
            if let super::session::AgentItemSource::CodeCell { cell } = &ownership.source {
                cell.validate()?;
                // Historical canonical Run IDs may differ from Core Turn IDs;
                // branches also retain ancestor cell identities. Do not treat
                // this provenance as authority to resume the original worker.
                if !state.conversation.transcript.iter().any(|item|
                    item.id().as_str() == item_id && matches!(item,
                        super::AgentItem::ToolCall { .. } | super::AgentItem::ToolResult { .. }))
                {
                    return Err("code item source must identify a retained tool record".into());
                }
                if state.conversation.model_context.iter().any(|item| item.id().as_str() == item_id) {
                    return Err("code intermediate record leaked into model context".into());
                }
            }
        }
        state.execution.input_queue.validate()?;
        let retired_turn = state.execution.retired_code_cells.values().next()
            .map(|cell| cell.identity().turn_id.as_str());
        for (id, cell) in &state.execution.retired_code_cells {
            cell.validate()?;
            if id != &cell.identity().cell_id || cell.identity().conversation_id != self.session_id
                || !cell.is_settled()
                || Some(cell.identity().turn_id.as_str()) != retired_turn
                || state.execution.active_turn.as_ref().is_some_and(|turn| turn.code_cells.contains_key(id)) {
                return Err("retired code cell ownership or lifecycle is inconsistent".into());
            }
        }
        state.execution.permissions.validate_receipt_sources()?;
        state.goal_projection.validate_operation_ledger()?;
        state.execution.requirements.validate(&state.execution.input_queue)?;
        state.configuration.turn_identity.validate_model_capability()?;
        if let Some(turn) = &state.execution.active_turn {
            if turn.code_observation_failures.iter().any(|id| !turn.code_cells.contains_key(id)) {
                return Err("code observation failure refers to an unknown cell".into());
            }
            for (cell_id, cell) in &turn.code_cells {
                cell.validate()?;
                let owner = cell.identity();
                if cell_id != &owner.cell_id || owner.conversation_id != self.session_id
                    || owner.turn_id != turn.turn_id.as_str()
                    || !state.conversation.projection.parts.values().any(|part| matches!(part,
                        super::parts::AgentPart::Tool(tool)
                        if tool.call_id == owner.parent_call_id && tool.capability_id == "code_mode_exec"
                            && tool.ownership.turn_id == owner.turn_id)) {
                    return Err("code cell checkpoint ownership is inconsistent".into());
                }
                if turn.status.is_terminal() && !cell.is_terminal() {
                    return Err("terminal turn still owns a running code cell".into());
                }
                if cell.has_yielded() && !state.conversation.projection.parts.values().any(|part| matches!(part,
                    super::parts::AgentPart::Tool(tool)
                    if tool.call_id == owner.parent_call_id && tool.ownership.turn_id == owner.turn_id
                        && tool.state == super::parts::ToolPartState::Completed)) {
                    return Err("yielded code cell has no completed foreground call".into());
                }
                for target in cell.pending_cancellation_effects() {
                    let effect = state.host.pending_effects.iter()
                        .find(|effect| effect.effect_id == target)
                        .ok_or("code cell cancellation target is absent from effect journal")?;
                    if effect.turn_id.as_deref() != Some(owner.turn_id.as_str()) {
                        return Err("code cell cancellation target belongs to another turn".into());
                    }
                    let control = effect.validated_control()?;
                    let owned_call = effect.call_id.as_deref()
                        .is_some_and(|id| cell.owns_execution_call(id));
                    let owned_receipt = matches!(&control.payload.policy,
                        nola_domain::host_control::HostResponsePolicy::ResolveCodeTool { cell: receipt_owner, .. }
                            | nola_domain::host_control::HostResponsePolicy::PollCodeCell { cell: receipt_owner }
                        if receipt_owner == owner);
                    if !owned_call && !owned_receipt {
                        return Err("code cell cancellation target belongs to another execution".into());
                    }
                }
                for pending in cell.pending_calls() {
                    let nested_id = cell.nested_call_id(pending.sequence);
                    if !turn.tools.iter().any(|tool| tool.call_id.as_str() == nested_id
                        && tool.name == pending.tool_name) {
                        return Err("code cell pending request has no canonical tool call".into());
                    }
                }
            }
            turn.engineering.no_progress_recovery.validate()?;
            let ledger = &turn.engineering.strategy_recovery;
            if ledger.model_requests.iter().any(|number| *number == 0 || *number > turn.model_request_count)
                || ledger.model_requests.len() > turn.context.configuration.max_strategy_recovery_attempts as usize
                || ledger.rejected_calls.iter().any(|(signature, call_id)| signature.len() != 64 || call_id.is_empty())
                || ledger.observation_witnesses.iter().any(|(call_id, witness)|
                    call_id.is_empty() || witness.signature.len() != 64 || witness.output_digest.len() != 32)
                || ledger.resource_generations.iter().any(|(resource, generation)| resource.is_empty() || *generation == 0)
                || ledger.observation_dependencies.iter().any(|(signature, dependencies)|
                    signature.len() != 64 || dependencies.iter().any(|(resource, generation)|
                        resource.is_empty() || *generation > ledger.generation_for_dependency(resource)))
            {
                return Err("strategy recovery ledger is inconsistent with its Turn budget".into());
            }
            turn.context.configuration.turn_identity.validate_model_capability()?;
        }
        let mut commands = std::collections::BTreeSet::new();
        for (id, wait) in &state.execution.user_input_waits {
            wait.validate()?;
            if id != &wait.wait_id || wait.core_session_id != self.session_id || wait.created_at_revision > self.revision {
                return Err("user input wait checkpoint ownership is inconsistent".into());
            }
            if wait.is_open() && !state.execution.active_turn.as_ref().is_some_and(|turn|
                turn.turn_id.as_str() == wait.turn_id && !turn.status.is_terminal()
                && wait.originating_call_id.as_ref().is_none_or(|id| turn.tools.iter().any(|tool|
                    tool.call_id.as_str() == id && tool.status == nola_domain::ToolCallStatus::AwaitingUserInput))) {
                return Err("open user input wait has no live owning Turn/tool".into());
            }
            if let nola_domain::user_input_wait::UserInputWaitState::Resolved { command_id, .. } = &wait.state {
                if !commands.insert(command_id) { return Err("duplicate user input response command identity".into()); }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointReason {
    SessionStarted,
    ConfigurationChanged,
    TurnCompleted,
    CompactionCompleted,
    PermissionResolved,
    PermissionTransportInterrupted,
    DelegatedExecutionInterrupted,
    CancellationProgress,
    TaskUpdated,
    GoalUpdated,
    MemoryUpdated,
    SkillUpdated,
    HostDisconnected,
    ApplicationSuspended,
    SessionClosing,
    SessionClosed,
    Manual,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSequencer {
    next_sequence: u64,
}

impl EventSequencer {
    pub fn resume_after(cursor: EventCursor) -> Self {
        Self {
            next_sequence: cursor.sequence.saturating_add(1),
        }
    }

    pub fn next_cursor(&mut self) -> EventCursor {
        let cursor = EventCursor {
            sequence: self.next_sequence,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        cursor
    }
}

/// Orders replayed or remotely delivered events without exposing gaps to UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayAccumulator<E> {
    pub applied: EventCursor,
    buffered: BTreeMap<u64, CoreEventEnvelope<E>>,
}

impl<E> ReplayAccumulator<E> {
    pub fn new(applied: EventCursor) -> Self {
        Self {
            applied,
            buffered: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, event: CoreEventEnvelope<E>) -> Vec<CoreEventEnvelope<E>> {
        if event.cursor.sequence <= self.applied.sequence {
            return Vec::new();
        }
        self.buffered.entry(event.cursor.sequence).or_insert(event);

        let mut ready = Vec::new();
        loop {
            let expected = self.applied.sequence.saturating_add(1);
            let Some(event) = self.buffered.remove(&expected) else {
                break;
            };
            self.applied = event.cursor;
            ready.push(event);
        }
        ready
    }

    pub fn missing_range(&self) -> Option<(EventCursor, EventCursor)> {
        let first_buffered = *self.buffered.keys().next()?;
        let expected = self.applied.sequence.saturating_add(1);
        if first_buffered <= expected {
            None
        } else {
            Some((
                EventCursor { sequence: expected },
                EventCursor {
                    sequence: first_buffered - 1,
                },
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingHostEffect {
    pub effect_id: String,
    pub request_id: String,
    pub host_id: String,
    #[serde(default)]
    pub lease_id: Option<String>,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub catalog_revision: u64,
    #[serde(default)]
    pub availability_epoch: u64,
    pub capability_id: String,
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub replay_safety: super::host::HostReplaySafety,
    pub state: PendingHostEffectState,
    #[serde(default)]
    pub target_effect_id: Option<String>,
    #[serde(default)]
    pub cancel_effect_id: Option<String>,
    #[serde(default)]
    pub last_response_sequence: Option<u64>,
    #[serde(default)]
    pub request: Option<serde_json::Value>,
    /// Derived once from the frozen payload; never trusted from persisted JSON.
    #[serde(skip)]
    pub control: std::sync::OnceLock<nola_domain::host_control::HostEffectControl>,
}

impl PartialEq for PendingHostEffect {
    fn eq(&self, other: &Self) -> bool {
        // A lazily reconstructed control cache is not checkpoint state.
        self.effect_id == other.effect_id
            && self.request_id == other.request_id
            && self.host_id == other.host_id
            && self.lease_id == other.lease_id
            && self.session_id == other.session_id
            && self.turn_id == other.turn_id
            && self.catalog_revision == other.catalog_revision
            && self.availability_epoch == other.availability_epoch
            && self.capability_id == other.capability_id
            && self.call_id == other.call_id
            && self.task_id == other.task_id
            && self.replay_safety == other.replay_safety
            && self.state == other.state
            && self.target_effect_id == other.target_effect_id
            && self.cancel_effect_id == other.cancel_effect_id
            && self.last_response_sequence == other.last_response_sequence
            && self.request == other.request
    }
}
impl Eq for PendingHostEffect {}

#[cfg(test)]
thread_local! {
    static FULL_REQUEST_MATERIALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl PendingHostEffect {
    pub fn validated_control(
        &self,
    ) -> Result<&nola_domain::host_control::HostEffectControl, String> {
        if self.control.get().is_none() {
            let request = self.validated_request()?;
            let control = nola_domain::host_control::HostEffectControl::from_request(&request)?;
            let _ = self.control.set(control);
        }
        let control = self
            .control
            .get()
            .ok_or("pending control was not initialized")?;
        if control.effect_id.0 != self.effect_id
            || control.request_id != self.request_id
            || control.host_id.0 != self.host_id
            || control.lease_id != self.lease_id
            || control.session_id != self.session_id
            || control.turn_id != self.turn_id
            || control.catalog_revision != self.catalog_revision
            || control.availability_epoch != self.availability_epoch
        {
            return Err("pending Host control differs from effect journal".into());
        }
        Ok(control)
    }
    /// The frozen request and journal identity form one durable effect. Never
    /// execute or accept a response against only one half of that identity.
    pub fn validated_request(&self) -> Result<super::host::HostRequestEnvelope, String> {
        #[cfg(test)]
        FULL_REQUEST_MATERIALIZATIONS.with(|count| count.set(count.get() + 1));
        let request: super::host::HostRequestEnvelope = serde::Deserialize::deserialize(
            self.request
                .as_ref()
                .ok_or("pending Host effect has no request")?,
        )
        .map_err(|error| format!("invalid pending Host request: {error}"))?;
        if request.effect_id.0 != self.effect_id
            || request.request_id != self.request_id
            || request.host_id.0 != self.host_id
            || request.lease_id != self.lease_id
            || request.session_id != self.session_id
            || request.turn_id != self.turn_id
            || request.catalog_revision != self.catalog_revision
            || request.availability_epoch != self.availability_epoch
        {
            return Err("pending Host request identity differs from effect journal".into());
        }
        Ok(request)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingHostEffectState {
    Dispatched,
    PermissionInterrupted {
        reason: nola_host_protocol::PermissionTransportInterruptionReason,
    },
    DelegatedInterrupted {
        reason: nola_host_protocol::DelegatedExecutionInterruptionReason,
    },
    Accepted,
    Streaming,
    Cancelling,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEffectJournal {
    effects: BTreeMap<String, PendingHostEffect>,
    #[serde(default)]
    completed_effects: BTreeMap<String, CompletedHostEffectIdentity>,
    #[serde(default)]
    completed_effect_order: std::collections::VecDeque<String>,
    #[serde(default)]
    buffered_batch_terminal_responses: BTreeMap<String, BTreeMap<usize, serde_json::Value>>,
}

const MAX_COMPLETED_HOST_EFFECT_IDENTITIES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CompletedHostEffectIdentity {
    request_id: String,
    host_id: String,
    lease_id: Option<String>,
    session_id: String,
    turn_id: Option<String>,
    catalog_revision: u64,
    availability_epoch: u64,
    last_response_sequence: Option<u64>,
}

impl HostEffectJournal {
    pub fn register(&mut self, mut effect: PendingHostEffect) -> bool {
        if self.effects.contains_key(&effect.effect_id)
            || self.completed_effects.contains_key(&effect.effect_id)
        {
            return false;
        }
        effect.control = Default::default();
        if effect.validated_control().is_err() {
            return false;
        }
        self.effects.insert(effect.effect_id.clone(), effect);
        true
    }

    pub fn complete(&mut self, effect_id: &str) -> Option<PendingHostEffect> {
        let effect = self.effects.remove(effect_id)?;
        self.completed_effects.insert(
            effect_id.to_string(),
            CompletedHostEffectIdentity {
                request_id: effect.request_id.clone(),
                host_id: effect.host_id.clone(),
                lease_id: effect.lease_id.clone(),
                session_id: effect.session_id.clone(),
                turn_id: effect.turn_id.clone(),
                catalog_revision: effect.catalog_revision,
                availability_epoch: effect.availability_epoch,
                last_response_sequence: effect.last_response_sequence,
            },
        );
        self.completed_effect_order.push_back(effect_id.to_string());
        while self.completed_effect_order.len() > MAX_COMPLETED_HOST_EFFECT_IDENTITIES {
            if let Some(expired) = self.completed_effect_order.pop_front() {
                self.completed_effects.remove(&expired);
            }
        }
        Some(effect)
    }

    pub fn batch_id_for_effect(&self, effect_id: &str) -> Option<String> {
        self.effects
            .get(effect_id)
            .and_then(effect_batch_identity)
            .map(|(batch_id, _)| batch_id)
    }

    /// Buffers only terminal responses that arrived before a lower-index
    /// member of the same Core-planned batch. Progress remains live.
    pub fn defer_out_of_order_batch_terminal(
        &mut self,
        response: &super::host::HostResponseEnvelope,
    ) -> Result<bool, String> {
        if !response.payload.is_terminal() {
            return Ok(false);
        }
        let Some((batch_id, index)) = self
            .effects
            .get(&response.effect_id.0)
            .and_then(effect_batch_identity)
        else {
            return Ok(false);
        };
        let lower_pending = self.effects.values().any(|effect| {
            effect_batch_identity(effect).is_some_and(|(candidate_batch, candidate_index)| {
                candidate_batch == batch_id && candidate_index < index
            })
        });
        if !lower_pending {
            return Ok(false);
        }
        let encoded = serde_json::to_value(response)
            .map_err(|error| format!("failed to buffer Host batch response: {error}"))?;
        self.buffered_batch_terminal_responses
            .entry(batch_id)
            .or_default()
            .entry(index)
            .or_insert(encoded);
        Ok(true)
    }

    pub fn take_ready_buffered_batch_terminal(
        &mut self,
        batch_id: &str,
    ) -> Result<Option<super::host::HostResponseEnvelope>, String> {
        loop {
            let Some((&index, encoded)) = self
                .buffered_batch_terminal_responses
                .get(batch_id)
                .and_then(|responses| responses.first_key_value())
                .map(|(index, response)| (index, response.clone()))
            else {
                self.buffered_batch_terminal_responses.remove(batch_id);
                return Ok(None);
            };
            let response: super::host::HostResponseEnvelope = serde_json::from_value(encoded)
                .map_err(|error| format!("invalid buffered Host batch response: {error}"))?;
            if !self.effects.contains_key(&response.effect_id.0) {
                if let Some(responses) = self.buffered_batch_terminal_responses.get_mut(batch_id) {
                    responses.remove(&index);
                }
                continue;
            }
            let lower_pending = self.effects.values().any(|effect| {
                effect_batch_identity(effect).is_some_and(|(candidate_batch, candidate_index)| {
                    candidate_batch == batch_id && candidate_index < index
                })
            });
            if lower_pending {
                return Ok(None);
            }
            if let Some(responses) = self.buffered_batch_terminal_responses.get_mut(batch_id) {
                responses.remove(&index);
                if responses.is_empty() {
                    self.buffered_batch_terminal_responses.remove(batch_id);
                }
            }
            return Ok(Some(response));
        }
    }

    pub fn accept_response(
        &mut self,
        response: &super::host::HostResponseEnvelope,
    ) -> HostEffectAcceptance {
        let acceptance = self.validate_delivery(
            &response.effect_id.0,
            &response.request_id,
            &response.host_id.0,
            response.lease_id.as_deref(),
            &response.session_id,
            response.turn_id.as_deref(),
            response.catalog_revision,
            response.availability_epoch,
            response.sequence,
        );
        if acceptance != HostEffectAcceptance::Accepted {
            return acceptance;
        }
        let Some(effect) = self.effects.get(&response.effect_id.0) else {
            return HostEffectAcceptance::UnknownEffect;
        };
        let Ok(request) = effect.validated_control() else {
            return HostEffectAcceptance::WrongResponseType;
        };
        if request.delegated_invocation_id != response.delegated_invocation_id {
            return HostEffectAcceptance::WrongDelegatedInvocation;
        }
        if !request.payload.accepts_response(&response.payload) {
            return HostEffectAcceptance::WrongResponseType;
        }
        if let Some(effect) = self.effects.get_mut(&response.effect_id.0) {
            effect.last_response_sequence = Some(response.sequence);
        }
        HostEffectAcceptance::Accepted
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_delivery(
        &self,
        effect_id: &str,
        request_id: &str,
        host_id: &str,
        lease_id: Option<&str>,
        session_id: &str,
        turn_id: Option<&str>,
        catalog_revision: u64,
        availability_epoch: u64,
        sequence: u64,
    ) -> HostEffectAcceptance {
        let Some(effect) = self.effects.get(effect_id) else {
            let Some(completed) = self.completed_effects.get(effect_id) else {
                return HostEffectAcceptance::UnknownEffect;
            };
            if completed.request_id != request_id {
                return HostEffectAcceptance::WrongRequest;
            }
            if completed.host_id != host_id || completed.session_id != session_id {
                return HostEffectAcceptance::WrongOwner;
            }
            if completed.lease_id.as_deref() != lease_id {
                return HostEffectAcceptance::StaleLease;
            }
            if completed.turn_id.as_deref() != turn_id {
                return HostEffectAcceptance::WrongTurn;
            }
            if completed.catalog_revision != catalog_revision {
                return HostEffectAcceptance::StaleCatalog;
            }
            if completed.availability_epoch != availability_epoch {
                return HostEffectAcceptance::WrongOwner;
            }
            return HostEffectAcceptance::DuplicateResponse;
        };
        if effect.request_id != request_id {
            return HostEffectAcceptance::WrongRequest;
        }
        if effect.host_id != host_id || effect.session_id != session_id {
            return HostEffectAcceptance::WrongOwner;
        }
        if effect.lease_id.as_deref() != lease_id {
            return HostEffectAcceptance::StaleLease;
        }
        if effect.turn_id.as_deref() != turn_id {
            return HostEffectAcceptance::WrongTurn;
        }
        if effect.catalog_revision != catalog_revision {
            return HostEffectAcceptance::StaleCatalog;
        }
        if effect.availability_epoch != availability_epoch {
            return HostEffectAcceptance::WrongOwner;
        }
        if effect
            .last_response_sequence
            .is_some_and(|last_sequence| sequence <= last_sequence)
        {
            return HostEffectAcceptance::NonMonotonicSequence;
        }
        HostEffectAcceptance::Accepted
    }

    pub fn mark_accepted(&mut self, effect_id: &str) {
        if let Some(effect) = self.effects.get_mut(effect_id) {
            if effect.state == PendingHostEffectState::Dispatched {
                effect.state = PendingHostEffectState::Accepted;
            }
        }
    }

    pub fn mark_permission_interrupted(
        &mut self,
        effect_id: &str,
        reason: nola_host_protocol::PermissionTransportInterruptionReason,
    ) {
        if let Some(effect) = self.effects.get_mut(effect_id) {
            if effect.state != PendingHostEffectState::Cancelling {
                effect.state = PendingHostEffectState::PermissionInterrupted { reason };
            }
        }
    }

    pub fn mark_delegated_interrupted(
        &mut self,
        effect_id: &str,
        reason: nola_host_protocol::DelegatedExecutionInterruptionReason,
    ) {
        if let Some(effect) = self.effects.get_mut(effect_id) {
            if effect.state != PendingHostEffectState::Cancelling {
                effect.state = PendingHostEffectState::DelegatedInterrupted { reason };
            }
        }
    }

    pub fn mark_streaming(&mut self, effect_id: &str) {
        if let Some(effect) = self.effects.get_mut(effect_id) {
            if effect.state != PendingHostEffectState::Cancelling {
                effect.state = PendingHostEffectState::Streaming;
            }
        }
    }

    pub fn bind_task(&mut self, effect_id: &str, task_id: &str) -> bool {
        let Some(effect) = self.effects.get_mut(effect_id) else {
            return false;
        };
        if effect.task_id.is_some() || task_id.trim().is_empty() {
            return false;
        }
        effect.task_id = Some(task_id.to_string());
        true
    }

    pub fn begin_cancellation(&mut self, target_effect_id: &str, cancel_effect_id: &str) -> bool {
        let Some(target) = self.effects.get_mut(target_effect_id) else {
            return false;
        };
        if target.state == PendingHostEffectState::Cancelling {
            return false;
        }
        target.state = PendingHostEffectState::Cancelling;
        target.cancel_effect_id = Some(cancel_effect_id.to_string());
        true
    }

    pub fn acknowledge_cancellation(
        &mut self,
        cancel_effect_id: &str,
        target_effect_id: &str,
    ) -> HostEffectAcceptance {
        let Some(cancel) = self.effects.get(cancel_effect_id) else {
            return if self.completed_effects.contains_key(cancel_effect_id) {
                HostEffectAcceptance::DuplicateResponse
            } else {
                HostEffectAcceptance::UnknownEffect
            };
        };
        if cancel.target_effect_id.as_deref() != Some(target_effect_id) {
            return HostEffectAcceptance::WrongRequest;
        }
        self.complete(cancel_effect_id);
        self.complete(target_effect_id);
        HostEffectAcceptance::Accepted
    }

    pub fn get(&self, effect_id: &str) -> Option<&PendingHostEffect> {
        self.effects.get(effect_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &PendingHostEffect> {
        self.effects.values()
    }

    pub fn restore_controls(&mut self) -> Result<(), String> {
        for effect in self.effects.values_mut() {
            effect.control = Default::default();
            effect.validated_control()?;
        }
        Ok(())
    }

    /// Removes effects owned by a Turn that cannot be recovered without
    /// persisting ephemeral model-visible context.
    pub fn discard_turn(&mut self, turn_id: &str) {
        let discarded_batches = self
            .effects
            .values()
            .filter(|effect| effect.turn_id.as_deref() == Some(turn_id))
            .filter_map(effect_batch_identity)
            .map(|(batch_id, _)| batch_id)
            .collect::<Vec<_>>();
        self.effects
            .retain(|_, effect| effect.turn_id.as_deref() != Some(turn_id));
        for batch_id in discarded_batches {
            self.buffered_batch_terminal_responses.remove(&batch_id);
        }
    }

    /// Keeps durable effect identity while removing the materialized request.
    /// The Application restores the request from its committed Store outbox.
    pub fn redact_turn_requests(&mut self, turn_id: &str) {
        for effect in self.effects.values_mut() {
            if effect.turn_id.as_deref() == Some(turn_id) {
                effect.request = None;
                effect.control = Default::default();
            }
        }
    }

    pub fn rebind_for_replay(
        &mut self,
        effect_id: &str,
        request_id: String,
        host_id: String,
        lease_id: Option<String>,
        catalog_revision: u64,
        availability_epoch: u64,
        request: serde_json::Value,
    ) -> Result<(), String> {
        let request = serde_json::from_value::<super::host::HostRequestEnvelope>(request)
            .map_err(|error| format!("invalid recovery request: {error}"))?;
        if request.effect_id.0 != effect_id
            || request.request_id != request_id
            || request.host_id.0 != host_id
            || request.lease_id != lease_id
            || request.catalog_revision != catalog_revision
            || request.availability_epoch != availability_epoch
        {
            return Err("recovery request binding does not match its envelope".into());
        }
        let control = nola_domain::host_control::HostEffectControl::from_request(&request)?;
        self.bind_control(&control)
    }

    /// Materialize a model input from Application's verified immutable outbox
    /// after pure reducer replay. This does not dispatch, renew a lease, change
    /// status/cursors, or authorize tools. Ordinary rebind remains hash-strict.
    pub fn restore_committed_model_input(
        &mut self,
        request: &super::host::HostRequestEnvelope,
    ) -> Result<(), String> {
        let effect = self.effects.get_mut(&request.effect_id.0)
            .ok_or("frozen model input has no pending effect")?;
        let frozen = nola_domain::host_control::HostEffectControl::from_request(request)?;
        let previous = effect.validated_control()?.clone();
        let mut expected = previous.clone();
        expected.apply_binding(&frozen.binding())?;
        expected.payload.payload_identity = frozen.payload.payload_identity.clone();
        if expected != frozen {
            return Err("frozen model input changed effect identity or response policy".into());
        }
        let raw = effect.request.as_ref().ok_or("pending model input is missing")?;
        let mut generated_payload = raw.get("payload").cloned().ok_or("pending model payload missing")?;
        let frozen_payload = serde_json::to_value(&request.payload).map_err(|e| e.to_string())?;
        let mut frozen_identity = frozen_payload.clone();
        for payload in [&mut generated_payload, &mut frozen_identity] {
            let object = payload.as_object_mut().ok_or("model payload is not an object")?;
            if object.get("type").and_then(|v| v.as_str()) != Some("model_stream") {
                return Err("frozen input materialization only accepts model streams".into());
            }
            for field in ["messages", "tools", "settings"] { object.remove(field); }
        }
        if generated_payload != frozen_identity {
            return Err("frozen model input changed model or output part identity".into());
        }
        let mut control = previous;
        control.payload = frozen.payload;
        let raw = effect.request.as_mut().and_then(|v| v.as_object_mut())
            .ok_or("pending model request is not an object")?;
        raw.insert("payload".into(), frozen_payload);
        effect.control = std::sync::OnceLock::from(control);
        Ok(())
    }

    pub fn bind_control(
        &mut self,
        control: &nola_domain::host_control::HostEffectControl,
    ) -> Result<(), String> {
        self.bind_control_inner(control, false)
    }

    /// Accept metadata for a correlated response without reopening execution.
    pub fn bind_response_control(&mut self, control: &nola_domain::host_control::HostEffectControl) -> Result<(), String> {
        self.bind_control_inner(control, true)
    }

    fn bind_control_inner(&mut self, control: &nola_domain::host_control::HostEffectControl, response_only: bool) -> Result<(), String> {
        let effect = self
            .effects
            .get_mut(&control.effect_id.0)
            .ok_or("committed Host effect is not pending")?;
        if effect.state == PendingHostEffectState::Cancelling && !response_only {
            return Err("cancelling Host effect cannot be rebound".into());
        }
        let previous = effect.validated_control()?;
        let mut expected = previous.clone();
        expected.apply_binding(&control.binding())?;
        if expected != *control {
            return Err(
                "committed Host effect changed immutable payload or response policy".into(),
            );
        }
        let metadata = serde_json::to_value(control).map_err(|e| e.to_string())?;
        // Validate every fallible operation before changing journal or payload.
        let raw = effect
            .request
            .as_ref()
            .and_then(|value| value.as_object())
            .ok_or("pending Host request is not an object")?;
        let payload_binding = match &control.payload.policy {
            nola_domain::host_control::HostResponsePolicy::ResolveDelegatedPermission {
                delivery,
                ..
            } => {
                if raw.get("payload").and_then(|v| v.as_object()).is_none() {
                    return Err("pending permission payload is not an object".into());
                }
                Some(("delivery", serde_json::to_value(delivery).map_err(|e| e.to_string())?, true))
            }
            nola_domain::host_control::HostResponsePolicy::Cancel { target_not_started, .. } => {
                if raw.get("payload").and_then(|v| v.as_object()).is_none() {
                    return Err("pending cancellation payload is not an object".into());
                }
                Some(("target_not_started", serde_json::to_value(target_not_started).map_err(|e| e.to_string())?, false))
            }
            _ => None,
        };
        let raw = effect.request.as_mut().unwrap().as_object_mut().unwrap();
        for key in ["attempt_id", "delegated_invocation_id", "batch", "delivery"] {
            raw.remove(key);
        }
        for (key, value) in metadata.as_object().unwrap() {
            if key != "payload" {
                raw.insert(key.clone(), value.clone());
            }
        }
        if let Some((key, value, omit_null)) = payload_binding {
            let payload = raw.get_mut("payload").unwrap().as_object_mut().unwrap();
            if omit_null && value.is_null() {
                payload.remove(key);
            } else {
                payload.insert(key.into(), value);
            }
        }
        effect.request_id = control.request_id.clone();
        effect.host_id = control.host_id.0.clone();
        effect.lease_id = control.lease_id.clone();
        effect.catalog_revision = control.catalog_revision;
        effect.availability_epoch = control.availability_epoch;
        effect.control = std::sync::OnceLock::from(control.clone());
        if effect.state != PendingHostEffectState::Cancelling { effect.state = PendingHostEffectState::Dispatched; }
        Ok(())
    }

    pub fn complete_matching(
        &mut self,
        turn_id: Option<&str>,
        capability_id: Option<&str>,
        call_id: Option<&str>,
    ) -> Option<PendingHostEffect> {
        let effect_id = self
            .effects
            .iter()
            .find(|(_, effect)| {
                turn_id.is_none_or(|turn_id| effect.turn_id.as_deref() == Some(turn_id))
                    && capability_id
                        .is_none_or(|capability_id| effect.capability_id == capability_id)
                    && call_id.is_none_or(|call_id| effect.call_id.as_deref() == Some(call_id))
            })
            .map(|(effect_id, _)| effect_id.clone())?;
        self.complete(&effect_id)
    }

    pub fn complete_task(&mut self, task_id: &str) -> Vec<PendingHostEffect> {
        let effect_ids = self
            .effects
            .iter()
            .filter(|(_, effect)| effect.task_id.as_deref() == Some(task_id))
            .map(|(effect_id, _)| effect_id.clone())
            .collect::<Vec<_>>();
        effect_ids
            .into_iter()
            .filter_map(|effect_id| self.complete(&effect_id))
            .collect()
    }

    pub fn effect_for_call(&self, turn_id: &str, call_id: &str) -> Option<&PendingHostEffect> {
        self.effects.values().find(|effect| {
            effect.turn_id.as_deref() == Some(turn_id)
                && effect.call_id.as_deref() == Some(call_id)
                && effect.target_effect_id.is_none()
        })
    }

    pub fn len(&self) -> usize {
        self.effects.len()
    }

    /// Local reducer fact, not a new Host grant. A buffered terminal response
    /// still owns its frozen execution context after its effect leaves `effects`.
    pub fn is_quiescent(&self) -> bool {
        self.effects.is_empty() && self.buffered_batch_terminal_responses.is_empty()
    }

    pub fn request_values(&self) -> Vec<serde_json::Value> {
        self.effects
            .values()
            .filter_map(|effect| effect.request.clone())
            .collect()
    }
}

fn effect_batch_identity(effect: &PendingHostEffect) -> Option<(String, usize)> {
    let batch = effect.request.as_ref()?.get("batch")?;
    Some((
        batch.get("batch_id")?.as_str()?.to_string(),
        batch.get("index")?.as_u64()?.try_into().ok()?,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostEffectAcceptance {
    Accepted,
    DuplicateResponse,
    UnknownEffect,
    WrongRequest,
    WrongResponseType,
    WrongDelegatedInvocation,
    WrongOwner,
    WrongTurn,
    StaleCatalog,
    StaleLease,
    NonMonotonicSequence,
}

impl HostEffectAcceptance {
    pub fn code(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::DuplicateResponse => "duplicate_response",
            Self::UnknownEffect => "unknown_effect",
            Self::WrongRequest => "wrong_request",
            Self::WrongResponseType => "wrong_response_type",
            Self::WrongDelegatedInvocation => "wrong_delegated_invocation",
            Self::WrongOwner => "wrong_owner",
            Self::WrongTurn => "wrong_turn",
            Self::StaleCatalog => "stale_catalog",
            Self::StaleLease => "stale_lease",
            Self::NonMonotonicSequence => "non_monotonic_sequence",
        }
    }

    pub fn category(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::DuplicateResponse | Self::NonMonotonicSequence => "benign_replay",
            Self::StaleCatalog | Self::StaleLease => "stale_authority",
            Self::UnknownEffect
            | Self::WrongRequest
            | Self::WrongResponseType
            | Self::WrongDelegatedInvocation
            | Self::WrongOwner
            | Self::WrongTurn => "protocol_violation",
        }
    }

    pub fn from_rejection_error(error: &str) -> Option<Self> {
        let code = error
            .strip_prefix("rejected Host response: code=")?
            .split_whitespace()
            .next()?;
        Some(match code {
            "duplicate_response" => Self::DuplicateResponse,
            "unknown_effect" => Self::UnknownEffect,
            "wrong_request" => Self::WrongRequest,
            "wrong_response_type" => Self::WrongResponseType,
            "wrong_delegated_invocation" => Self::WrongDelegatedInvocation,
            "wrong_owner" => Self::WrongOwner,
            "wrong_turn" => Self::WrongTurn,
            "stale_catalog" => Self::StaleCatalog,
            "stale_lease" => Self::StaleLease,
            "non_monotonic_sequence" => Self::NonMonotonicSequence,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::host::{
        HostId, HostRequest, HostRequestEnvelope, HostResponse, HostResponseEnvelope,
    };
    use serde_json::json;

    fn history_checkpoint() -> CoreCheckpoint<crate::agent::session::AgentThreadState> {
        use crate::agent::{AgentItem, AgentTurnContext, AgentTurnState, ItemId, TurnId};
        let core = crate::agent::AgentCore::try_new(crate::agent::AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(crate::agent::SessionId::from_string("history-session".into())),
            ..Default::default()
        }).unwrap();
        let mut checkpoint = core.checkpoint(CheckpointReason::TurnCompleted);
        let state = &mut checkpoint.state;
        state.conversation.transcript.push(AgentItem::user_message(
            ItemId::from_string("history-input".into()), "Keep this transcript unchanged",
        ));
        state.goal_projection.apply(crate::agent::goal::GoalRuntimeCommandEnvelope {
            operation_id: "history-goal-start".into(),
            command: crate::agent::goal::GoalRuntimeCommand::Start {
                id: "history-goal".into(), objective: "Preserve durable goal progress".into(),
                constraints: vec!["Never discard the user's work".into()],
                requirements: vec![crate::agent::goal::GoalRequirement {
                    acceptance_policy: crate::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                    semantic_revision: 1, id: "preserve-history".into(), description: "History is retained".into(),
                    dependencies: vec![], status: crate::agent::goal::GoalRecordStatus::Pending,
                    evidence_refs: vec![],
                }], now_epoch_ms: 1,
            },
        }).unwrap();
        state.execution.active_turn = Some(Box::new(AgentTurnState::new(
            TurnId::from_string("history-turn".into()),
            AgentTurnContext {
                input_origin: Default::default(),
                configuration: state.configuration.clone(),
                platform: Default::default(), components: vec![], component_metrics: vec![],
                component_payload_digest: String::new(), items: vec![],
                host_id: state.host.catalog.host_id.0.clone(),
                host_catalog_revision: state.host.catalog.revision,
                host_availability_epoch: state.host.catalog.availability_epoch,
                capability_projection: Default::default(), runtime_snapshot: Default::default(),
            },
        )));
        state.execution.active_turn.as_mut().unwrap().status = crate::agent::AgentTurnStatus::Completed;
        checkpoint.revision = 17;
        state.journal.revision = 17;
        checkpoint.validate_state_identity().unwrap();
        checkpoint
    }

    fn historical_binding_without_tier() -> serde_json::Value {
        use nola_host_protocol::*;
        let route = HostModelRouteBinding::Native(HostNativeModelRouteBinding {
            adapter: HostNativeModelAdapter::LlamaCpp, adapter_contract_revision: 1,
            model_id: "history-model".into(), profile_ref: None,
        });
        let admission = HostOutputAdmission::admit(vec![HostRouteTokenLimits {
            route: route.clone(), max_input_tokens: None, max_output_tokens: None,
            max_total_tokens: None,
        }], nola_domain::generation::OutputTokenPreference::Default).unwrap();
        let binding = HostInvocationBinding {
            mac_launch: None, version: HOST_INVOCATION_BINDING_VERSION,
            primary: route, fallbacks: vec![],
            generation: HostGenerationBinding {
                max_retries: 2, parallel_tool_calls: None, reasoning_selection: None,
                reasoning_tier: Default::default(), sampling_override: false,
                generation_options: nola_domain::generation::GenerationOptions::default().try_into().unwrap(),
                temperature: serde_json::Number::from_f64(0.7).unwrap(),
                top_p: serde_json::Number::from_f64(0.9).unwrap(),
                max_output_tokens: admission.resolved.tokens,
                thinking_enabled: false, reasoning_summary_supported: None, prompt_cache: false, prompt_cache_key_sha256: None,
                prompt_cache_key_prefix_sha256: None, prompt_cache_retention: None,
            },
            output_admission: admission, execution_policy_sha256: "a".repeat(64),
        };
        binding.validate().unwrap();
        let mut value = serde_json::to_value(binding).unwrap();
        value["generation"].as_object_mut().unwrap().remove("reasoning_tier");
        assert!(serde_json::from_value::<HostInvocationBinding>(value.clone())
            .unwrap_err().to_string().contains("reasoning_tier"));
        value
    }

    const HISTORY_BINDING_PATHS: [&str; 2] = [
        "/state/configuration/turn_identity/host_invocation",
        "/state/execution/active_turn/context/configuration/turn_identity/host_invocation",
    ];

    fn replace_history_bindings(value: &mut serde_json::Value, binding: &serde_json::Value) {
        // host_invocation is an optional field; insert even if serde omitted None.
        for path in HISTORY_BINDING_PATHS {
            let parent = path.strip_suffix("/host_invocation").unwrap();
            if let Some(identity) = value.pointer_mut(parent) {
                identity["host_invocation"] = binding.clone();
            }
        }
    }

    #[test]
    fn history_decoder_preserves_terminal_history_without_decoding_obsolete_bindings() {
        use crate::agent::AgentTurnStatus;
        for status in [AgentTurnStatus::Completed, AgentTurnStatus::Failed, AgentTurnStatus::Cancelled] {
            let mut checkpoint = history_checkpoint();
            checkpoint.state.execution.active_turn.as_mut().unwrap().status = status;
            let expected = serde_json::to_value(&checkpoint).unwrap();
            for binding in [historical_binding_without_tier(), json!({"unknown_future_binding": [1, 2, 3]})] {
                let mut persisted = expected.clone();
                replace_history_bindings(&mut persisted, &binding);
                assert!(serde_json::from_value::<CoreCheckpoint<crate::agent::session::AgentThreadState>>(persisted.clone()).is_err(),
                    "strict execution recovery must still reject an obsolete binding");
                let before = persisted.clone();
                let restored = CoreCheckpoint::decode_history_for_new_turn(persisted.clone()).unwrap();
                assert_eq!(persisted, before, "the caller's persisted snapshot is not mutated");
                assert_eq!(serde_json::to_value(restored).unwrap(), expected,
                    "only historical live bindings may change; transcript, revision, goals and all other state survive");
            }
        }
    }

    #[test]
    fn history_decoder_accepts_idle_history_without_a_live_binding() {
        let mut checkpoint = history_checkpoint();
        checkpoint.state.execution.active_turn = None;
        let expected = serde_json::to_value(&checkpoint).unwrap();
        let mut persisted = expected.clone();
        replace_history_bindings(&mut persisted, &json!({"retired_transport": true}));
        assert_eq!(serde_json::to_value(CoreCheckpoint::decode_history_for_new_turn(persisted).unwrap()).unwrap(), expected);
    }

    #[test]
    fn history_decoder_rejects_active_queued_and_pending_effect_execution() {
        use crate::agent::{AgentInputDelivery, AgentItem, ItemId};
        let mut active = history_checkpoint();
        active.state.execution.active_turn.as_mut().unwrap().status =
            crate::agent::AgentTurnStatus::AwaitingModel;
        let mut cases = vec![active];
        for delivery in [AgentInputDelivery::CurrentTurn, AgentInputDelivery::NextTurn] {
            let mut queued = history_checkpoint();
            queued.state.execution.input_queue.enqueue(delivery, [AgentItem::user_message(
                ItemId::from_string("pending-user".into()), "Pending request",
            )]).unwrap();
            cases.push(queued);
        }
        let mut pending = history_checkpoint();
        let mut request = model_request();
        request.session_id = pending.session_id.clone();
        request.turn_id = Some("history-turn".into());
        assert!(pending.state.host.pending_effects.register(pending_request(&request)));
        cases.push(pending);
        for checkpoint in cases {
            let mut value = serde_json::to_value(checkpoint).unwrap();
            replace_history_bindings(&mut value, &json!({"obsolete": true}));
            assert!(CoreCheckpoint::decode_history_for_new_turn(value).unwrap_err()
                .contains("exact Run recovery is required"));
        }
    }

    #[test]
    fn checkpoint_cannot_erase_goal_identity_by_omitting_projection_or_ledger() {
        let original = serde_json::to_value(history_checkpoint()).unwrap();
        for omit_projection in [true, false] {
            let mut value = original.clone();
            if omit_projection {
                value["state"].as_object_mut().unwrap().remove("goal_projection");
            } else {
                value["state"]["goal_projection"].as_object_mut().unwrap()
                    .remove("applied_operations");
            }
            assert!(serde_json::from_value::<CoreCheckpoint<crate::agent::session::AgentThreadState>>(
                value.clone()).is_err());
            assert!(CoreCheckpoint::decode_history_for_new_turn(value).is_err(),
                "history restore must not synthesize an empty operation ledger");
        }
        let decoded: CoreCheckpoint<crate::agent::session::AgentThreadState> =
            serde_json::from_value(original).unwrap();
        assert!(decoded.state.goal_projection.applied_operations.contains_key("history-goal-start"));
    }

    #[test]
    fn history_decoder_rejects_inconsistent_checkpoint_identity() {
        let original = serde_json::to_value(history_checkpoint()).unwrap();
        for (path, invalid) in [
            ("/schema_version", json!(AGENT_CHECKPOINT_SCHEMA_VERSION + 1)),
            ("/session_id", json!("foreign-session")),
            ("/session_id", json!("")),
            ("/revision", json!(99)),
            ("/last_event_cursor/sequence", json!(99)),
            ("/capability_catalog_revision", json!(99)),
        ] {
            let mut value = original.clone();
            *value.pointer_mut(path).unwrap() = invalid;
            replace_history_bindings(&mut value, &json!({"obsolete": true}));
            assert!(CoreCheckpoint::decode_history_for_new_turn(value).is_err(), "must reject {path}");
        }
    }

    fn model_request() -> HostRequestEnvelope {
        HostRequestEnvelope {
            attempt_started: false,
            delegated_invocation_id: None,
            attempt_id: None,
            request_id: "request-1".to_string(),
            effect_id: crate::agent::host::HostEffectId("effect-1".to_string()),
            host_id: HostId("host-1".to_string()),
            lease_id: Some("lease-1".to_string()),
            session_id: "session-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            catalog_revision: 7,
            availability_epoch: 9,
            created_at_ms: 1,
            deadline_ms: None,
            batch: None,
            resume_after_sequence: None,
            delivery: None,
            payload: HostRequest::ModelStream {
                model_id: "model".to_string(),
                text_part_id: "text-1".to_string(),
                reasoning_part_id: "reasoning-1".to_string(),
                response_mode: crate::agent::host::ModelResponseMode::Conversational,
                messages: Vec::new(),
                tools: Vec::new(),
                settings: json!({}),
            },
        }
    }

    fn pending_request(request: &HostRequestEnvelope) -> PendingHostEffect {
        PendingHostEffect {
            control: Default::default(),
            effect_id: request.effect_id.0.clone(),
            request_id: request.request_id.clone(),
            host_id: request.host_id.0.clone(),
            lease_id: request.lease_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            capability_id: "model.stream".into(),
            call_id: None,
            task_id: None,
            replay_safety: crate::agent::host::HostReplaySafety::Recoverable,
            state: PendingHostEffectState::Dispatched,
            target_effect_id: None,
            cancel_effect_id: None,
            last_response_sequence: None,
            request: Some(serde_json::to_value(request).unwrap()),
        }
    }

    #[test]
    fn cancellation_binding_mirrors_admitted_proof_into_cold_request_without_changing_payload_identity() {
        use nola_domain::host_control::{CoreHostPayloadBinding,HostEffectControl};
        let mut request=model_request();
        request.payload=HostRequest::Cancel {target_effect_id:crate::agent::host::HostEffectId("unstarted-target".into()),target_not_started:None};
        let mut journal=HostEffectJournal::default();
        assert!(journal.register(pending_request(&request)));
        let mut control=HostEffectControl::from_request(&request).unwrap();
        let digest=control.payload.payload_identity.clone();
        let mut binding=control.binding();
        binding.payload_binding=CoreHostPayloadBinding::CancellationNotStarted {proof:Some(
            nola_domain::cancellation_not_started::CancellationTargetNotStarted {
                conversation_id:"conversation".into(),run_id:request.turn_id.clone().unwrap(),
                core_session_id:request.session_id.clone(),host_id:request.host_id.0.clone(),
                cancellation_effect_id:request.effect_id.0.clone(),cancellation_request_id:request.request_id.clone(),
                target_effect_id:"unstarted-target".into(),target_request_id:"unstarted-request".into(),
                target_payload_identity:"a".repeat(64),checkpoint_revision:2,revoked_attempts:vec![],
            })};
        control.apply_binding(&binding).unwrap();
        journal.bind_control(&control).unwrap();
        let cold:HostEffectJournal=serde_json::from_value(serde_json::to_value(&journal).unwrap()).unwrap();
        let restored=cold.get(&request.effect_id.0).unwrap().validated_request().unwrap();
        assert!(matches!(&restored.payload,HostRequest::Cancel {target_not_started:Some(_),..}));
        assert_eq!(HostEffectControl::from_request(&restored).unwrap().payload.payload_identity,digest);
        assert_eq!(cold.get(&request.effect_id.0).unwrap().validated_control().unwrap(),&control);
        let before=serde_json::to_value(&journal).unwrap();
        let mut foreign=control.binding();
        let CoreHostPayloadBinding::CancellationNotStarted {proof:Some(proof)}=&mut foreign.payload_binding else {unreachable!()};
        proof.target_request_id="replaced-source".into();
        assert!(control.apply_binding(&foreign).is_err());
        assert_eq!(serde_json::to_value(&journal).unwrap(),before);
    }

    #[test]
    fn frozen_model_input_restore_preserves_identity_cursor_and_cancellation() {
        let original = model_request();
        let mut generated = original.clone();
        if let HostRequest::ModelStream { messages, tools, settings, .. } = &mut generated.payload {
            messages.push(json!({"role":"system","content":"new renderer"}));
            tools.push(json!({"name":"new_description"}));
            *settings = json!({"new_default":true});
        }
        let mut journal = HostEffectJournal::default();
        let mut pending = pending_request(&generated);
        pending.state = PendingHostEffectState::Cancelling;
        pending.last_response_sequence = Some(42);
        assert!(journal.register(pending));
        journal.restore_committed_model_input(&original).unwrap();
        let effect = journal.get(&original.effect_id.0).unwrap();
        assert_eq!(effect.state, PendingHostEffectState::Cancelling);
        assert_eq!(effect.last_response_sequence, Some(42));
        assert_eq!(effect.request.as_ref().unwrap()["payload"], serde_json::to_value(&original.payload).unwrap());
        let before = serde_json::to_vec(&journal).unwrap();
        let mut foreign = original.clone();
        if let HostRequest::ModelStream { model_id, .. } = &mut foreign.payload { *model_id = "other-model".into(); }
        assert!(journal.restore_committed_model_input(&foreign).is_err());
        foreign = original.clone();
        foreign.turn_id = Some("other-run".into());
        assert!(journal.restore_committed_model_input(&foreign).is_err());
        foreign = original.clone();
        if let HostRequest::ModelStream { text_part_id, .. } = &mut foreign.payload { *text_part_id = "other-part".into(); }
        assert!(journal.restore_committed_model_input(&foreign).is_err());
        assert_eq!(serde_json::to_vec(&journal).unwrap(), before);
    }

    #[test]
    fn large_frozen_prompt_is_materialized_once_not_per_delta_or_control_bind() {
        for prompt_bytes in [1024, 2 * 1024 * 1024] {
            let mut request = model_request();
            if let HostRequest::ModelStream { messages, .. } = &mut request.payload {
                messages.push(json!({"role":"user","content":"x".repeat(prompt_bytes)}));
            }
            FULL_REQUEST_MATERIALIZATIONS.with(|count| count.set(0));
            let mut journal = HostEffectJournal::default();
            assert!(journal.register(pending_request(&request)));
            assert_eq!(FULL_REQUEST_MATERIALIZATIONS.with(|count| count.get()), 1);
            let identity = journal
                .get(&request.effect_id.0)
                .unwrap()
                .validated_control()
                .unwrap()
                .payload
                .payload_identity
                .clone();
            for sequence in 1..=256 {
                let delta = response(
                    &request,
                    sequence,
                    HostResponse::ModelDelta {
                        channel: crate::agent::host::ModelDeltaChannel::Text,
                        part_id: "text-1".into(),
                        delta: "🙂".into(),
                    },
                );
                assert_eq!(
                    journal.accept_response(&delta),
                    HostEffectAcceptance::Accepted
                );
                let mut control = journal
                    .get(&request.effect_id.0)
                    .unwrap()
                    .validated_control()
                    .unwrap()
                    .clone();
                assert!(serde_json::to_vec(&control).unwrap().len() < 2048);
                control.resume_after_sequence = None;
                control.attempt_started = true;
                control.attempt_id = Some("attempt".into());
                journal.bind_control(&control).unwrap();
                assert_eq!(control.payload.payload_identity, identity);
            }
            assert_eq!(
                FULL_REQUEST_MATERIALIZATIONS.with(|count| count.get()),
                1,
                "256 actual journal accept/bind cycles must not decode the prompt"
            );
            let encoded = serde_json::to_vec(&journal).unwrap();
            let mut restored: HostEffectJournal = serde_json::from_slice(&encoded).unwrap();
            restored.restore_controls().unwrap();
            assert_eq!(
                FULL_REQUEST_MATERIALIZATIONS.with(|count| count.get()),
                2,
                "cold restore validates exactly once"
            );
            assert_eq!(restored, journal);
            assert_eq!(
                restored
                    .get(&request.effect_id.0)
                    .unwrap()
                    .validated_control()
                    .unwrap()
                    .payload
                    .payload_identity,
                identity
            );
            let before = serde_json::to_vec(&restored).unwrap();
            let mut foreign = restored
                .get(&request.effect_id.0)
                .unwrap()
                .validated_control()
                .unwrap()
                .clone();
            foreign.payload.payload_identity = "0".repeat(64);
            assert!(restored.bind_control(&foreign).is_err());
            assert_eq!(serde_json::to_vec(&restored).unwrap(), before);
            foreign = restored
                .get(&request.effect_id.0)
                .unwrap()
                .validated_control()
                .unwrap()
                .clone();
            foreign.request_id =
                "x".repeat(nola_domain::host_control::HOST_EFFECT_CONTROL_MAX_BYTES + 1);
            assert!(restored.bind_control(&foreign).is_err());
            assert_eq!(serde_json::to_vec(&restored).unwrap(), before);
        }
    }

    fn response(
        request: &HostRequestEnvelope,
        sequence: u64,
        payload: HostResponse,
    ) -> HostResponseEnvelope {
        HostResponseEnvelope {
            delegated_invocation_id: request.delegated_invocation_id.clone(),
            attempt_id: None,
            request_id: request.request_id.clone(),
            effect_id: request.effect_id.clone(),
            host_id: request.host_id.clone(),
            lease_id: request.lease_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            delivery: request.delivery.clone(),
            sequence,
            payload,
        }
    }

    #[test]
    fn quiescence_includes_buffered_terminal_responses_after_effects_settle() {
        let mut journal = HostEffectJournal::default();
        assert!(journal.is_quiescent());
        let mut requests = Vec::new();
        for index in 0..2 {
            let mut request = model_request();
            request.effect_id.0 = format!("batch-effect-{index}");
            request.request_id = format!("batch-request-{index}");
            request.batch = Some(crate::agent::host::HostRequestBatchIdentity {
                batch_id: "batch".into(),
                index,
                size: 2,
            });
            request.payload = HostRequest::ExecuteTool {
                permission_profile: nola_domain::PermissionProfile::Cautious,
                permission_instructions: vec![],
                capability_id: "test.read".into(),
                call_id: format!("call-{index}"),
                input: json!({}),
                grant: None,
            };
            assert!(journal.register(PendingHostEffect {
                control: Default::default(),
                effect_id: request.effect_id.0.clone(),
                request_id: request.request_id.clone(),
                host_id: request.host_id.0.clone(),
                lease_id: request.lease_id.clone(),
                session_id: request.session_id.clone(),
                turn_id: request.turn_id.clone(),
                catalog_revision: request.catalog_revision,
                availability_epoch: request.availability_epoch,
                capability_id: "test.read".into(),
                call_id: Some(format!("call-{index}")),
                task_id: None,
                replay_safety: crate::agent::host::HostReplaySafety::Recoverable,
                state: PendingHostEffectState::Dispatched,
                target_effect_id: None,
                cancel_effect_id: None,
                last_response_sequence: None,
                request: Some(serde_json::to_value(&request).unwrap()),
            }));
            requests.push(request);
        }
        assert!(!journal.is_quiescent());
        let terminal = response(
            &requests[1],
            1,
            HostResponse::ToolCompleted {
                call_id: "call-1".into(),
                output: json!({"observed":true}),
                outcome: crate::agent::host::HostToolOutcome::Succeeded,
                activation: None,
                full_output_ref: None,
                artifacts: vec![],
            },
        );
        assert!(journal
            .defer_out_of_order_batch_terminal(&terminal)
            .unwrap());
        journal.complete(&requests[0].effect_id.0).unwrap();
        journal.complete(&requests[1].effect_id.0).unwrap();
        assert_eq!(journal.len(), 0);
        assert!(
            !journal.is_quiescent(),
            "zero pending effects does not mean buffered work is gone"
        );
        let config = crate::agent::AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(crate::agent::SessionId::from_string("session-1".into())),
            ..Default::default()
        };
        let core = crate::agent::AgentCore::try_new(config.clone()).unwrap();
        let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
        checkpoint.state.host.pending_effects = journal;
        assert!(!checkpoint.state.permits_new_turn_execution_configuration());
        assert!(CoreCheckpoint::decode_history_for_new_turn(
            serde_json::to_value(&checkpoint).unwrap()
        ).unwrap_err().contains("exact Run recovery is required"),
            "buffered terminal work is not settled conversation history");
        let mut fresh = config;
        fresh.tools = vec![
            json!({"type":"function","function":{"name":"fresh.read","parameters":{"type":"object"}}}),
        ];
        assert!(
            crate::agent::AgentCore::restore(fresh, checkpoint.clone()).is_err(),
            "idle restore cannot replace configuration while buffered terminal work remains"
        );
        assert!(checkpoint
            .state
            .host
            .pending_effects
            .take_ready_buffered_batch_terminal("batch")
            .unwrap()
            .is_none());
        assert!(checkpoint.state.permits_new_turn_execution_configuration());
    }

    #[test]
    fn frozen_request_identity_mismatch_does_not_consume_response() {
        let request = model_request();
        let original = serde_json::to_value(&request).unwrap();
        let pending = PendingHostEffect {
            control: Default::default(),
            effect_id: request.effect_id.0.clone(),
            request_id: request.request_id.clone(),
            host_id: request.host_id.0.clone(),
            lease_id: request.lease_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            capability_id: "model.stream".into(),
            call_id: None,
            task_id: None,
            replay_safety: crate::agent::host::HostReplaySafety::RequiresRecoveryDecision,
            state: PendingHostEffectState::Dispatched,
            target_effect_id: None,
            cancel_effect_id: None,
            last_response_sequence: None,
            request: Some(original.clone()),
        };
        for (field, foreign) in [
            ("effect_id", json!("foreign-effect")),
            ("request_id", json!("foreign-request")),
            ("host_id", json!("foreign-host")),
            ("lease_id", json!("foreign-lease")),
            ("session_id", json!("foreign-session")),
            ("turn_id", json!("foreign-turn")),
            ("catalog_revision", json!(request.catalog_revision + 1)),
            ("availability_epoch", json!(request.availability_epoch + 1)),
        ] {
            let mut journal = HostEffectJournal::default();
            let mut corrupted = pending.clone();
            corrupted.request.as_mut().unwrap()[field] = foreign;
            assert!(corrupted.validated_request().is_err(), "{field}");
            assert!(
                !journal.register(corrupted.clone()),
                "invalid construction must fail closed"
            );
            // Inject corruption below the public construction boundary to
            // retain the independent response rejection/sequence assertion.
            journal
                .effects
                .insert(corrupted.effect_id.clone(), corrupted);
            let delivered = response(&request, 1, HostResponse::Accepted);
            assert_eq!(
                journal.accept_response(&delivered),
                HostEffectAcceptance::WrongResponseType,
                "{field}"
            );
            let effect = journal.effects.get_mut(&request.effect_id.0).unwrap();
            assert_eq!(effect.last_response_sequence, None, "{field}");
            effect.request = Some(original.clone());
            assert!(effect.validated_request().is_ok());
            assert_eq!(
                journal.accept_response(&delivered),
                HostEffectAcceptance::Accepted,
                "{field}"
            );
        }
    }

    #[test]
    fn delegated_identity_mismatch_does_not_consume_delivery_sequence() {
        let mut request = model_request();
        request.payload = HostRequest::DelegatedAgent {
            request: serde_json::from_value(json!({
                "contract_version": 1,
                "backend_id": "claude",
                "text_part_id": "text-1",
                "reasoning_part_id": "reasoning-1",
                "context": {
                    "mode": "bootstrap", "identity_digest": "identity",
                    "target_anchor_item_id": "item-1", "messages": [{"role":"user","content":"fixture"}]
                },
                "tools": [], "settings": {}
            })).expect("delegated fixture")
        };
        request.delegated_invocation_id = Some("invocation-1".into());
        let mut journal = HostEffectJournal::default();
        assert!(journal.register(PendingHostEffect {
            control: Default::default(),
            effect_id: request.effect_id.0.clone(),
            request_id: request.request_id.clone(),
            host_id: request.host_id.0.clone(),
            lease_id: request.lease_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            capability_id: "agent.delegate".to_string(),
            call_id: None,
            task_id: None,
            replay_safety: crate::agent::host::HostReplaySafety::Recoverable,
            state: PendingHostEffectState::Dispatched,
            target_effect_id: None,
            cancel_effect_id: None,
            last_response_sequence: None,
            request: Some(serde_json::to_value(&request).expect("request JSON")),
        }));
        let mut delivered = response(&request, 1, HostResponse::Accepted);
        for identity in [None, Some("invocation-2".to_string())] {
            delivered.delegated_invocation_id = identity;
            assert_eq!(
                journal.accept_response(&delivered),
                HostEffectAcceptance::WrongDelegatedInvocation
            );
            assert_eq!(
                journal
                    .get(&request.effect_id.0)
                    .unwrap()
                    .last_response_sequence,
                None
            );
        }
        delivered.delegated_invocation_id = request.delegated_invocation_id.clone();
        assert_eq!(
            journal.accept_response(&delivered),
            HostEffectAcceptance::Accepted
        );
        assert_eq!(
            journal
                .get(&request.effect_id.0)
                .unwrap()
                .last_response_sequence,
            Some(1)
        );
    }

    #[test]
    fn mismatched_response_kind_is_rejected_without_consuming_sequence() {
        let request = model_request();
        let mut journal = HostEffectJournal::default();
        assert!(journal.register(PendingHostEffect {
            control: Default::default(),
            effect_id: request.effect_id.0.clone(),
            request_id: request.request_id.clone(),
            host_id: request.host_id.0.clone(),
            lease_id: request.lease_id.clone(),
            session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(),
            catalog_revision: request.catalog_revision,
            availability_epoch: request.availability_epoch,
            capability_id: "model.stream".to_string(),
            call_id: None,
            task_id: None,
            replay_safety: crate::agent::host::HostReplaySafety::Recoverable,
            state: PendingHostEffectState::Dispatched,
            target_effect_id: None,
            cancel_effect_id: None,
            last_response_sequence: None,
            request: Some(serde_json::to_value(&request).expect("request JSON")),
        }));

        let mismatched = response(
            &request,
            1,
            HostResponse::Completed {
                output: json!({}),
                usage: None,
                full_output_ref: None,
                artifacts: Vec::new(),
            },
        );
        let mut wrong_request = mismatched.clone();
        wrong_request.request_id = "request-from-another-effect".to_string();
        assert_eq!(
            journal.accept_response(&wrong_request),
            HostEffectAcceptance::WrongRequest
        );
        assert_eq!(
            journal.accept_response(&mismatched),
            HostEffectAcceptance::WrongResponseType
        );

        let restarted = response(
            &request,
            1,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "test-model".into(),
                reason: "resampling".into(),
            },
        );
        assert_eq!(
            journal.accept_response(&restarted),
            HostEffectAcceptance::Accepted
        );

        let valid = response(
            &request,
            2,
            HostResponse::ModelCompleted {
                items: Vec::new(),
                tool_calls: Vec::new(),
                usage: json!({}),
            },
        );
        assert_eq!(
            journal.accept_response(&valid),
            HostEffectAcceptance::Accepted
        );
        assert!(journal.complete(&request.effect_id.0).is_some());
        assert_eq!(
            journal.accept_response(&valid),
            HostEffectAcceptance::DuplicateResponse
        );

        let mut forged_duplicate = valid.clone();
        forged_duplicate.request_id = "forged-request".to_string();
        assert_eq!(
            journal.accept_response(&forged_duplicate),
            HostEffectAcceptance::WrongRequest
        );

        let mut unknown = valid;
        unknown.effect_id = crate::agent::host::HostEffectId("unknown-effect".to_string());
        assert_eq!(
            journal.accept_response(&unknown),
            HostEffectAcceptance::UnknownEffect
        );
        assert_eq!(
            HostEffectAcceptance::DuplicateResponse.code(),
            "duplicate_response"
        );
        assert_eq!(
            HostEffectAcceptance::DuplicateResponse.category(),
            "benign_replay"
        );
        assert_eq!(
            HostEffectAcceptance::from_rejection_error(
                "rejected Host response: code=wrong_response_type category=protocol_violation effect_id=e"
            ),
            Some(HostEffectAcceptance::WrongResponseType)
        );
        let rejection = HostEffectAcceptance::from_rejection_error(
            "rejected Host response: code=stale_lease category=stale_authority effect_id=e request_id=r",
        ).expect("stable Host rejection classification");
        assert_eq!(rejection.code(), "stale_lease");
        assert_eq!(rejection.category(), "stale_authority");
    }
}
