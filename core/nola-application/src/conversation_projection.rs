// SPDX-License-Identifier: Apache-2.0

//! Canonical product projection materialization.
//!
//! Store rows and Core checkpoint Parts meet at this application boundary.
//! Platform clients must query the resulting domain projection rather than
//! replaying Host events or diagnostic traces.

use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
use nola_domain::ModelAttemptState;
use nola_domain::{
    ActivityState, AffectedResourceView, AttachmentId, CapabilityId, ContextUsageView,
    ConversationId, ConversationProjection, ConversationSurfaceView, MessageId, MessagePartId,
    MessagePartView, MessageRole, MessageState, MessageView, ModelAttemptRecord, RunActivityView,
    RunId, StructuredValue, ToolCallId, ToolOutputReference,
};
use nola_agent_engine::agent::context_window::ContextUsageSnapshot;
use nola_agent_engine::agent::parts::{
    AgentPart, ConversationProjection as CoreConversationProjection, StepStatus, ToolPartState,
};
use nola_agent_engine::agent::task::TaskProjection;
use nola_store_port::conversation_history::{
    ConversationProjectionRepository, StoredConversation, StoredMessage,
};
use nola_store_port::historical_part_projection::HistoricalPartProjectionRepository;

use crate::assistant_owner::{validate_checkpoint_identity, AssistantOwnerResolver};
use crate::assistant_synthetic_semantics::{model_attempt_part, skill_part, task_part};
use crate::public_part_semantics::PublicPart;
use crate::SessionSkillProjection;
use nola_store_port::assistant_ownership::AssistantOwnershipFacts;

pub const CONVERSATION_PROJECTION_CONTRACT_VERSION: u32 = 1;

pub fn load_conversation_context_usage_v1<S: ConversationProjectionRepository>(
    store: &S,
    conversation_id: &str,
) -> Result<ContextUsageView, ConversationProjectionContractError> {
    let record = store.conversation_committed_context_usage(conversation_id)
        .map_err(|error|ConversationProjectionContractError::Store(error.to_string()))?;
    let conversation_cache_usage = store.conversation_cache_usage(conversation_id)
        .map_err(|error|ConversationProjectionContractError::Store(error.to_string()))?;
    Ok(match record {
        Some(record) => ContextUsageView {
            measurement: record.usage.measurement,
            prompt_cache_measurement: record.usage.prompt_cache_measurement,
            prompt_cache_diagnostic: record.usage.prompt_cache_diagnostic,
            conversation_cache_usage,
            cache_usage: record.usage.cache_usage,
            used_tokens:record.usage.used_tokens.0,
            context_window_tokens:record.usage.context_window_tokens.0,
            prompt_token_limit:record.usage.prompt_token_limit.0,
            estimated:record.usage.estimated,
            measured_at_revision:record.event_sequence.0,
        },
        None => ContextUsageView { measurement:None,prompt_cache_measurement:None,prompt_cache_diagnostic:None,
            conversation_cache_usage,cache_usage:None,used_tokens:0,context_window_tokens:0,prompt_token_limit:0,
            estimated:true,measured_at_revision:0 },
    })
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationProjectionWindowV1 {
    pub assistant_journal_heads: Vec<nola_domain::assistant_journal::AssistantJournalCursor>,
    pub projection: ConversationProjection,
    /// None means restoration ownership is unavailable; empty means no live text.
    pub stream_owners: Option<Vec<nola_domain::ProjectionStreamOwner>>,
    pub event_sequence: u64,
    pub total_message_count: u64,
    pub before_ordinal: Option<i64>,
    pub has_more: bool,
    pub deferred_text_parts: Vec<nola_store_port::DeferredTextPart>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationProjectionContractError {
    Store(String),
    MissingConversation(String),
    MissingCheckpoint(String),
    MissingAssistantProjection(String),
    UnknownMessageRole(String),
    UnknownMessageState(String),
    EmptyCapabilityId(String),
    InvalidStructuredValue(String),
}

impl std::fmt::Display for ConversationProjectionContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(detail) => write!(formatter, "store error: {detail}"),
            Self::MissingConversation(id) => write!(formatter, "missing conversation {id}"),
            Self::MissingCheckpoint(id) => {
                write!(
                    formatter,
                    "missing canonical checkpoint for conversation {id}"
                )
            }
            Self::MissingAssistantProjection(id) => {
                write!(formatter, "assistant message {id} has no Core projection")
            }
            Self::UnknownMessageRole(role) => write!(formatter, "unknown message role {role}"),
            Self::UnknownMessageState(state) => {
                write!(formatter, "unknown message state {state}")
            }
            Self::EmptyCapabilityId(part_id) => {
                write!(formatter, "tool Part {part_id} has an empty capability_id")
            }
            Self::InvalidStructuredValue(detail) => {
                write!(formatter, "invalid structured value: {detail}")
            }
        }
    }
}

impl std::error::Error for ConversationProjectionContractError {}

pub fn load_conversation_projection_v1<S: ConversationProjectionRepository>(
    store: &S,
    conversation_id: &str,
) -> Result<ConversationProjection, ConversationProjectionContractError> {
    let stored = store
        .load_conversation(conversation_id)
        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?
        .ok_or_else(|| {
            ConversationProjectionContractError::MissingConversation(conversation_id.to_owned())
        })?;
    let is_remote = uses_remote_presentation(store, conversation_id)?;
    if is_remote {
        return load_remote_projection_subset(store, conversation_id, &stored);
    }
    let projection = materialize_local_presentation(store, conversation_id, stored)?;
    project_active_run(store, conversation_id, projection)
}

#[derive(serde::Deserialize)]
struct LocalPresentationInputs {
    conversation_id: String,
    session_id: String,
    state_session_id: String,
    cursor: u64,
    cursor_type: String,
    event_cursor: u64,
    projection: CoreConversationProjection,
    tasks: TaskProjection,
    skill_revision: u64,
    catalog_revision: u64,
    skill_runs: BTreeMap<String, nola_agent_engine::agent::capability::SkillRun>,
    #[serde(default)]
    model_attempts: Vec<ModelAttemptRecord>,
    ownership: AssistantOwnershipFacts,
}

fn materialize_local_presentation<S: ConversationProjectionRepository>(
    store: &S,
    conversation_id: &str,
    stored: StoredConversation,
) -> Result<ConversationProjection, ConversationProjectionContractError> {
    if stored.id != conversation_id {
        return Err(ConversationProjectionContractError::Store(
            "conversation repository key mismatch".into(),
        ));
    }
    let message_ids = stored
        .messages
        .iter()
        .map(|message| message.id.as_str())
        .collect::<Vec<_>>();
    let message_ids_json = serde_json::to_string(&message_ids)
        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?;
    let current = crate::checkpoint_read_override(store, conversation_id)
        .map_err(ConversationProjectionContractError::Store)?;
    let json = store
        .load_local_presentation_inputs(conversation_id, &message_ids_json, current.as_ref())
        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?;
    let Some(json) = json else {
        return materialize_imported_conversation_projection_v1(store, stored, conversation_id);
    };
    let inputs: LocalPresentationInputs = serde_json::from_str(&json).map_err(|error| {
        ConversationProjectionContractError::Store(format!(
            "decode local presentation inputs failed: {error}"
        ))
    })?;
    if stored.id != conversation_id
        || validate_checkpoint_identity(
            conversation_id,
            &inputs.conversation_id,
            &inputs.session_id,
            &inputs.state_session_id,
        )
        .is_err()
        || inputs.cursor != inputs.event_cursor
        || inputs.cursor_type != "integer"
    {
        return Err(ConversationProjectionContractError::Store(
            "local presentation identity or cursor mismatch".into(),
        ));
    }
    for (id, run) in &inputs.skill_runs {
        if id != &run.run_id || run.owner.session_id != inputs.session_id {
            return Err(ConversationProjectionContractError::Store(format!(
                "local presentation Skill identity mismatch: {id}"
            )));
        }
    }
    let skills = SessionSkillProjection {
        session_id: inputs.session_id.clone(),
        cursor: inputs.cursor,
        revision: inputs.skill_revision,
        catalog_revision: inputs.catalog_revision,
        order: inputs.skill_runs.keys().cloned().collect(),
        runs: inputs.skill_runs,
    };
    let owners =
        AssistantOwnerResolver::new(conversation_id, Some(&inputs.session_id), &inputs.ownership)
            .map_err(|e| {
            ConversationProjectionContractError::Store(format!("assistant ownership: {e}"))
        })?;
    let mut projection = materialize_conversation_projection_with_owners(
        Some(store),
        stored,
        Some(&inputs.projection),
        &inputs.tasks,
        &skills,
        ContextUsageSnapshot::default(),
        true,
        &owners,
    )?;
    project_model_attempts(&mut projection, &inputs.model_attempts, &owners)?;
    projection.surface = conversation_surface(
        projection.revision,
        &projection.messages,
        projection.active_run.as_ref(),
    );
    Ok(projection)
}

