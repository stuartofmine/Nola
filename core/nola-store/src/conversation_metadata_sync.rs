// SPDX-License-Identifier: Apache-2.0
//! Atomic persistence adapter for the Kernel's metadata admission decision.
//! The caller owns the page transaction and records its cursor only on success.
use super::*;
use nola_domain::ConversationMetadataReplica;
use nola_kernel::{
    evaluate_conversation_metadata, ConversationMetadataDecision, ConversationMetadataState,
};

struct Header {
    revision: i64,
    title: String,
    settings: Option<String>,
    updated_at_ms: i64,
}

fn header(connection: &Connection, id: &str) -> StoreResult<Option<Header>> {
    // Reject oversized legacy/local fields before bringing them into Rust. Do
    // not consult Message/Part rows to apply or compare metadata.
    connection.query_row(
        "SELECT c.revision,
            CASE WHEN length(CAST(c.title AS BLOB))<=4096 THEN c.title ELSE NULL END,
            s.settings_json IS NOT NULL,
            CASE WHEN length(CAST(s.settings_json AS BLOB))<=65536 THEN s.settings_json ELSE NULL END,
            c.updated_at_ms
         FROM conversations c LEFT JOIN conversation_settings s ON s.conversation_id=c.id
         WHERE c.id=?1",
        params![id], |row| {
            let has_settings: bool = row.get(2)?;
            let settings: Option<String> = row.get(3)?;
            if has_settings && settings.is_none() {
                return Err(rusqlite::Error::InvalidColumnType(3, "settings_json".into(), rusqlite::types::Type::Null));
            }
            Ok(Header { revision: row.get(0)?, title: row.get(1)?, settings,
                updated_at_ms: row.get(4)? })
        },
    ).optional().map_err(StoreError::from)
}

pub(super) fn apply(connection: &Connection, change: &RemoteSyncChangeInput) -> StoreResult<bool> {
    if change.deleted {
        return Err(StoreError::Invariant(
            "metadata cannot delete a conversation; use its canonical tombstone".into(),
        ));
    }
    let incoming: ConversationMetadataReplica = serde_json::from_str(&change.payload_json)?;
    if incoming.conversation_id() != change.entity_id
        || incoming.metadata_revision() != change.revision as u64
    {
        return Err(StoreError::Invariant(
            "remote conversation metadata identity/revision mismatch".into(),
        ));
    }
    let base: Option<(i64, bool)> = connection.query_row(
        "SELECT revision,deleted FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id=?1",
        params![change.entity_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let current = header(connection, &change.entity_id)?;
    let content_revision = base
        .filter(|(_, deleted)| !deleted && current.is_some())
        .map(|(revision, _)| revision as u64);
    let previous = match (&current, content_revision) {
        (Some(header), Some(base)) => Some(
            ConversationMetadataReplica::new(
                change.entity_id.clone(),
                header.revision as u64,
                base.min(header.revision as u64),
                header.title.clone(),
                header.settings.clone(),
                header.updated_at_ms,
            )
            .map_err(StoreError::Invariant)?,
        ),
        _ => None,
    };
    let state = ConversationMetadataState {
        conversation_id: &change.entity_id,
        content_revision,
        metadata: previous.as_ref(),
        tombstone_revision: base
            .filter(|(_, deleted)| *deleted)
            .map(|(revision, _)| revision as u64),
    };
    match evaluate_conversation_metadata(&state, &incoming) {
        ConversationMetadataDecision::Apply => {}
        ConversationMetadataDecision::Replay
        | ConversationMetadataDecision::Stale
        | ConversationMetadataDecision::Deleted => return Ok(false),
        ConversationMetadataDecision::MissingContentBase { required_revision } => {
            return Err(StoreError::Conflict(format!(
                "conversation {} requires content base {required_revision}; sync page not acknowledged", change.entity_id,
            )));
        }
        decision => {
            return Err(StoreError::Conflict(format!(
                "conversation metadata rejected: {decision:?}"
            )))
        }
    }
    let updated = connection.execute(
        "UPDATE conversations SET title=?2,revision=?3,updated_at_ms=?4 WHERE id=?1",
        params![
            change.entity_id,
            incoming.title(),
            change.revision,
            incoming.updated_at_ms()
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::Invariant(
            "metadata content base disappeared".into(),
        ));
    }
    if let Some(settings) = incoming.settings_json() {
        connection.execute(
            "INSERT INTO conversation_settings(conversation_id,settings_json,updated_at_ms) VALUES(?1,?2,?3)
             ON CONFLICT(conversation_id) DO UPDATE SET settings_json=excluded.settings_json,updated_at_ms=excluded.updated_at_ms",
            params![change.entity_id, settings, incoming.updated_at_ms()],
        )?;
    } else {
        connection.execute(
            "DELETE FROM conversation_settings WHERE conversation_id=?1",
            params![change.entity_id],
        )?;
    }
    Ok(true)
}

pub(super) fn enqueue(connection: &Connection, id: &str) -> StoreResult<()> {
    let current = header(connection, id)?.ok_or_else(|| {
        StoreError::Invariant("metadata publication has no canonical conversation".into())
    })?;
    let base: Option<i64> = connection
        .query_row(
            "SELECT revision FROM sync_entity_revisions
         WHERE entity_type='conversation' AND entity_id=?1 AND deleted=0",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let base = base.ok_or_else(|| {
        StoreError::Conflict(format!(
            "conversation {id} has no published content base; metadata mutation was not committed",
        ))
    })?;
    let metadata = ConversationMetadataReplica::new(
        id.into(),
        current.revision as u64,
        base as u64,
        current.title,
        current.settings,
        current.updated_at_ms,
    )
    .map_err(StoreError::Invariant)?;
    let payload = serde_json::to_string(&metadata)?;
    enqueue_sync_change(
        connection,
        "conversation_metadata",
        id,
        current.revision,
        false,
        &payload,
        current.updated_at_ms,
    )
}

pub(super) fn preserve_newer_header(
    connection: &Connection,
    incoming: &mut StoredConversation,
) -> StoreResult<()> {
    let newer: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_entity_revisions
         WHERE entity_type='conversation_metadata' AND entity_id=?1 AND revision>?2 AND deleted=0)",
        params![incoming.id, incoming.revision],
        |row| row.get(0),
    )?;
    if newer {
        let current = header(connection, &incoming.id)?.ok_or_else(|| {
            StoreError::Invariant("metadata revision has no canonical conversation".into())
        })?;
        incoming.title = current.title;
        incoming.settings_json = current.settings;
        incoming.revision = current.revision;
        incoming.updated_at_ms = current.updated_at_ms;
    }
    Ok(())
}
