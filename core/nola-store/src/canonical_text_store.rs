// SPDX-License-Identifier: Apache-2.0
//! Immutable canonical content storage, owned exclusively by Application Service.
//! Content is committed before a separate canonical-message transaction publishes
//! its reference. Unreferenced objects are not messages and require explicit GC.

use rusqlite::TransactionBehavior;

use super::{params, Connection, OptionalExtension, StoreError, StoreResult};
use nola_domain::{CanonicalBodyReference, CANONICAL_BODY_CHUNK_MAX_BYTES};
use std::path::Path;

const APPLICATION_ID: i64 = 0x4E544558;
const MAX_CHUNKS: u32 = 16_384;

pub struct CanonicalTextStore {
    connection: Connection,
    maximum_bytes: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct CanonicalTextRange {
    pub offset_bytes: u64,
    pub next_offset_bytes: u64,
    pub text: String,
    pub end_of_content: bool,
}

impl CanonicalTextStore {
    /// Query paths must not initialize missing storage or upgrade unrelated files.
    pub fn open_existing(path: &Path, maximum_bytes: u64) -> StoreResult<Self> {
        if maximum_bytes == 0 || maximum_bytes > i64::MAX as u64 {
            return Err(invalid("invalid canonical content quota"));
        }
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let application_id: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let tables: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )?;
        if application_id != APPLICATION_ID || version != 1 || tables != 2 {
            return Err(invalid(
                "not a canonical text store; no migration is performed",
            ));
        }
        // Verify the expected read surface without applying any schema commands.
        connection.prepare("SELECT id,reference_json FROM canonical_text_objects LIMIT 0")?;
        connection
            .prepare("SELECT content_id,offset_bytes,text FROM canonical_text_chunks LIMIT 0")?;
        Ok(Self {
            connection,
            maximum_bytes,
        })
    }

    pub fn open(path: &Path, maximum_bytes: u64) -> StoreResult<Self> {
        if maximum_bytes == 0 || maximum_bytes > i64::MAX as u64 {
            return Err(invalid("invalid canonical content quota"));
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let tables: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| row.get(0),
        )?;
        let application_id: i64 =
            connection.pragma_query_value(None, "application_id", |row| row.get(0))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if tables == 0 && application_id == 0 && version == 0 {
            connection.execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; BEGIN IMMEDIATE;
                 CREATE TABLE canonical_text_objects(
                    id TEXT PRIMARY KEY, reference_json TEXT NOT NULL);
                 CREATE TABLE canonical_text_chunks(
                    content_id TEXT NOT NULL REFERENCES canonical_text_objects(id) ON DELETE CASCADE,
                    offset_bytes INTEGER NOT NULL CHECK(offset_bytes>=0),
                    text TEXT NOT NULL CHECK(length(CAST(text AS BLOB)) BETWEEN 1 AND 32768),
                    PRIMARY KEY(content_id,offset_bytes));")?;
            connection.pragma_update(None, "application_id", APPLICATION_ID)?;
            connection.pragma_update(None, "user_version", 1)?;
            connection.execute_batch("COMMIT;")?;
        } else if application_id != APPLICATION_ID || version != 1 || tables != 2 {
            return Err(invalid(
                "not a canonical text store; no migration is performed",
            ));
        }
        connection.pragma_update(None, "foreign_keys", true)?;
        Ok(Self {
            connection,
            maximum_bytes,
        })
    }

    /// A retry must repeat the same reference AND bytes. There is no overwrite
    /// operation, and a failed iterator rolls the entire object back.
    pub fn put<I>(&mut self, reference: &CanonicalBodyReference, chunks: I) -> StoreResult<()>
    where
        I: IntoIterator<Item = StoreResult<(u64, String)>>,
    {
        crate::canonical_body::require_text_reference(reference)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        put_chunks(&tx, reference, chunks, self.maximum_bytes)?;
        tx.commit()?;
        Ok(())
    }

    /// Application must first resolve this reference from the canonical Part.
    /// Client-supplied owner strings alone are not authorization.
    pub fn read_range(
        &self,
        reference: &CanonicalBodyReference,
        offset: u64,
        limit: usize,
    ) -> StoreResult<CanonicalTextRange> {
        crate::canonical_body::require_text_reference(reference)?;
        read_range_from_connection(&self.connection, reference, offset, limit)
    }
}

