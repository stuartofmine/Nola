// SPDX-License-Identifier: Apache-2.0
//! Transcript append publication. Payload size depends on new messages only.
use super::*;
use nola_domain::CanonicalBodyReference;
use serde::{Deserialize, Serialize};

const APPEND_PUBLICATION_SCHEMA_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AppendKind {
    FinalizedImport,
    TurnOpened {
        title: String,
    },
    TurnResumed {
        message_id: String,
        previous_run_id: String,
        run_id: String,
    },
}

#[derive(Serialize)]
struct PublicationRef<'a> {
    schema_version: u32,
    kind: AppendKind,
    base_content_revision: i64,
    revision: i64,
    input: &'a AppendMessagesInput,
    initial_assistant_part_mutations: &'a [PartMutation],
    #[serde(skip_serializing_if = "<[CanonicalBodyReference]>::is_empty")]
    references: &'a [CanonicalBodyReference],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    schema_version: u32,
    kind: AppendKind,
    base_content_revision: i64,
    revision: i64,
    input: AppendMessagesInput,
    initial_assistant_part_mutations: Vec<PartMutation>,
    #[serde(default)]
    references: Vec<CanonicalBodyReference>,
}

fn content_marker(connection: &Connection, id: &str) -> StoreResult<Option<(i64, bool)>> {
    connection.query_row(
        "SELECT revision,deleted FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id=?1",
        params![id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(StoreError::from)
}

/// A pending submission can reserve a conversation without publishing any
/// transcript. Its metadata revision is not evidence of a content base.
/// Only an actual first turn may establish that base; missing provenance for
/// existing history must still fail the append gate, never trigger a full
/// historical republish or resurrect a deleted publication.
pub(super) fn needs_initial_turn_publication(
    connection: &Connection,
    conversation_id: &str,
    first_ordinal: i64,
) -> StoreResult<bool> {
    Ok(first_ordinal == 1 && content_marker(connection, conversation_id)?.is_none())
}

fn mark_content(
    connection: &Connection,
    id: &str,
    revision: i64,
    operation: &str,
    at: i64,
) -> StoreResult<()> {
    connection.execute(
        "UPDATE sync_entity_revisions SET revision=?2,operation_id=?3,updated_at_ms=?4
         WHERE entity_type='conversation' AND entity_id=?1 AND deleted=0 AND revision<?2",
        params![id, revision, operation, at],
    )?;
    Ok(())
}

pub(super) fn enqueue(
    connection: &Connection,
    input: &AppendMessagesInput,
    revision: i64,
) -> StoreResult<()> {
    enqueue_kind(connection, input, revision, AppendKind::FinalizedImport)
}

pub(super) fn enqueue_turn(
    connection: &Connection,
    turn: &BeginTurnInput,
    revision: i64,
    first_ordinal: i64,
) -> StoreResult<()> {
    let common = AdmittedReplicaTurnInput {
        user_part_kind: turn.user_part_kind.clone(),
        source_host_id: String::new(), // Publication-only adapter, never Run admission.
        conversation_id: turn.conversation_id.clone(),
        conversation_title: turn.conversation_title.clone(),
        user_message_id: turn.user_message_id.clone(),
        user_part_id: turn.user_part_id.clone(),
        user_text: turn.user_text.clone(),
        user_content_json: turn.user_content_json.clone(),
        assistant_message_id: turn.assistant_message_id.clone(),
        assistant_part_id: turn.assistant_part_id.clone(),
        run_id: turn.run_id.clone(),
        context_checkpoint_id: turn.context_checkpoint_id.clone(),
        created_at_ms: turn.created_at_ms,
    };
    enqueue_turn_content(
        connection,
        &common,
        &turn.additional_user_parts,
        &turn.initial_assistant_part_mutations,
        false,
        revision,
        first_ordinal,
    )
}

pub(super) fn enqueue_remote_turn(
    connection: &Connection,
    turn: &AdmittedReplicaTurnInput,
    revision: i64,
    first_ordinal: i64,
) -> StoreResult<()> {
    enqueue_turn_content(connection, turn, &[], &[], true, revision, first_ordinal)
}

fn enqueue_turn_content(
    connection: &Connection,
    turn: &AdmittedReplicaTurnInput,
    additional_user_parts: &[MessagePartInput],
    initial_assistant_part_mutations: &[PartMutation],
    remote_shell: bool,
    revision: i64,
    first_ordinal: i64,
) -> StoreResult<()> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM messages WHERE conversation_id=?1",
        params![turn.conversation_id],
        |row| row.get(0),
    )?;
    let title: String = connection.query_row(
        "SELECT CASE WHEN length(CAST(title AS BLOB))<=4096 THEN title ELSE NULL END FROM conversations WHERE id=?1",
        params![turn.conversation_id], |row| row.get(0),
    )?;
    let mut user_parts = vec![MessagePartInput {
        id: turn.user_part_id.clone(),
        kind: turn.user_part_kind.clone(),
        ordinal: 1,
        state: "completed".into(),
        content_text: turn.user_text.clone(),
        content_json: turn.user_content_json.clone(),
        content_digest: None,
        created_at_ms: turn.created_at_ms,
        updated_at_ms: turn.created_at_ms,
    }];
    user_parts.extend(additional_user_parts.iter().cloned());
    // A source receipt admits identities, not the client's draft metadata.
    // The authenticated source admission page supplies the actual user Parts.
    if remote_shell {
        user_parts.clear();
    }
    let input = AppendMessagesInput {
        conversation_id: turn.conversation_id.clone(),
        expected_message_count: count
            .checked_sub(2)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| StoreError::Invariant("turn append has no two new messages".into()))?,
        updated_at_ms: turn.created_at_ms,
        messages: vec![
            MessageInput {
                id: turn.user_message_id.clone(),
                run_id: Some(turn.run_id.clone()),
                role: "user".into(),
                status: "completed".into(),
                ordinal: first_ordinal,
                created_at_ms: turn.created_at_ms,
                completed_at_ms: None,
                parts: user_parts,
            },
            MessageInput {
                id: turn.assistant_message_id.clone(),
                run_id: Some(turn.run_id.clone()),
                role: "assistant".into(),
                status: "queued".into(),
                ordinal: first_ordinal + 1,
                created_at_ms: turn.created_at_ms,
                completed_at_ms: None,
                parts: vec![MessagePartInput {
                    id: turn.assistant_part_id.clone(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "open".into(),
                    content_text: String::new(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: turn.created_at_ms,
                    updated_at_ms: turn.created_at_ms,
                }],
            },
        ],
    };
    enqueue_publication(
        connection,
        &input,
        revision,
        AppendKind::TurnOpened { title },
        &[],
        initial_assistant_part_mutations,
    )
}

pub(super) fn enqueue_resume(
    connection: &Connection,
    turn: &BeginInternalTurnInput,
    revision: i64,
    message_id: &str,
    previous_run_id: &str,
) -> StoreResult<()> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM messages WHERE conversation_id=?1",
        params![turn.conversation_id],
        |row| row.get(0),
    )?;
    let input = AppendMessagesInput {
        conversation_id: turn.conversation_id.clone(),
        expected_message_count: usize::try_from(count)
            .map_err(|_| StoreError::Invariant("invalid message count".into()))?,
        messages: Vec::new(),
        updated_at_ms: turn.created_at_ms,
    };
    enqueue_publication(
        connection,
        &input,
        revision,
        AppendKind::TurnResumed {
            message_id: message_id.into(),
            previous_run_id: previous_run_id.into(),
            run_id: turn.run_id.clone(),
        },
        &[],
        &turn.initial_assistant_part_mutations,
    )
}

