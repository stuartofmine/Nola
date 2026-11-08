// SPDX-License-Identifier: Apache-2.0
//! Metadata-only archive pages and bounded original-field reads.
use crate::{params, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_store_port::transcript_export::*;

const VISIBLE: &str = "COALESCE((SELECT e.visible FROM message_projection_events e
    JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
    WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
    ORDER BY o.resulting_revision DESC LIMIT 1),1)=1";

impl TranscriptExportRepository for NolaStore {
    type Error = StoreError;

    fn load_export_page(
        &self,
        conversation_id: &str,
        after_ordinal: Option<i64>,
        expected_revision: Option<i64>,
    ) -> StoreResult<Option<ExportPage>> {
        identity(conversation_id)?;
        if after_ordinal.is_some_and(|at| at < 1)
            || expected_revision.is_some_and(|revision| revision < 1)
            || (after_ordinal.is_some() && expected_revision.is_none())
        {
            return Err(invalid(
                "export continuation requires positive cursor and pinned revision",
            ));
        }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let conversation = self.connection.query_row(
            "SELECT c.id,CASE WHEN length(CAST(c.title AS BLOB))<=4096 THEN c.title END,
             c.revision,c.created_at_ms,c.updated_at_ms,
             CASE WHEN length(CAST(s.settings_json AS BLOB))<=131072 THEN s.settings_json END,
             COALESCE(length(CAST(s.settings_json AS BLOB)),0),c.configuration_revision
             FROM conversations c LEFT JOIN conversation_settings s ON s.conversation_id=c.id WHERE c.id=?1",
            params![conversation_id], |row| {
                if row.get::<_,i64>(6)? > 131072 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(ExportConversation { id:row.get(0)?,title:row.get(1)?,revision:row.get(2)?,configuration_revision:row.get(7)?,
                    created_at_ms:row.get(3)?,updated_at_ms:row.get(4)?,settings_json:row.get(5)? })
            },
        ).optional()?;
        let Some(conversation) = conversation else {
            return Ok(None);
        };
        revision(conversation.revision, expected_revision)?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT id,run_id,role,status,ordinal,created_at_ms,completed_at_ms FROM messages m
             WHERE conversation_id=?1 AND ordinal>?2 AND {VISIBLE} ORDER BY ordinal LIMIT 2"
        ))?;
        let mut messages = statement
            .query_map(
                params![conversation_id, after_ordinal.unwrap_or(0)],
                |row| {
                    Ok(ExportMessage {
                        id: row.get(0)?,
                        run_id: row.get(1)?,
                        role: row.get(2)?,
                        status: row.get(3)?,
                        ordinal: row.get(4)?,
                        created_at_ms: row.get(5)?,
                        completed_at_ms: row.get(6)?,
                        parts: Vec::new(),
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = messages.len() > 1;
        messages.truncate(1);
        for message in &mut messages {
            identity(&message.id)?;
            let mut statement = self.connection.prepare(
                "SELECT id,kind,ordinal,state,length(CAST(content_text AS BLOB)),
                 length(CAST(content_json AS BLOB)),
                 CASE WHEN length(CAST(content_digest AS BLOB))<=4096 THEN content_digest END,created_at_ms,updated_at_ms,
                 COALESCE(length(CAST(content_digest AS BLOB)),0),
                 EXISTS(SELECT 1 FROM message_part_derivations d WHERE d.target_part_id=message_parts.id) FROM message_parts WHERE message_id=?1 ORDER BY ordinal LIMIT 513")?;
            let mut parts = statement
                .query_map(params![message.id], |row| {
                    if row.get::<_, i64>(9)? > 4096 {
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                    Ok(ExportPart {
                        has_derivation: row.get(10)?,
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        ordinal: row.get(2)?,
                        state: row.get(3)?,
                        content_text_utf8_bytes: row.get(4)?,
                        content_json_utf8_bytes: row.get(5)?,
                        content_digest: row.get(6)?,
                        created_at_ms: row.get(7)?,
                        updated_at_ms: row.get(8)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            if parts.len() > 512 {
                return Err(invalid(
                    "export message exceeds bounded Part metadata cardinality",
                ));
            }
            for part in &mut parts {
                identity(&part.id)?;
                let text = crate::published_part_body::resolve_field(
                    &self.connection,
                    conversation_id,
                    &message.id,
                    &part.id,
                    ExportBodyField::ContentText,
                )?;
                let json = crate::published_part_body::resolve_field(
                    &self.connection,
                    conversation_id,
                    &message.id,
                    &part.id,
                    ExportBodyField::ContentJson,
                )?;
                part.content_text_utf8_bytes = text
                    .total_utf8_bytes()
                    .ok_or_else(|| invalid("canonical text field is absent"))?;
                part.content_json_utf8_bytes = json.total_utf8_bytes();
            }
            message.parts = parts;
        }
        let next_ordinal = messages.last().map(|message| message.ordinal);
        let result = ExportPage {
            conversation,
            messages,
            next_ordinal,
            has_more,
        };
        // Metadata is bounded independently of either original body length.
        if serde_json::to_vec(&result)?.len() > 256 * 1024 {
            return Err(invalid("export metadata exceeds its encoded frame budget"));
        }
        Ok(Some(result))
    }

    fn read_part_derivation(
        &self,
        owner: &CanonicalPartOwner,
        expected_conversation_revision: i64,
    ) -> StoreResult<Option<StoredPartDerivation>> {
        if expected_conversation_revision < 1 {
            return Err(invalid("invalid derivation export revision"));
        }
        for id in [owner.conversation_id(), owner.message_id(), owner.part_id()] {
            identity(id)?;
        }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let current: i64 = self
            .connection
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                params![owner.conversation_id()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| invalid("export conversation is unavailable"))?;
        revision(current, Some(expected_conversation_revision))?;
        let visible: bool = self.connection.query_row(&format!(
            "SELECT EXISTS(SELECT 1 FROM messages m JOIN message_parts p ON p.message_id=m.id WHERE m.conversation_id=?1 AND m.id=?2 AND p.id=?3 AND {VISIBLE})"
        ), params![owner.conversation_id(), owner.message_id(), owner.part_id()], |row| row.get(0))?;
        if !visible {
            return Err(invalid("derivation export owner is unavailable"));
        }
        let result = self.part_derivation_record(owner)?;
        if serde_json::to_vec(&result)?.len() > 24 * 1024 {
            return Err(invalid("derivation export exceeds encoded response budget"));
        }
        Ok(result)
    }

    fn read_export_range(
        &self,
        conversation_id: &str,
        message_id: &str,
        part_id: &str,
        field: ExportBodyField,
        expected_revision: i64,
        offset_bytes: u64,
        limit_bytes: usize,
    ) -> StoreResult<ExportRange> {
        for id in [conversation_id, message_id, part_id] {
            identity(id)?;
        }
        if expected_revision < 1
            || !(4..=32768).contains(&limit_bytes)
            || offset_bytes >= i64::MAX as u64
        {
            return Err(invalid("invalid export revision or byte range"));
        }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let current = self
            .connection
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| invalid("export conversation is unavailable"))?;
        revision(current, Some(expected_revision))?;
        let (total, range) = crate::published_part_body::field_range(
            &self.connection,
            conversation_id,
            message_id,
            part_id,
            field,
            offset_bytes,
            limit_bytes,
        )?;
        let next = range.next_offset_bytes;
        Ok(ExportRange {
            conversation_id: conversation_id.into(),
            message_id: message_id.into(),
            part_id: part_id.into(),
            field,
            revision: current,
            offset_bytes,
            next_offset_bytes: next,
            total_utf8_bytes: total,
            text: range.text,
            end_of_content: next == total,
        })
    }
}

fn identity(id: &str) -> StoreResult<()> {
    if id.is_empty() || id.trim() != id || id.len() > 128 || id.chars().any(char::is_control) {
        return Err(invalid("invalid export identity"));
    }
    Ok(())
}
fn revision(current: i64, expected: Option<i64>) -> StoreResult<()> {
    if expected.is_some_and(|expected| expected != current) {
        return Err(StoreError::Conflict(
            "conversation changed during export; archive was not completed".into(),
        ));
    }
    Ok(())
}
fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_store_port::transcript_publication::TranscriptPublicationRepository;

    fn store() -> NolaStore {
        let store = NolaStore::open(":memory:").unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Archive',1,1,2);
            INSERT INTO messages VALUES('m','c',NULL,'user','completed',1,1,2);
            INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
            VALUES('p','m','text',1,'completed','',NULL,1,2);").unwrap();
        store
    }

    #[test]
    fn derivation_export_pins_revision_visibility_and_current_fingerprints() {
        let mut store = store();
        store.connection.execute("UPDATE message_parts SET content_text='original',content_json=' { \"owner\": \"source\" } ' WHERE id='p'", []).unwrap();
        store
            .create_conversation_branch(&nola_domain::CanonicalBranchIntent {
                operation_id: "proof".into(),
                source_conversation_id: "c".into(),
                expected_source_revision: 1,
                through_message_id: "m".into(),
                conversation: nola_domain::CanonicalBranchTarget {
                    id: "derived".into(),
                    title: "Derived".into(),
                    settings_json: None,
                    created_at_ms: 3,
                    updated_at_ms: 3,
                },
                committed_at_ms: 3,
            })
            .unwrap();
        let page = store
            .load_export_page("derived", None, None)
            .unwrap()
            .unwrap();
        let part = &page.messages[0].parts[0];
        assert!(part.has_derivation);
        let owner = CanonicalPartOwner::new(
            "derived".into(),
            page.messages[0].id.clone(),
            part.id.clone(),
        )
        .unwrap();
        let rev = page.conversation.revision;
        let record = store.read_part_derivation(&owner, rev).unwrap().unwrap();
        assert_eq!(record.provenance, PartDerivationProvenance::LocalBranch);
        assert!(serde_json::to_vec(&record).unwrap().len() < 24576);
        assert!(store.read_part_derivation(&owner, rev + 1).is_err());
        let wrong = CanonicalPartOwner::new(
            "c".into(),
            owner.message_id().into(),
            owner.part_id().into(),
        )
        .unwrap();
        assert!(store.read_part_derivation(&wrong, 1).is_err());
        store
            .connection
            .execute("DELETE FROM conversations WHERE id='c'", [])
            .unwrap();
        assert_eq!(
            store.read_part_derivation(&owner, rev).unwrap(),
            Some(record)
        );
        store.connection.execute("INSERT INTO conversation_projection_operations VALUES('hide','derived','hide',NULL,99,1,5)", []).unwrap();
        store.connection.execute("INSERT INTO message_projection_events VALUES('hide-event','hide','derived',?1,0,'test',5)", [owner.message_id()]).unwrap();
        assert!(store.read_part_derivation(&owner, rev).is_err());
        store
            .connection
            .execute(
                "DELETE FROM conversation_projection_operations WHERE operation_id='hide'",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_text='modified' WHERE id=?1",
                [owner.part_id()],
            )
            .unwrap();
        assert!(
            store.read_part_derivation(&owner, rev).is_err(),
            "equal byte counts cannot substitute for SHA verification"
        );
    }

    #[test]
    fn ten_million_byte_reference_exports_bounded_original_ranges_not_a_preview() {
        let mut store = store();
        let bytes = 10_000_000;
        let reference = nola_domain::CanonicalBodyReference::for_field(
            "body".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            bytes,
            bytes.div_ceil(32768) as u32,
        )
        .unwrap();
        store
            .prepare_transcript_content(
                &reference,
                &mut (0..reference.chunk_count()).map(|index| {
                    let offset = u64::from(index) * 32768;
                    Ok((
                        offset,
                        "é".repeat(((bytes - offset).min(32768) / 2) as usize),
                    ))
                }),
                bytes,
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES('p','content_text',?1)",
                [serde_json::to_string(&reference).unwrap()],
            )
            .unwrap();
        let page = store.load_export_page("c", None, None).unwrap().unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() < 2048);
        assert_eq!(page.messages[0].parts[0].content_text_utf8_bytes, bytes);
        assert_eq!(page.messages[0].parts[0].content_json_utf8_bytes, None);
        let mut offset = 0;
        while offset < bytes {
            let range = store
                .read_export_range(
                    "c",
                    "m",
                    "p",
                    ExportBodyField::ContentText,
                    1,
                    offset,
                    32768,
                )
                .unwrap();
            assert!(range.text.len() <= 32768);
            assert_eq!(range.text, "é".repeat(range.text.len() / 2));
            assert!(serde_json::to_vec(&range).unwrap().len() < 200_000);
            offset = range.next_offset_bytes;
        }
        assert_eq!(offset, bytes);
    }

    #[test]
    fn structured_json_and_text_export_independently_with_lossless_unicode_boundaries() {
        let store = store();
        let text = "abc中😀\n\\\"tail";
        let json = serde_json::json!({"text":text,"extra":"原始JSON"}).to_string();
        store
            .connection
            .execute(
                "UPDATE message_parts SET kind='reasoning',content_text=?1,content_json=?2",
                params![text, json],
            )
            .unwrap();
        let page = store.load_export_page("c", None, None).unwrap().unwrap();
        let part = &page.messages[0].parts[0];
        assert_eq!(part.content_text_utf8_bytes, text.len() as u64);
        assert_eq!(part.content_json_utf8_bytes, Some(json.len() as u64));
        for (field, expected) in [
            (ExportBodyField::ContentText, text),
            (ExportBodyField::ContentJson, json.as_str()),
        ] {
            let mut restored = String::new();
            let mut offset = 0;
            loop {
                let range = store
                    .read_export_range("c", "m", "p", field, 1, offset, 4)
                    .unwrap();
                restored.push_str(&range.text);
                offset = range.next_offset_bytes;
                if range.end_of_content {
                    break;
                }
            }
            assert_eq!(restored, expected);
        }
        assert!(store
            .read_export_range("c", "m", "p", ExportBodyField::ContentText, 1, 4, 4)
            .is_err());
    }

    #[test]
    fn owner_revision_and_visibility_are_checked_on_every_page_and_body_request() {
        let store = store();
        store
            .connection
            .execute("UPDATE message_parts SET content_text='original'", [])
            .unwrap();
        let page = store.load_export_page("c", None, None).unwrap().unwrap();
        assert_eq!(page.next_ordinal, Some(1));
        for (c, m, p) in [
            ("foreign", "m", "p"),
            ("c", "foreign", "p"),
            ("c", "m", "foreign"),
        ] {
            assert!(store
                .read_export_range(c, m, p, ExportBodyField::ContentText, 1, 0, 32768)
                .is_err());
        }
        assert!(store.load_export_page("c", Some(1), None).is_err());
        assert!(store
            .read_export_range("c", "m", "p", ExportBodyField::ContentJson, 1, 0, 32768)
            .is_err());
        store
            .connection
            .execute("UPDATE conversations SET revision=2", [])
            .unwrap();
        assert!(store.load_export_page("c", Some(1), Some(1)).is_err());
        assert!(store
            .read_export_range("c", "m", "p", ExportBodyField::ContentText, 1, 0, 32768)
            .is_err());
        assert!(store
            .load_export_page("c", Some(1), Some(2))
            .unwrap()
            .unwrap()
            .messages
            .is_empty());
    }

    #[test]
    fn export_header_keeps_configuration_revision_distinct_from_transcript_revision() {
        let store = store();
        store.connection.execute(
            "UPDATE conversations SET revision=13, configuration_revision=7 WHERE id='c'", [],
        ).unwrap();
        let page = store.load_export_page("c", None, Some(13)).unwrap().unwrap();
        assert_eq!(page.conversation.revision, 13);
        assert_eq!(page.conversation.configuration_revision, 7);
    }
}