pub(crate) fn project_model_attempts(
    projection: &mut ConversationProjection,
    attempts: &[ModelAttemptRecord],
    owners: &AssistantOwnerResolver,
) -> Result<(), ConversationProjectionContractError> {
    let attempt_index = attempts
        .iter()
        .map(|attempt| (attempt.id.as_str(), attempt))
        .collect::<BTreeMap<_, _>>();
    if attempt_index.len() != attempts.len() {
        return Err(ConversationProjectionContractError::Store(
            "duplicate Attempt projection identity".into(),
        ));
    }
    for child in attempts {
        if let Some(parent_id) = &child.predecessor_id {
            let parent = attempt_index.get(parent_id.as_str()).ok_or_else(|| {
                ConversationProjectionContractError::Store(
                    "Attempt projection predecessor is missing".into(),
                )
            })?;
            if parent.run_id != child.run_id
                || parent.effect_id != child.effect_id
                || parent.invocation_id != child.invocation_id
            {
                return Err(ConversationProjectionContractError::Store(
                    "Attempt projection predecessor belongs to another execution".into(),
                ));
            }
        }
    }
    let successors = attempts
        .iter()
        .filter_map(|attempt| {
            attempt
                .predecessor_id
                .as_ref()
                .map(|predecessor| (predecessor.as_str(), attempt))
        })
        .collect::<BTreeMap<_, _>>();

    for attempt in attempts {
        if attempt.failure.is_none() {
            continue;
        }
        let Some(owner) = owners
            .resolve_attempt(attempt, &attempt_index)
            .map_err(|e| {
                ConversationProjectionContractError::Store(format!("attempt ownership: {e}"))
            })?
        else {
            continue;
        };
        let message_id = &owner.assistant_message_id;
        let Some(message) = projection
            .messages
            .iter_mut()
            .find(|message| &message.message_id.0 == message_id)
        else {
            continue;
        };
        let part = model_attempt_part(attempt, successors.get(attempt.id.as_str()).copied())
            .map_err(|error| ConversationProjectionContractError::Store(error.into()))?;
        if let Some(part) = part {
            upsert_semantic_parts(&mut message.parts,vec![part.to_message_part()])?;
        }
    }
    Ok(())
}

fn load_remote_projection_subset<S: ConversationProjectionRepository>(
    store: &S,
    conversation_id: &str,
    stored: &StoredConversation,
) -> Result<ConversationProjection, ConversationProjectionContractError> {
    let projection = materialize_imported_conversation_projection_v1(store, stored.clone(), conversation_id)?;
    project_active_run(store, conversation_id, projection)
}

fn uses_remote_presentation<S: ConversationProjectionRepository>(
    store: &S,
    conversation_id: &str,
) -> Result<bool, ConversationProjectionContractError> {
    let active_remote = store
        .has_active_remote_run(conversation_id)
        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?;
    if active_remote {
        return Ok(true);
    }
    store
        .has_remote_presentation_owner(conversation_id)
        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))
}

fn local_stream_owners<S: ConversationProjectionRepository>(
    store: &S,
    projection: &ConversationProjection,
) -> Result<Option<Vec<nola_domain::ProjectionStreamOwner>>, ConversationProjectionContractError> {
    let mut owners = Vec::new();
    for message in &projection.messages {
        for part in &message.parts {
            if let MessagePartView::Text {
                part_id,
                is_final: false,
                ..
            } = part
            {
                let owner = store
                    .model_text_part_owner(&projection.conversation_id.0, &part_id.0)
                    .map_err(|error| {
                        ConversationProjectionContractError::Store(error.to_string())
                    })?;
                let delegated = store
                    .delegated_text_part_owner(&projection.conversation_id.0, &part_id.0)
                    .map_err(|error| {
                        ConversationProjectionContractError::Store(error.to_string())
                    })?;
                let (run_id, execution) = match (owner, delegated) {
                    (Some(_), Some(_)) => {
                        return Err(ConversationProjectionContractError::Store(
                            "text Part has conflicting model and delegated execution owners".into(),
                        ))
                    }
                    (Some(owner), None) => (
                        RunId(owner.run_id),
                        nola_domain::ProjectionExecutionOwner::ModelAttempt {
                            attempt_id: nola_domain::RunAttemptId(owner.attempt_id),
                        },
                    ),
                    (None, Some(owner)) => (
                        RunId(owner.run_id),
                        nola_domain::ProjectionExecutionOwner::DelegatedInvocation {
                            invocation_id: owner.invocation_id,
                        },
                    ),
                    (None, None) => return Ok(None),
                };
                owners.push(nola_domain::ProjectionStreamOwner {
                    message_id: message.message_id.clone(),
                    part_id: part_id.clone(),
                    run_id,
                    execution,
                });
            }
        }
    }
    Ok(Some(owners))
}

pub fn load_conversation_projection_window_v1<S: ConversationProjectionRepository>(
    store: &S,
    conversation_id: &str,
    before_ordinal: Option<i64>,
    limit: usize,
    content_byte_limit: usize,
) -> Result<ConversationProjectionWindowV1, ConversationProjectionContractError> {
    // The Application command wraps this aggregate in one consistent read.
    // This journal position is independent from the history ordinal below.
    let event_sequence = store
        .projection_cursor(&nola_store_port::protocol_journal::ProjectionScopeKey {
            scope_type: "conversation".into(),
            scope_id: conversation_id.to_owned(),
        })
        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?
        .map(|cursor| cursor.current_sequence)
        .unwrap_or(0);
    let window = store
        .load_conversation_message_window(
            conversation_id,
            before_ordinal,
            limit,
            content_byte_limit,
        )
        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?
        .ok_or_else(|| {
            ConversationProjectionContractError::MissingConversation(conversation_id.to_owned())
        })?;
    let is_remote = uses_remote_presentation(store, conversation_id)?;
    if is_remote {
        return bounded_window(ConversationProjectionWindowV1 {
            assistant_journal_heads: window.assistant_journal_heads,
            deferred_text_parts: window.deferred_text_parts,
            projection: load_remote_projection_subset(
                store,
                conversation_id,
                &window.conversation,
            )?,
            stream_owners: None,
            event_sequence,
            total_message_count: u64::try_from(window.total_message_count).unwrap_or_default(),
            before_ordinal: window.before_ordinal,
            has_more: window.has_more,
        });
    }
    let projection = materialize_local_presentation(store, conversation_id, window.conversation)?;
    let stream_owners = local_stream_owners(store, &projection)?;
    bounded_window(ConversationProjectionWindowV1 {
        assistant_journal_heads: window.assistant_journal_heads,
        deferred_text_parts: window.deferred_text_parts,
        projection: project_active_run(store, conversation_id, projection)?,
        stream_owners,
        event_sequence,
        total_message_count: u64::try_from(window.total_message_count).unwrap_or_default(),
        before_ordinal: window.before_ordinal,
        has_more: window.has_more,
    })
}

pub fn load_conversation_projection_page_v1<S: ConversationProjectionRepository>(
    store: &S,
    conversation_id: &str,
    offset: usize,
    limit: usize,
) -> Result<ConversationProjection, ConversationProjectionContractError> {
    let stored = store
        .load_conversation_page(conversation_id, offset, limit)
        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?
        .ok_or_else(|| {
            ConversationProjectionContractError::MissingConversation(conversation_id.to_owned())
        })?;
    let is_remote = uses_remote_presentation(store, conversation_id)?;
    if is_remote {
        return load_remote_projection_subset(store, conversation_id, &stored);
    }
    let projection = materialize_local_presentation(store, conversation_id, stored)?;
    project_active_run(store, conversation_id, projection)
}

fn project_active_run<S: ConversationProjectionRepository>(
    store: &S,
    conversation_id: &str,
    mut projection: ConversationProjection,
) -> Result<ConversationProjection, ConversationProjectionContractError> {
    // Full, paged, local and replica projections share the same committed
    // control fact as the lightweight usage endpoint. Execution checkpoint
    // statistics and imported summaries are never a second authority.
    projection.context_usage = load_conversation_context_usage_v1(store, conversation_id)?;
    projection.execution_ownership = store.conversation_execution_ownership(conversation_id)
        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?;
    // The local canonical Run owns liveness for both local and mirrored turns;
    // an older remote terminal snapshot cannot end a newly opened Run.
    projection.active_run = store
        .active_run(conversation_id)
        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?
        .and_then(|run| {
            let assistant_message_id = run.assistant_message_id?;
            projection
                .messages
                .iter()
                .find(|message| message.message_id.0 == assistant_message_id)
                .and_then(|message| active_run_for_message(message, RunId(run.run_id)))
        });
    projection.surface = conversation_surface(
        projection.revision,
        &projection.messages,
        projection.active_run.as_ref(),
    );
    Ok(projection)
}

pub fn materialize_conversation_projection_v1(
    stored: StoredConversation,
    core: &CoreConversationProjection,
    tasks: &TaskProjection,
    skills: &SessionSkillProjection,
    context_usage: ContextUsageSnapshot,
    core_session_id: &str,
    ownership: &AssistantOwnershipFacts,
) -> Result<ConversationProjection, ConversationProjectionContractError> {
    materialize_conversation_projection::<String>(
        None,
        stored,
        Some(core),
        tasks,
        skills,
        context_usage,
        false,
        Some(core_session_id),
        ownership,
    )
}

fn materialize_imported_conversation_projection_v1<S: ConversationProjectionRepository>(
    store: &S,
    stored: StoredConversation,
    conversation_id: &str,
) -> Result<ConversationProjection, ConversationProjectionContractError> {
    materialize_conversation_projection(
        Some(store),
        stored,
        None,
        &TaskProjection::default(),
        &empty_skill_projection(conversation_id),
        ContextUsageSnapshot::default(),
        false,
        None,
        &AssistantOwnershipFacts::default(),
    )
}

