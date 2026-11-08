// SPDX-License-Identifier: Apache-2.0
//! Bounded immutable content facts. A seal precedes any message reference.
use super::*;
use nola_domain::{CanonicalBodyReference, CANONICAL_BODY_CHUNK_MAX_BYTES};
use serde::{Deserialize, Serialize};

const MAX_REPLICA_CONTENT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkFact {
    reference: CanonicalBodyReference,
    offset_bytes: u64,
    text: String,
}

fn chunk_id(reference: &CanonicalBodyReference, offset: u64) -> String {
    format!("{}:{offset}", reference.content_id())
}

pub(super) fn require_complete(
    connection: &Connection,
    reference: &CanonicalBodyReference,
) -> StoreResult<()> {
    let encoded: String = connection.query_row(
        "SELECT CASE WHEN length(CAST(reference_json AS BLOB))<=4096 THEN reference_json END FROM canonical_text_objects WHERE id=?1", params![reference.content_id()], |row| row.get(0))?;
    if serde_json::from_str::<CanonicalBodyReference>(&encoded)? != *reference {
        return Err(StoreError::Conflict(
            "replica content owner mismatch".into(),
        ));
    }
    validate_chunks(connection, reference, false)?;
    if reference.field() == nola_domain::PartBodyField::ContentJson {
        crate::canonical_body::validate_json_object(connection, reference)?;
    }
    Ok(())
}

fn validate_chunks(
    connection: &Connection,
    reference: &CanonicalBodyReference,
    staging: bool,
) -> StoreResult<()> {
    let sql = if staging {
        "SELECT offset_bytes,CASE WHEN length(CAST(text AS BLOB)) BETWEEN 1 AND 32768 THEN text END,CASE WHEN length(CAST(reference_json AS BLOB))<=4096 THEN reference_json END FROM canonical_text_replica_chunks WHERE content_id=?1 ORDER BY offset_bytes"
    } else {
        "SELECT offset_bytes,CASE WHEN length(CAST(text AS BLOB)) BETWEEN 1 AND 32768 THEN text END,NULL FROM canonical_text_chunks WHERE content_id=?1 ORDER BY offset_bytes"
    };
    let mut statement = connection.prepare(sql)?;
    let mut rows = statement.query(params![reference.content_id()])?;
    let mut next = 0_u64;
    let mut count = 0_u32;
    while let Some(row) = rows.next()? {
        let offset: i64 = row.get(0)?;
        let text: String = row.get(1)?;
        let owner: Option<String> = row.get(2)?;
        if offset < 0
            || offset as u64 != next
            || text.is_empty()
            || text.len() as u64 > CANONICAL_BODY_CHUNK_MAX_BYTES
            || (staging && owner.is_none())
            || owner
                .as_deref()
                .map(serde_json::from_str::<CanonicalBodyReference>)
                .transpose()?
                .is_some_and(|value| value != *reference)
        {
            return Err(StoreError::Conflict(
                "replica content chunks are inconsistent".into(),
            ));
        }
        next += text.len() as u64;
        count += 1;
        if next > reference.total_utf8_bytes() || count > reference.chunk_count() {
            return Err(StoreError::Conflict(
                "replica content exceeds declaration".into(),
            ));
        }
    }
    if next != reference.total_utf8_bytes() || count != reference.chunk_count() {
        return Err(StoreError::Conflict(
            "replica content is incomplete; reference cannot publish".into(),
        ));
    }
    Ok(())
}

pub(super) fn enqueue(
    connection: &Connection,
    reference: &CanonicalBodyReference,
    at: i64,
) -> StoreResult<()> {
    require_complete(connection, reference)?;
    let mut statement = connection.prepare("SELECT offset_bytes,CASE WHEN length(CAST(text AS BLOB)) BETWEEN 1 AND 32768 THEN text END FROM canonical_text_chunks WHERE content_id=?1 ORDER BY offset_bytes")?;
    let mut rows = statement.query(params![reference.content_id()])?;
    while let Some(row) = rows.next()? {
        let offset = row.get::<_, i64>(0)? as u64;
        let fact = ChunkFact {
            reference: reference.clone(),
            offset_bytes: offset,
            text: row.get(1)?,
        };
        enqueue_sync_change(
            connection,
            "canonical_text_chunk",
            &chunk_id(reference, offset),
            1,
            false,
            &serde_json::to_string(&fact)?,
            at,
        )?;
    }
    enqueue_sync_change(
        connection,
        "canonical_text_seal",
        reference.content_id(),
        1,
        false,
        &serde_json::to_string(reference)?,
        at,
    )
}

