// SPDX-License-Identifier: Apache-2.0
//! Canonical content mutations, excluding runtime checkpoints and Host effects.
use super::*;
use nola_domain::{
    ConversationTransitionReplica as Publication, ReplicaAssistantStatus as AssistantStatus,
    ReplicaMessageDeclaration as MessageDeclaration, ReplicaOwnedPartMutation as OwnedMutation,
};
use nola_kernel::{
    accepts_replica_assistant_owner, evaluate_conversation_transition, ConversationContentDecision,
    ConversationContentState, ReplicaMessageOwner,
};

fn validate_owner(
    connection: &Connection,
    conversation: &str,
    message: &str,
    run: Option<&str>,
) -> StoreResult<()> {
    let stored: Option<(String, String, Option<String>)> = connection
        .query_row(
            "SELECT conversation_id,role,run_id FROM messages WHERE id=?1",
            params![message],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let owner = stored
        .as_ref()
        .map(|(conversation, role, run)| ReplicaMessageOwner {
            conversation_id: conversation,
            role,
            run_id: run.as_deref(),
        });
    if !accepts_replica_assistant_owner(conversation, run, owner.as_ref()) {
        return Err(StoreError::Conflict(
            "replica assistant conversation/Run ownership mismatch".into(),
        ));
    }
    Ok(())
}

fn owner(
    connection: &Connection,
    conversation: &str,
    message: &str,
) -> StoreResult<Option<String>> {
    connection.query_row(
        "SELECT run_id FROM messages WHERE conversation_id=?1 AND id=?2 AND role='assistant'",
        params![conversation, message], |row| row.get(0),
    ).map_err(|error| StoreError::Invariant(format!("replica assistant owner lookup failed: conversation={conversation} message={message}: {error}")))
}

/// Replica intent observed immediately before each canonical write. The caller
/// applies writes in the original order between observations, so later metadata
/// sees earlier Stream/metadata updates, never the final post-transition state.
#[derive(Default)]
pub(super) struct PreparedChanges {
    parts: Vec<OwnedMutation>,
    assistant_status: Option<AssistantStatus>,
}

impl PreparedChanges {
    pub(super) fn record_part_before_apply(
        &mut self,
        connection: &Connection,
        conversation: &str,
        message: &str,
        mutation: &PartMutation,
    ) -> StoreResult<()> {
        let unchanged = match mutation {
            PartMutation::Metadata {
                part_id,
                state,
                content_json,
                updated_at_ms,
            } => {
                // Compare only a bounded metadata value, and return a boolean
                // rather than hydrating a potentially large stored body. A
                // larger value is conservatively retained, even if identical.
                match content_json {
                    Some(json) if json.len() <= 4096 => connection
                        .query_row(
                            "SELECT state=?2 AND updated_at_ms=?3 AND
                            CASE WHEN length(CAST(content_json AS BLOB))<=4096
                                 THEN content_json IS ?4 ELSE 0 END
                         FROM message_parts WHERE id=?1 AND message_id=?5",
                            params![part_id, state, updated_at_ms, json, message],
                            |row| row.get::<_, bool>(0),
                        )
                        .optional()?
                        .unwrap_or(false),
                    // None preserves JSON via COALESCE, regardless of its size.
                    None => connection
                        .query_row(
                            "SELECT state=?2 AND updated_at_ms=?3
                         FROM message_parts WHERE id=?1 AND message_id=?4",
                            params![part_id, state, updated_at_ms, message],
                            |row| row.get::<_, bool>(0),
                        )
                        .optional()?
                        .unwrap_or(false),
                    Some(_) => false,
                }
            }
            // Stream operations are exact head CAS facts, not mergeable text.
            // All other mutations retain their existing publication semantics.
            _ => false,
        };
        if !unchanged {
            self.parts.push(OwnedMutation {
                message_id: message.into(),
                run_id: owner(connection, conversation, message)?,
                mutation: mutation.clone(),
            });
        }
        Ok(())
    }

    pub(super) fn record_status_before_apply(
        &mut self,
        connection: &Connection,
        conversation: &str,
        message: &str,
        status: &str,
        at: i64,
    ) -> StoreResult<()> {
        let unchanged = connection
            .query_row(
                "SELECT status=?3 AND (?4=0 OR completed_at_ms IS ?5)
             FROM messages WHERE conversation_id=?1 AND id=?2 AND role='assistant'",
                params![conversation, message, status, is_message_terminal(status), at],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false);
        if !unchanged {
            self.assistant_status = Some(AssistantStatus {
                message_id: message.into(),
                run_id: owner(connection, conversation, message)?,
                status: status.into(),
            });
        }
        Ok(())
    }
}

/// Capture insertion intent before canonical mutation. Existing seeds are
/// validated by ensure_transition_message, but are not content changes and must
/// not be copied into the replica on every delta.
pub(super) fn prepare_messages(
    connection: &Connection,
    conversation: &str,
    inputs: &[TransitionMessageInput],
) -> StoreResult<Vec<MessageDeclaration>> {
    let mut declarations = Vec::new();
    for message in inputs {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND conversation_id=?2)",
            params![message.id, conversation],
            |row| row.get(0),
        )?;
        let mut initial_parts = Vec::new();
        for part in &message.initial_parts {
            let part_exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM message_parts WHERE id=?1)",
                params![part.id],
                |row| row.get(0),
            )?;
            if !part_exists {
                initial_parts.push(part.clone());
            }
        }
        if !exists || !initial_parts.is_empty() {
            declarations.push(MessageDeclaration {
                execution_ownership: crate::conversation_history_port::source_execution_ownership(connection, conversation)?,
                ordinal: 0,
                input: TransitionMessageInput {
                    id: message.id.clone(),
                    run_id: message.run_id.clone(),
                    role: message.role.clone(),
                    status: message.status.clone(),
                    created_at_ms: message.created_at_ms,
                    completed_at_ms: message.completed_at_ms,
                    initial_parts,
                },
            });
        }
    }
    Ok(declarations)
}

