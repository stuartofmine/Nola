// SPDX-License-Identifier: Apache-2.0
//! Atomic same-owner rewind, including expert histories and compacted context.
use nola_agent_engine::agent::recovery::CheckpointReason;
use nola_agent_engine::agent::{AgentCore, AgentCoreConfig, AgentSubmission, SessionId};
use nola_agent_engine::AgentReducerPort;
use nola_domain::edited_resend::{PrepareInPlaceResend, PreparedInPlaceResend};
use nola_domain::{
    ConversationSourceOwner, MessagePartInput, MessagePartView, MessageRole, MessageState,
};
use nola_store_port::conversation_revert::*;
use nola_store_port::transcript_write::{AppendMessagesInput, MessageInput};
use nola_store_port::transcript_write::{TranscriptEditRepository, TranscriptWriteRepository};

pub fn prepare<S: crate::repositories::ApplicationCommandRepositories>(
    store: &mut S,
    input: PrepareInPlaceResend,
) -> Result<PreparedInPlaceResend, String> {
    uuid::Uuid::parse_str(&input.operation_id)
        .map_err(|_| "resend_requires_stable_operation_uuid")?;
    if input.conversation_id.trim().is_empty()
        || input.message_id.trim().is_empty()
        || input.expected_revision < 1
        || input.expected_message_revision < 1
        || input.created_at_ms < 0
        || input.edited_text.trim().is_empty()
        || input.edited_text.len() > 1_000_000
        || input.edited_text.contains('\0')
    {
        return Err("invalid_in_place_resend".into());
    }
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        // Retry after a successful admission must never rewind its new answer.
        if let Some(previous) = store
            .prepared_in_place_resend(&input.operation_id)
            .map_err(|e| e.to_string())?
        {
            return if previous.input == input {
                Ok(previous)
            } else {
                Err("resend_operation_conflict".into())
            };
        }
        if store
            .conversation_source_owner(&input.conversation_id)
            .map_err(|e| e.to_string())?
            != ConversationSourceOwner::Local
        {
            return Err("resend_requires_local_conversation".into());
        }
        let projection = crate::load_conversation_projection_v1(store, &input.conversation_id)
            .map_err(|e| e.to_string())?;
        let message = projection
            .messages
            .iter()
            .find(|m| m.message_id.0 == input.message_id)
            .ok_or("resend_message_not_visible")?;
        if message.revision != input.expected_message_revision {
            return Err(format!("resend_message_revision_changed: expected {}, current {}",input.expected_message_revision,message.revision));
        }
        if message.role != MessageRole::User || message.state != MessageState::Completed {
            return Err("resend_requires_completed_user_message".into());
        }
        let plain = match message.parts.as_slice() {
            [MessagePartView::Text { is_final: true, .. }] => true,
            // iOS stores independent message metadata beside content_text.
            // Edit the fully disclosed canonical text; never interpret or replay
            // opaque JSON as a Part, attachment, model config or authorization.
            [MessagePartView::Historical { part_id, projection, .. }] => projection.canonical_kind == "text"
                && projection.schema.is_none()
                && projection.owner.conversation_id() == input.conversation_id
                && projection.owner.message_id() == input.message_id
                && projection.owner.part_id() == part_id.0
                && projection.canonical_text.as_ref().is_some_and(|text| text.total_utf8_bytes == text.text.len() as u64),
            _ => false,
        };
        if !plain {
            return Err(format!("resend_requires_plain_text_message: {} parts",message.parts.len()));
        }
        let boundary = store
            .conversation_revert_boundary(&input.conversation_id, &input.message_id, false)
            .map_err(|e| e.to_string())?;
        if boundary.conversation_revision != input.expected_revision {
            return Err("resend_conversation_changed".into());
        }
        let mut experts = Vec::new();
        let mut children = std::collections::BTreeMap::new();
        // Include experts first introduced after the edit point, not just those
        // selected on the edited message. No abandoned child context may leak.
        for message_id in &boundary.hidden_message_ids {
            if let Some(manifest) = store
                .expert_message_manifest(&input.conversation_id, message_id)
                .map_err(|e| e.to_string())?
            {
                for entry in manifest.entries {
                    if message_id == &input.message_id {
                        experts.push(entry.participant.clone());
                    }
                    children
                        .entry(entry.participant.context_conversation_id)
                        .or_insert(entry.child_user_message_id);
                }
            }
        }
        for (index, (conversation, message)) in children.into_iter().enumerate() {
            crate::expert_message::require_context_available(
                store,
                &conversation,
                input.created_at_ms,
            )?;
            rewind(
                store,
                &conversation,
                &message,
                &format!("{}:child:{index}", input.operation_id),
                input.created_at_ms,
            )?;
        }
        let ledger_count = rewind(
            store,
            &input.conversation_id,
            &input.message_id,
            &input.operation_id,
            input.created_at_ms,
        )?;
        let user_message_id = if experts.is_empty() {
            format!("{}:user", input.operation_id)
        } else {
            input.operation_id.clone()
        };
        if !experts.is_empty() {
            store
                .conversation_write_repository()
                .append_messages(&AppendMessagesInput {
                    conversation_id: input.conversation_id.clone(),
                    expected_message_count: ledger_count,
                    messages: vec![MessageInput {
                        id: user_message_id.clone(),
                        run_id: None,
                        role: "user".into(),
                        status: "completed".into(),
                        ordinal: (ledger_count as i64) + 1,
                        created_at_ms: input.created_at_ms,
                        completed_at_ms: Some(input.created_at_ms),
                        parts: vec![MessagePartInput {
                            id: format!("{user_message_id}:text"),
                            kind: "text".into(),
                            ordinal: 1,
                            state: "completed".into(),
                            content_text: input.edited_text.clone(),
                            content_json: None,
                            content_digest: None,
                            created_at_ms: input.created_at_ms,
                            updated_at_ms: input.created_at_ms,
                        }],
                    }],
                    updated_at_ms: input.created_at_ms,
                })
                .map_err(|e| e.to_string())?;
        }
        let prepared = PreparedInPlaceResend {
            run_id: input.operation_id.clone(),
            input,
            user_message_id,
            experts,
        };
        store
            .put_prepared_in_place_resend(&prepared)
            .map_err(|e| e.to_string())?;
        Ok(prepared)
    })
}