pub(crate) fn materialize_conversation_projection<E: std::fmt::Display>(
    store: Option<&dyn HistoricalPartProjectionRepository<Error = E>>,
    stored: StoredConversation,
    core: Option<&CoreConversationProjection>,
    tasks: &TaskProjection,
    skills: &SessionSkillProjection,
    context_usage: ContextUsageSnapshot,
    partial: bool,
    core_session_id: Option<&str>,
    ownership: &AssistantOwnershipFacts,
) -> Result<ConversationProjection, ConversationProjectionContractError> {
    if core.is_some() != core_session_id.is_some() {
        return Err(ConversationProjectionContractError::Store(
            "Core projection requires its explicit session identity".into(),
        ));
    }
    if core_session_id.is_none()
        && (!tasks.tasks.is_empty() || !tasks.order.is_empty() || !skills.runs.is_empty())
    {
        return Err(ConversationProjectionContractError::Store(
            "Task and Skill projection requires an explicit Core session".into(),
        ));
    }
    if core_session_id.is_some_and(|session| skills.session_id != session) {
        return Err(ConversationProjectionContractError::Store(
            "Skill projection belongs to another Core session".into(),
        ));
    }
    // Pure callers supply actual typed facts; Part bodies and visible window
    // order cannot manufacture admission or Turn ownership.
    let owners =
        AssistantOwnerResolver::new(&stored.id, core_session_id, ownership).map_err(|e| {
            ConversationProjectionContractError::Store(format!("assistant ownership: {e}"))
        })?;
    materialize_conversation_projection_with_owners(
        store,
        stored,
        core,
        tasks,
        skills,
        context_usage,
        partial,
        &owners,
    )
}

/// A Surface is published by an exact Tool Part, outside the Agent checkpoint.
/// Anchor it to that committed identity instead of appending every card after
/// subsequent reasoning/prose. No timestamps, prose matching or Store mutation.
fn merge_canonical_stored_parts<E: std::fmt::Display>(
    store: Option<&dyn HistoricalPartProjectionRepository<Error = E>>,
    conversation: &StoredConversation,
    message: &StoredMessage,
    revision: u64,
    mut projected: Vec<MessagePartView>,
    core_parts: Option<&[&AgentPart]>,
    surfaces: Vec<MessagePartView>,
    remaining: &mut usize,
    owners: &AssistantOwnerResolver,
) -> Result<Vec<MessagePartView>, ConversationProjectionContractError> {
    use nola_domain::{CanonicalPartOwner, HistoricalPartProjection, HistoricalPartSchema,
        PartBodyField, SemanticIdentityAssessment, SemanticValue, generative_ui::UiSurfaceRevisionRef};
    let invalid = || ConversationProjectionContractError::Store(
        "Surface lacks its exact canonical render_ui Tool owner".into());
    let Some(source)=store else {
        // Pure typed builders have no canonical Store ordinal witness. Keep
        // their current Core order; do not invent order from map keys.
        return if surfaces.is_empty() { Ok(projected) } else { Err(invalid()) };
    };
    fn verified(projection: &HistoricalPartProjection, owner: &CanonicalPartOwner, revision: i64) -> bool {
        projection.owner == *owner && projection.canonical_revision == revision
            && matches!(projection.schema, Some(HistoricalPartSchema::MessagePartView | HistoricalPartSchema::AgentPart))
            && matches!(projection.binding, Some(SemanticIdentityAssessment::Canonical | SemanticIdentityAssessment::VerifiedLocalDerivation))
            && projection.source.as_ref().is_some_and(|source| source.owner() == owner && source.field() == PartBodyField::ContentJson)
    }
    fn reference(projection: &HistoricalPartProjection) -> Option<UiSurfaceRevisionRef> {
        let SemanticValue::Complete { value } = projection.fields.get("surface")? else { return None; };
        serde_json::from_value(serde_json::to_value(value).ok()?).ok()
    }
    let mut surface_parts = Vec::new();
    let mut verified_tool_turns = BTreeMap::<String,String>::new();
    for part in surfaces {
        let surface_id = canonical_part_id(&part).0.clone();
        let owner = CanonicalPartOwner::new(conversation.id.clone(), message.id.clone(), surface_id.clone())
            .map_err(|_| invalid())?;
        let surface = match &part {
            MessagePartView::UiSurface { surface, .. } => surface.clone(),
            MessagePartView::Historical { projection, .. } => {
                if projection.owner != owner || projection.canonical_revision != conversation.revision
                    || projection.canonical_kind != "ui_surface" { return Err(invalid()); }
                let public_reference = if verified(projection, &owner, conversation.revision)
                    && projection.schema == Some(HistoricalPartSchema::MessagePartView)
                    && projection.binding == Some(SemanticIdentityAssessment::Canonical) { reference(projection) } else { None };
                if let Some(reference) = public_reference { reference }
                else {
                    // Only a deferred origin needs an independent bounded read.
                    // Keep its public wrapper unchanged; this is not permission
                    // to lift its payload or expand the public selection budget.
                    let fact = source.read_historical_part(&owner, conversation.revision, 32768)
                        .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?
                        .ok_or_else(invalid)?;
                    if !verified(&fact, &owner, conversation.revision)
                        || fact.schema != Some(HistoricalPartSchema::MessagePartView)
                        || fact.binding != Some(SemanticIdentityAssessment::Canonical)
                        || fact.canonical_kind != "ui_surface" { return Err(invalid()); }
                    reference(&fact).ok_or_else(invalid)?
                }
            }
            _ => return Err(invalid()),
        };
        surface.validate().map_err(|_| invalid())?;
        let origin = &surface.identity.owner;
        if origin.conversation_id != conversation.id || origin.message_id != message.id { return Err(invalid()); }
        let bound = owners.resolve_part(&origin.originating_run_id, &origin.part_id)
            .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?
            .ok_or_else(invalid)?;
        if bound.conversation_id != conversation.id || bound.assistant_message_id != message.id { return Err(invalid()); }
        let tool = message.parts.iter().find(|value| value.id == origin.part_id && value.kind == "tool")
            .ok_or_else(invalid)?;
        if !verified_tool_turns.contains_key(&tool.id) {
            let tool_owner = CanonicalPartOwner::new(conversation.id.clone(), message.id.clone(), tool.id.clone())
                .map_err(|_| invalid())?;
            let tool_fact = source.read_historical_part(&tool_owner, conversation.revision, 32768)
                .map_err(|error| ConversationProjectionContractError::Store(error.to_string()))?
                .ok_or_else(invalid)?;
            if !verified(&tool_fact, &tool_owner, conversation.revision) || tool_fact.canonical_kind != "tool"
                || !matches!(tool_fact.fields.get("capability_id"), Some(SemanticValue::Complete { value: nola_domain::StructuredValue::String(value) }) if value == "render_ui") {
                return Err(invalid());
            }
            let Some(SemanticValue::Complete { value:nola_domain::StructuredValue::String(turn) })=tool_fact.fields.get("turn_id")
                else { return Err(invalid()); };
            owners.resolve_part_with_turn(&origin.originating_run_id,&tool.id,turn)
                .map_err(|error|ConversationProjectionContractError::Store(error.to_string()))?.ok_or_else(invalid)?;
            verified_tool_turns.insert(tool.id.clone(),turn.clone());
        }
        if let Some(current) = projected.iter().find(|value| canonical_part_id(value).0 == tool.id) {
            let witness=core_parts.and_then(|parts|parts.iter().find(|part|part.id()==tool.id));
            if !matches!(current, MessagePartView::Tool { capability_id, .. } if capability_id.0 == "render_ui")
                || !matches!(witness,Some(AgentPart::Tool(value)) if value.capability_id=="render_ui"
                    && verified_tool_turns.get(&tool.id).is_some_and(|turn|turn==&value.ownership.turn_id)) {
                return Err(invalid());
            }
        }
        surface_parts.push(part);
    }
    // Rebase replaces a checkpoint's retained context, not the product's
    // canonical transcript. Preserve every original semantic Part at its Store
    // ordinal. Missing checkpoint Parts remain readonly historical projections;
    // withheld/deferred bodies go through the same existing disclosure reader.
    projected.extend(surface_parts);
    let mut ids: BTreeSet<_> = projected.iter().map(|part| canonical_part_id(part).0.clone()).collect();
    if ids.len() != projected.len() { return Err(invalid()); }
    let mut original: Vec<_> = message.parts.iter().filter(|part| part.kind != "compaction").collect();
    original.sort_by_key(|part| part.ordinal);
    let mut order = Vec::with_capacity(original.len());
    for part in original {
        order.push(part.id.as_str());
        if ids.insert(part.id.clone()) {
            let history = crate::historical_part_projection::read(source, conversation, message, part, remaining)
                .map_err(ConversationProjectionContractError::InvalidStructuredValue)?;
            if let Some(history)=history {
                projected.push(MessagePartView::Historical { part_id: MessagePartId(part.id.clone()), revision,
                    projection: Box::new(history) });
            } else {
                // Current plain Stored text is also a valid historical source.
                // Reuse the existing reader's disclosure/bounded-text handling,
                // never guess a JSON body or manufacture a live Tool.
                let mut selected=message.clone();
                selected.parts=vec![part.clone()];
                projected.extend(project_stored_parts(Some(source),conversation,&selected,revision,remaining)?);
            }
        }
    }
    weave_stored_part_order(projected, &order)
}

