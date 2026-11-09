// SPDX-License-Identifier: Apache-2.0
//! Resolve immutable text only through a committed canonical Part and its owner.
//! Prepared objects are never discovered by content ID or client owner strings.
use crate::canonical_text_store::{read_range_from_connection, validate_reference_from_connection};
use crate::{params, Connection, OptionalExtension, StoreError, StoreResult};

/// Logical original field state, independent of its physical inline slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResolvedPartField {
    Absent,
    Inline { total_utf8_bytes: u64 },
    Referenced(nola_domain::CanonicalBodyReference),
    Chunked(nola_domain::CanonicalStreamHead),
}
impl ResolvedPartField {
    pub(super) fn total_utf8_bytes(&self) -> Option<u64> {
        match self {
            Self::Absent => None,
            Self::Inline { total_utf8_bytes } => Some(*total_utf8_bytes),
            Self::Referenced(reference) => Some(reference.total_utf8_bytes()),
            Self::Chunked(head) => Some(head.total_utf8_bytes),
        }
    }
}

fn field_name(field: nola_domain::PartBodyField) -> &'static str {
    match field {
        nola_domain::PartBodyField::ContentText => "content_text",
        nola_domain::PartBodyField::ContentJson => "content_json",
    }
}

/// Resolve only a visible Part belonging to the complete canonical owner.
/// Inline values retain their existing state semantics; immutable references
/// additionally require finalized Part state and a finalized Message or trusted
/// internal continuation. Continuation registration validates existing sealed
/// bindings before changing Message ownership; a queued Message alone is never
/// publication authority. SQL NULL is not absence
/// when the corresponding field has a committed immutable reference.
pub(super) fn resolve_field(
    connection: &Connection,
    conversation_id: &str,
    message_id: &str,
    part_id: &str,
    field: nola_domain::PartBodyField,
) -> StoreResult<ResolvedPartField> {
    resolve_field_with_lifecycle(
        connection,
        conversation_id,
        message_id,
        part_id,
        field,
        true,
    )
}

/// Write-side metadata derivation is not a publication grant. It must work
/// before finality and while a message is hidden; product reads retain both
/// gates. Exact physical owner/backing validation is identical.
pub(super) fn resolve_disclosure_field(
    connection: &Connection,
    conversation_id: &str,
    message_id: &str,
    part_id: &str,
) -> StoreResult<ResolvedPartField> {
    resolve_field_with_lifecycle(
        connection,
        conversation_id,
        message_id,
        part_id,
        nola_domain::PartBodyField::ContentJson,
        false,
    )
}