pub(crate) fn validate_reference_from_connection(
    connection: &Connection,
    reference: &CanonicalBodyReference,
) -> StoreResult<()> {
    let stored: Option<String> = connection.query_row(
            "SELECT CASE WHEN length(CAST(reference_json AS BLOB)) <= 4096 THEN reference_json END FROM canonical_text_objects WHERE id=?1",
            params![reference.content_id()], |row| row.get(0)).optional()?;
    let stored: CanonicalBodyReference = serde_json::from_str(
        stored
            .as_deref()
            .ok_or_else(|| invalid("canonical content unavailable"))?,
    )
    .map_err(|error| invalid(&error.to_string()))?;
    if stored != *reference {
        return Err(StoreError::Conflict("canonical reference mismatch".into()));
    }
    Ok(())
}

pub(crate) fn read_range_from_connection(
    connection: &Connection,
    reference: &CanonicalBodyReference,
    offset: u64,
    limit: usize,
) -> StoreResult<CanonicalTextRange> {
    if !(4..=CANONICAL_BODY_CHUNK_MAX_BYTES as usize).contains(&limit)
        || offset > reference.total_utf8_bytes()
    {
        return Err(invalid("invalid canonical text range"));
    }
    validate_reference_from_connection(connection, reference)?;
    if offset == reference.total_utf8_bytes() {
        return Ok(CanonicalTextRange {
            offset_bytes: offset,
            next_offset_bytes: offset,
            text: String::new(),
            end_of_content: true,
        });
    }
    let mut statement = connection.prepare(
            "SELECT offset_bytes,CASE WHEN length(CAST(text AS BLOB)) BETWEEN 1 AND 32768 THEN text END FROM canonical_text_chunks WHERE content_id=?1
             AND offset_bytes >= COALESCE((SELECT MAX(offset_bytes) FROM canonical_text_chunks
                 WHERE content_id=?1 AND offset_bytes<=?2),0)
             AND offset_bytes < ?3 ORDER BY offset_bytes")?;
    let mut rows = statement.query(params![
        reference.content_id(),
        offset as i64,
        offset
            .saturating_add(limit as u64)
            .min(reference.total_utf8_bytes()) as i64
    ])?;
    let mut result = String::with_capacity(limit);
    let mut cursor = offset;
    let mut stopped_at_utf8_boundary = false;
    while let Some(row) = rows.next()? {
        let start: i64 = row.get(0)?;
        let text: String = row.get(1)?;
        if start < 0
            || start as u64 > cursor
            || (!result.is_empty() && start as u64 != cursor)
            || text.len() as u64 > CANONICAL_BODY_CHUNK_MAX_BYTES
        {
            return Err(invalid("invalid persisted canonical chunk"));
        }
        let skip =
            usize::try_from(cursor - start as u64).map_err(|_| invalid("invalid chunk offset"))?;
        if skip >= text.len() || !text.is_char_boundary(skip) {
            return Err(invalid("offset is not a canonical UTF-8 boundary"));
        }
        let mut end = (skip + limit - result.len()).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        result.push_str(&text[skip..end]);
        cursor += (end - skip) as u64;
        if end < text.len() {
            stopped_at_utf8_boundary = true;
            break;
        }
        if result.len() == limit {
            break;
        }
    }
    if result.is_empty() || cursor > reference.total_utf8_bytes() {
        return Err(invalid("canonical range made no valid progress"));
    }
    let expected_end = offset
        .saturating_add(limit as u64)
        .min(reference.total_utf8_bytes());
    if cursor < expected_end && !stopped_at_utf8_boundary {
        return Err(invalid("canonical range is missing persisted chunks"));
    }
    Ok(CanonicalTextRange {
        offset_bytes: offset,
        next_offset_bytes: cursor,
        text: result,
        end_of_content: cursor == reference.total_utf8_bytes(),
    })
}

fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_tail_chunk_is_not_returned_as_a_successful_partial_range() {
        let mut store = CanonicalTextStore::open(Path::new(":memory:"), 100).unwrap();
        let reference = reference(6, 2);
        store
            .put(&reference, [Ok((0, "abc".into())), Ok((3, "def".into()))])
            .unwrap();
        store
            .connection
            .execute("DELETE FROM canonical_text_chunks WHERE offset_bytes=3", [])
            .unwrap();
        let error = store.read_range(&reference, 0, 6).unwrap_err();
        assert!(error.to_string().contains("missing persisted chunks"));
        assert!(store.search_literal(&reference, "abc").is_err());
    }

    #[test]
    fn utf8_budget_short_read_is_distinct_from_missing_content() {
        let mut store = CanonicalTextStore::open(Path::new(":memory:"), 100).unwrap();
        let reference = reference(7, 2);
        store
            .put(&reference, [Ok((0, "abc".into())), Ok((3, "😀".into()))])
            .unwrap();
        let first = store.read_range(&reference, 0, 4).unwrap();
        assert_eq!(first.text, "abc");
        assert_eq!(first.next_offset_bytes, 3);
        assert!(!first.end_of_content);
        let second = store
            .read_range(&reference, first.next_offset_bytes, 4)
            .unwrap();
        assert_eq!(second.text, "😀");
        assert!(second.end_of_content);
    }
    fn quota_reference(id: &str, bytes: u64) -> CanonicalBodyReference {
        CanonicalBodyReference::for_field(
            id.into(),
            "conversation".into(),
            format!("message-{id}"),
            format!("part-{id}"),
            nola_domain::PartBodyField::ContentText,
            bytes,
            1,
        )
        .unwrap()
    }

    #[test]
    fn quota_is_shared_across_objects_and_reopened_writer_handles() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("quota.db");
        let mut first = CanonicalTextStore::open(&path, 100).unwrap();
        let mut second = CanonicalTextStore::open(&path, 100).unwrap();
        let a = quota_reference("a", 60);
        first.put(&a, [Ok((0, "a".repeat(60)))]).unwrap();
        let oversized = quota_reference("b", 60);
        let error = second
            .put(&oversized, [Ok((0, "b".repeat(60)))])
            .unwrap_err();
        assert!(error.to_string().contains("quota exhausted"));
        assert!(second.read_range(&oversized, 0, 60).is_err());
        let b = quota_reference("b", 40);
        second.put(&b, [Ok((0, "b".repeat(40)))]).unwrap();
        // Exact replay at capacity does not allocate another object.
        first.put(&a, [Ok((0, "a".repeat(60)))]).unwrap();
        drop(first);
        let mut reopened = CanonicalTextStore::open(&path, 100).unwrap();
        assert_eq!(reopened.read_range(&a, 0, 60).unwrap().text, "a".repeat(60));
        assert!(reopened
            .put(&quota_reference("c", 1), [Ok((0, "c".into()))])
            .is_err());
    }

    #[test]
    fn failed_chunk_stream_releases_its_entire_quota_reservation() {
        let root = tempfile::tempdir().unwrap();
        let mut store = CanonicalTextStore::open(&root.path().join("rollback.db"), 100).unwrap();
        let failed = CanonicalBodyReference::for_field(
            "failed".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            100,
            2,
        )
        .unwrap();
        assert!(store
            .put(
                &failed,
                [Ok((0, "a".repeat(50))), Err(invalid("source disconnected"))]
            )
            .is_err());
        let complete = quota_reference("complete", 100);
        store.put(&complete, [Ok((0, "b".repeat(100)))]).unwrap();
        assert_eq!(store.read_range(&complete, 0, 100).unwrap().text.len(), 100);
        assert!(store.read_range(&failed, 0, 100).is_err());
    }
    #[test]
    fn corrupted_oversized_chunk_is_rejected_before_text_decoding() {
        let mut store = CanonicalTextStore::open(Path::new(":memory:"), 100).unwrap();
        let reference = reference(6, 1);
        store.put(&reference, [Ok((0, "abcdef".into()))]).unwrap();
        store
            .connection
            .execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE canonical_text_chunks SET text=?1",
                ["x".repeat(100_000)],
            )
            .unwrap();
        assert!(store.read_range(&reference, 0, 6).is_err());
        assert!(store.put(&reference, [Ok((0, "abcdef".into()))]).is_err());
    }

    #[test]
    fn overlapping_chunks_are_not_silently_spliced() {
        let mut store = CanonicalTextStore::open(Path::new(":memory:"), 100).unwrap();
        let reference = reference(6, 2);
        store
            .put(&reference, [Ok((0, "abc".into())), Ok((3, "def".into()))])
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE canonical_text_chunks SET offset_bytes=2 WHERE offset_bytes=3",
                [],
            )
            .unwrap();
        assert!(store.read_range(&reference, 0, 6).is_err());
    }

    #[test]
    fn read_only_open_never_creates_missing_content_storage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.db");
        assert!(CanonicalTextStore::open_existing(&path, 100).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn read_only_open_rejects_foreign_database_without_modifying_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("foreign.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE retained(value TEXT); INSERT INTO retained VALUES('unchanged');",
            )
            .unwrap();
        assert!(CanonicalTextStore::open_existing(&path, 100).is_err());
        let value: String = connection
            .query_row("SELECT value FROM retained", [], |row| row.get(0))
            .unwrap();
        let tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "unchanged");
        assert_eq!(tables, 1);
    }

    #[test]
    fn read_only_handle_reads_content_but_cannot_publish_new_objects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("content.db");
        let reference = reference(6, 1);
        let mut writer = CanonicalTextStore::open(&path, 100).unwrap();
        writer.put(&reference, [Ok((0, "abcdef".into()))]).unwrap();
        drop(writer);
        let mut reader = CanonicalTextStore::open_existing(&path, 100).unwrap();
        assert_eq!(reader.read_range(&reference, 0, 6).unwrap().text, "abcdef");
        let other = CanonicalBodyReference::for_field(
            "other".into(),
            "conversation".into(),
            "other-message".into(),
            "other-part".into(),
            nola_domain::PartBodyField::ContentText,
            6,
            1,
        )
        .unwrap();
        assert!(reader.put(&other, [Ok((0, "abcdef".into()))]).is_err());
        assert!(reader.read_range(&other, 0, 6).is_err());
    }

    fn reference(bytes: u64, count: u32) -> CanonicalBodyReference {
        CanonicalBodyReference::for_field(
            "content".into(),
            "conversation".into(),
            "message".into(),
            "part".into(),
            nola_domain::PartBodyField::ContentText,
            bytes,
            count,
        )
        .unwrap()
    }

    #[test]
    fn immutable_unicode_content_replays_and_reads_across_chunk_boundaries_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("content.db");
        let reference = reference(12, 3);
        let chunks =
            || [(0, "ab中"), (5, "😀"), (9, "end")].map(|(offset, text)| Ok((offset, text.into())));
        let mut store = CanonicalTextStore::open(&path, 100).unwrap();
        store.put(&reference, chunks()).unwrap();
        store.put(&reference, chunks()).unwrap();
        assert!(store
            .put(
                &reference,
                [
                    (Ok((0, "xxxxx".into()))),
                    Ok((5, "😀".into())),
                    Ok((9, "end".into()))
                ]
            )
            .is_err());
        drop(store);
        let store = CanonicalTextStore::open(&path, 100).unwrap();
        let mut offset = 0;
        let mut restored = String::new();
        while offset < 12 {
            let page = store.read_range(&reference, offset, 4).unwrap();
            assert!(page.text.len() <= 4);
            restored.push_str(&page.text);
            offset = page.next_offset_bytes;
        }
        assert_eq!(restored, "ab中😀end");
        assert!(store.read_range(&reference, 3, 4).is_err());
        assert!(store.read_range(&reference, 12, 4).unwrap().end_of_content);
        let foreign = CanonicalBodyReference::for_field(
            "content".into(),
            "foreign".into(),
            "message".into(),
            "part".into(),
            nola_domain::PartBodyField::ContentText,
            12,
            3,
        )
        .unwrap();
        assert!(store.read_range(&foreign, 0, 4).is_err());
    }

    #[test]
    fn failed_iterator_does_not_publish_partial_content() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("content.db");
        let reference = reference(6, 2);
        let mut store = CanonicalTextStore::open(&path, 100).unwrap();
        assert!(store
            .put(
                &reference,
                [
                    Ok((0, "abc".into())),
                    Err(invalid("injected source failure"))
                ]
            )
            .is_err());
        drop(store);
        let mut store = CanonicalTextStore::open(&path, 100).unwrap();
        assert!(store.read_range(&reference, 0, 4).is_err());
        store
            .put(&reference, [Ok((0, "abc".into())), Ok((3, "def".into()))])
            .unwrap();
        assert_eq!(store.read_range(&reference, 0, 6).unwrap().text, "abcdef");
    }

    #[test]
    fn ten_million_bytes_are_never_assembled_by_storage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("content.db");
        let total = 10_000_000_u64;
        let reference = reference(total, total.div_ceil(CANONICAL_BODY_CHUNK_MAX_BYTES) as u32);
        let mut store = CanonicalTextStore::open(&path, total).unwrap();
        store
            .put(
                &reference,
                (0..reference.chunk_count()).map(|index| {
                    let offset = u64::from(index) * CANONICAL_BODY_CHUNK_MAX_BYTES;
                    Ok((
                        offset,
                        "x".repeat((total - offset).min(CANONICAL_BODY_CHUNK_MAX_BYTES) as usize),
                    ))
                }),
            )
            .unwrap();
        drop(store);
        let store = CanonicalTextStore::open(&path, total).unwrap();
        for offset in [0, 32_766, 8_000_003, total - 17] {
            let page = store.read_range(&reference, offset, 4096).unwrap();
            assert_eq!(page.text.len(), (total - offset).min(4096) as usize);
            assert!(page.text.bytes().all(|byte| byte == b'x'));
        }
    }
}