fn canonical_part_id(part: &MessagePartView) -> &MessagePartId {
    match part {
        MessagePartView::UiAction { part_id, .. } | MessagePartView::UiSurface { part_id, .. }
        | MessagePartView::Historical { part_id, .. } | MessagePartView::Text { part_id, .. }
        | MessagePartView::Reasoning { part_id, .. } | MessagePartView::Tool { part_id, .. }
        | MessagePartView::Source { part_id, .. } | MessagePartView::Citation { part_id, .. }
        | MessagePartView::Attachment { part_id, .. } | MessagePartView::Task { part_id, .. }
        | MessagePartView::Skill { part_id, .. } | MessagePartView::ModelAttempt { part_id, .. }
        | MessagePartView::Compaction { part_id, .. } | MessagePartView::Error { part_id, .. } => part_id,
    }
}

fn semantic_kind(part: &MessagePartView) -> Option<&str> {
    match part {
        MessagePartView::Task { .. } => Some("task"),
        MessagePartView::Skill { .. } => Some("skill"),
        MessagePartView::ModelAttempt { .. } => Some("model_attempt"),
        MessagePartView::Historical { projection, .. } => Some(&projection.canonical_kind),
        _ => None,
    }
}

fn upsert_semantic_parts(parts: &mut Vec<MessagePartView>, updates: Vec<MessagePartView>)
    -> Result<(), ConversationProjectionContractError> {
    for update in updates {
        if let Some(index)=parts.iter().position(|part|canonical_part_id(part)==canonical_part_id(&update)) {
            let kind = semantic_kind(&update).filter(|kind| matches!(*kind,"task"|"skill"|"model_attempt"));
            if kind.is_none() || semantic_kind(&parts[index]) != kind {
                return Err(ConversationProjectionContractError::Store("Canonical semantic Part kind collision".into()));
            }
            parts[index]=update;
        } else { parts.push(update); }
    }
    Ok(())
}

fn weave_stored_part_order(
    projected: Vec<MessagePartView>, original_order: &[&str],
) -> Result<Vec<MessagePartView>, ConversationProjectionContractError> {
    let mut parts = BTreeMap::new();
    for part in projected {
        if parts.insert(canonical_part_id(&part).0.clone(), part).is_some() {
            return Err(ConversationProjectionContractError::Store("Duplicate canonical Part during Surface merge".into()));
        }
    }
    let mut result = Vec::with_capacity(parts.len());
    for id in original_order {
        let part = parts.remove(*id).ok_or_else(|| ConversationProjectionContractError::Store("Canonical Part order is incomplete or duplicated".into()))?;
        result.push(part);
    }
    if !parts.is_empty() { return Err(ConversationProjectionContractError::Store("Core Part lacks its canonical Store ordinal".into())); }
    Ok(result)
}

pub(crate) fn materialize_conversation_projection_with_owners<E: std::fmt::Display>(
    store: Option<&dyn HistoricalPartProjectionRepository<Error = E>>,
    stored: StoredConversation,
    core: Option<&CoreConversationProjection>,
    tasks: &TaskProjection,
    skills: &SessionSkillProjection,
    context_usage: ContextUsageSnapshot,
    partial: bool,
    owners: &AssistantOwnerResolver,
) -> Result<ConversationProjection, ConversationProjectionContractError> {
    let revision = core
        .map(|projection| projection.revision)
        .unwrap_or_else(|| u64::try_from(stored.revision).unwrap_or_default());
    let mut assistant_message_ids_by_run = BTreeMap::new();
    let mut assistant_message_ids_by_part = BTreeMap::new();
    let mut stored_assistant_message_ids = BTreeSet::new();
    for message in stored
        .messages
        .iter()
        .filter(|message| message.role == "assistant")
    {
        stored_assistant_message_ids.insert(message.id.clone());
        if let Some(run_id) = message.run_id.as_deref() {
            // A steered Core run can contain more than one visible assistant
            // message. The first message is the Host-owned placeholder used
            // to resolve Core's internal `<run>:assistant` owner; later
            // messages keep their explicit Part message IDs.
            assistant_message_ids_by_run
                .entry(run_id.to_owned())
                .or_insert_with(|| message.id.clone());
        }
        for part in &message.parts {
            assistant_message_ids_by_part.insert(part.id.clone(), message.id.clone());
        }
    }
    let mut core_parts_by_message: BTreeMap<String, Vec<&AgentPart>> = BTreeMap::new();
    if let Some(core) = core {
        for part_id in &core.order {
            let Some(part) = core.parts.get(part_id) else {
                continue;
            };
            // Compaction Parts describe a maintenance Run. They belong to the
            // session/context projection, not to a visible assistant Message.
            // Keeping that boundary here prevents internal Run identities
            // from becoming manufactured chat bubbles.
            if matches!(part, AgentPart::Compaction(_)) {
                continue;
            }
            let message_id = if let Some(message_id) = assistant_message_ids_by_part.get(part.id())
            {
                // Internal autonomous Turns can share one Host-owned message.
                // The canonical Store has already bound each Part to that
                // message, so it is a stronger owner than the latest run_id
                // recorded on the message row.
                message_id.clone()
            } else if stored_assistant_message_ids.contains(part.message_id()) {
                part.message_id().to_owned()
            } else {
                assistant_message_ids_by_run
                    .get(part.turn_id())
                    .cloned()
                    .unwrap_or_else(|| part.message_id().to_owned())
            };
            if partial && !stored_assistant_message_ids.contains(&message_id) {
                continue;
            }
            core_parts_by_message
                .entry(message_id)
                .or_default()
                .push(part);
        }
    }

    let mut remaining = 32768;
    let mut represented_assistant_messages = BTreeSet::new();
    let mut messages = Vec::with_capacity(stored.messages.len());
    for message in &stored.messages {
        let role = message_role(&message.role)?;
        let state = message_state(&message.status)?;
        let parts = match role {
            MessageRole::Assistant => {
                let mut projected =
                    if let Some(core_parts) = core_parts_by_message.get(message.id.as_str()) {
                        represented_assistant_messages.insert(message.id.clone());
                        core_parts
                            .iter()
                            .map(|part| project_core_part(part, revision, &message.id))
                            .collect::<Result<Vec<_>, _>>()?
                    } else if core.is_none() {
                        project_stored_parts(store, &stored, message, revision, &mut remaining)?
                    } else {
                        // Application creates the canonical assistant Message before the
                        // model emits its first Part. The Turn can also be cancelled or
                        // fail before the first semantic chunk is committed. Those are
                        // valid zero-output product states. Completed describes the
                        // message lifecycle, not successful task fulfillment: an
                        // explicit unknown-effect decision can seal a blocked Run
                        // before any model Part exists. RunResult owns that outcome;
                        // projection must not invent a second completion gate.
                        Vec::new()
                    };
                // Surface references are Application-owned durable Parts, not Agent
                // checkpoint Parts. Merge them even while the Agent is streaming.
                if core.is_some() {
                    let mut surface_message = message.clone();
                    surface_message.parts.retain(|part| part.kind == "ui_surface");
                    let surfaces = project_stored_parts(store, &stored, &surface_message, revision, &mut remaining)?;
                    projected = merge_canonical_stored_parts(store, &stored, message, revision, projected,
                        core_parts_by_message.get(message.id.as_str()).map(|parts|parts.as_slice()), surfaces, &mut remaining, owners)?;
                }
                upsert_semantic_parts(&mut projected,project_tasks(message, tasks, owners)?)?;
                upsert_semantic_parts(&mut projected,project_skills(message, skills, owners)?)?;
                projected
            }
            MessageRole::User | MessageRole::System => {
                project_stored_parts(store, &stored, message, revision, &mut remaining)?
            }
        };
        messages.push(MessageView {
            message_id: MessageId(message.id.clone()),
            role,
            state,
            revision,
            created_at_ms: message.created_at_ms,
            parts,
        });
    }

    if core.is_some() && !partial {
        if let Some(unowned) = core_parts_by_message
            .keys()
            .find(|message_id| !represented_assistant_messages.contains(*message_id))
        {
            return Err(
                ConversationProjectionContractError::MissingAssistantProjection(
                    (*unowned).to_owned(),
                ),
            );
        }
    }

    // Resolve by persisted message identity; the loading boundary subsequently
    // checks whether this Run still owns execution.
    let active_run = stored.messages.iter().rev().find_map(|stored_message| {
        let run_id = stored_message.run_id.as_ref()?;
        messages
            .iter()
            .find(|message| message.message_id.0 == stored_message.id)
            .and_then(|message| active_run_for_message(message, RunId(run_id.clone())))
    });
    let surface = conversation_surface(
        u64::try_from(stored.revision).unwrap_or_default(),
        &messages,
        active_run.as_ref(),
    );
    Ok(ConversationProjection {
        execution_ownership: nola_domain::ConversationExecutionOwnership::Unavailable,
        contract_version: CONVERSATION_PROJECTION_CONTRACT_VERSION,
        conversation_id: ConversationId(stored.id),
        revision: u64::try_from(stored.revision).unwrap_or_default(),
        title: stored.title,
        messages,
        active_run,
        surface,
        context_usage: ContextUsageView {
            measurement: None,
            prompt_cache_measurement: None,
            prompt_cache_diagnostic: None,
            conversation_cache_usage: None,
            cache_usage: context_usage.cache_usage,
            used_tokens: u64::try_from(context_usage.used_tokens).unwrap_or(u64::MAX),
            context_window_tokens: u64::try_from(context_usage.context_window_tokens)
                .unwrap_or(u64::MAX),
            prompt_token_limit: u64::try_from(context_usage.prompt_token_limit).unwrap_or(u64::MAX),
            estimated: context_usage.usage_is_estimated,
            measured_at_revision: revision,
        },
    })
}