pub(super) fn apply(connection: &Connection, change: &RemoteSyncChangeInput) -> StoreResult<()> {
    if change.deleted || change.revision != 1 || change.payload_json.len() > 256 * 1024 {
        return Err(StoreError::Invariant(
            "invalid immutable content fact".into(),
        ));
    }
    if change.entity_type == "canonical_text_chunk" {
        let fact: ChunkFact = serde_json::from_str(&change.payload_json)?;
        if fact.reference.total_utf8_bytes() > MAX_REPLICA_CONTENT_BYTES
            || fact.reference.chunk_count() > 16_384
            || change.entity_id != chunk_id(&fact.reference, fact.offset_bytes)
            || fact.text.is_empty()
            || fact.text.len() as u64 > CANONICAL_BODY_CHUNK_MAX_BYTES
            || fact
                .offset_bytes
                .checked_add(fact.text.len() as u64)
                .is_none_or(|end| end > fact.reference.total_utf8_bytes())
        {
            return Err(StoreError::Invariant(
                "invalid replica content chunk".into(),
            ));
        }
        let encoded = serde_json::to_string(&fact.reference)?;
        // A locally prepared object may have no replica staging rows yet. Its
        // immutable owner and bytes still constrain the first incoming chunk.
        let canonical_owner: Option<String> = connection.query_row(
            "SELECT CASE WHEN length(CAST(reference_json AS BLOB))<=4096 THEN reference_json END FROM canonical_text_objects WHERE id=?1",
            params![fact.reference.content_id()], |row| row.get(0)).optional()?;
        if let Some(owner) = canonical_owner {
            if serde_json::from_str::<CanonicalBodyReference>(&owner)? != fact.reference {
                return Err(StoreError::Conflict(
                    "replica chunk canonical owner mismatch".into(),
                ));
            }
            let canonical_chunk: Option<String> = connection.query_row(
                "SELECT CASE WHEN length(CAST(text AS BLOB)) BETWEEN 1 AND 32768 THEN text END FROM canonical_text_chunks WHERE content_id=?1 AND offset_bytes=?2",
                params![fact.reference.content_id(),fact.offset_bytes as i64], |row| row.get(0)).optional()?;
            if canonical_chunk.as_deref() != Some(fact.text.as_str()) {
                return Err(StoreError::Conflict(
                    "replica chunk differs from prepared canonical bytes".into(),
                ));
            }
        }
        let existing: Option<(String, String)> = connection.query_row(
            "SELECT CASE WHEN length(CAST(reference_json AS BLOB))<=4096 THEN reference_json END,CASE WHEN length(CAST(text AS BLOB)) BETWEEN 1 AND 32768 THEN text END FROM canonical_text_replica_chunks WHERE content_id=?1 AND offset_bytes=?2",
            params![fact.reference.content_id(), fact.offset_bytes as i64], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((owner, text)) = existing {
            if owner != encoded || text != fact.text {
                return Err(StoreError::Conflict(
                    "replica content replay differs".into(),
                ));
            }
        } else {
            let geometry: (i64, i64, Option<String>) = connection.query_row(
                "SELECT COALESCE(SUM(length(CAST(text AS BLOB))),0),COUNT(*),MIN(CASE WHEN length(CAST(reference_json AS BLOB))<=4096 THEN reference_json END)
                 FROM canonical_text_replica_chunks WHERE content_id=?1", params![fact.reference.content_id()],
                |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
            if geometry.0 < 0
                || geometry.0 as u64 != fact.offset_bytes
                || geometry.1 >= i64::from(fact.reference.chunk_count())
                || (geometry.1 > 0 && geometry.2.as_deref() != Some(encoded.as_str()))
            {
                return Err(StoreError::Conflict(
                    "replica chunks must preserve contiguous owner-bound order".into(),
                ));
            }
            if geometry.1 == 0 {
                let (reserved, canonical): (i64, bool) = connection.query_row(
                    "SELECT
                       COALESCE((SELECT SUM(json_extract(reference_json,'$.total_utf8_bytes')) FROM canonical_text_objects),0)
                       + COALESCE((SELECT SUM(bytes) FROM (
                           SELECT MAX(json_extract(s.reference_json,'$.total_utf8_bytes')) AS bytes
                           FROM canonical_text_replica_chunks s
                           WHERE NOT EXISTS(SELECT 1 FROM canonical_text_objects c WHERE c.id=s.content_id)
                           GROUP BY s.content_id)),0),
                       EXISTS(SELECT 1 FROM canonical_text_objects WHERE id=?1)",
                    params![fact.reference.content_id()], |row| Ok((row.get(0)?,row.get(1)?)))?;
                let additional = if canonical {
                    0
                } else {
                    fact.reference.total_utf8_bytes()
                };
                if reserved < 0
                    || (reserved as u64).saturating_add(additional) > MAX_REPLICA_CONTENT_BYTES
                {
                    return Err(StoreError::Invariant(
                        "replica staging quota exhausted".into(),
                    ));
                }
            }
            connection.execute("INSERT INTO canonical_text_replica_chunks(content_id,offset_bytes,reference_json,text) VALUES(?1,?2,?3,?4)",
                params![fact.reference.content_id(),fact.offset_bytes as i64,encoded,fact.text])?;
        }
    } else {
        let reference: CanonicalBodyReference = serde_json::from_str(&change.payload_json)?;
        if reference.total_utf8_bytes() > MAX_REPLICA_CONTENT_BYTES
            || reference.chunk_count() > 16_384
            || change.entity_id != reference.content_id()
        {
            return Err(StoreError::Invariant(
                "replica seal identity mismatch".into(),
            ));
        }
        if reference.field() == nola_domain::PartBodyField::ContentJson
            && connection.is_autocommit()
        {
            return Err(StoreError::Invariant(
                "JSON replica seal requires an owned transaction".into(),
            ));
        }
        validate_chunks(connection, &reference, true)?;
        let encoded = serde_json::to_string(&reference)?;
        let existing: Option<String> = connection.query_row("SELECT CASE WHEN length(CAST(reference_json AS BLOB))<=4096 THEN reference_json END FROM canonical_text_objects WHERE id=?1",
            params![reference.content_id()], |row| row.get(0)).optional()?;
        if let Some(existing) = existing {
            if existing != encoded {
                return Err(StoreError::Conflict("replica seal owner mismatch".into()));
            }
            let different: i64 = connection.query_row(
                "SELECT COUNT(*) FROM canonical_text_replica_chunks s LEFT JOIN canonical_text_chunks c
                 ON c.content_id=s.content_id AND c.offset_bytes=s.offset_bytes
                 WHERE s.content_id=?1 AND (c.text IS NULL OR c.text<>s.text)", params![reference.content_id()], |row| row.get(0))?;
            if different != 0 {
                return Err(StoreError::Conflict("replica sealed bytes differ".into()));
            }
        } else {
            connection.execute(
                "INSERT INTO canonical_text_objects(id,reference_json) VALUES(?1,?2)",
                params![reference.content_id(), encoded],
            )?;
            connection.execute("INSERT INTO canonical_text_chunks(content_id,offset_bytes,text)
                SELECT content_id,offset_bytes,text FROM canonical_text_replica_chunks WHERE content_id=?1", params![reference.content_id()])?;
        }
        // The receiving page still owns the transaction. JSON syntax is checked
        // for new seals, existing objects, and immutable operation replays alike.
        require_complete(connection, &reference)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(store: &mut NolaStore, text: &str) -> CanonicalBodyReference {
        store
            .create_conversation(&CreateConversationInput {
                id: "c".into(),
                title: "large".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        let chunks: Vec<&str> = text
            .as_bytes()
            .chunks(30_000)
            .map(|bytes| std::str::from_utf8(bytes).unwrap())
            .collect();
        let reference = CanonicalBodyReference::for_field(
            "body".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            text.len() as u64,
            chunks.len() as u32,
        )
        .unwrap();
        let tx = store.connection.transaction().unwrap();
        tx.execute(
            "INSERT INTO canonical_text_objects VALUES(?1,?2)",
            params![
                reference.content_id(),
                serde_json::to_string(&reference).unwrap()
            ],
        )
        .unwrap();
        let mut offset = 0;
        for chunk in chunks {
            tx.execute(
                "INSERT INTO canonical_text_chunks VALUES(?1,?2,?3)",
                params![reference.content_id(), offset, chunk],
            )
            .unwrap();
            offset += chunk.len() as i64;
        }
        let input = AppendMessagesInput {
            conversation_id: "c".into(),
            expected_message_count: 0,
            updated_at_ms: 2,
            messages: vec![MessageInput {
                id: "m".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "p".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: String::new(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            }],
        };
        crate::conversation_append_store::append_in_transaction(
            &tx,
            &input,
            Some(1),
            std::slice::from_ref(&reference),
        )
        .unwrap();
        tx.commit().unwrap();
        reference
    }

    fn page(entries: &[StoredSyncChange], cursor: i64) -> ApplyRemoteSyncPageInput {
        ApplyRemoteSyncPageInput {
            replica_id: "remote".into(),
            next_cursor: cursor + entries.len() as i64,
            applied_at_ms: 3,
            changes: entries
                .iter()
                .enumerate()
                .map(|(index, entry)| RemoteSyncChangeInput {
                    cursor: cursor + index as i64 + 1,
                    operation_id: entry.operation_id.clone(),
                    entity_type: entry.entity_type.clone(),
                    entity_id: entry.entity_id.clone(),
                    revision: entry.revision,
                    deleted: entry.deleted,
                    payload_json: entry.payload_json.clone(),
                    created_at_ms: entry.created_at_ms,
                })
                .collect(),
        }
    }

    #[test]
    fn reference_waits_for_ack_and_cross_database_restore_survives_chunk_restart() {
        let root = tempfile::tempdir().unwrap();
        let source_path = root.path().join("source.db");
        let destination_path = root.path().join("destination.db");
        let mut source = NolaStore::open(&source_path).unwrap();
        let body = "界".repeat(3_400_000);
        let reference = seed(&mut source, &body);
        let mut cursor = 0;
        let mut saw_seal = false;
        loop {
            let entries = source.pending_sync_changes(37).unwrap();
            if entries.is_empty() {
                break;
            }
            assert!(entries
                .iter()
                .all(|entry| entry.payload_json.len() < 200_000));
            if !saw_seal {
                assert!(entries
                    .iter()
                    .all(|entry| entry.entity_type != "conversation_append"));
            }
            let mut destination = NolaStore::open(&destination_path).unwrap();
            let applied = destination
                .apply_remote_sync_page(&page(&entries, cursor))
                .unwrap();
            cursor = applied.cursor;
            for entry in &entries {
                source
                    .resolve_sync_change(&entry.operation_id, "acknowledged", Some(cursor), 3)
                    .unwrap();
                saw_seal |= entry.entity_type == "canonical_text_seal";
            }
            if entries
                .iter()
                .all(|entry| entry.entity_type != "conversation_append")
            {
                let visible: i64 = destination
                    .connection
                    .query_row("SELECT COUNT(*) FROM message_parts", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(visible, 0);
            }
            // Both endpoints restart between real sync pages.
            drop(destination);
            drop(source);
            source = NolaStore::open(&source_path).unwrap();
        }
        assert!(saw_seal);
        let destination = NolaStore::open(&destination_path).unwrap();
        require_complete(&destination.connection, &reference).unwrap();
        let (inline, bound): (String, String) = destination.connection.query_row(
            "SELECT p.content_text,r.reference_json FROM message_parts p JOIN message_part_body_references r ON r.part_id=p.id AND r.field='content_text' WHERE p.id='p'", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert!(inline.is_empty());
        assert_eq!(
            serde_json::from_str::<CanonicalBodyReference>(&bound).unwrap(),
            reference
        );
        let mut statement = destination.connection.prepare("SELECT offset_bytes,text FROM canonical_text_chunks WHERE content_id='body' ORDER BY offset_bytes").unwrap();
        let chunks = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap();
        let mut length = 0;
        for chunk in chunks {
            let (offset, text) = chunk.unwrap();
            assert_eq!(offset as usize, length);
            assert_eq!(text, body[length..length + text.len()]);
            length += text.len();
        }
        assert_eq!(length, body.len());
        assert!(
            destination.pending_sync_changes(10).unwrap().is_empty(),
            "remote content must not echo"
        );
    }

    #[test]
    fn missing_chunk_rejects_seal_without_advancing_cursor_or_publishing_part() {
        let root = tempfile::tempdir().unwrap();
        let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
        seed(&mut source, &"界".repeat(20_000));
        let entries = source.pending_sync_changes(100).unwrap();
        let filtered: Vec<_> = entries
            .into_iter()
            .filter(|entry| {
                entry.entity_type != "canonical_text_chunk" || entry.entity_id.ends_with(":0")
            })
            .collect();
        let mut destination = NolaStore::open(root.path().join("destination.db")).unwrap();
        assert!(destination
            .apply_remote_sync_page(&page(&filtered, 0))
            .unwrap_err()
            .to_string()
            .contains("incomplete"));
        assert!(destination.sync_cursor("remote").unwrap().is_none());
        let objects: i64 = destination
            .connection
            .query_row("SELECT COUNT(*) FROM canonical_text_objects", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(objects, 0);
    }
    #[test]
    fn changed_replay_and_foreign_reference_leave_prior_content_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
        let reference = seed(&mut source, &"界".repeat(20_000));
        let entries = source.pending_sync_changes(100).unwrap();
        let mut destination = NolaStore::open(root.path().join("destination.db")).unwrap();
        let initial = page(&entries, 0);
        destination.apply_remote_sync_page(&initial).unwrap();
        let mut replay = initial
            .changes
            .iter()
            .find(|change| change.entity_type == "canonical_text_chunk")
            .unwrap()
            .clone();
        replay.cursor = initial.next_cursor + 1;
        // Reusing the original operation ID must not hide changed bytes.
        let mut changed: serde_json::Value = serde_json::from_str(&replay.payload_json).unwrap();
        changed["text"] = "x".into();
        replay.payload_json = serde_json::to_string(&changed).unwrap();
        let altered = ApplyRemoteSyncPageInput {
            replica_id: "remote".into(),
            next_cursor: replay.cursor,
            applied_at_ms: 4,
            changes: vec![replay],
        };
        assert!(destination.apply_remote_sync_page(&altered).is_err());
        assert_eq!(
            destination.sync_cursor("remote").unwrap().unwrap().cursor,
            initial.next_cursor
        );
        require_complete(&destination.connection, &reference).unwrap();
        let foreign = CanonicalBodyReference::for_field(
            "body".into(),
            "foreign".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            reference.total_utf8_bytes(),
            reference.chunk_count(),
        )
        .unwrap();
        assert!(require_complete(&destination.connection, &foreign).is_err());
    }

    #[test]
    fn distinct_incomplete_content_reservations_share_one_quota() {
        let root = tempfile::tempdir().unwrap();
        let mut destination = NolaStore::open(root.path().join("destination.db")).unwrap();
        for (index, megabytes) in [200_u64, 100].into_iter().enumerate() {
            let reference = CanonicalBodyReference::for_field(
                format!("body-{index}"),
                "c".into(),
                format!("m-{index}"),
                format!("p-{index}"),
                nola_domain::PartBodyField::ContentText,
                megabytes * 1024 * 1024,
                (megabytes * 32) as u32,
            )
            .unwrap();
            let fact = ChunkFact {
                reference: reference.clone(),
                offset_bytes: 0,
                text: "x".into(),
            };
            let input = ApplyRemoteSyncPageInput {
                replica_id: "remote".into(),
                next_cursor: index as i64 + 1,
                applied_at_ms: 1,
                changes: vec![RemoteSyncChangeInput {
                    cursor: index as i64 + 1,
                    operation_id: format!("op-{index}"),
                    entity_type: "canonical_text_chunk".into(),
                    entity_id: chunk_id(&reference, 0),
                    revision: 1,
                    deleted: false,
                    payload_json: serde_json::to_string(&fact).unwrap(),
                    created_at_ms: 1,
                }],
            };
            let result = destination.apply_remote_sync_page(&input);
            if index == 0 {
                result.unwrap();
            } else {
                assert!(result.unwrap_err().to_string().contains("quota"));
            }
        }
        assert_eq!(
            destination.sync_cursor("remote").unwrap().unwrap().cursor,
            1
        );
        let count: i64 = destination
            .connection
            .query_row(
                "SELECT COUNT(DISTINCT content_id) FROM canonical_text_replica_chunks",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn receiving_reference_before_content_never_publishes_an_empty_part() {
        let root = tempfile::tempdir().unwrap();
        let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
        seed(&mut source, &"界".repeat(20_000));
        let base = source.pending_sync_changes(1).unwrap();
        let mut destination = NolaStore::open(root.path().join("destination.db")).unwrap();
        destination.apply_remote_sync_page(&page(&base, 0)).unwrap();
        let append: (String,String,i64) = source.connection.query_row(
            "SELECT operation_id,payload_json,revision FROM sync_outbox WHERE entity_type='conversation_append'",[],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        let input = ApplyRemoteSyncPageInput {
            replica_id: "remote".into(),
            next_cursor: 2,
            applied_at_ms: 3,
            changes: vec![RemoteSyncChangeInput {
                cursor: 2,
                operation_id: append.0,
                payload_json: append.1,
                revision: append.2,
                entity_type: "conversation_append".into(),
                entity_id: "c".into(),
                deleted: false,
                created_at_ms: 2,
            }],
        };
        assert!(destination.apply_remote_sync_page(&input).is_err());
        assert_eq!(
            destination.sync_cursor("remote").unwrap().unwrap().cursor,
            1
        );
        let parts: i64 = destination
            .connection
            .query_row("SELECT COUNT(*) FROM message_parts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(parts, 0);
    }
}

#[cfg(test)]
#[path = "canonical_body_replica_tests.rs"]
mod body_replica_tests;
