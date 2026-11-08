// SPDX-License-Identifier: Apache-2.0
//! Field-aware preparation on the one canonical database. Binding/publication
//! is separately admitted by the owning append, branch or archive transaction.
use crate::{params, NolaStore, StoreError, StoreResult};
use nola_domain::{CanonicalBodyReference, PartBodyField};
use nola_store_port::canonical_body::{CanonicalBodyRepository, PreparedBodyRange};

/// Text-specific adapters must not reinterpret a prepared JSON object as text.
/// Generic field-aware append/archive paths validate the actual field instead.
pub(crate) fn require_text_reference(reference: &CanonicalBodyReference) -> StoreResult<()> {
    if reference.field() != PartBodyField::ContentText {
        return Err(StoreError::Invariant(
            "JSON body publication is not enabled on the text-only path".into(),
        ));
    }
    Ok(())
}

impl CanonicalBodyRepository for NolaStore {
    type Error = StoreError;

    fn prepare_body(
        &mut self,
        reference: &CanonicalBodyReference,
        chunks: &mut dyn Iterator<Item = Result<(u64, String), String>>,
        maximum_bytes: u64,
    ) -> StoreResult<()> {
        let transaction = self.write_transaction()?;
        crate::canonical_text_store::put_chunks(
            &transaction,
            reference,
            chunks.map(|chunk| chunk.map_err(StoreError::Invariant)),
            maximum_bytes,
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn read_prepared_body_range(
        &self,
        reference: &CanonicalBodyReference,
        offset_bytes: u64,
        limit_bytes: usize,
    ) -> StoreResult<PreparedBodyRange> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let range = crate::canonical_text_store::read_range_from_connection(
            &self.connection,
            reference,
            offset_bytes,
            limit_bytes,
        )?;
        Ok(PreparedBodyRange {
            offset_bytes: range.offset_bytes,
            next_offset_bytes: range.next_offset_bytes,
            text: range.text,
            end_of_content: range.end_of_content,
        })
    }
}

/// Called by the shared chunk writer before its caller can commit an object.
pub(crate) fn validate_json_object(
    connection: &crate::Connection,
    reference: &CanonicalBodyReference,
) -> StoreResult<()> {
    let mut statement=connection.prepare("SELECT offset_bytes,CASE WHEN length(CAST(text AS BLOB)) BETWEEN 1 AND 32768 THEN text END FROM canonical_text_chunks WHERE content_id=?1 ORDER BY offset_bytes")?;
    let chunks = statement.query_map(params![reference.content_id()], |row| {
        Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
    })?;
    let stats = crate::structured_json_validation::validate_json_chunks(
        chunks.map(|chunk| chunk.map_err(|error| error.to_string())),
        reference.total_utf8_bytes(),
    )?;
    if stats.chunk_count != u64::from(reference.chunk_count()) {
        return Err(StoreError::Invariant(
            "prepared JSON chunk count differs".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{canonical_text_replica, RemoteSyncChangeInput};
    use nola_store_port::{
        archive_restore::ArchiveRestoreRepository,
        transcript_publication::TranscriptPublicationRepository,
    };

    fn reference(id: &str, field: PartBodyField, bytes: u64, count: u32) -> CanonicalBodyReference {
        CanonicalBodyReference::for_field(
            id.into(),
            "c".into(),
            "m".into(),
            "p".into(),
            field,
            bytes,
            count,
        )
        .unwrap()
    }
    fn prepare(
        store: &mut NolaStore,
        reference: &CanonicalBodyReference,
        text: &str,
    ) -> StoreResult<()> {
        store.prepare_body(
            reference,
            &mut vec![Ok((0, text.into()))].into_iter(),
            20_000_000,
        )
    }
    fn object_count(store: &NolaStore) -> i64 {
        store
            .connection
            .query_row("SELECT COUNT(*) FROM canonical_text_objects", [], |r| {
                r.get(0)
            })
            .unwrap()
    }

    #[test]
    fn two_original_fields_prepare_on_one_database_and_owner_mismatch_cannot_read_or_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("body.db");
        let mut store = NolaStore::open(&path).unwrap();
        let original = " {\"result\" : \"中\\uD83D\\uDE00\", \"result\": 3 } \n";
        let json = reference("json", PartBodyField::ContentJson, original.len() as u64, 1);
        let text = reference("text", PartBodyField::ContentText, 3, 1);
        prepare(&mut store, &json, original).unwrap();
        prepare(&mut store, &text, "raw").unwrap();
        assert_eq!(object_count(&store), 2);
        assert!(store.list_conversations().unwrap().is_empty());
        assert!(store.pending_sync_changes(100).unwrap().is_empty());
        assert!(store
            .read_conversation_text_range("c", "m", "p", 0, 100)
            .is_err());
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        let mut actual = String::new();
        let mut offset = 0;
        loop {
            let range = store.read_prepared_body_range(&json, offset, 7).unwrap();
            actual.push_str(&range.text);
            offset = range.next_offset_bytes;
            if range.end_of_content {
                break;
            }
        }
        assert_eq!(actual, original);
        for key in ["field", "conversation_id", "message_id", "part_id"] {
            let mut encoded = serde_json::to_value(&json).unwrap();
            encoded[key] = if key == "field" {
                "content_text".into()
            } else {
                "foreign".into()
            };
            let foreign = serde_json::from_value(encoded).unwrap();
            assert!(store.read_prepared_body_range(&foreign, 0, 10).is_err());
            assert!(prepare(&mut store, &foreign, original).is_err());
        }
        prepare(&mut store, &json, original).unwrap();
        assert_eq!(object_count(&store), 2);
        assert_eq!(
            store.read_prepared_body_range(&text, 0, 4).unwrap().text,
            "raw"
        );
    }

    #[test]
    fn invalid_json_and_interrupted_sources_roll_back_objects_chunks_and_quota_without_destroying_prior_body(
    ) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("body.db");
        let mut store = NolaStore::open(&path).unwrap();
        let original = "{\"preserved\":true}";
        let good = reference("good", PartBodyField::ContentJson, original.len() as u64, 1);
        prepare(&mut store, &good, original).unwrap();
        for (index, invalid) in ["\"\\uD800\"", "1e309", "[1,]", "{} trailing"]
            .into_iter()
            .enumerate()
        {
            let body = reference(
                &format!("invalid-{index}"),
                PartBodyField::ContentJson,
                invalid.len() as u64,
                1,
            );
            assert!(prepare(&mut store, &body, invalid).is_err());
            assert!(store.read_prepared_body_range(&body, 0, 100).is_err());
        }
        let interrupted = reference("interrupted", PartBodyField::ContentJson, 4, 2);
        assert!(store
            .prepare_body(
                &interrupted,
                &mut vec![Ok((0, "nu".into())), Err("disconnected".into())].into_iter(),
                20_000_000
            )
            .is_err());
        assert_eq!(object_count(&store), 1);
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM canonical_text_chunks", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(prepare(&mut store, &good, &"x".repeat(original.len())).is_err());
        store.begin_write_unit_of_work().unwrap();
        let pending = reference("pending", PartBodyField::ContentJson, 4, 1);
        prepare(&mut store, &pending, "null").unwrap();
        store.rollback_write_unit_of_work().unwrap();
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        assert_eq!(object_count(&store), 1);
        assert_eq!(
            store.read_prepared_body_range(&good, 0, 100).unwrap().text,
            original
        );
    }

    #[test]
    fn replicated_prepared_json_still_cannot_enter_text_publication_or_visible_reader() {
        let mut store = NolaStore::open(":memory:").unwrap();
        let json = reference("json", PartBodyField::ContentJson, 4, 1);
        prepare(&mut store, &json, "null").unwrap();
        assert!(store
            .prepare_transcript_content(&json, &mut vec![Ok((0, "null".into()))].into_iter(), 100)
            .is_err());
        assert!(store
            .prepare_archive_content(
                "unavailable",
                &json,
                &mut vec![Ok((0, "null".into()))].into_iter(),
                100
            )
            .is_err());
        canonical_text_replica::require_complete(&store.connection, &json).unwrap();
        canonical_text_replica::enqueue(&store.connection, &json, 1).unwrap();
        assert_eq!(store.pending_sync_changes(100).unwrap().len(), 2);
        let mut receiver = NolaStore::open(":memory:").unwrap();
        let transaction = receiver.connection.transaction().unwrap();
        for (entity_type, entity_id, payload) in [
            (
                "canonical_text_chunk",
                "json:0",
                serde_json::json!({"reference":json,"offset_bytes":0,"text":"null"}),
            ),
            (
                "canonical_text_seal",
                "json",
                serde_json::to_value(&json).unwrap(),
            ),
        ] {
            let fact = RemoteSyncChangeInput {
                cursor: 1,
                operation_id: entity_type.into(),
                entity_type: entity_type.into(),
                entity_id: entity_id.into(),
                revision: 1,
                deleted: false,
                payload_json: payload.to_string(),
                created_at_ms: 1,
            };
            canonical_text_replica::apply(&transaction, &fact).unwrap();
        }
        transaction.commit().unwrap();
        canonical_text_replica::require_complete(&receiver.connection, &json).unwrap();
        assert!(receiver.list_conversations().unwrap().is_empty());
        assert_eq!(
            receiver
                .connection
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(receiver.pending_sync_changes(100).unwrap().is_empty());
        assert!(store.list_conversations().unwrap().is_empty());
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Closed JSON',1,1,1);INSERT INTO messages VALUES('m','c',NULL,'user','completed',1,1,1);INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms) VALUES('p','m','text',1,'completed','',1,1)").unwrap();
        store
            .connection
            .execute(
                "INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES('p','content_text',?1)",
                params![serde_json::to_string(&json).unwrap()],
            )
            .unwrap();
        assert!(crate::published_part_body::resolve_field(
            &store.connection, "c", "m", "p", PartBodyField::ContentText,
        ).is_err());
        assert!(store
            .read_conversation_text_range("c", "m", "p", 0, 100)
            .is_err());
        assert!(store
            .load_conversation_message_window("c", None, 1, 100)
            .is_err());
        assert_eq!(store.pending_sync_changes(100).unwrap().len(), 2);
    }

    #[test]
    fn ten_million_byte_json_preparation_and_reopen_ranges_preserve_original_chunks() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("large.db");
        let mut store = NolaStore::open(&path).unwrap();
        let total = 10_000_000u64;
        let json = reference(
            "large",
            PartBodyField::ContentJson,
            total,
            total.div_ceil(32768) as u32,
        );
        let mut chunks = (0..json.chunk_count()).map(|index| {
            let offset = u64::from(index) * 32768;
            let mut text = "x".repeat((total - offset).min(32768) as usize);
            if offset == 0 {
                text.replace_range(0..1, "\"");
            }
            if offset + text.len() as u64 == total {
                let end = text.len();
                text.replace_range(end - 1..end, "\"");
            }
            Ok((offset, text))
        });
        store.prepare_body(&json, &mut chunks, 20_000_000).unwrap();
        drop(chunks);
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        for offset in [0, 32766, 8_000_003, total - 17] {
            let range = store.read_prepared_body_range(&json, offset, 4096).unwrap();
            for (index, byte) in range.text.bytes().enumerate() {
                assert_eq!(
                    byte,
                    if offset + index as u64 == 0 || offset + index as u64 == total - 1 {
                        b'"'
                    } else {
                        b'x'
                    }
                );
            }
        }
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM canonical_text_chunks", [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            json.chunk_count()
        );
        assert!(store.list_conversations().unwrap().is_empty());
    }
    #[test]
    fn unbound_json_does_not_publish_or_masquerade_as_text() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("body.db");
        let mut store = NolaStore::open(&path).unwrap();
        let json = reference("json", PartBodyField::ContentJson, 4, 1);
        prepare(&mut store, &json, "null").unwrap();
        let append = crate::AppendMessagesInput {
            conversation_id: "c".into(),
            expected_message_count: 0,
            messages: vec![],
            updated_at_ms: 1,
        };
        assert!(crate::conversation_append_store::append_in_transaction(
            &store.connection,
            &append,
            None,
            std::slice::from_ref(&json)
        )
        .unwrap_err()
        .to_string()
        .contains("MissingPart"));
        assert!(crate::conversation_append_sync::enqueue_with_references(
            &store.connection,
            &append,
            2,
            std::slice::from_ref(&json)
        )
        .is_err());
        assert!(crate::canonical_text_search::search_literal_with_reader(
            &json,
            "null",
            |_, _| panic!("JSON must not enter text search")
        )
        .is_err());
        // JSON retirement now has its own production-path tests in
        // content_upload_lifecycle: exact owner/bytes/field, not a type ban.
    }
}
