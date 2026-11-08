// SPDX-License-Identifier: Apache-2.0
//! Bounded conversation, message-window, settings, and checkpoint snapshot queries.

use super::*;

fn field_chunks<'a, 'c: 'a>(
    field: &'a canonical_field_reader::CanonicalFieldReader<'c>,
    start: u64,
    end: u64,
) -> Box<dyn Iterator<Item = Result<(u64, String), String>> + 'a> {
    let mut offset = start;
    Box::new(std::iter::from_fn(move || {
        if offset >= end {
            return None;
        }
        let at = offset;
        match field.read_range(at, (end - at).clamp(4, 32768) as usize) {
            Ok(mut range) if range.next_offset_bytes > at => {
                let length = range.text.len().min((end - at) as usize);
                if !range.text.is_char_boundary(length) {
                    offset = end;
                    return Some(Err("provider token ends inside UTF-8 scalar".into()));
                }
                range.text.truncate(length);
                offset = at + length as u64;
                Some(Ok((at - start, range.text)))
            }
            Ok(_) => {
                offset = end;
                Some(Err(
                    "provider JSON range did not advance within token".into()
                ))
            }
            Err(error) => {
                offset = end;
                Some(Err(error.to_string()))
            }
        }
    }))
}

fn token_has_tool_result(
    field: &canonical_field_reader::CanonicalFieldReader<'_>,
    start: u64,
    end: u64,
) -> StoreResult<bool> {
    use semantic_json_projection::{ExtractedValue, ScalarValue};
    let projection = semantic_json_projection::extract_top_level(
        || field_chunks(field, start, end),
        end - start,
        &["type"],
        1024,
    )?;
    let Some(kind) = projection.fields.first() else {
        return Ok(false);
    };
    if kind.duplicate_known_key {
        return Err(StoreError::Invariant(
            "ambiguous provider content type".into(),
        ));
    }
    Ok(
        matches!(&kind.value, ExtractedValue::Complete(ScalarValue::String(value)) if value == "tool_result"),
    )
}

fn message_has_provider_tool_result(
    connection: &Connection,
    conversation: &str,
    message: &str,
) -> StoreResult<bool> {
    let mut statement = connection
        .prepare("SELECT id FROM message_parts WHERE message_id=?1 AND kind='provider_message'")?;
    let mut rows = statement.query(params![message])?;
    let mut found = false;
    while let Some(row) = rows.next()? {
        let part: String = row.get(0)?;
        found |= part_has_provider_tool_result(connection, conversation, message, &part)?;
    }
    Ok(found)
}

pub(super) fn part_has_provider_tool_result(
    connection: &Connection,
    conversation: &str,
    message: &str,
    part: &str,
) -> StoreResult<bool> {
    use semantic_json_projection::TokenKind;
    let mut found = false;
    let field = canonical_field_reader::CanonicalFieldReader::open(
        connection,
        conversation,
        message,
        &part,
        nola_domain::PartBodyField::ContentJson,
    )?;
    let Some(total) = field.total_utf8_bytes() else {
        return Ok(false);
    };
    let projection = semantic_json_projection::extract_top_level(
        || field_chunks(&field, 0, total),
        total,
        &["content"],
        1024,
    )?;
    let Some(content) = projection.fields.first() else {
        return Ok(false);
    };
    if content.duplicate_known_key {
        return Err(StoreError::Invariant(
            "ambiguous provider content field".into(),
        ));
    }
    let start = content.span.start_bytes;
    let end = content.span.end_bytes;
    if content.span.kind == TokenKind::Object {
        found |= token_has_tool_result(&field, start, end)?;
    }
    let mut visit = |span: semantic_json_projection::TokenSpan| {
        if span.kind == TokenKind::Object {
            found |=
                token_has_tool_result(&field, start + span.start_bytes, start + span.end_bytes)?;
        }
        Ok(())
    };
    match content.span.kind {
        TokenKind::Array => semantic_json_projection::visit_array_item_spans(
            || field_chunks(&field, start, end),
            end - start,
            &mut visit,
        )?,
        TokenKind::Object => semantic_json_projection::visit_object_value_spans(
            || field_chunks(&field, start, end),
            end - start,
            &mut visit,
        )?,
        _ => {}
    }
    Ok(found)
}

#[cfg(test)]
#[path = "conversation_tail_classification_tests.rs"]
mod tail_classification_tests;

impl NolaStore {
    pub fn load_conversation(
        &self,
        conversation_id: &str,
    ) -> StoreResult<Option<StoredConversation>> {
        load_conversation_projection(&self.connection, conversation_id)
    }

    pub fn load_conversation_page(
        &self,
        conversation_id: &str,
        offset: usize,
        limit: usize,
    ) -> StoreResult<Option<StoredConversation>> {
        load_conversation_projection_page(&self.connection, conversation_id, offset, limit)
    }