pub(super) fn enqueue_with_references(
    connection: &Connection,
    input: &AppendMessagesInput,
    revision: i64,
    references: &[CanonicalBodyReference],
) -> StoreResult<()> {
    crate::body_binding::validate_field_bindings(
        &input.conversation_id,
        input.messages.iter().flat_map(|message| {
            message
                .parts
                .iter()
                .map(move |part| (message.id.as_str(), message.status.as_str(), part))
        }),
        references,
    )?;
    enqueue_publication(
        connection,
        input,
        revision,
        AppendKind::FinalizedImport,
        references,
        &[],
    )
}

fn enqueue_kind(
    connection: &Connection,
    input: &AppendMessagesInput,
    revision: i64,
    kind: AppendKind,
) -> StoreResult<()> {
    enqueue_publication(connection, input, revision, kind, &[], &[])
}

fn enqueue_publication(
    connection: &Connection,
    input: &AppendMessagesInput,
    revision: i64,
    kind: AppendKind,
    references: &[CanonicalBodyReference],
    initial_assistant_part_mutations: &[PartMutation],
) -> StoreResult<()> {
    let base = content_marker(connection, &input.conversation_id)?
        .filter(|(base, deleted)| !deleted && *base > 0 && *base < revision)
        .ok_or_else(|| {
            StoreError::Conflict("transcript append requires a published content base".into())
        })?
        .0;
    let payload = serde_json::to_string(&PublicationRef {
        schema_version: APPEND_PUBLICATION_SCHEMA_VERSION,
        kind,
        base_content_revision: base,
        revision,
        input,
        references,
        initial_assistant_part_mutations,
    })?;
    enqueue_sync_change(
        connection,
        "conversation_append",
        &input.conversation_id,
        revision,
        false,
        &payload,
        input.updated_at_ms,
    )?;
    let operation = sync_operation_id(
        "conversation_append",
        &input.conversation_id,
        revision,
        false,
        &payload,
    );
    mark_content(
        connection,
        &input.conversation_id,
        revision,
        &operation,
        input.updated_at_ms,
    )
}

