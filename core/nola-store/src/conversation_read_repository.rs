// SPDX-License-Identifier: Apache-2.0
//! SQLite canonical conversation read repository.
//! Window selection and Part hydration share one implementation; callers must
//! explicitly choose unbounded export, offset paging, or byte-budgeted history.
//! This module neither writes conversation state nor materializes UI models.

use super::{
    bounded_message, params, Connection, DeferredTextPart, OptionalExtension, StoreResult,
    StoredConversation, StoredMessage, StoredPart,
};

#[path = "conversation_json_projection.rs"]
mod json_projection;

const TAIL_MESSAGE_SQL: &str = "SELECT id,run_id,role,status,ordinal,created_at_ms,completed_at_ms
    FROM messages m WHERE conversation_id=?1
    AND COALESCE((SELECT e.visible FROM message_projection_events e
      JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
      WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
      ORDER BY o.resulting_revision DESC LIMIT 1),1)=1
    ORDER BY ordinal DESC LIMIT ?2";

pub(super) fn load_conversation_projection(
    connection: &Connection,
    conversation_id: &str,
) -> StoreResult<Option<StoredConversation>> {
    load_conversation_projection_with_window(
        connection,
        conversation_id,
        ConversationMessageWindow::All,
        None,
    )
    .map(|loaded| loaded.map(|(conversation, _)| conversation))
}

pub(super) fn load_conversation_projection_page(
    connection: &Connection,
    conversation_id: &str,
    offset: usize,
    limit: usize,
) -> StoreResult<Option<StoredConversation>> {
    load_conversation_projection_with_window(
        connection,
        conversation_id,
        ConversationMessageWindow::Offset { offset, limit },
        None,
    )
    .map(|loaded| loaded.map(|(conversation, _)| conversation))
}

pub(super) enum ConversationMessageWindow {
    All,
    Tail { limit: usize },
    Offset {
        offset: usize,
        limit: usize,
    },
    BeforeOrdinal {
        before_ordinal: Option<i64>,
        limit: usize,
        content_byte_limit: usize,
    },
}

