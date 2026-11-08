// SPDX-License-Identifier: Apache-2.0
//! Bounded reads of original canonical text, never summaries or display caches.
use super::{params, NolaStore, OptionalExtension, StoreError, StoreResult};
pub use nola_store_port::StoredTranscriptRange;

/// The same eligibility fact supplies both bounded windows and the original
/// range gate. Finality alone never grants publication or historical ownership.
/// Callers resolving a body must still validate its exact physical backing.
pub(super) fn can_read_original(
    connection: &super::Connection,
    conversation_id: &str,
    message_id: &str,
    part_id: &str,
) -> StoreResult<bool> {
    if [conversation_id, message_id, part_id]
        .iter()
        .any(|id| id.trim().is_empty() || id.len() > 128 || id.chars().any(char::is_control))
    {
        return Ok(false);
    }
    let message_status: Option<String> = connection
        .query_row(
            "SELECT m.status FROM message_parts p JOIN messages m ON m.id=p.message_id
         WHERE m.conversation_id=?1 AND m.id=?2 AND p.id=?3 AND p.kind='text'
           AND p.state IN ('complete','completed')
           AND COALESCE((SELECT e.visible FROM message_projection_events e
               JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
               WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
               ORDER BY o.resulting_revision DESC LIMIT 1),1)=1",
            params![conversation_id, message_id, part_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(message_status) = message_status else {
        return Ok(false);
    };
    let lifecycle = message_status == "completed"
        || crate::canonical_message_ownership::has_trusted_resumption(
            connection,
            conversation_id,
            message_id,
        )?;
    Ok(lifecycle
        && crate::canonical_text_disclosure::allows_product_text(
            connection,
            conversation_id,
            message_id,
            part_id,
        )?)
}

impl NolaStore {
    pub fn read_conversation_text_range(
        &self,
        conversation_id: &str,
        message_id: &str,
        part_id: &str,
        offset_bytes: u64,
        limit_bytes: usize,
    ) -> StoreResult<StoredTranscriptRange> {
        for id in [conversation_id, message_id, part_id] {
            if id.trim().is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
                return Err(invalid("invalid transcript identity"));
            }
        }
        // Four bytes guarantee progress for any scalar at a valid boundary.
        if !(4..=32 * 1024).contains(&limit_bytes) {
            return Err(invalid(
                "text range limit must be between 4 and 32768 bytes",
            ));
        }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        if !can_read_original(&self.connection, conversation_id, message_id, part_id)? {
            return Err(invalid(
                "canonical text is unavailable for this owner or lifecycle",
            ));
        }
        let field = crate::canonical_field_reader::CanonicalFieldReader::open(
            &self.connection,
            conversation_id,
            message_id,
            part_id,
            nola_domain::PartBodyField::ContentText,
        )?;
        let total = field
            .total_utf8_bytes()
            .ok_or_else(|| invalid("canonical text is absent"))?;
        let range = field.read_range(offset_bytes, limit_bytes)?;
        Ok(StoredTranscriptRange {
            conversation_id: conversation_id.into(),
            message_id: message_id.into(),
            part_id: part_id.into(),
            offset_bytes,
            next_offset_bytes: range.next_offset_bytes,
            total_utf8_bytes: total,
            text: range.text,
            end_of_content: range.end_of_content,
        })
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}

#[cfg(test)]
#[path = "transcript_range_resumption_tests.rs"]
mod resumption_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_range_and_search_read_the_text_field_beside_an_independent_json_reference() {
        use nola_domain::{CanonicalBodyReference, PartBodyField};
        use nola_store_port::canonical_body::CanonicalBodyRepository;
        let text = "前needle🙂suffix";
        let json = r#"{"text":"json-only","large_integer":9007199254740993}"#;
        let mut store = store(text);
        for (id, field, raw) in [
            ("text-body", PartBodyField::ContentText, text),
            ("json-body", PartBodyField::ContentJson, json),
        ] {
            let reference = CanonicalBodyReference::for_field(
                id.into(),
                "c".into(),
                "m".into(),
                "p".into(),
                field,
                raw.len() as u64,
                1,
            )
            .unwrap();
            store
                .prepare_body(
                    &reference,
                    &mut std::iter::once(Ok((0, raw.to_owned()))),
                    10000,
                )
                .unwrap();
            store
                .connection
                .execute(
                    "INSERT INTO message_part_body_references VALUES('p',?1,?2)",
                    params![
                        match field {
                            PartBodyField::ContentText => "content_text",
                            PartBodyField::ContentJson => "content_json",
                        },
                        serde_json::to_string(&reference).unwrap()
                    ],
                )
                .unwrap();
        }
        store
            .connection
            .execute("UPDATE message_parts SET content_text='' WHERE id='p'", [])
            .unwrap();
        let mut restored = String::new();
        crate::canonical_text_disclosure::flush(&store.connection).unwrap();
        let mut offset = 0;
        loop {
            let range = store
                .read_conversation_text_range("c", "m", "p", offset, 4)
                .unwrap();
            restored.push_str(&range.text);
            offset = range.next_offset_bytes;
            if range.end_of_content {
                break;
            }
        }
        assert_eq!(restored, text);
        let hits = store.search_transcript_text("c", "needle", 1).unwrap();
        assert_eq!(hits.len(), 1);
        // Search offsets identify the excerpt's start, including context.
        assert_eq!(hits[0].start_character, 0);
        assert!(hits[0].snippet.contains("前needle"));
        assert!(store
            .search_transcript_text("c", "json-only", 1)
            .unwrap()
            .is_empty());
        let conversation = store.load_conversation("c").unwrap().unwrap();
        assert_eq!(
            conversation.messages[0].parts[0].content_json.as_deref(),
            Some(json)
        );
    }
    fn store(text: &str) -> NolaStore {
        let store = NolaStore::open(":memory:").unwrap();
        store
            .connection
            .execute_batch(
                "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','History',1,1,1);
            INSERT INTO messages VALUES('m','c',NULL,'user','completed',1,1,1);",
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO message_parts
            (id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
            VALUES('p','m','text',1,'completed',?1,1,1)",
                params![text],
            )
            .unwrap();
        crate::canonical_text_disclosure::flush(&store.connection).unwrap();
        store
    }
    #[test]
    fn unicode_ranges_make_lossless_bounded_progress() {
        let original = "ab中😀z";
        let store = store(original);
        let mut offset = 0;
        let mut restored = String::new();
        loop {
            let range = store
                .read_conversation_text_range("c", "m", "p", offset, 4)
                .unwrap();
            assert!(range.text.len() <= 4);
            assert_eq!(range.total_utf8_bytes, original.len() as u64);
            assert!(range.next_offset_bytes > offset);
            restored.push_str(&range.text);
            offset = range.next_offset_bytes;
            if range.end_of_content {
                break;
            }
        }
        assert_eq!(restored, original);
        let end = store
            .read_conversation_text_range("c", "m", "p", offset, 4)
            .unwrap();
        assert!(end.text.is_empty() && end.end_of_content);
        for bad in [3, 6, u64::MAX, 11] {
            assert!(store
                .read_conversation_text_range("c", "m", "p", bad, 4)
                .is_err());
        }
        for bad in [0, 3, 32769] {
            assert!(store
                .read_conversation_text_range("c", "m", "p", 0, bad)
                .is_err());
        }
    }
    #[test]
    fn ranges_enforce_owner_and_original_completed_text() {
        let store = store("evidence");
        for (c, m, p) in [
            ("other", "m", "p"),
            ("c", "other", "p"),
            ("c", "m", "other"),
        ] {
            assert!(store.read_conversation_text_range(c, m, p, 0, 32).is_err());
        }
        store
            .connection
            .execute("UPDATE message_parts SET kind='reasoning'", [])
            .unwrap();
        assert!(store
            .read_conversation_text_range("c", "m", "p", 0, 32)
            .is_err());
        store
            .connection
            .execute("UPDATE message_parts SET kind='text',state='streaming'", [])
            .unwrap();
        assert!(store
            .read_conversation_text_range("c", "m", "p", 0, 32)
            .is_err());
    }
    #[test]
    fn ten_million_byte_tail_does_not_expand_the_response() {
        let store = store(&("x".repeat(10_000_000 - 9) + "TAIL-DONE"));
        let range = store
            .read_conversation_text_range("c", "m", "p", 9_999_991, 32)
            .unwrap();
        assert_eq!(range.text, "TAIL-DONE");
        assert!(range.end_of_content);
        assert_eq!(range.total_utf8_bytes, 10_000_000);
        assert!(serde_json::to_vec(&range).unwrap().len() < 1024);
    }
}