pub(super) fn enqueue(
    connection: &Connection,
    conversation: &str,
    revision: i64,
    input: &CommitTransitionInput,
    mut messages: Vec<MessageDeclaration>,
    changes: PreparedChanges,
) -> StoreResult<()> {
    if messages.is_empty() && changes.parts.is_empty() && changes.assistant_status.is_none() {
        // Runtime/checkpoint revisions may still advance locally. Do not mark a
        // new published content base without a fact: the next actual change
        // references the last sync_entity_revisions base, across any such gap.
        return Ok(());
    }
    let base: i64 = connection.query_row(
        "SELECT revision FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id=?1 AND deleted=0",
        params![conversation], |row| row.get(0),
    )?;
    if base <= 0 || base >= revision {
        return Err(StoreError::Conflict(
            "content transition has no prior published base".into(),
        ));
    }
    for message in &mut messages {
        message.ordinal = connection.query_row(
            "SELECT ordinal FROM messages WHERE conversation_id=?1 AND id=?2",
            params![conversation, message.input.id],
            |row| row.get(0),
        )?;
    }
    let publication = Publication {
        conversation_id: conversation.into(),
        base_content_revision: base,
        revision,
        updated_at_ms: input.committed_at_ms,
        messages,
        parts: changes.parts,
        assistant_status: changes.assistant_status,
    };
    enqueue_publication(connection, &publication)
}

fn enqueue_publication(connection: &Connection, publication: &Publication) -> StoreResult<()> {
    let conversation = &publication.conversation_id;
    let revision = publication.revision;
    let payload = serde_json::to_string(&publication)?;
    enqueue_sync_change(
        connection,
        "conversation_transition",
        conversation,
        revision,
        false,
        &payload,
        publication.updated_at_ms,
    )?;
    mark(
        connection,
        conversation,
        revision,
        &sync_operation_id(
            "conversation_transition",
            conversation,
            revision,
            false,
            &payload,
        ),
        publication.updated_at_ms,
    )
}