pub(super) fn load_conversation_projection_with_window(
    connection: &Connection,
    conversation_id: &str,
    window: ConversationMessageWindow,
    part_limits: Option<(usize, usize)>,
) -> StoreResult<Option<(StoredConversation, Vec<DeferredTextPart>)>> {
    let _snapshot = super::read_snapshot::pin(connection)?;
    let header = connection
        .query_row(
            "SELECT c.id,c.title,c.revision,c.created_at_ms,c.updated_at_ms,s.settings_json,c.configuration_revision
             FROM conversations c LEFT JOIN conversation_settings s ON s.conversation_id=c.id
             WHERE c.id=?1",
            params![conversation_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let Some((id, title, revision, created_at_ms, updated_at_ms, settings_json, configuration_revision)) = header else {
        return Ok(None);
    };
    let base_query = "SELECT id,run_id,role,status,ordinal,created_at_ms,completed_at_ms
         FROM messages m WHERE conversation_id=?1
         AND COALESCE((SELECT e.visible FROM message_projection_events e
           JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
           WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
           ORDER BY o.resulting_revision DESC LIMIT 1),1)=1 ORDER BY ordinal";
    let mut window_byte_limit = None;
    let mut messages = match window {
        ConversationMessageWindow::All => {
            let mut statement = connection.prepare(base_query)?;
            let rows = statement
                .query_map(params![conversation_id], stored_message_row)?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        }
        ConversationMessageWindow::Tail { limit } => {
            // Walk the ordinal index backwards. OFFSET(total - limit) walked
            // every earlier visible message before hydrating the same tail.
            let mut statement = connection.prepare(TAIL_MESSAGE_SQL)?;
            let mut recent = statement.query_map(params![conversation_id, limit as i64], stored_message_row)?
                .collect::<Result<Vec<_>, _>>()?;
            recent.reverse();
            recent
        }
        ConversationMessageWindow::Offset { offset, limit } => {
            let mut statement = connection.prepare(&format!("{base_query} LIMIT ?2 OFFSET ?3"))?;
            let rows = statement
                .query_map(
                    params![conversation_id, limit as i64, offset as i64],
                    stored_message_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        }
        ConversationMessageWindow::BeforeOrdinal {
            before_ordinal,
            limit,
            content_byte_limit,
        } => {
            window_byte_limit = Some(content_byte_limit);
            let mut statement = connection.prepare(
                "SELECT id,run_id,role,status,ordinal,created_at_ms,completed_at_ms
                 FROM messages m
                 WHERE conversation_id = ?1
                   AND (?2 IS NULL OR ordinal < ?2)
                   AND COALESCE((
                     SELECT e.visible FROM message_projection_events e
                     JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
                     WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
                     ORDER BY o.resulting_revision DESC LIMIT 1
                   ),1)=1
                 ORDER BY ordinal DESC LIMIT ?3",
            )?;
            let mut recent = statement
                .query_map(
                    params![conversation_id, before_ordinal, limit as i64],
                    stored_message_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            // Inspect byte sizes before loading any Part payload. Trimming
            // after hydration still allocated every large message in the page.
            let mut retained_bytes = 0usize;
            let mut retained_count = 0usize;
            for message in &recent {
                let bytes = crate::bounded_message::message_bytes(connection, &message.id)?;
                if retained_count > 0 && retained_bytes.saturating_add(bytes) > content_byte_limit {
                    break;
                }
                // Keep its identity so the cursor progresses, but hydrate only
                // a budgeted preview. The window explicitly identifies deferred text.
                retained_bytes = retained_bytes.saturating_add(bytes);
                retained_count += 1;
            }
            recent.truncate(retained_count);
            recent.reverse();
            recent
        }
    };
    // Text tails retain their character contract. JSON hydration is bounded by
    // UTF-8 bytes, including any final review projection; zero means unbounded.
    let (part_text_limit, part_json_limit) = part_limits.unwrap_or((0, 0));
    let json_byte_limit = (part_json_limit > 0).then_some(part_json_limit);
    let mut deferred_text_parts = Vec::new();
    for message in &mut messages {
        if let Some(budget) = window_byte_limit {
            let (parts, deferred) = bounded_message::load_parts(connection, &message.id, budget)?;
            let used = parts
                .iter()
                .map(|part| part.content_text.len())
                .sum::<usize>();
            window_byte_limit = Some(budget.saturating_sub(used));
            message.parts = parts;
            deferred_text_parts.extend(deferred);
            continue;
        }
        let mut parts = connection.prepare(
            "SELECT id,
                    kind,
                    ordinal,
                    state,
                    '',
                    NULL,
                    content_digest,created_at_ms,updated_at_ms
             FROM message_parts WHERE message_id=?1 ORDER BY ordinal",
        )?;
        message.parts = parts
            .query_map(params![message.id], |row| {
                Ok(StoredPart {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    ordinal: row.get(2)?,
                    state: row.get(3)?,
                    content_text: row.get(4)?,
                    content_json: row.get(5)?,
                    content_json_deferred: false,
                    content_digest: row.get(6)?,
                    created_at_ms: row.get(7)?,
                    updated_at_ms: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for part in &mut message.parts {
            let field = crate::canonical_field_reader::CanonicalFieldReader::open(
                connection,
                conversation_id,
                &message.id,
                &part.id,
                nola_domain::PartBodyField::ContentText,
            )?;
            // Both original fields are resolved independently. This consumer no
            // longer needs the text-only gate; other consumers retain theirs.
            part.content_text = field.read_text(part_text_limit)?;
            // A tail is a projection, never the digest-bearing original.
            if part_text_limit > 0 && part.content_text.chars().count() > part_text_limit {
                part.content_digest = None;
            }
            let json = crate::canonical_field_reader::CanonicalFieldReader::open(
                connection,
                conversation_id,
                &message.id,
                &part.id,
                nola_domain::PartBodyField::ContentJson,
            )?;
            part.content_json = match json_projection::hydrate(
                &json,
                &part.kind,
                &part.content_text,
                json_byte_limit,
            )? {
                json_projection::JsonProjection::Absent => None,
                json_projection::JsonProjection::Original(raw) => Some(raw),
                json_projection::JsonProjection::Review(projected) => {
                    part.content_digest = None;
                    Some(projected)
                }
                json_projection::JsonProjection::Deferred => {
                    // Preserve omission evidence through the display DTO so
                    // clients cannot mistake unknown review metadata for none.
                    part.content_json_deferred = true;
                    part.content_digest = None;
                    None
                }
            };
        }
    }
    Ok(Some((
        StoredConversation {
            id,
            title,
            revision,
            configuration_revision,
            created_at_ms,
            updated_at_ms,
            settings_json,
            messages,
        },
        deferred_text_parts,
    )))
}

#[cfg(test)]
#[path = "conversation_read_repository_tests.rs"]
mod tests;

pub(super) fn stored_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    Ok(StoredMessage {
        id: row.get(0)?,
        run_id: row.get(1)?,
        role: row.get(2)?,
        status: row.get(3)?,
        ordinal: row.get(4)?,
        created_at_ms: row.get(5)?,
        completed_at_ms: row.get(6)?,
        parts: Vec::new(),
    })
}

impl super::NolaStore {
    /// Admission reads only the entity revision, never hydrates its transcript.
    /// The Application write unit of work keeps this check and mutation atomic.
    pub fn conversation_revision(&self, conversation_id: &str) -> StoreResult<Option<i64>> {
        super::require_id("conversation_id", conversation_id)?;
        Ok(self
            .connection
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()?)
    }
}