/// Caller owns the transaction containing this immutable-object copy.
pub(crate) fn put_chunks<I>(
    tx: &Connection,
    reference: &CanonicalBodyReference,
    chunks: I,
    maximum_bytes: u64,
) -> StoreResult<()>
where
    I: IntoIterator<Item = StoreResult<(u64, String)>>,
{
    if reference.field() == nola_domain::PartBodyField::ContentJson && tx.is_autocommit() {
        return Err(invalid(
            "JSON body preparation requires an owned transaction",
        ));
    }
    if reference.total_utf8_bytes() > maximum_bytes || reference.chunk_count() > MAX_CHUNKS {
        return Err(invalid("canonical content exceeds configured limits"));
    }
    let encoded = serde_json::to_string(reference).map_err(|error| invalid(&error.to_string()))?;
    let existing: Option<String> = tx.query_row(
            "SELECT CASE WHEN length(CAST(reference_json AS BLOB)) <= 4096 THEN reference_json END FROM canonical_text_objects WHERE id=?1",
            params![reference.content_id()], |row| row.get(0)).optional()?;
    if existing.as_deref().is_some_and(|value| value != encoded) {
        return Err(StoreError::Conflict(
            "canonical content identity already belongs to another reference".into(),
        ));
    }
    if existing.is_none() {
        // The quota is for all canonical text, not independently reusable
        // by every object. Admission shares the writer transaction with
        // publication so competing handles cannot reserve the same space.
        let allocated: i64 = tx.query_row(
            "SELECT COALESCE(SUM(json_extract(reference_json,'$.total_utf8_bytes')),0)
                 FROM canonical_text_objects",
            [],
            |row| row.get(0),
        )?;
        if allocated < 0
            || (allocated as u64)
                .checked_add(reference.total_utf8_bytes())
                .is_none_or(|total| total > maximum_bytes)
        {
            return Err(invalid("canonical content store quota exhausted"));
        }
        tx.execute(
            "INSERT INTO canonical_text_objects VALUES(?1,?2)",
            params![reference.content_id(), encoded],
        )?;
    }
    let mut next = 0_u64;
    let mut count = 0_u32;
    for chunk in chunks {
        let (offset, text) = chunk?;
        count = count
            .checked_add(1)
            .ok_or_else(|| invalid("canonical chunk count overflow"))?;
        if offset != next
            || text.is_empty()
            || text.len() as u64 > CANONICAL_BODY_CHUNK_MAX_BYTES
            || count > reference.chunk_count()
        {
            return Err(invalid("canonical chunks must be contiguous and bounded"));
        }
        next = next
            .checked_add(text.len() as u64)
            .ok_or_else(|| invalid("canonical content length overflow"))?;
        if next > reference.total_utf8_bytes() {
            return Err(invalid("canonical content exceeds declared length"));
        }
        if existing.is_some() {
            let previous: Option<String> = tx.query_row(
                    "SELECT CASE WHEN length(CAST(text AS BLOB)) BETWEEN 1 AND 32768 THEN text END FROM canonical_text_chunks WHERE content_id=?1 AND offset_bytes=?2",
                    params![reference.content_id(), offset as i64], |row| row.get(0)).optional()?;
            if previous.as_deref() != Some(text.as_str()) {
                return Err(StoreError::Conflict(
                    "canonical content replay differs".into(),
                ));
            }
        } else {
            tx.execute(
                "INSERT INTO canonical_text_chunks VALUES(?1,?2,?3)",
                params![reference.content_id(), offset as i64, text],
            )?;
        }
    }
    if next != reference.total_utf8_bytes() || count != reference.chunk_count() {
        return Err(invalid("canonical content is incomplete"));
    }
    if reference.field() == nola_domain::PartBodyField::ContentJson {
        crate::canonical_body::validate_json_object(tx, reference)?;
    }
    Ok(())
}