fn conversation_surface(
    revision: u64,
    messages: &[MessageView],
    active_run: Option<&RunActivityView>,
) -> ConversationSurfaceView {
    let live_message_id = messages
        .iter()
        .rev()
        .find(|message| {
            message.role == MessageRole::Assistant
                && matches!(message.state, MessageState::Pending | MessageState::Running)
        })
        .map(|message| message.message_id.clone());
    let committed_message_ids = messages
        .iter()
        .filter(|message| Some(&message.message_id) != live_message_id.as_ref())
        .map(|message| message.message_id.clone())
        .collect();
    let current_activity_part_id = active_run.and_then(|run| run.active_part_id.clone());
    let archived_activity_part_ids = messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| {
            let (part_id, state) = activity_part_identity(part)?;
            (Some(part_id) != current_activity_part_id.as_ref() && is_terminal_activity(state))
                .then(|| part_id.clone())
        })
        .collect();
    let show_assistant_placeholder = live_message_id
        .as_ref()
        .and_then(|message_id| {
            messages
                .iter()
                .find(|message| &message.message_id == message_id)
        })
        .is_some_and(|message| {
            !message.parts.iter().any(|part| {
                matches!(part, MessagePartView::Text { markdown, .. } if !markdown.trim().is_empty())
            })
        });

    ConversationSurfaceView {
        revision,
        committed_message_ids,
        live_message_id,
        current_activity_part_id,
        archived_activity_part_ids,
        show_assistant_placeholder,
    }
}

fn activity_part_identity(part: &MessagePartView) -> Option<(&MessagePartId, &ActivityState)> {
    match part {
        MessagePartView::Reasoning { part_id, state, .. }
        | MessagePartView::Tool { part_id, state, .. }
        | MessagePartView::Task { part_id, state, .. }
        | MessagePartView::Skill { part_id, state, .. }
        | MessagePartView::ModelAttempt { part_id, state, .. } => Some((part_id, state)),
        _ => None,
    }
}

fn is_terminal_activity(state: &ActivityState) -> bool {
    matches!(
        state,
        ActivityState::Sealed | ActivityState::Completed | ActivityState::Cancelled | ActivityState::Failed
    )
}

fn empty_skill_projection(conversation_id: &str) -> SessionSkillProjection {
    SessionSkillProjection {
        session_id: conversation_id.to_owned(),
        cursor: 0,
        revision: 0,
        catalog_revision: 0,
        order: Vec::new(),
        runs: BTreeMap::new(),
    }
}

fn project_tasks(
    message: &StoredMessage,
    projection: &TaskProjection,
    owners: &AssistantOwnerResolver,
) -> Result<Vec<MessagePartView>, ConversationProjectionContractError> {
    let selected = projection
        .order
        .iter()
        .filter_map(|task_id| projection.tasks.get(task_id))
        .map(|task| {
            owners
                .resolve_activity(task.owner_session_id.as_str(), task.owner_turn_id.as_str(), "task", &task.id)
                .map(|owner| (task, owner))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ConversationProjectionContractError::Store(format!("Task ownership: {e}")))?;
    Ok(selected
        .into_iter()
        .filter(|(_, owner)| owner.is_some_and(|owner| owner.assistant_message_id == message.id))
        .map(|(task, _)| task_part(task, projection.revision).to_message_part())
        .collect())
}

fn project_skills(
    message: &StoredMessage,
    projection: &SessionSkillProjection,
    owners: &AssistantOwnerResolver,
) -> Result<Vec<MessagePartView>, ConversationProjectionContractError> {
    let selected = projection
        .order
        .iter()
        .filter_map(|run_id| projection.runs.get(run_id))
        .map(|run| {
            owners
                .resolve_activity(&run.owner.session_id, &run.owner.parent_turn_id, "skill", &run.run_id)
                .map(|owner| (run, owner))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ConversationProjectionContractError::Store(format!("Skill ownership: {e}")))?;
    Ok(selected
        .into_iter()
        .filter(|(_, owner)| owner.is_some_and(|owner| owner.assistant_message_id == message.id))
        .map(|(run, _)| skill_part(run, projection.revision).to_message_part())
        .collect())
}

fn message_role(role: &str) -> Result<MessageRole, ConversationProjectionContractError> {
    match role {
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        "system" => Ok(MessageRole::System),
        other => Err(ConversationProjectionContractError::UnknownMessageRole(
            other.to_owned(),
        )),
    }
}

fn message_state(state: &str) -> Result<MessageState, ConversationProjectionContractError> {
    match state {
        "pending" | "queued" => Ok(MessageState::Pending),
        // Waiting for an answer suspends execution, not the assistant message.
        // The exact wait and its questions remain in the shared wait projection.
        "running" | "streaming" | "waiting_input" => Ok(MessageState::Running),
        "completed" | "complete" => Ok(MessageState::Completed),
        "cancelled" | "canceled" => Ok(MessageState::Cancelled),
        "failed" | "error" => Ok(MessageState::Failed),
        other => Err(ConversationProjectionContractError::UnknownMessageState(
            other.to_owned(),
        )),
    }
}

pub(crate) fn read_historical_part<S: HistoricalPartProjectionRepository + ?Sized>(
    store: &S,
    owner: &nola_domain::CanonicalPartOwner,
    revision: i64,
    maximum_output_bytes: usize,
) -> Result<Option<nola_domain::HistoricalPartProjection>, String> {
    if !(1024..=32768).contains(&maximum_output_bytes) {
        return Err("historical Part query budget must be 1024..32768 bytes".into());
    }
    let result = match store
        .read_historical_part_with_budget(owner, revision, maximum_output_bytes)
        .map_err(|error| error.to_string())?
    {
        nola_domain::HistoricalPartRead::Absent => None,
        nola_domain::HistoricalPartRead::Available(projection) => Some(*projection),
        nola_domain::HistoricalPartRead::BudgetDeferred { canonical_kind } => {
            Some(crate::historical_part_projection::not_loaded_history(
                owner.clone(),
                revision,
                canonical_kind,
            ))
        }
    };
    check_encoded_budget(&result, maximum_output_bytes).map_err(|_| {
        "historical Part query budget cannot hold its canonical metadata".to_owned()
    })?;
    Ok(result)
}

fn check_encoded_budget(
    value: &impl serde::Serialize,
    budget: usize,
) -> Result<(), serde_json::Error> {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| {
                std::io::Error::other("projection metadata exceeds encoded budget")
            })?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(budget), value)
}
fn bounded_window(
    window: ConversationProjectionWindowV1,
) -> Result<ConversationProjectionWindowV1, ConversationProjectionContractError> {
    // Includes placeholder owners, field catalogs, live Parts, text and all
    // pagination metadata; this is the same cap as the native presenter FFI.
    check_encoded_budget(&window, 2 * 1024 * 1024).map_err(|error| {
        ConversationProjectionContractError::InvalidStructuredValue(error.to_string())
    })?;
    Ok(window)
}

fn project_stored_parts<E: std::fmt::Display>(
    store: Option<&dyn HistoricalPartProjectionRepository<Error = E>>,
    conversation: &StoredConversation,
    message: &StoredMessage,
    revision: u64,
    remaining: &mut usize,
) -> Result<Vec<MessagePartView>, ConversationProjectionContractError> {
    message
        .parts
        .iter()
        .map(|part| {
            if let Some(store) = store {
                // Always inspect the canonical source. A bounded StoredPart may
                // omit JSON even though the immutable original field exists.
                if let Some(projection) = crate::historical_part_projection::read(
                    store,
                    conversation,
                    message,
                    part,
                    remaining,
                )
                .map_err(ConversationProjectionContractError::InvalidStructuredValue)?
                {
                    return Ok(crate::stored_public_part::project(projection, revision));
                }
            } else if part.content_json.is_some() {
                return Err(ConversationProjectionContractError::InvalidStructuredValue(
                    "historical JSON projection requires its canonical Store owner".into(),
                ));
            }
            if part.kind == "text" {
                return Ok(MessagePartView::Text {
                    part_id: MessagePartId(part.id.clone()),
                    revision,
                    markdown: part.content_text.clone(),
                    is_final: matches!(part.state.as_str(), "complete" | "completed"),
                    review: None,
                });
            }
            Err(ConversationProjectionContractError::InvalidStructuredValue(
                format!("stored Part {} has no structured original", part.id),
            ))
        })
        .collect()
}