fn rewind<S: crate::repositories::ApplicationCommandRepositories>(
    store: &mut S,
    conversation: &str,
    message: &str,
    operation: &str,
    at: i64,
) -> Result<usize, String> {
    if store
        .conversation_source_owner(conversation)
        .map_err(|e| e.to_string())?
        != ConversationSourceOwner::Local
    {
        return Err("resend_requires_local_context".into());
    }
    if store
        .active_run(conversation)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("Wait for the current reply to finish before editing this message.".into());
    }
    let boundary = store
        .conversation_revert_boundary(conversation, message, false)
        .map_err(|e| e.to_string())?;
    let stored = crate::load_current_checkpoint(store, conversation)?;
    if stored.is_none() {
        // Without a checkpoint there is no compacted runtime state to retire.
        // The next ordinary turn restores only the remaining canonical prefix.
        return store
            .conversation_write_repository()
            .apply_conversation_projection(&projection(conversation, operation, at, boundary))
            .map(|result| result.ledger_message_count)
            .map_err(|e| e.to_string());
    }
    let stored = stored.expect("checkpoint presence checked above");
    let expected_revision = stored.revision;
    let checkpoint = crate::decode_idle_checkpoint(&stored.checkpoint_json)?;
    let prior = &checkpoint.state.configuration;
    let config = AgentCoreConfig {
        session_id: Some(SessionId::from_string(conversation.into())),
        model: prior.model.clone(),
        tools: prior.tools.clone(),
        max_model_iterations: prior.max_model_iterations,
        max_strategy_recovery_attempts: prior.max_strategy_recovery_attempts,
        context_window: prior.context_window.clone(),
        memory_policy: prior.memory_policy.clone(),
        turn_identity: prior.turn_identity.clone(),
        ..Default::default()
    };
    let mut core = AgentCore::recover(config.clone(), checkpoint)?;
    // Clear summaries and provider continuation from the abandoned suffix while
    // preserving the checkpoint's monotonic event cursor and kernel identity.
    core.submit_at(
        AgentSubmission::RebaseConversation {
            items: Vec::new(),
            item_ownership: Default::default(),
        },
        crate::decision_time_ms(at)?,
    )?;
    crate::canonical_branch::restore_canonical_context_before(
        store,
        conversation,
        &mut core,
        config.context_window.normalized().prompt_token_limit(),
        Some(boundary.boundary_ordinal),
    )?;
    let checkpoint = core.into_checkpoint(CheckpointReason::Manual);
    let result = store
        .commit_checkpoint_projection(&CommitCheckpointProjectionInput {
            checkpoint: SaveCheckpointInput {
                conversation_id: conversation.into(),
                expected_revision,
                revision: expected_revision + 1,
                event_cursor: checkpoint.last_event_cursor.sequence as i64,
                checkpoint_json: serde_json::to_string(&checkpoint).map_err(|e| e.to_string())?,
                updated_at_ms: at,
            },
            projection: projection(conversation, operation, at, boundary),
        })
        .map_err(|e| e.to_string())?;
    Ok(result.projection.ledger_message_count)
}

fn projection(
    conversation: &str,
    operation: &str,
    at: i64,
    boundary: ConversationRevertBoundary,
) -> ApplyConversationProjectionInput {
    ApplyConversationProjectionInput {
        operation_id: operation.into(),
        conversation_id: conversation.into(),
        expected_revision: boundary.conversation_revision,
        operation_kind: "edit".into(),
        parent_message_id: boundary.parent_message_id,
        mutations: boundary
            .hidden_message_ids
            .into_iter()
            .enumerate()
            .map(|(index, message_id)| MessageProjectionMutation {
                event_id: format!("{operation}:hide:{index}"),
                message_id,
                visible: false,
                reason: "edited_resend".into(),
            })
            .collect(),
        appended_messages: Vec::new(),
        created_at_ms: at,
    }
}