    pub fn load_conversation_message_window(
        &self,
        conversation_id: &str,
        before_ordinal: Option<i64>,
        limit: usize,
        content_byte_limit: usize,
    ) -> StoreResult<Option<StoredConversationMessageWindow>> {
        let _snapshot = read_snapshot::pin(&self.connection)?;
        if limit == 0 || content_byte_limit == 0 {
            return Err(StoreError::Invariant(
                "conversation message window limits must be positive".to_string(),
            ));
        }
        if before_ordinal.is_some_and(|ordinal| ordinal <= 0) {
            return Err(StoreError::Invariant(
                "conversation message cursor must be positive".to_string(),
            ));
        }
        let total_message_count = self.connection.query_row(
            "SELECT COUNT(*) FROM messages m
             WHERE m.conversation_id = ?1
               AND COALESCE((
                 SELECT e.visible
                 FROM message_projection_events e
                 JOIN conversation_projection_operations o ON o.operation_id = e.operation_id
                 WHERE e.conversation_id = m.conversation_id AND e.message_id = m.id
                 ORDER BY o.resulting_revision DESC LIMIT 1
               ), 1) = 1",
            params![conversation_id],
            |row| row.get::<_, i64>(0),
        )?;
        let Some((conversation, deferred_text_parts)) = load_conversation_projection_with_window(
            &self.connection,
            conversation_id,
            ConversationMessageWindow::BeforeOrdinal {
                before_ordinal,
                limit,
                content_byte_limit,
            },
            None,
        )?
        else {
            return Ok(None);
        };
        let before_ordinal = conversation.messages.first().map(|message| message.ordinal);
        let has_more = if let Some(ordinal) = before_ordinal {
            self.connection.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM messages m
                   WHERE m.conversation_id = ?1 AND m.ordinal < ?2
                     AND COALESCE((
                       SELECT e.visible
                       FROM message_projection_events e
                       JOIN conversation_projection_operations o ON o.operation_id = e.operation_id
                       WHERE e.conversation_id = m.conversation_id AND e.message_id = m.id
                       ORDER BY o.resulting_revision DESC LIMIT 1
                     ), 1) = 1
                 )",
                params![conversation_id, ordinal],
                |row| row.get::<_, bool>(0),
            )?
        } else {
            false
        };
        Ok(Some(StoredConversationMessageWindow {
            assistant_journal_heads: crate::assistant_journal::window_heads(
                &self.connection,
                &conversation,
            )?,
            deferred_text_parts,
            conversation,
            total_message_count,
            before_ordinal,
            has_more,
        }))
    }

    /// Bounded display tail. Text limits are characters; JSON limits are UTF-8
    /// bytes. This DTO is not the original-field input for model history.
    pub fn load_conversation_tail_snapshot(
        &self,
        conversation_id: &str,
        message_limit: usize,
        part_text_limit: usize,
        part_json_limit: usize,
    ) -> StoreResult<Option<StoredConversationTailSnapshot>> {
        let _snapshot = read_snapshot::pin(&self.connection)?;
        if message_limit == 0 || part_text_limit == 0 || part_json_limit == 0 {
            return Err(StoreError::Invariant(
                "conversation tail limits must be positive".to_string(),
            ));
        }
        let mut visible = self.connection.prepare(
            "SELECT m.id,
               CASE WHEN m.role='user' THEN COALESCE((SELECT MAX(CASE
                 WHEN c.generation=d.generation AND d.evaluated_generation=d.generation
                   AND c.message_state=m.status AND c.part_state=p.state
                   AND NOT EXISTS(SELECT 1 FROM canonical_stream_heads h WHERE h.part_id=p.id)
                 THEN c.has_tool_result ELSE 2 END)
                 FROM message_parts p
                 LEFT JOIN canonical_part_disclosures d ON d.part_id=p.id
                 LEFT JOIN canonical_provider_classifications c ON c.part_id=p.id
                 WHERE p.message_id=m.id AND p.kind='provider_message'),0) ELSE 3 END
             FROM messages m
             WHERE m.conversation_id = ?1
               AND COALESCE((
                 SELECT e.visible
                 FROM message_projection_events e
                 JOIN conversation_projection_operations o ON o.operation_id = e.operation_id
                 WHERE e.conversation_id = m.conversation_id AND e.message_id = m.id
                 ORDER BY o.resulting_revision DESC
                 LIMIT 1
               ), 1) = 1",
        )?;
        let mut total_message_count = 0i64;
        let mut total_user_turns = 0i64;
        let mut rows = visible.query(params![conversation_id])?;
        while let Some(row) = rows.next()? {
            total_message_count += 1;
            // 0: user input, 1: provider tool result, 2: needs original-field
            // validation, 3: not a user message. Ordinary reads allocate no
            // historical IDs or role strings and never inspect assistant parts.
            match row.get::<_, i64>(1)? {
                0 => total_user_turns += 1,
                2 => {
                    let message_id: String = row.get(0)?;
                    if !message_has_provider_tool_result(&self.connection, conversation_id, &message_id)? {
                        total_user_turns += 1;
                    }
                }
                _ => {}
            }
        }
        let Some((conversation, _)) = load_conversation_projection_with_window(
            &self.connection,
            conversation_id,
            ConversationMessageWindow::Tail {
                limit: message_limit,
            },
            Some((part_text_limit, part_json_limit)),
        )?
        else {
            return Ok(None);
        };
        Ok(Some(StoredConversationTailSnapshot {
            conversation,
            total_message_count,
            total_user_turns,
        }))
    }

    pub fn load_conversation_settings(&self, conversation_id: &str) -> StoreResult<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT settings_json FROM conversation_settings WHERE conversation_id=?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn load_checkpoint(&self, conversation_id: &str) -> StoreResult<Option<StoredCheckpoint>> {
        Ok(self
            .connection
            .query_row(
                "SELECT conversation_id, revision, event_cursor, checkpoint_json, updated_at_ms
                 FROM core_checkpoints LEFT JOIN core_checkpoint_bodies USING(conversation_id) WHERE conversation_id = ?1",
                params![conversation_id],
                |row| {
                    Ok(StoredCheckpoint {
                        conversation_id: row.get(0)?,
                        revision: row.get(1)?,
                        event_cursor: row.get(2)?,
                        checkpoint_json: row.get(3)?,
                        updated_at_ms: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }
}