fn project_core_part(
    part: &AgentPart,
    revision: u64,
    canonical_message_id: &str,
) -> Result<MessagePartView, ConversationProjectionContractError> {
    let projected = match PublicPart::from_core(part) {
        PublicPart::Text(value) => MessagePartView::Text {
            part_id: MessagePartId(value.identity.part_id.to_owned()),
            revision,
            // Review is explicit display evidence, not permission to disclose
            // an unapproved candidate through ordinary Markdown consumers.
            markdown: if value.review.is_none()
                || value.review == Some(nola_domain::TextReviewState::Approved)
            {
                value.text.to_owned()
            } else {
                String::new()
            },
            is_final: value.is_complete,
            review: value.review,
        },
        PublicPart::Reasoning(value) => MessagePartView::Reasoning {
            part_id: MessagePartId(value.identity.part_id.to_owned()),
            revision,
            state: if value.is_complete {
                ActivityState::Completed
            } else {
                ActivityState::Running
            },
            title: value.title.to_owned(),
            summary: Some(value.detail.to_owned()),
        },
        PublicPart::Tool(value) => {
            if value.capability_id.trim().is_empty() {
                return Err(ConversationProjectionContractError::EmptyCapabilityId(
                    value.identity.part_id.to_owned(),
                ));
            }
            let output = value
                .structured_output
                .cloned()
                .map(structured_value)
                .transpose()?;
            MessagePartView::Tool {
                execution_source: value.execution_source.clone(),
                part_id: MessagePartId(value.identity.part_id.to_owned()),
                revision,
                tool_call_id: ToolCallId(value.call_id.to_owned()),
                capability_id: CapabilityId(value.capability_id.to_owned()),
                state: tool_state(value.state),
                display_key: None,
                title: value.title.to_owned(),
                summary: value.summary.map(str::to_owned),
                origin: value.origin.clone(),
                permission: value.permission.cloned(),
                input: structured_value(value.input.clone())?,
                output,
                output_preview: value.output_preview.map(str::to_owned),
                full_output: value.has_full_output.then(|| ToolOutputReference {
                    message_id: MessageId(canonical_message_id.to_owned()),
                    part_id: MessagePartId(value.identity.part_id.to_owned()),
                    tool_call_id: ToolCallId(value.call_id.to_owned()),
                }),
                artifacts: value.artifacts.to_vec(),
                started_at_ms: value.started_at_ms,
                finished_at_ms: value.finished_at_ms,
                affected_resources: affected_resources_from_artifacts(value.artifacts),
            }
        }
        PublicPart::Source(value) => MessagePartView::Source {
            part_id: MessagePartId(value.identity.part_id.to_owned()),
            revision,
            source_id: value.source_id.to_owned(),
            title: value.title.to_owned(),
            uri: value.uri.to_owned(),
            mime_type: value.mime_type.map(str::to_owned),
            snippet: None,
            evidence: value.evidence.cloned(),
        },
        PublicPart::Citation(value) => MessagePartView::Citation {
            part_id: MessagePartId(value.identity.part_id.to_owned()),
            revision,
            citation_id: value.citation_id.to_owned(),
            source_id: value.source_id.to_owned(),
            title: value.title.to_owned(),
            uri: value.uri.to_owned(),
            mime_type: value.mime_type.map(str::to_owned),
            origin_tool_call_id: value.origin_tool_call_id.to_owned(),
            location: value.location.cloned(),
            section: value.location.and_then(|location| location.section.clone()),
            page: value.location.and_then(|location| location.page),
            line_start: value.location.and_then(|location| location.line_start),
            line_end: value.location.and_then(|location| location.line_end),
            range_start: value.range.map(|range| range.start),
            range_end: value.range.map(|range| range.end),
        },
        PublicPart::Attachment(value) => MessagePartView::Attachment {
            part_id: MessagePartId(value.identity.part_id.to_owned()),
            revision,
            attachment_id: AttachmentId(value.identity.part_id.to_owned()),
            name: value.name.to_owned(),
            mime_type: value.mime_type.map(str::to_owned),
            uri: value.uri.to_owned(),
            operation: value.operation.map(str::to_owned),
        },
        PublicPart::Step(value) => MessagePartView::Task {
            part_id: MessagePartId(value.identity.part_id.to_owned()),
            revision,
            task_id: value.identity.part_id.to_owned(),
            display_key: value.title.to_owned(),
            state: step_state(value.status),
            progress: None,
            current_step: Some(value.title.to_owned()),
            result_ref: None,
            cancel_requested: false,
        },
        PublicPart::Compaction(value) => MessagePartView::Compaction {
            part_id: MessagePartId(value.identity.part_id.to_owned()),
            revision,
            summary: value.summary.to_owned(),
            retained_item_ids: value.retained_item_ids.to_vec(),
            source_fingerprint: value.source_fingerprint.to_owned(),
            original_token_count: value.original_token_count,
            compacted_token_count: value.compacted_token_count,
        },
    };
    projected
        .validate_public_semantics(&MessageId(canonical_message_id.to_owned()))
        .map_err(|error| {
            ConversationProjectionContractError::InvalidStructuredValue(error.to_string())
        })?;
    Ok(projected)
}

/// Legacy resource conveniences derive only from declared artifacts. The
/// complete original artifacts remain separate, including absent operations.
fn affected_resources_from_artifacts(
    artifacts: &[nola_domain::ArtifactReference],
) -> Vec<AffectedResourceView> {
    artifacts
        .iter()
        .map(|artifact| AffectedResourceView {
            resource_type: artifact
                .mime_type
                .clone()
                .unwrap_or_else(|| "file".to_owned()),
            uri: artifact.uri.clone(),
            operation: artifact
                .operation
                .clone()
                .unwrap_or_else(|| "read".to_owned()),
        })
        .collect()
}

fn structured_value(
    value: serde_json::Value,
) -> Result<StructuredValue, ConversationProjectionContractError> {
    serde_json::from_value(value).map_err(|error| {
        ConversationProjectionContractError::InvalidStructuredValue(error.to_string())
    })
}

fn tool_state(state: ToolPartState) -> ActivityState {
    match state {
        ToolPartState::Pending => ActivityState::Pending,
        ToolPartState::WaitingForPermission => ActivityState::WaitingForPermission,
        ToolPartState::WaitingForInput => ActivityState::WaitingForInput,
        ToolPartState::Running => ActivityState::Running,
        ToolPartState::Completed => ActivityState::Completed,
        ToolPartState::Failed => ActivityState::Failed,
        ToolPartState::Cancelled => ActivityState::Cancelled,
        ToolPartState::Sealed => ActivityState::Sealed,
    }
}

fn step_state(state: StepStatus) -> ActivityState {
    match state {
        StepStatus::Started => ActivityState::Running,
        StepStatus::Completed => ActivityState::Completed,
        StepStatus::Failed => ActivityState::Failed,
        StepStatus::Cancelled => ActivityState::Cancelled,
    }
}

