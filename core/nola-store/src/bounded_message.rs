// SPDX-License-Identifier: Apache-2.0
//! Window metadata and explicit original content_text previews. JSON is resolved
//! by the common historical reader; None here means not hydrated, not absent.
use super::{params, Connection, DeferredTextPart, StoreError, StoreResult, StoredPart};
use crate::canonical_field_reader::CanonicalFieldReader;
use nola_domain::PartBodyField;

fn owner(connection: &Connection, message_id: &str) -> StoreResult<String> {
    connection
        .query_row(
            "SELECT conversation_id FROM messages WHERE id=?1",
            params![message_id],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
}
fn metadata(
    connection: &Connection,
    message_id: &str,
) -> StoreResult<Vec<(String, String, i64, String, i64, i64)>> {
    let mut statement=connection.prepare("SELECT id,kind,ordinal,state,created_at_ms,updated_at_ms FROM message_parts WHERE message_id=?1 ORDER BY ordinal LIMIT 513")?;
    let rows = statement
        .query_map(params![message_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() > 512 {
        return Err(StoreError::Invariant(
            "message has too many Parts for one window".into(),
        ));
    }
    Ok(rows)
}
/// Original byte cost for selection, including both fields without joining a
/// Part twice. Each field validates published ownership and visibility.
pub(super) fn message_bytes(connection: &Connection, message_id: &str) -> StoreResult<usize> {
    let _snapshot = crate::read_snapshot::pin(connection)?;
    let conversation = owner(connection, message_id)?;
    let mut total = 0u64;
    for (part, _, _, _, _, _) in metadata(connection, message_id)? {
        for field in [PartBodyField::ContentText, PartBodyField::ContentJson] {
            let resolved =
                CanonicalFieldReader::open(connection, &conversation, message_id, &part, field)?;
            total = total
                .checked_add(resolved.total_utf8_bytes().unwrap_or(0))
                .ok_or_else(|| {
                    StoreError::Invariant("window original byte size overflow".into())
                })?;
        }
    }
    usize::try_from(total).map_err(|_| {
        StoreError::Invariant("window original byte size exceeds platform range".into())
    })
}
pub(super) fn load_parts(
    connection: &Connection,
    message_id: &str,
    budget: usize,
) -> StoreResult<(Vec<StoredPart>, Vec<DeferredTextPart>)> {
    let _snapshot = crate::read_snapshot::pin(connection)?;
    let conversation = owner(connection, message_id)?;
    let mut remaining = budget;
    let mut parts = Vec::new();
    let mut deferred = Vec::new();
    for (id, kind, ordinal, state, created_at_ms, updated_at_ms) in
        metadata(connection, message_id)?
    {
        let text_source = CanonicalFieldReader::open(
            connection,
            &conversation,
            message_id,
            &id,
            PartBodyField::ContentText,
        )?;
        // Validate even an omitted JSON field: a foreign/prepared/unfinalized
        // reference must not sneak through this metadata-only route.
        let json_source = CanonicalFieldReader::open(
            connection,
            &conversation,
            message_id,
            &id,
            PartBodyField::ContentJson,
        )?;
        let total = text_source.total_utf8_bytes().unwrap_or(0);
        let wanted = total.min(remaining as u64) as usize;
        let mut text = String::with_capacity(wanted);
        let mut offset = 0u64;
        while text.len() < wanted {
            let limit = (wanted - text.len()).clamp(4, 32768);
            let range = text_source.read_range(offset, limit)?;
            let mut take = range.text.len().min(wanted - text.len());
            while !range.text.is_char_boundary(take) {
                take -= 1;
            }
            if take == 0 {
                break;
            }
            text.push_str(&range.text[..take]);
            offset += take as u64;
        }
        remaining -= text.len();
        if total > text.len() as u64 {
            deferred.push(
                DeferredTextPart::new(
                    message_id.into(),
                    id.clone(),
                    total,
                    text.len() as u64,
                    matches!(state.as_str(), "complete" | "completed"),
                    crate::transcript_range::can_read_original(
                        connection,
                        &conversation,
                        message_id,
                        &id,
                    )?,
                )
                .map_err(|error| StoreError::Invariant(error.into()))?,
            );
        }
        parts.push(StoredPart {
            id,
            kind,
            ordinal,
            state,
            content_text: text,
            content_json: None,
            content_json_deferred: json_source.total_utf8_bytes().is_some(),
            content_digest: None,
            created_at_ms,
            updated_at_ms,
        });
    }
    Ok((parts, deferred))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dual_field_fixture() -> crate::NolaStore {
        let store = crate::NolaStore::open(":memory:").unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','title',1,1,1);
            INSERT INTO messages VALUES('m','c',NULL,'user','completed',1,1,1);
            INSERT INTO message_parts VALUES('p','m','text',1,'completed','你好world','{\"text\":\"原文\"}',NULL,1,1)").unwrap();
        crate::canonical_text_disclosure::flush(&store.connection).unwrap();
        store
    }

    #[test]
    fn deferred_finality_is_part_lifecycle_not_message_completion() {
        let store = dual_field_fixture();
        for message_state in ["queued", "completed"] {
            store
                .connection
                .execute(
                    "UPDATE messages SET status=?1 WHERE id='m'",
                    [message_state],
                )
                .unwrap();
            for (part_state, expected) in [
                ("open", false),
                ("streaming", false),
                ("complete", true),
                ("completed", true),
            ] {
                store
                    .connection
                    .execute(
                        "UPDATE message_parts SET state=?1 WHERE id='p'",
                        [part_state],
                    )
                    .unwrap();
                let (_, deferred) = load_parts(&store.connection, "m", 4).unwrap();
                assert_eq!(deferred.len(), 1);
                assert_eq!(
                    deferred[0].is_final, expected,
                    "message={message_state}, part={part_state}"
                );
                assert_eq!(
                    deferred[0].can_read_original,
                    expected && message_state == "completed",
                    "message={message_state}, part={part_state}"
                );
            }
        }
    }

    // Isolated reader fixture, not an assertion that public JSON publication
    // has been enabled. Both references retain exact canonical field ownership.
    fn seal_fields(store: &crate::NolaStore) {
        for (id, field, name, text) in [
            (
                "text-body",
                PartBodyField::ContentText,
                "content_text",
                "你好world",
            ),
            (
                "json-body",
                PartBodyField::ContentJson,
                "content_json",
                "{\"text\":\"原文\"}",
            ),
        ] {
            let reference = nola_domain::CanonicalBodyReference::for_field(
                id.into(),
                "c".into(),
                "m".into(),
                "p".into(),
                field,
                text.len() as u64,
                1,
            )
            .unwrap();
            let encoded = serde_json::to_string(&reference).unwrap();
            store
                .connection
                .execute(
                    "INSERT INTO canonical_text_objects VALUES(?1,?2)",
                    params![id, encoded],
                )
                .unwrap();
            store
                .connection
                .execute(
                    "INSERT INTO canonical_text_chunks VALUES(?1,0,?2)",
                    params![id, text],
                )
                .unwrap();
            store
                .connection
                .execute(
                    "INSERT INTO message_part_body_references VALUES('p',?1,?2)",
                    params![name, encoded],
                )
                .unwrap();
        }
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_text='',content_json=NULL WHERE id='p'",
                [],
            )
            .unwrap();
        crate::canonical_text_disclosure::flush(&store.connection).unwrap();
    }

    #[test]
    fn inline_and_sealed_dual_fields_have_equal_cost_and_bounded_text_only_preview() {
        let store = dual_field_fixture();
        let bytes = message_bytes(&store.connection, "m").unwrap();
        assert_eq!(bytes, "你好world".len() + "{\"text\":\"原文\"}".len());
        let (inline, deferred) = load_parts(&store.connection, "m", 7).unwrap();
        assert_eq!(inline[0].content_text, "你好w");
        assert!(inline[0].content_json.is_none());
        seal_fields(&store);
        assert_eq!(message_bytes(&store.connection, "m").unwrap(), bytes);
        let (sealed, sealed_deferred) = load_parts(&store.connection, "m", 7).unwrap();
        assert_eq!(sealed[0].content_text, inline[0].content_text);
        assert!(sealed[0].content_json.is_none());
        assert_eq!(sealed_deferred, deferred);
    }

    #[test]
    fn omitted_json_still_validates_reference_and_missing_text_chunk_is_not_empty() {
        let store = dual_field_fixture();
        seal_fields(&store);
        store.connection.execute("UPDATE message_part_body_references SET reference_json='{}' WHERE field='content_json'", []).unwrap();
        assert!(message_bytes(&store.connection, "m").is_err());
        assert!(load_parts(&store.connection, "m", 7).is_err());

        let store = dual_field_fixture();
        seal_fields(&store);
        store
            .connection
            .execute(
                "DELETE FROM canonical_text_chunks WHERE content_id='text-body'",
                [],
            )
            .unwrap();
        assert!(load_parts(&store.connection, "m", 7).is_err());
    }

    #[test]
    fn one_load_pairs_utf8_previews_with_original_lengths_after_budget_exhaustion() {
        let store = crate::NolaStore::open(":memory:").unwrap();
        let connection = &store.connection;
        connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','title',1,1,1);INSERT INTO messages VALUES('m','c',NULL,'user','completed',1,1,1)").unwrap();
        for (id, ordinal, text) in [("a", 1, "abcde"), ("b", 2, "\u{e9}\u{e9}"), ("c", 3, "xyz")] {
            connection.execute("INSERT INTO message_parts VALUES(?1,'m','text',?2,'completed',?3,NULL,NULL,1,1)",
                params![id,ordinal,text]).unwrap();
        }
        crate::canonical_text_disclosure::flush(connection).unwrap();
        let (parts, deferred) = load_parts(&connection, "m", 6).unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|p| p.content_text.as_str())
                .collect::<Vec<_>>(),
            vec!["abcde", "", "x"]
        );
        assert_eq!(
            deferred,
            vec![
                DeferredTextPart::new("m".into(), "b".into(), 4, 0, true, true).unwrap(),
                DeferredTextPart::new("m".into(), "c".into(), 3, 1, true, true).unwrap(),
            ]
        );
        let (parts, deferred) = load_parts(&connection, "m", 5).unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|p| p.content_text.as_str())
                .collect::<Vec<_>>(),
            vec!["abcde", "", ""]
        );
        assert_eq!(
            deferred,
            vec![
                DeferredTextPart::new("m".into(), "b".into(), 4, 0, true, true).unwrap(),
                DeferredTextPart::new("m".into(), "c".into(), 3, 0, true, true).unwrap(),
            ]
        );
        let (parts, deferred) = load_parts(&connection, "m", 12).unwrap();
        assert!(deferred.is_empty());
        assert_eq!(
            parts.iter().map(|p| p.content_text.len()).sum::<usize>(),
            12
        );
    }
}