/// Publish only newly materialized successor messages. A NextTurn input may
/// already be visible; it must not be appended again or assigned a new owner.
pub(super) fn enqueue_admitted_messages(
    connection: &Connection,
    conversation: &str,
    revision: i64,
    message_ids: &[String],
    at_ms: i64,
) -> StoreResult<()> {
    let base: i64 = connection.query_row(
        "SELECT revision FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id=?1 AND deleted=0",
        [conversation], |row| row.get(0))?;
    if base<=0 || base>=revision { return Err(StoreError::Conflict("successor content has no prior published base".into())); }
    let mut messages=Vec::new();
    for id in message_ids {
        let execution_ownership = crate::conversation_history_port::source_execution_ownership(connection, conversation)?;
        let mut declaration=connection.query_row(
            "SELECT ordinal,run_id,role,status,created_at_ms,completed_at_ms FROM messages WHERE id=?1 AND conversation_id=?2",
            params![id,conversation], |row| Ok(MessageDeclaration { execution_ownership: execution_ownership.clone(), ordinal:row.get(0)?, input:TransitionMessageInput {
                id:id.clone(),run_id:row.get(1)?,role:row.get(2)?,status:row.get(3)?,
                created_at_ms:row.get(4)?,completed_at_ms:row.get(5)?,initial_parts:Vec::new() }}))?;
        let mut query=connection.prepare("SELECT id,kind,ordinal,state,content_text,content_json,content_digest,created_at_ms,updated_at_ms FROM message_parts WHERE message_id=?1 ORDER BY ordinal")?;
        declaration.input.initial_parts=query.query_map([id],|row|Ok(MessagePartInput {
            id:row.get(0)?,kind:row.get(1)?,ordinal:row.get(2)?,state:row.get(3)?,content_text:row.get(4)?,
            content_json:row.get(5)?,content_digest:row.get(6)?,created_at_ms:row.get(7)?,updated_at_ms:row.get(8)?
        }))?.collect::<Result<Vec<_>,_>>()?;
        messages.push(declaration);
    }
    enqueue_publication(connection,&Publication { conversation_id:conversation.into(),base_content_revision:base,
        revision,updated_at_ms:at_ms,messages,parts:Vec::new(),assistant_status:None })
}

fn mark(
    connection: &Connection,
    conversation: &str,
    revision: i64,
    operation: &str,
    at: i64,
) -> StoreResult<()> {
    connection.execute(
        "UPDATE sync_entity_revisions SET revision=?2,operation_id=?3,updated_at_ms=?4
        WHERE entity_type='conversation' AND entity_id=?1 AND deleted=0 AND revision<?2",
        params![conversation, revision, operation, at],
    )?;
    Ok(())
}

pub(super) fn apply(connection: &Connection, change: &RemoteSyncChangeInput) -> StoreResult<bool> {
    let value: Publication = serde_json::from_str(&change.payload_json)?;
    let current: Option<(i64, bool)> = connection.query_row(
        "SELECT revision,deleted FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id=?1",
        params![change.entity_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let state = ConversationContentState {
        conversation_id: &change.entity_id,
        content_revision: current
            .filter(|(_, deleted)| !deleted)
            .map(|(revision, _)| revision),
        tombstone_revision: current
            .filter(|(_, deleted)| *deleted)
            .map(|(revision, _)| revision),
    };
    match evaluate_conversation_transition(&state, &value, change.revision, change.deleted) {
        ConversationContentDecision::Apply => {}
        ConversationContentDecision::Stale | ConversationContentDecision::Deleted => {
            return Ok(false)
        }
        decision => {
            return Err(StoreError::Conflict(format!(
                "content transition rejected: {decision:?}; requires its exact published base"
            )))
        }
    }
    for declaration in &value.messages {
        ensure_transition_message(connection, &value.conversation_id, &declaration.input)?;
        let ordinal: i64 = connection.query_row(
            "SELECT ordinal FROM messages WHERE conversation_id=?1 AND id=?2",
            params![value.conversation_id, declaration.input.id],
            |row| row.get(0),
        )?;
        if ordinal != declaration.ordinal {
            return Err(StoreError::Conflict(
                "replica message ordinal mismatch".into(),
            ));
        }
    }
    for part in &value.parts {
        validate_owner(
            connection,
            &value.conversation_id,
            &part.message_id,
            part.run_id.as_deref(),
        )?;
        apply_part_mutation(connection, &part.message_id, &part.mutation)?;
    }
    if let Some(status) = &value.assistant_status {
        validate_owner(
            connection,
            &value.conversation_id,
            &status.message_id,
            status.run_id.as_deref(),
        )?;
        connection.execute("UPDATE messages SET status=?2,completed_at_ms=CASE WHEN ?3 THEN ?4 ELSE completed_at_ms END WHERE id=?1",
            params![status.message_id, status.status, is_message_terminal(&status.status), value.updated_at_ms])?;
    }
    connection.execute(
        "UPDATE conversations SET revision=MAX(revision,?2),
        updated_at_ms=CASE WHEN revision<=?2 THEN ?3 ELSE updated_at_ms END WHERE id=?1",
        params![value.conversation_id, value.revision, value.updated_at_ms],
    )?;
    mark(
        connection,
        &value.conversation_id,
        value.revision,
        &change.operation_id,
        value.updated_at_ms,
    )?;
    Ok(true)
}

#[cfg(test)]
#[path = "conversation_transition_sync_tests.rs"]
mod tests;