fn active_run_for_message(message: &MessageView, run_id: RunId) -> Option<RunActivityView> {
    if message.role != MessageRole::Assistant || message.state != MessageState::Running {
        return None;
    }
    let active_part_id = message.parts.iter().rev().find_map(|part| match part {
        MessagePartView::Reasoning {
            part_id,
            state: ActivityState::Running,
            ..
        }
        | MessagePartView::Tool {
            part_id,
            state: ActivityState::Running | ActivityState::WaitingForPermission,
            ..
        }
        | MessagePartView::Task {
            part_id,
            state: ActivityState::Running,
            ..
        }
        | MessagePartView::Skill {
            part_id,
            state: ActivityState::Running,
            ..
        }
        | MessagePartView::ModelAttempt {
            part_id,
            state: ActivityState::Running,
            ..
        } => Some(part_id.clone()),
        _ => None,
    });
    Some(RunActivityView {
        run_id,
        state: ActivityState::Running,
        active_part_id,
        display_key: None,
        progress: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_agent_engine::agent::parts::{
        CompactionPart, PartOwnership, TextPart, ToolPart,
    };
    use nola_store::StoredPart;
    use nola_store_port::assistant_ownership::{
        AssistantOwner, AssistantPartOwner, AssistantTurnOwner,
    };

    fn fixture_ownership(message: &str, turn: &str, parts: &[&str]) -> AssistantOwnershipFacts {
        let owner = AssistantOwner {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            assistant_message_id: message.into(),
        };
        AssistantOwnershipFacts {
            activities: Vec::new(),
            admissions: vec![owner.clone()],
            turns: vec![AssistantTurnOwner {
                turn_id: turn.into(),
                owner: owner.clone(),
                admission: true,
            }],
            parts: parts
                .iter()
                .map(|part| AssistantPartOwner {
                    part_id: (*part).into(),
                    turn_id: Some(turn.into()),
                    owner: owner.clone(),
                })
                .collect(),
        }
    }

    fn stored_conversation() -> StoredConversation {
        StoredConversation {
            configuration_revision: 1,
            id: "conversation-1".to_owned(),
            title: "Contract".to_owned(),
            revision: 4,
            created_at_ms: 1,
            updated_at_ms: 2,
            settings_json: None,
            messages: vec![
                StoredMessage {
                    id: "user-1".to_owned(),
                    run_id: None,
                    role: "user".to_owned(),
                    status: "complete".to_owned(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(1),
                    parts: vec![StoredPart {
                        id: "user-part".to_owned(),
                        kind: "text".to_owned(),
                        ordinal: 1,
                        state: "complete".to_owned(),
                        content_text: "hello".to_owned(),
                        content_json: None,
                        content_json_deferred: false,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    }],
                },
                StoredMessage {
                    id: "turn-1:assistant".to_owned(),
                    run_id: Some("run-1".to_owned()),
                    role: "assistant".to_owned(),
                    status: "streaming".to_owned(),
                    ordinal: 2,
                    created_at_ms: 2,
                    completed_at_ms: None,
                    parts: vec![],
                },
            ],
        }
    }

    #[test]
    fn legacy_text_view_exposes_only_approved_reviewed_text() {
        use nola_agent_engine::agent::parts::TextReviewState;
        for state in [
            TextReviewState::Pending,
            TextReviewState::Rejected,
            TextReviewState::Interrupted,
            TextReviewState::Approved,
        ] {
            let part = AgentPart::Text(TextPart {
                id: "reviewed".into(),
                ownership: PartOwnership::for_turn("turn"),
                text: "candidate".into(),
                is_complete: true,
                review: Some(state),
            });
            let MessagePartView::Text {
                markdown, review, ..
            } = project_core_part(&part, 1, "canonical-assistant").unwrap()
            else {
                panic!("expected text")
            };
            assert_eq!(
                markdown,
                if state == TextReviewState::Approved {
                    "candidate"
                } else {
                    ""
                }
            );
            assert_eq!(review, Some(state));
        }
    }

    #[test]
    fn materializes_one_stable_assistant_message_from_core_parts() {
        let ownership = PartOwnership::for_turn("turn-1");
        let mut core = CoreConversationProjection::default();
        core.apply(
            nola_agent_engine::agent::parts::PartUpdate::Created {
                part: AgentPart::Text(TextPart {
                    review: None,
                    id: "text-1".to_owned(),
                    ownership: ownership.clone(),
                    text: "answer".to_owned(),
                    is_complete: false,
                }),
            },
        )
        .expect("text Part");
        core.apply(
            nola_agent_engine::agent::parts::PartUpdate::Created {
                part: AgentPart::Tool(ToolPart {
                    execution_source: Default::default(),
                    id: "tool-1".to_owned(),
                    ownership,
                    call_id: "call-1".to_owned(),
                    capability_id: "file.read".to_owned(),
                    title: "ignored display metadata".to_owned(),
                    origin: Default::default(),
                    state: ToolPartState::Running,
                    input: serde_json::json!({"path": "/tmp/a"}),
                    raw_input: String::new(),
                    permission: None,
                    summary: None,
                    output_preview: None,
                    structured_output: None,
                    full_output_ref: None,
                    artifacts: vec![],
                    started_at_ms: Some(1),
                    finished_at_ms: None,
                }),
            },
        )
        .expect("tool Part");

        let projection = materialize_conversation_projection_v1(
            stored_conversation(),
            &core,
            &TaskProjection::default(),
            &empty_skill_projection("conversation-1"),
            ContextUsageSnapshot {
                used_tokens: 512,
                context_window_tokens: 65_536,
                usage_is_estimated: false,
                ..Default::default()
            },
            "conversation-1",
            &fixture_ownership("turn-1:assistant", "turn-1", &["text-1", "tool-1"]),
        )
        .expect("materialize projection");

        assert_eq!(projection.messages.len(), 2);
        assert_eq!(projection.messages[1].parts.len(), 2);
        assert_eq!(projection.context_usage.used_tokens, 512);
        assert_eq!(
            projection.surface.live_message_id,
            Some(MessageId("turn-1:assistant".to_owned()))
        );
        assert_eq!(
            projection.surface.current_activity_part_id,
            Some(MessagePartId("tool-1".to_owned()))
        );
        assert!(!projection.surface.show_assistant_placeholder);
        assert!(matches!(
            &projection.messages[1].parts[1],
            MessagePartView::Tool { capability_id, .. }
                if capability_id.0 == "file.read"
        ));
    }

    #[test]
    fn surface_keeps_placeholder_live_and_archives_terminal_activity() {
        let mut stored = stored_conversation();
        stored.messages[1].status = "pending".to_owned();
        let projection = materialize_conversation_projection::<String>(
            None,
            stored,
            None,
            &TaskProjection::default(),
            &empty_skill_projection("conversation-1"),
            ContextUsageSnapshot::default(),
            false,
            None,
            &AssistantOwnershipFacts::default(),
        )
        .expect("materialize placeholder");

        assert_eq!(
            projection.surface.committed_message_ids,
            vec![MessageId("user-1".to_owned())]
        );
        assert_eq!(
            projection.surface.live_message_id,
            Some(MessageId("turn-1:assistant".to_owned()))
        );
        assert!(projection.surface.show_assistant_placeholder);
    }

    #[test]
    fn preserves_distinct_assistant_messages_inside_one_steered_run() {
        let mut stored = stored_conversation();
        stored.messages[1].id = "host-assistant-1".to_owned();
        stored.messages[1].run_id = Some("run-1".to_owned());
        stored.messages[1].status = "complete".to_owned();
        stored.messages[1].completed_at_ms = Some(3);
        stored.messages.push(StoredMessage {
            id: "steering-assistant-2".to_owned(),
            run_id: Some("run-1".to_owned()),
            role: "assistant".to_owned(),
            status: "complete".to_owned(),
            ordinal: 3,
            created_at_ms: 4,
            completed_at_ms: Some(5),
            parts: vec![],
        });

        let mut core = CoreConversationProjection::default();
        core.apply(
            nola_agent_engine::agent::parts::PartUpdate::Created {
                part: AgentPart::Text(TextPart {
                    review: None,
                    id: "text-1".to_owned(),
                    ownership: PartOwnership::for_turn("run-1"),
                    text: "first answer".to_owned(),
                    is_complete: true,
                }),
            },
        )
        .expect("initial assistant Part");
        core.apply(
            nola_agent_engine::agent::parts::PartUpdate::Created {
                part: AgentPart::Text(TextPart {
                    review: None,
                    id: "text-2".to_owned(),
                    ownership: PartOwnership {
                        message_id: "steering-assistant-2".to_owned(),
                        turn_id: "run-1".to_owned(),
                    },
                    text: "steering answer".to_owned(),
                    is_complete: true,
                }),
            },
        )
        .expect("steering assistant Part");

        let mut facts = fixture_ownership("host-assistant-1", "run-1", &["text-1"]);
        facts.parts.push(AssistantPartOwner {
            part_id: "text-2".into(),
            turn_id: Some("run-1".into()),
            owner: AssistantOwner {
                conversation_id: "conversation-1".into(),
                run_id: "run-1".into(),
                assistant_message_id: "steering-assistant-2".into(),
            },
        });
        let projection = materialize_conversation_projection_v1(
            stored,
            &core,
            &TaskProjection::default(),
            &empty_skill_projection("conversation-1"),
            ContextUsageSnapshot::default(),
            "conversation-1",
            &facts,
        )
        .expect("materialize multi-message steered run");

        assert_eq!(projection.messages.len(), 3);
        assert!(matches!(
            &projection.messages[1].parts[0],
            MessagePartView::Text { markdown, .. } if markdown == "first answer"
        ));
        assert!(matches!(
            &projection.messages[2].parts[0],
            MessagePartView::Text { markdown, .. } if markdown == "steering answer"
        ));
    }

    #[test]
    fn permits_an_in_flight_assistant_before_the_first_core_part() {
        let projection = materialize_conversation_projection_v1(
            stored_conversation(),
            &CoreConversationProjection::default(),
            &TaskProjection::default(),
            &empty_skill_projection("conversation-1"),
            ContextUsageSnapshot::default(),
            "conversation-1",
            &fixture_ownership("turn-1:assistant", "turn-1", &[]),
        )
        .expect("in-flight assistant without a first Part is valid");

        assert_eq!(projection.messages.len(), 2);
        assert_eq!(projection.messages[1].state, MessageState::Running);
        assert!(projection.messages[1].parts.is_empty());
        assert!(projection.active_run.is_some());
    }

    #[test]
    fn permits_a_closed_assistant_without_inventing_a_success_requirement() {
        let mut stored = stored_conversation();
        stored.messages[1].status = "complete".to_owned();
        stored.messages[1].completed_at_ms = Some(3);

        let projection = materialize_conversation_projection_v1(
                stored,
                &CoreConversationProjection::default(),
                &TaskProjection::default(),
                &empty_skill_projection("conversation-1"),
                ContextUsageSnapshot::default(),
                "conversation-1",
                &fixture_ownership("turn-1:assistant", "turn-1", &[]),
            ).expect("closed message lifecycle does not imply a successful answer");
        assert_eq!(projection.messages[1].state, MessageState::Completed);
        assert!(projection.messages[1].parts.is_empty());
    }

    #[test]
    fn permits_cancelled_and_failed_assistants_before_the_first_core_part() {
        for (stored_state, expected_state) in [
            ("cancelled", MessageState::Cancelled),
            ("failed", MessageState::Failed),
        ] {
            let mut stored = stored_conversation();
            stored.messages[1].status = stored_state.to_owned();
            stored.messages[1].completed_at_ms = Some(3);

            let projection = materialize_conversation_projection_v1(
                stored,
                &CoreConversationProjection::default(),
                &TaskProjection::default(),
                &empty_skill_projection("conversation-1"),
                ContextUsageSnapshot::default(),
                "conversation-1",
                &fixture_ownership("turn-1:assistant", "turn-1", &[]),
            )
            .expect("zero-output terminal assistant is a valid projection");

            assert_eq!(projection.messages[1].state, expected_state);
            assert!(projection.messages[1].parts.is_empty());
            assert!(projection.active_run.is_none());
        }
    }

    #[test]
    fn rejects_tool_without_capability_identity() {
        let ownership = PartOwnership::for_turn("turn-1");
        let mut core = CoreConversationProjection::default();
        core.apply(
            nola_agent_engine::agent::parts::PartUpdate::Created {
                part: AgentPart::Tool(ToolPart {
                    execution_source: Default::default(),
                    id: "tool-1".to_owned(),
                    ownership,
                    call_id: "call-1".to_owned(),
                    capability_id: "".to_owned(),
                    title: "write_file".to_owned(),
                    origin: Default::default(),
                    state: ToolPartState::Running,
                    input: serde_json::json!({}),
                    raw_input: String::new(),
                    permission: None,
                    summary: None,
                    output_preview: None,
                    structured_output: None,
                    full_output_ref: None,
                    artifacts: vec![],
                    started_at_ms: None,
                    finished_at_ms: None,
                }),
            },
        )
        .expect("tool Part");

        assert!(matches!(
            materialize_conversation_projection_v1(
                stored_conversation(),
                &core,
                &TaskProjection::default(),
                &empty_skill_projection("conversation-1"),
                ContextUsageSnapshot::default(),
                "conversation-1",
                &fixture_ownership("turn-1:assistant", "turn-1", &["tool-1"]),
            ),
            Err(ConversationProjectionContractError::EmptyCapabilityId(_))
        ));
    }

    #[test]
    fn maintenance_compaction_part_does_not_require_a_visible_message_owner() {
        let mut core = CoreConversationProjection::default();
        core.apply(
            nola_agent_engine::agent::parts::PartUpdate::Created {
                part: AgentPart::Text(TextPart {
                    review: None,
                    id: "text-1".to_owned(),
                    ownership: PartOwnership::for_turn("turn-1"),
                    text: "answer".to_owned(),
                    is_complete: true,
                }),
            },
        )
        .expect("assistant text Part");
        core.apply(
            nola_agent_engine::agent::parts::PartUpdate::Created {
                part: AgentPart::Compaction(CompactionPart {
                    id: "maintenance-1:compaction".to_owned(),
                    ownership: PartOwnership::for_turn("maintenance-1"),
                    summary: "summary".to_owned(),
                    retained_item_ids: Vec::new(),
                    source_fingerprint: "fixture-fingerprint".to_owned(),
                    original_token_count: 100,
                    compacted_token_count: 40,
                }),
            },
        )
        .expect("maintenance compaction Part");

        let projection = materialize_conversation_projection_v1(
            stored_conversation(),
            &core,
            &TaskProjection::default(),
            &empty_skill_projection("conversation-1"),
            ContextUsageSnapshot::default(),
            "conversation-1",
            &fixture_ownership("turn-1:assistant", "turn-1", &["text-1"]),
        )
        .expect("maintenance Part must not poison conversation projection");

        assert_eq!(projection.messages.len(), 2);
        assert_eq!(projection.messages[1].parts.len(), 1);
        assert!(matches!(
            projection.messages[1].parts[0],
            MessagePartView::Text { .. }
        ));
    }

    #[test]
    fn folds_failed_attempt_and_fallback_identity_without_trace_replay() {
        let mut core = CoreConversationProjection::default();
        core.apply(
            nola_agent_engine::agent::parts::PartUpdate::Created {
                part: AgentPart::Text(TextPart {
                    review: None,
                    id: "text-1".to_owned(),
                    ownership: PartOwnership::for_turn("turn-1"),
                    text: "answer".to_owned(),
                    is_complete: true,
                }),
            },
        )
        .unwrap();
        let mut projection = materialize_conversation_projection_v1(
            stored_conversation(),
            &core,
            &TaskProjection::default(),
            &empty_skill_projection("conversation-1"),
            ContextUsageSnapshot::default(),
            "conversation-1",
            &fixture_ownership("turn-1:assistant", "turn-1", &["text-1"]),
        )
        .unwrap();
        let record = |id: &str,
                      predecessor_id: Option<&str>,
                      provider: &str,
                      model: &str,
                      failure: Option<nola_domain::ModelAttemptFailure>| {
            ModelAttemptRecord {
                id: id.to_owned(),
                run_id: "run-1".to_owned(),
                invocation_id: "invocation".to_owned(),
                effect_id: "effect".to_owned(),
                text_part_id: format!("{id}:text"),
                reasoning_part_id: format!("{id}:reasoning"),
                predecessor_id: predecessor_id.map(str::to_owned),
                state: if failure.is_some() {
                    ModelAttemptState::Superseded
                } else {
                    ModelAttemptState::Succeeded
                },
                provider: Some(provider.to_owned()),
                model: Some(model.to_owned()),
                failure,
                usage: serde_json::json!({}),
                created_at_ms: 1,
                started_at_ms: Some(1),
                completed_at_ms: Some(2),
                revision: 2,
            }
        };
        let attempts = vec![
            record(
                "attempt-a",
                None,
                "provider-a",
                "model-a",
                Some(nola_domain::ModelAttemptFailure {
                    code: "rate_limited".to_owned(),
                    message: "The provider rate limit was reached.".to_owned(),
                    retryable: true,
                    http_status: None,
                }),
            ),
            record(
                "attempt-b",
                Some("attempt-a"),
                "provider-b",
                "model-b",
                None,
            ),
        ];
        let owners = AssistantOwnerResolver::new(
            "conversation-1",
            Some("conversation-1"),
            &AssistantOwnershipFacts {
            activities: Vec::new(),
                admissions: vec![],
                turns: vec![],
                parts: vec![AssistantPartOwner {
                    part_id: "attempt-a:text".into(),
                    turn_id: None,
                    owner: AssistantOwner {
                        conversation_id: "conversation-1".into(),
                        run_id: "run-1".into(),
                        assistant_message_id: "turn-1:assistant".into(),
                    },
                }],
            },
        )
        .unwrap();
        project_model_attempts(&mut projection, &attempts, &owners).unwrap();

        assert!(
            matches!(projection.messages[1].parts.last(), Some(MessagePartView::ModelAttempt {
            provider: Some(provider), model: Some(model), fallback_provider: Some(fallback_provider),
            fallback_model: Some(fallback_model), failure, ..
        }) if provider == "provider-a" && model == "model-a" && fallback_provider == "provider-b"
            && fallback_model == "model-b" && failure.code == "rate_limited")
        );
    }

    #[test]
    fn semantic_refresh_preserves_history_slot_and_rejects_cross_kind_collision() {
        let task: MessagePartView = serde_json::from_value(serde_json::json!({
            "kind":"task","part_id":"task-part","revision":2,"task_id":"task",
            "display_key":"task","state":"completed","cancel_requested":false
        })).unwrap();
        let historical = |kind: &str| MessagePartView::Historical {
            part_id: MessagePartId("task-part".into()), revision:1,
            projection:Box::new(crate::historical_part_projection::not_loaded_history(
                nola_domain::CanonicalPartOwner::new("conversation".into(),"message".into(),"task-part".into()).unwrap(),1,kind.into())),
        };
        let text = MessagePartView::Text {part_id:MessagePartId("after".into()),revision:1,
            markdown:"after".into(),is_final:true,review:None};
        let mut parts=vec![historical("task"),text];
        upsert_semantic_parts(&mut parts,vec![task.clone()]).unwrap();
        assert!(matches!(&parts[0],MessagePartView::Task{revision:2,..}));
        assert_eq!(canonical_part_id(&parts[1]).0,"after");
        let mut collision=vec![historical("tool")];
        assert!(upsert_semantic_parts(&mut collision,vec![task]).is_err());
        assert!(matches!(&collision[0],MessagePartView::Historical{projection,..} if projection.canonical_kind=="tool"));
    }

    #[test]
    fn several_surfaces_from_one_verified_tool_keep_original_creation_order() {
        use nola_domain::generative_ui::{UiSurfaceIdentity, UiSurfaceOwner, UiSurfaceRevisionRef};
        let text = |id: &str| MessagePartView::Text { part_id:MessagePartId(id.into()),revision:1,
            markdown:id.into(),is_final:true,review:None };
        let tool:MessagePartView=serde_json::from_value(serde_json::json!({
            "kind":"tool","execution_source":{"kind":"model"},"part_id":"tool","revision":1,
            "tool_call_id":"render-call","capability_id":"render_ui","state":"completed","title":"Render",
            "origin":{"kind":"builtin"},"input":{},"artifacts":[],"affected_resources":[]
        })).unwrap();
        let card = |id: &str| MessagePartView::UiSurface {part_id:MessagePartId(id.into()),revision:1,
            surface:UiSurfaceRevisionRef {identity:UiSurfaceIdentity {owner:UiSurfaceOwner {
                authority_id:"authority".into(),conversation_id:"conversation".into(),participant_id:"assistant".into(),
                context_id:"conversation".into(),originating_run_id:"canonical-run".into(),message_id:"message".into(),part_id:"tool".into(),
            },surface_id:id.into(),generation:1},revision:1},summary:id.into()};
        // Deliberately reverse lexical order. Group insertion must keep the
        // already-verified Store ordinal vector rather than sort Surface IDs.
        let merged=weave_stored_part_order(vec![text("before"),tool,text("after"),card("surface-z"),card("surface-a")],
            &["before","tool","surface-z","surface-a","after"]).unwrap();
        assert_eq!(merged.iter().map(|part|canonical_part_id(part).0.as_str()).collect::<Vec<_>>(),
            vec!["before","tool","surface-z","surface-a","after"]);
        assert!(weave_stored_part_order(vec![card("surface")],&[]).is_err());
    }

    #[test]
    fn surface_origin_resolves_canonical_run_independently_of_core_turn() {
        let facts=fixture_ownership("turn-1:assistant","a-distinct-core-turn",&["tool"]);
        let owners=AssistantOwnerResolver::new("conversation-1",Some("conversation-1"),&facts).unwrap();
        assert_eq!(owners.resolve_part("run-1","tool").unwrap().unwrap().assistant_message_id,"turn-1:assistant");
        assert!(owners.resolve_part("a-distinct-core-turn","tool").is_err());
        assert!(owners.resolve_part_with_turn("run-1","tool","a-distinct-core-turn").unwrap().is_some());
        assert!(owners.resolve_part_with_turn("run-1","tool","a-rebound-core-turn").is_err());
        assert!(owners.resolve_part("foreign-run","tool").is_err());
        assert!(owners.resolve_part("run-1","absent").unwrap().is_none());
    }

}