pub(super) fn apply(connection: &Connection, change: &RemoteSyncChangeInput) -> StoreResult<bool> {
    if change.deleted {
        return Err(StoreError::Invariant(
            "append cannot delete a conversation".into(),
        ));
    }
    let publication: Publication = serde_json::from_str(&change.payload_json)?;
    let input = publication.input;
    if publication.schema_version != APPEND_PUBLICATION_SCHEMA_VERSION
        || input.conversation_id != change.entity_id
        || publication.revision != change.revision
        || publication.base_content_revision <= 0
        || publication.base_content_revision >= publication.revision
    {
        return Err(StoreError::Invariant(
            "append identity/revision/base mismatch".into(),
        ));
    }
    if matches!(publication.kind, AppendKind::FinalizedImport)
        && !publication.initial_assistant_part_mutations.is_empty()
    {
        return Err(StoreError::Invariant(
            "finalized append cannot contain live admission mutations".into(),
        ));
    }
    if !publication.references.is_empty()
        && !matches!(publication.kind, AppendKind::FinalizedImport)
    {
        return Err(StoreError::Invariant(
            "only finalized imports can bind immutable fields".into(),
        ));
    }
    crate::body_binding::validate_field_bindings(
        &input.conversation_id,
        input.messages.iter().flat_map(|message| {
            message
                .parts
                .iter()
                .map(move |part| (message.id.as_str(), message.status.as_str(), part))
        }),
        &publication.references,
    )?;
    for reference in &publication.references {
        canonical_text_replica::require_complete(connection, reference)?;
    }
    let current = content_marker(connection, &change.entity_id)?;
    if current.is_some_and(|(_, deleted)| deleted) {
        return Ok(false);
    }
    if current.is_some_and(|(revision, _)| revision > change.revision) {
        return Ok(false);
    }
    if current != Some((publication.base_content_revision, false)) {
        return Err(StoreError::Conflict(format!(
            "append requires content base {}; page not acknowledged",
            publication.base_content_revision
        )));
    }
    let counts: Option<(i64, i64)> = connection.query_row(
        "SELECT COUNT(m.id),COALESCE(MAX(m.ordinal),0) FROM conversations c LEFT JOIN messages m ON m.conversation_id=c.id
         WHERE c.id=?1 GROUP BY c.id", params![change.entity_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    if counts.and_then(|(value, _)| usize::try_from(value).ok())
        != Some(input.expected_message_count)
    {
        return Err(StoreError::Conflict(
            "append message count does not match content base".into(),
        ));
    }
    if let AppendKind::TurnResumed {
        message_id,
        previous_run_id,
        run_id,
    } = &publication.kind
    {
        require_id("resume.message_id", message_id)?;
        require_id("resume.previous_run_id", previous_run_id)?;
        require_id("resume.run_id", run_id)?;
        if !input.messages.is_empty() || run_id == previous_run_id {
            return Err(StoreError::Invariant(
                "resume must change Run ownership without replacing messages".into(),
            ));
        }
        crate::canonical_message_ownership::record_resumption(
            connection,
            &input.conversation_id,
            message_id,
            previous_run_id,
            run_id,
        )?;
        let changed = connection.execute(
            "UPDATE messages SET run_id=?3,status='queued',completed_at_ms=NULL
             WHERE conversation_id=?1 AND id=?2 AND role='assistant' AND run_id=?4",
            params![input.conversation_id, message_id, run_id, previous_run_id],
        )?;
        if changed != 1 {
            return Err(StoreError::Conflict(
                "resume assistant ownership does not match content base".into(),
            ));
        }
        apply_initial_replica_parts(
            connection,
            &input.conversation_id,
            message_id,
            run_id,
            &publication.initial_assistant_part_mutations,
        )?;
        return finish_content(connection, &input, change);
    }
    input
        .expected_message_count
        .checked_add(input.messages.len())
        .and_then(|count| i64::try_from(count).ok())
        .ok_or_else(|| StoreError::Invariant("append exceeds ordinal range".into()))?;
    let turn_title = match &publication.kind {
        AppendKind::TurnResumed { .. } => {
            unreachable!("resumption handled before append validation")
        }
        AppendKind::FinalizedImport => None,
        AppendKind::TurnOpened { title } => {
            if title.len() > 4096 || input.messages.len() != 2 {
                return Err(StoreError::Invariant(
                    "turn append requires a bounded title and two messages".into(),
                ));
            }
            let user = &input.messages[0];
            let assistant = &input.messages[1];
            if user.role != "user"
                || user.status != "completed"
                || user.completed_at_ms.is_some()
                || assistant.role != "assistant"
                || assistant.status != "queued"
                || assistant.completed_at_ms.is_some()
                || user.run_id != assistant.run_id
                || !user
                    .run_id
                    .as_ref()
                    .is_some_and(|run| !run.trim().is_empty())
                || assistant.parts.len() != 1
                || assistant.parts[0].kind != "text"
                || assistant.parts[0].state != "open"
                || !assistant.parts[0].content_text.is_empty()
                || assistant.parts[0].content_json.is_some()
                || assistant.parts[0].content_digest.is_some()
            {
                return Err(StoreError::Invariant(
                    "turn append has invalid message/Run/placeholder ownership".into(),
                ));
            }
            Some(title)
        }
    };
    let first_ordinal = if turn_title.is_some() {
        counts.unwrap().1
    } else {
        input.expected_message_count as i64
    }
    .checked_add(1)
    .ok_or_else(|| StoreError::Invariant("append ordinal overflow".into()))?;
    for (offset, message) in input.messages.iter().enumerate() {
        require_id("append.message.id", &message.id)?;
        if Some(message.ordinal) != first_ordinal.checked_add(offset as i64)
            || message.parts.is_empty()
            || (turn_title.is_none()
                && (message.status != "completed"
                    || !message
                        .completed_at_ms
                        .is_some_and(|at| at >= message.created_at_ms)))
        {
            return Err(StoreError::Invariant(
                "replica append requires contiguous finalized messages".into(),
            ));
        }
        connection.execute(
            "INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms,completed_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![message.id, input.conversation_id, message.run_id, message.role, message.status,
                message.ordinal, message.created_at_ms, message.completed_at_ms],
        )?;
        for (offset, part) in message.parts.iter().enumerate() {
            require_id("append.part.id", &part.id)?;
            let expected_state = if turn_title.is_some() && message.role == "assistant" {
                "open"
            } else {
                "completed"
            };
            if part.ordinal != offset as i64 + 1
                || part.state != expected_state
                || part.updated_at_ms < part.created_at_ms
            {
                return Err(StoreError::Invariant(
                    "replica append requires contiguous finalized parts".into(),
                ));
            }
            if let Some(json) = &part.content_json {
                validate_json("append.part.content_json", json)?;
            }
            connection.execute(
                "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,content_digest,created_at_ms,updated_at_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![part.id, message.id, part.kind, part.ordinal, part.state, part.content_text,
                    part.content_json, part.content_digest, part.created_at_ms, part.updated_at_ms],
            )?;
        }
    }
    for reference in &publication.references {
        connection.execute("INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES(?1,?2,?3)",
            params![reference.part_id(), match reference.field() {
                nola_domain::PartBodyField::ContentText => "content_text",
                nola_domain::PartBodyField::ContentJson => "content_json",
            }, serde_json::to_string(reference)?])?;
    }
    if turn_title.is_some() {
        let assistant = &input.messages[1];
        apply_initial_replica_parts(
            connection,
            &input.conversation_id,
            &assistant.id,
            assistant
                .run_id
                .as_deref()
                .ok_or_else(|| StoreError::Invariant("missing admission Run".into()))?,
            &publication.initial_assistant_part_mutations,
        )?;
    }
    // A delayed content append must not regress a newer metadata revision.
    if let Some(title) = turn_title {
        connection.execute(
            "UPDATE conversations SET title=?2 WHERE id=?1 AND revision<=?3",
            params![input.conversation_id, title, change.revision],
        )?;
    }
    finish_content(connection, &input, change)
}

fn apply_initial_replica_parts(
    connection: &Connection,
    conversation: &str,
    message: &str,
    run: &str,
    mutations: &[PartMutation],
) -> StoreResult<()> {
    for mutation in mutations {
        if let PartMutation::Stream { operation, .. } = mutation {
            let owner = operation.owner();
            if owner.conversation_id != conversation
                || owner.message_id != message
                || owner.run_id != run
            {
                return Err(StoreError::Invariant(
                    "replica initial stream owner differs from admission".into(),
                ));
            }
        }
        apply_part_mutation(connection, message, mutation)?;
    }
    Ok(())
}

fn finish_content(
    connection: &Connection,
    input: &AppendMessagesInput,
    change: &RemoteSyncChangeInput,
) -> StoreResult<bool> {
    connection.execute(
        "UPDATE conversations SET revision=MAX(revision,?2),
         updated_at_ms=CASE WHEN revision<=?2 THEN ?3 ELSE updated_at_ms END WHERE id=?1",
        params![input.conversation_id, change.revision, input.updated_at_ms],
    )?;
    mark_content(
        connection,
        &input.conversation_id,
        change.revision,
        &change.operation_id,
        input.updated_at_ms,
    )?;
    Ok(true)
}