fn resolve_field_with_lifecycle(
    connection: &Connection,
    conversation_id: &str,
    message_id: &str,
    part_id: &str,
    field: nola_domain::PartBodyField,
    require_lifecycle: bool,
) -> StoreResult<ResolvedPartField> {
    let _snapshot = crate::read_snapshot::pin(connection)?;
    let name = field_name(field);
    let row: Option<(String, String, Option<u64>, Option<String>, Option<String>)> = connection
        .query_row(
            &format!(
                "SELECT m.status,p.state,octet_length(p.{name}),r.part_id,
            CASE WHEN length(CAST(r.reference_json AS BLOB))<=4096 THEN r.reference_json END
         FROM message_parts p JOIN messages m ON m.id=p.message_id
         LEFT JOIN message_part_body_references r ON r.part_id=p.id AND r.field=?4
         WHERE m.conversation_id=?1 AND m.id=?2 AND p.id=?3
           AND (?5=0 OR COALESCE((SELECT e.visible FROM message_projection_events e
                JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
                WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
                ORDER BY o.resulting_revision DESC LIMIT 1),1)=1)"
            ),
            params![
                conversation_id,
                message_id,
                part_id,
                name,
                require_lifecycle
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let (message_state, part_state, inline_bytes, bound, encoded) = row.ok_or_else(|| {
        invalid("original Part field is unavailable for this owner or visibility")
    })?;
    if field == nola_domain::PartBodyField::ContentText {
        if let Some(head) =
            crate::canonical_stream_read::resolve(connection, conversation_id, message_id, part_id)?
        {
            if inline_bytes != Some(0) || bound.is_some() {
                return Err(invalid(
                    "stream field conflicts with another original text backing",
                ));
            }
            return Ok(ResolvedPartField::Chunked(head));
        }
    }
    if bound.is_none() {
        return Ok(match inline_bytes {
            Some(total_utf8_bytes) => ResolvedPartField::Inline { total_utf8_bytes },
            None => ResolvedPartField::Absent,
        });
    }
    let finalized_or_resumed = !require_lifecycle
        || message_state == "completed"
        || crate::canonical_message_ownership::has_trusted_resumption(
            connection,
            conversation_id,
            message_id,
        )?;
    if !finalized_or_resumed
        || (require_lifecycle && !matches!(part_state.as_str(), "complete" | "completed"))
        || match field {
            nola_domain::PartBodyField::ContentText => inline_bytes != Some(0),
            nola_domain::PartBodyField::ContentJson => inline_bytes.is_some(),
        }
    {
        return Err(invalid(
            "published field conflicts with its inline slot or finalized state",
        ));
    }
    let reference: nola_domain::CanonicalBodyReference = serde_json::from_str(
        encoded
            .as_deref()
            .ok_or_else(|| invalid("oversized published field reference"))?,
    )?;
    if reference.conversation_id() != conversation_id
        || reference.message_id() != message_id
        || reference.part_id() != part_id
        || reference.field() != field
    {
        return Err(invalid("published field reference owner or field mismatch"));
    }
    validate_reference_from_connection(connection, &reference)?;
    Ok(ResolvedPartField::Referenced(reference))
}

/// Shared bounded original-field retrieval for inline and immutable storage.
pub(super) fn field_range(
    connection: &Connection,
    conversation_id: &str,
    message_id: &str,
    part_id: &str,
    field: nola_domain::PartBodyField,
    offset: u64,
    limit: usize,
) -> StoreResult<(u64, crate::canonical_text_store::CanonicalTextRange)> {
    if !(4..=32768).contains(&limit) || offset >= i64::MAX as u64 {
        return Err(invalid("invalid original field range"));
    }
    let _snapshot = crate::read_snapshot::pin(connection)?;
    let resolved = resolve_field(connection, conversation_id, message_id, part_id, field)?;
    let total = resolved
        .total_utf8_bytes()
        .ok_or_else(|| invalid("original Part field is absent"))?;
    if offset > total {
        return Err(invalid("original field offset is after end"));
    }
    match resolved {
        ResolvedPartField::Referenced(reference) => {
            return Ok((
                total,
                read_range_from_connection(connection, &reference, offset, limit)?,
            ))
        }
        ResolvedPartField::Chunked(head) => {
            return Ok((
                total,
                crate::canonical_stream_read::range(connection, &head, offset, limit)?,
            ))
        }
        _ => {}
    }
    let name = field_name(field);
    // SQLite incremental reads work for TEXT too. substr(CAST(... AS BLOB))
    // copies the entire value for every small range (quadratic archive scans).
    let row_id: i64 = connection.query_row(
        "SELECT rowid FROM message_parts WHERE id=?1", [part_id], |row| row.get(0))?;
    let blob = connection.blob_open(rusqlite::DatabaseName::Main, "message_parts", name, row_id, true)?;
    let mut bytes = vec![0; limit.min((total - offset) as usize)];
    blob.read_at_exact(&mut bytes, offset as usize)?;
    match std::str::from_utf8(&bytes) {
        Ok(_) => {}
        Err(error) if error.error_len().is_none() && error.valid_up_to() > 0 => {
            bytes.truncate(error.valid_up_to())
        }
        Err(_) => return Err(invalid("original field offset is not a UTF-8 boundary")),
    }
    let next = offset + bytes.len() as u64;
    if (next == offset && next < total) || next > total {
        return Err(invalid("original field range made invalid progress"));
    }
    Ok((
        total,
        crate::canonical_text_store::CanonicalTextRange {
            offset_bytes: offset,
            next_offset_bytes: next,
            text: String::from_utf8(bytes).map_err(|error| invalid(&error.to_string()))?,
            end_of_content: next == total,
        },
    ))
}

fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}

#[cfg(test)]
#[path = "published_part_body_resume_tests.rs"]
mod resume_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NolaStore;
    use nola_domain::CanonicalBodyReference;

    fn message(store: &NolaStore, id: &str, ordinal: i64) {
        store
            .connection
            .execute(
                "INSERT INTO messages VALUES(?1,'c',NULL,'user','completed',?2,1,1)",
                params![id, ordinal],
            )
            .unwrap();
    }

    fn inline(store: &NolaStore, message: &str, part: &str, ordinal: i64, text: &str) {
        store.connection.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
            VALUES(?1,?2,'text',?3,'completed',?4,1,1)",params![part,message,ordinal,text]).unwrap();
        crate::canonical_text_disclosure::flush(&store.connection).unwrap();
    }

    fn published(
        store: &NolaStore,
        message: &str,
        part: &str,
        ordinal: i64,
        chunks: &[&str],
    ) -> CanonicalBodyReference {
        inline(store, message, part, ordinal, "");
        let reference = CanonicalBodyReference::for_field(
            format!("body-{part}"),
            "c".into(),
            message.into(),
            part.into(),
            nola_domain::PartBodyField::ContentText,
            chunks.iter().map(|text| text.len() as u64).sum(),
            chunks.len() as u32,
        )
        .unwrap();
        let encoded = serde_json::to_string(&reference).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO canonical_text_objects VALUES(?1,?2)",
                params![reference.content_id(), encoded],
            )
            .unwrap();
        let mut offset = 0;
        for text in chunks {
            store
                .connection
                .execute(
                    "INSERT INTO canonical_text_chunks VALUES(?1,?2,?3)",
                    params![reference.content_id(), offset, text],
                )
                .unwrap();
            offset += text.len() as i64;
        }
        store
            .connection
            .execute(
                "INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES(?1,'content_text',?2)",
                params![part, encoded],
            )
            .unwrap();
        reference
    }

    fn fixture() -> NolaStore {
        let store = NolaStore::open(":memory:").unwrap();
        store
            .connection
            .execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','History',1,1,1)", [])
            .unwrap();
        store
    }

    #[test]
    fn bounded_history_preserves_text_review_authority_metadata() {
        let store = fixture();
        message(&store, "m", 1);
        let text = "draft".repeat(2000);
        inline(&store, "m", "p", 1, &text);
        let envelope = serde_json::json!({"kind":"text", "id":"p", "message_id":"m",
            "turn_id":"turn", "text":text, "review":"rejected", "is_complete":true});
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_json=?1 WHERE id='p'",
                params![envelope.to_string()],
            )
            .unwrap();
        crate::canonical_text_disclosure::flush(&store.connection).unwrap();
        let tail = store
            .load_conversation_tail_snapshot("c", 1, 100, 512)
            .unwrap()
            .unwrap();
        let part = &tail.conversation.messages[0].parts[0];
        let metadata: serde_json::Value =
            serde_json::from_str(part.content_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["review"], "rejected");
        assert_eq!(metadata["id"], "p");
        assert!(part.content_json.as_ref().unwrap().len() < 512);
    }

    #[test]
    fn committed_body_unifies_original_ranges_search_full_export_and_character_tail() {
        let store = fixture();
        message(&store, "m", 1);
        let chunks = ["前文中", "😀Ne", "eDle%_\\结束"];
        let original = chunks.concat();
        published(&store, "m", "p", 1, &chunks);
        let full = store.load_conversation("c").unwrap().unwrap();
        assert_eq!(full.messages[0].parts[0].content_text, original);
        let page = store.load_conversation_page("c", 0, 1).unwrap().unwrap();
        assert_eq!(page.messages[0].parts[0].content_text, original);
        let tail = store
            .load_conversation_tail_snapshot("c", 1, 5, 100)
            .unwrap()
            .unwrap();
        assert_eq!(
            tail.conversation.messages[0].parts[0].content_text,
            "…%_\\结束"
        );
        let mut restored = String::new();
        while restored.len() < original.len() {
            let range = store
                .read_conversation_text_range("c", "m", "p", restored.len() as u64, 4)
                .unwrap();
            assert_eq!(range.total_utf8_bytes, original.len() as u64);
            assert!(range.text.len() <= 4);
            restored.push_str(&range.text);
        }
        assert_eq!(restored, original);
        for query in ["中😀needle", "%_\\", "结束"] {
            let matches = store.search_transcript_text("c", query, 8).unwrap();
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].part_id, "p");
            assert_eq!(matches[0].snippet, original);
        }
        assert!(store
            .read_conversation_text_range("foreign", "m", "p", 0, 4)
            .is_err());
        assert!(store
            .read_conversation_text_range("c", "m", "p", 1, 4)
            .is_err());
    }

    #[test]
    fn viewport_accounts_original_bytes_and_pairs_previews_after_budget_exhaustion() {
        let store = fixture();
        message(&store, "older", 1);
        inline(&store, "older", "old", 1, "older");
        message(&store, "m", 2);
        inline(&store, "m", "a", 1, "abcde");
        published(&store, "m", "b", 2, &["é", "é"]);
        published(&store, "m", "c", 3, &["xyz"]);
        let window = store
            .load_conversation_message_window("c", None, 10, 6)
            .unwrap()
            .unwrap();
        assert_eq!(window.conversation.messages.len(), 1);
        assert!(window.has_more);
        let parts = &window.conversation.messages[0].parts;
        assert_eq!(
            parts
                .iter()
                .map(|part| part.content_text.as_str())
                .collect::<Vec<_>>(),
            vec!["abcde", "", "x"]
        );
        assert_eq!(
            window.deferred_text_parts,
            vec![
                crate::DeferredTextPart::new("m".into(), "b".into(), 4, 0, true, true).unwrap(),
                crate::DeferredTextPart::new("m".into(), "c".into(), 3, 1, true, true).unwrap(),
            ]
        );
    }

    #[test]
    fn mixed_search_returns_latest_matches_in_message_and_part_order() {
        let store = fixture();
        for ordinal in 1..=8 {
            let id = format!("m-{ordinal}");
            message(&store, &id, ordinal);
            inline(
                &store,
                &id,
                &format!("inline-{ordinal}"),
                1,
                "needle inline",
            );
            published(
                &store,
                &id,
                &format!("published-{ordinal}"),
                2,
                &["needle ", "chunk"],
            );
        }
        let ranked = store.ranked_transcript_candidates("c", "NEEDLE").unwrap();
        assert_eq!(ranked.iter().take(3).map(|(_, part)| part.as_str()).collect::<Vec<_>>(),
            vec!["published-8", "inline-8", "published-7"],
            "equal BM25 scores must use complete transcript order before limiting");
        let matches = store.search_transcript_text("c", "NEEDLE", 3).unwrap();
        assert_eq!(
            matches
                .iter()
                .map(|hit| hit.part_id.as_str())
                .collect::<Vec<_>>(),
            vec!["published-7", "inline-8", "published-8"]
        );
    }

    #[test]
    fn conflicting_body_source_wrong_owner_and_missing_chunks_fail_closed() {
        let store = fixture();
        message(&store, "m", 1);
        let reference = published(&store, "m", "p", 1, &["first", "tail"]);
        for update in ["content_text='conflict'", "state='streaming'"] {
            store
                .connection
                .execute(
                    &format!("UPDATE message_parts SET {update} WHERE id='p'"),
                    [],
                )
                .unwrap();
            assert!(store.load_conversation("c").is_err());
            store.connection.execute("UPDATE message_parts SET content_text='',content_json=NULL,state='completed',kind='text' WHERE id='p'",[]).unwrap();
        }
        let foreign = CanonicalBodyReference::for_field(
            reference.content_id().into(),
            "foreign".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            9,
            2,
        )
        .unwrap();
        store
            .connection
            .execute(
                "UPDATE message_part_body_references SET reference_json=?1 WHERE part_id='p'",
                [serde_json::to_string(&foreign).unwrap()],
            )
            .unwrap();
        assert!(store.load_conversation("c").is_err());
        assert!(store
            .read_conversation_text_range("c", "m", "p", 0, 4)
            .is_err());
        store
            .connection
            .execute(
                "UPDATE message_part_body_references SET reference_json=?1 WHERE part_id='p'",
                [serde_json::to_string(&reference).unwrap()],
            )
            .unwrap();
        store
            .connection
            .execute(
                "DELETE FROM canonical_text_chunks WHERE content_id=?1 AND offset_bytes=5",
                [reference.content_id()],
            )
            .unwrap();
        assert!(store
            .read_conversation_text_range("c", "m", "p", 0, 32)
            .is_err());
        assert!(store.load_conversation("c").is_err());
        assert!(store.search_transcript_text("c", "absent", 8).is_err());
    }

    #[test]
    fn field_aware_hydration_keeps_independent_metadata_and_product_kind_gate() {
        let store = fixture();
        message(&store, "m", 1);
        published(&store, "m", "p", 1, &["first", "tail"]);
        for (update, expected_kind, expected_json) in [
            ("content_json='{}'", "text", Some("{}")),
            ("content_json=NULL,kind='reasoning'", "reasoning", None),
        ] {
            store
                .connection
                .execute(
                    &format!("UPDATE message_parts SET {update} WHERE id='p'"),
                    [],
                )
                .unwrap();
            crate::canonical_text_disclosure::flush(&store.connection).unwrap();
            let conversation = store.load_conversation("c").unwrap().unwrap();
            let part = &conversation.messages[0].parts[0];
            assert_eq!(part.content_text, "firsttail");
            assert_eq!(part.content_json.as_deref(), expected_json);
            assert_eq!(part.kind, expected_kind);
            assert_eq!(
                crate::transcript_range::can_read_original(&store.connection, "c", "m", "p",)
                    .unwrap(),
                expected_kind == "text"
            );
        }
    }
    #[test]
    fn ten_million_byte_published_body_survives_reopen_through_canonical_readers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("published.sqlite");
        let store = NolaStore::open(&path).unwrap();
        store
            .connection
            .execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','History',1,1,1)", [])
            .unwrap();
        message(&store, "m", 1);
        let full_chunk = "x".repeat(32768);
        let marker = "TAIL-DONE";
        let full_chunks = (10_000_000 - marker.len()) / full_chunk.len();
        let tail = "x".repeat(10_000_000 - full_chunks * full_chunk.len() - marker.len()) + marker;
        let mut chunks = vec![full_chunk.as_str(); full_chunks];
        chunks.push(&tail);
        published(&store, "m", "p", 1, &chunks);
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        let window = store
            .load_conversation_message_window("c", None, 10, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(window.conversation.messages.len(), 1);
        assert_eq!(
            window.conversation.messages[0].parts[0].content_text.len(),
            1024
        );
        assert_eq!(
            window.deferred_text_parts,
            vec![crate::DeferredTextPart::new(
                "m".into(),
                "p".into(),
                10_000_000,
                1024,
                true,
                true
            )
            .unwrap()]
        );
        let range = store
            .read_conversation_text_range("c", "m", "p", 9_999_991, 32)
            .unwrap();
        assert_eq!(range.text, marker);
        assert!(range.end_of_content);
        let hits = store.search_transcript_text("c", "tail-done", 8).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.ends_with(marker));
        assert!(hits[0].snippet.len() < 1024);
        assert_eq!(hits[0].start_character, 9_999_751);
        let full = store.load_conversation("c").unwrap().unwrap();
        assert_eq!(full.messages[0].parts[0].content_text.len(), 10_000_000);
        assert!(full.messages[0].parts[0].content_text.ends_with(marker));
        let tail = store
            .load_conversation_tail_snapshot("c", 1, 9, 100)
            .unwrap()
            .unwrap();
        assert_eq!(
            tail.conversation.messages[0].parts[0].content_text,
            "…TAIL-DONE"
        );
    }
}
