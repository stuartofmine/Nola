// SPDX-License-Identifier: Apache-2.0
//! Durable, bounded upload staging. Only the Application Service owns this store.
//! Sealing does not publish canonical messages; import finalization is separate.

use rusqlite::TransactionBehavior;

use super::{params, Connection, OptionalExtension, StoreError, StoreResult};
use std::path::Path;

const MAX_CHUNK_BYTES: usize = 32 * 1024;
const MAX_STAGED_BYTES: i64 = 1024 * 1024 * 1024;
const MAX_STAGED_UPLOADS: i64 = 128;

pub use nola_store_port::content_upload::{UploadOwner, UploadProgress};

#[path = "content_upload_lifecycle.rs"]
mod lifecycle;

pub struct ContentUploadStore {
    connection: Connection,
    maximum_upload_bytes: u64,
}

impl ContentUploadStore {
    /// Opens an existing staging database without schema or journal-mode writes.
    pub fn open_existing(path: &Path, maximum_upload_bytes: u64) -> StoreResult<Self> {
        if maximum_upload_bytes == 0 || maximum_upload_bytes > i64::MAX as u64 {
            return Err(invalid("invalid upload quota"));
        }
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        Ok(Self {
            connection,
            maximum_upload_bytes,
        })
    }

    pub fn open(path: &Path, maximum_upload_bytes: u64) -> StoreResult<Self> {
        if maximum_upload_bytes == 0 || maximum_upload_bytes > i64::MAX as u64 {
            return Err(invalid("invalid upload quota"));
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS uploads(
                id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL,
                import_id TEXT NOT NULL, part_id TEXT NOT NULL,
                expected_bytes INTEGER NOT NULL CHECK(expected_bytes>0),
                committed_bytes INTEGER NOT NULL DEFAULT 0,
                sealed INTEGER NOT NULL DEFAULT 0 CHECK(sealed IN (0,1)));
             CREATE TABLE IF NOT EXISTS upload_chunks(
                upload_id TEXT NOT NULL REFERENCES uploads(id) ON DELETE CASCADE,
                offset_bytes INTEGER NOT NULL, text TEXT NOT NULL,
                PRIMARY KEY(upload_id,offset_bytes));",
        )?;
        Ok(Self {
            connection,
            maximum_upload_bytes,
        })
    }

    pub fn begin(
        &mut self,
        id: &str,
        owner: &UploadOwner,
        expected: u64,
    ) -> StoreResult<UploadProgress> {
        validate_identity(id, owner)?;
        self.initialize_retirement_schema()?;
        if expected == 0 || expected > self.maximum_upload_bytes {
            return Err(invalid("upload length exceeds quota"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO uploads(id,conversation_id,import_id,part_id,expected_bytes)
             VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO NOTHING",
            params![
                id,
                owner.conversation_id,
                owner.import_id,
                owner.part_id,
                expected as i64
            ],
        )?;
        let progress = load(&tx, id, owner)?;
        if progress.cancelled {
            return Err(conflict("upload is cancelled"));
        }
        if progress.expected_bytes != expected {
            return Err(conflict("upload identity already has a different length"));
        }
        // Reserve declared bytes, not only received bytes, while holding the
        // writer lock. Otherwise concurrent empty uploads can all pass quota
        // checks and later consume unbounded disk space. Sealed content remains
        // charged until an explicit safe lifecycle operation releases it.
        let (count, reserved): (i64, i64) = tx.query_row(
            "SELECT COUNT(*),COALESCE(SUM(expected_bytes),0) FROM uploads WHERE committed_bytes>=0 AND NOT EXISTS(SELECT 1 FROM upload_retirements r WHERE r.upload_id=uploads.id)",
            [], |row| Ok((row.get(0)?,row.get(1)?)),
        )?;
        if count > MAX_STAGED_UPLOADS || reserved > MAX_STAGED_BYTES {
            return Err(invalid("staging capacity exhausted"));
        }
        tx.commit()?;
        Ok(progress)
    }

    pub fn progress(&self, id: &str, owner: &UploadOwner) -> StoreResult<UploadProgress> {
        validate_identity(id, owner)?;
        load(&self.connection, id, owner)
    }

    pub fn append(
        &mut self,
        id: &str,
        owner: &UploadOwner,
        offset: u64,
        text: &str,
    ) -> StoreResult<UploadProgress> {
        validate_identity(id, owner)?;
        if text.is_empty() || text.len() > MAX_CHUNK_BYTES {
            return Err(invalid("invalid chunk length"));
        }
        let end = offset
            .checked_add(text.len() as u64)
            .ok_or_else(|| invalid("offset overflow"))?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut progress = load(&tx, id, owner)?;
        if progress.cancelled {
            return Err(conflict("upload is cancelled"));
        }
        if progress.sealed {
            return Err(conflict("upload is sealed"));
        }
        if end > progress.expected_bytes {
            return Err(invalid("chunk exceeds declared length"));
        }
        if offset < progress.committed_bytes {
            let previous: Option<String> = tx
                .query_row(
                    "SELECT text FROM upload_chunks WHERE upload_id=?1 AND offset_bytes=?2",
                    params![id, offset as i64],
                    |r| r.get(0),
                )
                .optional()?;
            if previous.as_deref() != Some(text) {
                return Err(conflict("chunk replay differs"));
            }
        } else {
            if offset != progress.committed_bytes {
                return Err(conflict("chunk gap"));
            }
            tx.execute(
                "INSERT INTO upload_chunks VALUES(?1,?2,?3)",
                params![id, offset as i64, text],
            )?;
            tx.execute(
                "UPDATE uploads SET committed_bytes=?2 WHERE id=?1",
                params![id, end as i64],
            )?;
            progress.committed_bytes = end;
        }
        tx.commit()?;
        Ok(progress)
    }

    pub fn seal(&mut self, id: &str, owner: &UploadOwner) -> StoreResult<UploadProgress> {
        validate_identity(id, owner)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut progress = load(&tx, id, owner)?;
        if progress.cancelled {
            return Err(conflict("upload is cancelled"));
        }
        if progress.committed_bytes != progress.expected_bytes {
            return Err(conflict("upload is incomplete"));
        }
        if !lifecycle::is_retired(&tx, id)? {
            validate_complete_chunks(&tx, id, progress.expected_bytes)?;
        }
        if !progress.sealed {
            tx.execute("UPDATE uploads SET sealed=1 WHERE id=?1", params![id])?;
        }
        progress.sealed = true;
        tx.commit()?;
        Ok(progress)
    }

    /// Copy immutable sealed data into canonical content storage before the
    /// Application transaction publishes a Part reference. Keep the source for
    /// retry; successful copying alone does not authorize staging reclamation.
    pub fn sealed_content_reference(
        &self,
        id: &str,
        owner: &UploadOwner,
        content_id: String,
        message_id: String,
    ) -> StoreResult<nola_domain::CanonicalBodyReference> {
        validate_identity(id, owner)?;
        let snapshot = self.connection.unchecked_transaction()?;
        let progress = load(&snapshot, id, owner)?;
        if !progress.sealed || progress.cancelled {
            return Err(conflict("canonical reference requires a sealed upload"));
        }
        // Derive geometry from the same snapshot as the owner and progress.
        // Never trust a client-supplied chunk count or load the complete text.
        validate_complete_chunks(&snapshot, id, progress.expected_bytes)?;
        let count: i64 = snapshot.query_row(
            "SELECT COUNT(*) FROM upload_chunks WHERE upload_id=?1",
            params![id],
            |row| row.get(0),
        )?;
        let count =
            u32::try_from(count).map_err(|_| invalid("invalid sealed upload chunk count"))?;
        let reference = nola_domain::CanonicalBodyReference::for_field(
            content_id,
            owner.conversation_id.clone(),
            message_id,
            owner.part_id.clone(),
            nola_domain::PartBodyField::ContentText,
            progress.expected_bytes,
            count,
        )
        .map_err(|error| invalid(&error.to_string()))?;
        snapshot.commit()?;
        Ok(reference)
    }

    /// Copy immutable sealed data into canonical content storage before the
    /// Application transaction publishes a Part reference. Keep the source for
    /// retry; successful copying alone does not authorize staging reclamation.
    pub fn copy_sealed_to_canonical(
        &self,
        id: &str,
        owner: &UploadOwner,
        reference: &nola_domain::CanonicalBodyReference,
        destination: &mut crate::canonical_text_store::CanonicalTextStore,
    ) -> StoreResult<()> {
        crate::canonical_body::require_text_reference(reference)?;
        validate_identity(id, owner)?;
        if reference.conversation_id() != owner.conversation_id
            || reference.part_id() != owner.part_id
        {
            return Err(conflict("canonical reference does not match upload owner"));
        }
        // Hold a consistent source snapshot through validation and consumption.
        // The destination has its own atomic transaction; a later publication
        // failure may leave an unreferenced object, never an incomplete message.
        let snapshot = self.connection.unchecked_transaction()?;
        let progress = load(&snapshot, id, owner)?;
        if !progress.sealed || progress.cancelled {
            return Err(conflict("canonical adoption requires a sealed upload"));
        }
        if progress.expected_bytes != reference.total_utf8_bytes() {
            return Err(conflict("canonical reference length differs from upload"));
        }
        validate_complete_chunks(&snapshot, id, progress.expected_bytes)?;
        {
            let mut statement = snapshot.prepare(
                "SELECT offset_bytes,text FROM upload_chunks WHERE upload_id=?1 ORDER BY offset_bytes"
            )?;
            let chunks = statement.query_map(params![id], |row| {
                let offset: i64 = row.get(0)?;
                Ok((offset as u64, row.get::<_, String>(1)?))
            })?;
            destination.put(
                reference,
                chunks.map(|chunk| chunk.map_err(StoreError::from)),
            )?;
        }
        snapshot.commit()?;
        Ok(())
    }

    /// Retain an owner-bound tombstone so delayed retries cannot resurrect an
    /// aborted upload. Tombstones still count against the bounded identity quota.
    /// A negative durable offset is reserved for cancellation, never exposed on wire.
    pub fn cancel(&mut self, id: &str, owner: &UploadOwner) -> StoreResult<UploadProgress> {
        validate_identity(id, owner)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut progress = load(&tx, id, owner)?;
        if progress.sealed {
            return Err(conflict("sealed upload cannot be cancelled"));
        }
        tx.execute("DELETE FROM upload_chunks WHERE upload_id=?1", params![id])?;
        tx.execute(
            "UPDATE uploads SET committed_bytes=-1 WHERE id=?1",
            params![id],
        )?;
        progress.committed_bytes = 0;
        progress.cancelled = true;
        tx.commit()?;
        Ok(progress)
    }

    /// Reads exactly one committed chunk at an acknowledged UTF-8 byte boundary.
    /// No API here assembles a full upload in memory.
    pub fn read_chunk(
        &self,
        id: &str,
        owner: &UploadOwner,
        offset: u64,
    ) -> StoreResult<Option<String>> {
        let progress = self.progress(id, owner)?;
        if progress.cancelled {
            return Err(conflict("upload is cancelled"));
        }
        if offset > progress.committed_bytes {
            return Err(invalid("read offset exceeds committed length"));
        }
        if offset == progress.committed_bytes {
            return Ok(None);
        }
        let text: Option<String> = self
            .connection
            .query_row(
                "SELECT text FROM upload_chunks WHERE upload_id=?1 AND offset_bytes=?2
             AND length(CAST(text AS BLOB)) BETWEEN 1 AND ?3",
                params![id, offset as i64, MAX_CHUNK_BYTES as i64],
                |r| r.get(0),
            )
            .optional()?;
        let Some(text) = text else {
            return Err(invalid("read offset has no valid bounded chunk"));
        };
        let end = offset
            .checked_add(text.len() as u64)
            .ok_or_else(|| invalid("chunk offset overflow"))?;
        if end > progress.committed_bytes {
            return Err(invalid("stored chunk exceeds committed length"));
        }
        Ok(Some(text))
    }
}

/// Verify structural integrity before sealed content can become an import input.
/// Only one offset/length pair is retained; no whole-upload String is allocated.
fn validate_complete_chunks(connection: &Connection, id: &str, expected: u64) -> StoreResult<()> {
    let mut statement = connection.prepare(
        "SELECT offset_bytes,length(CAST(text AS BLOB)) FROM upload_chunks
         WHERE upload_id=?1 ORDER BY offset_bytes",
    )?;
    let mut rows = statement.query(params![id])?;
    let mut next = 0_u64;
    while let Some(row) = rows.next()? {
        let offset: i64 = row.get(0)?;
        let length: i64 = row.get(1)?;
        if offset < 0 || offset as u64 != next || !(1..=MAX_CHUNK_BYTES as i64).contains(&length) {
            return Err(invalid(
                "stored upload chunks are not contiguous and bounded",
            ));
        }
        next = next
            .checked_add(length as u64)
            .ok_or_else(|| invalid("stored upload length overflow"))?;
        if next > expected {
            return Err(invalid("stored upload exceeds declared length"));
        }
    }
    if next != expected {
        return Err(invalid("stored upload is missing content"));
    }
    Ok(())
}

fn load(connection: &Connection, id: &str, owner: &UploadOwner) -> StoreResult<UploadProgress> {
    let row: Option<(String,String,String,i64,i64,bool)> = connection.query_row(
        "SELECT conversation_id,import_id,part_id,expected_bytes,committed_bytes,sealed FROM uploads WHERE id=?1",
        params![id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    ).optional()?;
    let Some((conversation, import, part, expected, committed, sealed)) = row else {
        return Err(invalid("unknown upload"));
    };
    if conversation != owner.conversation_id || import != owner.import_id || part != owner.part_id {
        return Err(conflict("upload owner mismatch"));
    }
    if expected <= 0 || committed < -1 || committed > expected || (sealed && committed != expected)
    {
        return Err(invalid("invalid stored upload progress"));
    }
    Ok(UploadProgress {
        expected_bytes: expected as u64,
        committed_bytes: committed.max(0) as u64,
        sealed,
        cancelled: committed < 0,
    })
}

fn validate_identity(id: &str, owner: &UploadOwner) -> StoreResult<()> {
    for value in [id, &owner.conversation_id, &owner.import_id, &owner.part_id] {
        if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            return Err(invalid("invalid upload identity"));
        }
    }
    Ok(())
}
fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}
fn conflict(message: &str) -> StoreError {
    StoreError::Conflict(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sealed_reference_uses_verified_owner_and_chunk_geometry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.db");
        let owner = owner();
        let mut source = ContentUploadStore::open(&path, 100).unwrap();
        source.begin("u", &owner, 8).unwrap();
        source.append("u", &owner, 0, "abcd").unwrap();
        assert!(source
            .sealed_content_reference("u", &owner, "content".into(), "message".into())
            .is_err());
        source.append("u", &owner, 4, "efgh").unwrap();
        source.seal("u", &owner).unwrap();
        drop(source);
        let source = ContentUploadStore::open_existing(&path, 100).unwrap();
        let reference = source
            .sealed_content_reference("u", &owner, "content".into(), "message".into())
            .unwrap();
        assert_eq!(reference.total_utf8_bytes(), 8);
        assert_eq!(reference.chunk_count(), 2);
        assert_eq!(reference.conversation_id(), owner.conversation_id);
        assert_eq!(reference.part_id(), owner.part_id);
        let mut foreign = owner.clone();
        foreign.import_id = "other-import".into();
        assert!(source
            .sealed_content_reference("u", &foreign, "content".into(), "message".into())
            .is_err());
        source
            .connection
            .execute(
                "DELETE FROM upload_chunks WHERE upload_id='u' AND offset_bytes=4",
                [],
            )
            .unwrap();
        assert!(source
            .sealed_content_reference("u", &owner, "content".into(), "message".into())
            .is_err());
    }

    #[test]
    fn sealed_upload_streams_to_canonical_content_and_replays_after_restart() {
        use crate::canonical_text_store::CanonicalTextStore;
        use nola_domain::CanonicalBodyReference;
        let directory = tempfile::tempdir().unwrap();
        let source_path = directory.path().join("upload.db");
        let target_path = directory.path().join("canonical.db");
        let owner = owner();
        let reference = CanonicalBodyReference::for_field(
            "content".into(),
            owner.conversation_id.clone(),
            "message".into(),
            owner.part_id.clone(),
            nola_domain::PartBodyField::ContentText,
            10,
            2,
        )
        .unwrap();
        let mut source = ContentUploadStore::open(&source_path, 100).unwrap();
        let mut target = CanonicalTextStore::open(&target_path, 100).unwrap();
        source.begin("upload", &owner, 10).unwrap();
        source.append("upload", &owner, 0, "中😀").unwrap();
        source.append("upload", &owner, 7, "end").unwrap();
        assert!(source
            .copy_sealed_to_canonical("upload", &owner, &reference, &mut target)
            .is_err());
        source.seal("upload", &owner).unwrap();
        source
            .copy_sealed_to_canonical("upload", &owner, &reference, &mut target)
            .unwrap();
        drop(source);
        drop(target);
        let source = ContentUploadStore::open_existing(&source_path, 100).unwrap();
        let mut target = CanonicalTextStore::open(&target_path, 100).unwrap();
        source
            .copy_sealed_to_canonical("upload", &owner, &reference, &mut target)
            .unwrap();
        assert_eq!(
            target.read_range(&reference, 0, 10).unwrap().text,
            "中😀end"
        );
        assert!(source.progress("upload", &owner).unwrap().sealed);
    }

    #[test]
    fn canonical_adoption_rejects_foreign_owner_and_rolls_back_wrong_chunk_count() {
        use crate::canonical_text_store::CanonicalTextStore;
        use nola_domain::CanonicalBodyReference;
        let directory = tempfile::tempdir().unwrap();
        let owner = owner();
        let mut source =
            ContentUploadStore::open(&directory.path().join("upload.db"), 100).unwrap();
        let mut target =
            CanonicalTextStore::open(&directory.path().join("canonical.db"), 100).unwrap();
        source.begin("upload", &owner, 6).unwrap();
        source.append("upload", &owner, 0, "abc").unwrap();
        source.append("upload", &owner, 3, "def").unwrap();
        source.seal("upload", &owner).unwrap();
        let foreign = CanonicalBodyReference::for_field(
            "content".into(),
            "foreign".into(),
            "message".into(),
            owner.part_id.clone(),
            nola_domain::PartBodyField::ContentText,
            6,
            2,
        )
        .unwrap();
        assert!(source
            .copy_sealed_to_canonical("upload", &owner, &foreign, &mut target)
            .is_err());
        assert!(target.read_range(&foreign, 0, 4).is_err());
        let wrong = CanonicalBodyReference::for_field(
            "content".into(),
            owner.conversation_id.clone(),
            "message".into(),
            owner.part_id.clone(),
            nola_domain::PartBodyField::ContentText,
            6,
            1,
        )
        .unwrap();
        assert!(source
            .copy_sealed_to_canonical("upload", &owner, &wrong, &mut target)
            .is_err());
        assert!(target.read_range(&wrong, 0, 4).is_err());
        let correct = CanonicalBodyReference::for_field(
            "content".into(),
            owner.conversation_id.clone(),
            "message".into(),
            owner.part_id.clone(),
            nola_domain::PartBodyField::ContentText,
            6,
            2,
        )
        .unwrap();
        source
            .copy_sealed_to_canonical("upload", &owner, &correct, &mut target)
            .unwrap();
        assert_eq!(target.read_range(&correct, 0, 6).unwrap().text, "abcdef");
    }

    #[test]
    fn ten_million_byte_adoption_uses_bounded_chunks_in_both_stores() {
        use crate::canonical_text_store::CanonicalTextStore;
        use nola_domain::CanonicalBodyReference;
        let directory = tempfile::tempdir().unwrap();
        let owner = owner();
        let total = 10_000_000_u64;
        let reference = CanonicalBodyReference::for_field(
            "content".into(),
            owner.conversation_id.clone(),
            "message".into(),
            owner.part_id.clone(),
            nola_domain::PartBodyField::ContentText,
            total,
            total.div_ceil(MAX_CHUNK_BYTES as u64) as u32,
        )
        .unwrap();
        let mut source =
            ContentUploadStore::open(&directory.path().join("upload.db"), total).unwrap();
        let mut target =
            CanonicalTextStore::open(&directory.path().join("canonical.db"), total).unwrap();
        source.begin("upload", &owner, total).unwrap();
        let mut offset = 0;
        while offset < total {
            let text = "x".repeat((total - offset).min(MAX_CHUNK_BYTES as u64) as usize);
            offset = source
                .append("upload", &owner, offset, &text)
                .unwrap()
                .committed_bytes;
        }
        source.seal("upload", &owner).unwrap();
        source
            .copy_sealed_to_canonical("upload", &owner, &reference, &mut target)
            .unwrap();
        assert_eq!(
            target.read_range(&reference, total - 31, 32).unwrap().text,
            "x".repeat(31)
        );
        assert_eq!(
            source.progress("upload", &owner).unwrap().committed_bytes,
            total
        );
    }
    #[test]
    fn sealing_rejects_missing_and_misaligned_persisted_chunks() {
        let mut store = ContentUploadStore::open(Path::new(":memory:"), 100).unwrap();
        let owner = owner();
        store.begin("broken", &owner, 6).unwrap();
        store.append("broken", &owner, 0, "abc").unwrap();
        store.append("broken", &owner, 3, "def").unwrap();
        store
            .connection
            .execute("DELETE FROM upload_chunks WHERE offset_bytes=3", [])
            .unwrap();
        assert!(store.seal("broken", &owner).is_err());
        assert!(!store.progress("broken", &owner).unwrap().sealed);
        store
            .connection
            .execute("INSERT INTO upload_chunks VALUES('broken',4,'ef')", [])
            .unwrap();
        assert!(store.seal("broken", &owner).is_err());
        assert!(!store.progress("broken", &owner).unwrap().sealed);
        store
            .connection
            .execute(
                "UPDATE upload_chunks SET offset_bytes=3,text='def' WHERE offset_bytes=4",
                [],
            )
            .unwrap();
        assert!(store.seal("broken", &owner).unwrap().sealed);
    }

    #[test]
    fn oversized_or_overrunning_stored_chunks_cannot_be_read_or_sealed() {
        let mut store = ContentUploadStore::open(Path::new(":memory:"), 100).unwrap();
        let owner = owner();
        store.begin("broken", &owner, 3).unwrap();
        store.append("broken", &owner, 0, "abc").unwrap();
        for text in ["four".to_string(), "x".repeat(MAX_CHUNK_BYTES + 1)] {
            store
                .connection
                .execute("UPDATE upload_chunks SET text=?1", params![text])
                .unwrap();
            assert!(store.read_chunk("broken", &owner, 0).is_err());
            assert!(store.seal("broken", &owner).is_err());
        }
    }

    #[test]
    fn malformed_durable_progress_is_not_cast_into_valid_wire_state() {
        let mut store = ContentUploadStore::open(Path::new(":memory:"), 100).unwrap();
        let owner = owner();
        store.begin("broken", &owner, 3).unwrap();
        for (committed, sealed) in [(-2, 0), (4, 0), (0, 1), (-1, 1)] {
            store
                .connection
                .execute(
                    "UPDATE uploads SET committed_bytes=?1,sealed=?2",
                    params![committed, sealed],
                )
                .unwrap();
            assert!(store.progress("broken", &owner).is_err());
        }
    }

    fn owner() -> UploadOwner {
        UploadOwner {
            conversation_id: "conversation".into(),
            import_id: "import".into(),
            part_id: "part".into(),
        }
    }

    #[test]
    fn cancellation_releases_bytes_but_rejects_resurrection_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cancel.db");
        let owner = owner();
        let mut store = ContentUploadStore::open(&path, MAX_STAGED_BYTES as u64).unwrap();
        store
            .begin("upload", &owner, MAX_STAGED_BYTES as u64)
            .unwrap();
        store.append("upload", &owner, 0, "abc").unwrap();
        let mut other = owner.clone();
        other.part_id = "other".into();
        assert!(store.cancel("upload", &other).is_err());
        assert_eq!(store.progress("upload", &owner).unwrap().committed_bytes, 3);
        assert!(store.cancel("upload", &owner).unwrap().cancelled);
        drop(store);
        let mut store = ContentUploadStore::open_existing(&path, MAX_STAGED_BYTES as u64).unwrap();
        assert!(store.cancel("upload", &owner).unwrap().cancelled);
        assert_eq!(store.progress("upload", &owner).unwrap().committed_bytes, 0);
        assert!(store
            .begin("upload", &owner, MAX_STAGED_BYTES as u64)
            .is_err());
        assert!(store.append("upload", &owner, 0, "abc").is_err());
        assert!(store.seal("upload", &owner).is_err());
        assert!(store.read_chunk("upload", &owner, 0).is_err());
        let chunks: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM upload_chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(chunks, 0);
        store
            .begin("replacement", &owner, MAX_STAGED_BYTES as u64)
            .unwrap();
    }

    #[test]
    fn sealed_content_cannot_be_cancelled() {
        let mut store = ContentUploadStore::open(Path::new(":memory:"), 10).unwrap();
        let owner = owner();
        store.begin("upload", &owner, 3).unwrap();
        store.append("upload", &owner, 0, "abc").unwrap();
        store.seal("upload", &owner).unwrap();
        assert!(store.cancel("upload", &owner).is_err());
        assert_eq!(
            store.read_chunk("upload", &owner, 0).unwrap().as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn concurrent_reservations_cannot_exceed_the_shared_quota() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("concurrent.db");
        drop(ContentUploadStore::open(&path, MAX_STAGED_BYTES as u64).unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|index| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut store =
                        ContentUploadStore::open_existing(&path, MAX_STAGED_BYTES as u64).unwrap();
                    barrier.wait();
                    match store.begin(
                        &format!("upload-{index}"),
                        &owner(),
                        MAX_STAGED_BYTES as u64 / 2,
                    ) {
                        Ok(_) => true,
                        Err(StoreError::Invariant(message))
                            if message == "staging capacity exhausted" =>
                        {
                            false
                        }
                        Err(error) => panic!("unexpected reservation failure: {error}"),
                    }
                })
            })
            .collect();
        let outcomes: Vec<_> = workers.into_iter().map(|worker| worker.join()).collect();
        let accepted = outcomes
            .into_iter()
            .map(|outcome| outcome.expect("reservation worker panicked"))
            .filter(|accepted| *accepted)
            .count();
        assert_eq!(accepted, 2);
        let store = ContentUploadStore::open_existing(&path, MAX_STAGED_BYTES as u64).unwrap();
        let (count, reserved): (i64, i64) = store
            .connection
            .query_row(
                "SELECT COUNT(*),SUM(expected_bytes) FROM uploads",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(reserved, MAX_STAGED_BYTES);
    }

    #[test]
    fn failed_progress_update_rolls_back_the_inserted_chunk() {
        let directory = tempfile::tempdir().unwrap();
        let mut store =
            ContentUploadStore::open(&directory.path().join("rollback.db"), 100).unwrap();
        let owner = owner();
        store.begin("upload", &owner, 3).unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TEMP TRIGGER fail_progress BEFORE UPDATE OF committed_bytes ON uploads
             BEGIN SELECT RAISE(ABORT,'injected progress failure'); END;",
            )
            .unwrap();
        assert!(store.append("upload", &owner, 0, "abc").is_err());
        assert_eq!(store.progress("upload", &owner).unwrap().committed_bytes, 0);
        let chunks: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM upload_chunks WHERE upload_id='upload'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(chunks, 0);
        store
            .connection
            .execute_batch("DROP TRIGGER fail_progress")
            .unwrap();
        assert_eq!(
            store
                .append("upload", &owner, 0, "abc")
                .unwrap()
                .committed_bytes,
            3
        );
        assert!(store.seal("upload", &owner).unwrap().sealed);
    }

    #[test]
    fn reservations_are_atomic_and_exact_replays_do_not_consume_extra_quota() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("quota.db");
        let o = owner();
        let mut first = ContentUploadStore::open(&path, MAX_STAGED_BYTES as u64).unwrap();
        let mut second = ContentUploadStore::open_existing(&path, MAX_STAGED_BYTES as u64).unwrap();
        let half = MAX_STAGED_BYTES as u64 / 2;
        first.begin("one", &o, half).unwrap();
        second.begin("two", &o, half).unwrap();
        assert!(first.begin("overflow", &o, 1).is_err());
        assert!(second.progress("overflow", &o).is_err());
        assert!(first.begin("one", &o, half).is_ok());
        assert_eq!(second.progress("one", &o).unwrap().committed_bytes, 0);
    }

    #[test]
    fn many_small_uploads_have_a_count_limit() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ContentUploadStore::open(&d.path().join("quota.db"), 100).unwrap();
        let o = owner();
        for index in 0..MAX_STAGED_UPLOADS {
            s.begin(&format!("u-{index}"), &o, 1).unwrap();
        }
        assert!(s.begin("overflow", &o, 1).is_err());
        assert!(s.progress("overflow", &o).is_err());
        assert!(s.begin("u-0", &o, 1).is_ok());
    }

    #[test]
    fn resume_replay_and_seal_survive_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("uploads.db");
        let o = owner();
        let mut s = ContentUploadStore::open(&path, 100).unwrap();
        s.begin("u", &o, 10).unwrap();
        s.append("u", &o, 0, "中😀").unwrap();
        assert!(s.seal("u", &o).is_err());
        drop(s);
        let mut s = ContentUploadStore::open(&path, 100).unwrap();
        assert_eq!(s.begin("u", &o, 10).unwrap().committed_bytes, 7);
        assert_eq!(s.append("u", &o, 0, "中😀").unwrap().committed_bytes, 7);
        assert!(s.append("u", &o, 0, "different").is_err());
        assert!(s.append("u", &o, 8, "x").is_err());
        s.append("u", &o, 7, "end").unwrap();
        assert!(s.seal("u", &o).unwrap().sealed);
        assert!(s.seal("u", &o).unwrap().sealed);
        assert!(s.append("u", &o, 7, "end").is_err());
        assert_eq!(s.read_chunk("u", &o, 0).unwrap().as_deref(), Some("中😀"));
        assert_eq!(s.read_chunk("u", &o, 7).unwrap().as_deref(), Some("end"));
        assert_eq!(s.read_chunk("u", &o, 10).unwrap(), None);
        assert!(s.read_chunk("u", &o, 1).is_err());
    }

    #[test]
    fn owner_conflicts_and_limits_do_not_change_progress() {
        let d = tempfile::tempdir().unwrap();
        let mut s = ContentUploadStore::open(&d.path().join("u.db"), 50_000).unwrap();
        let o = owner();
        s.begin("u", &o, 40_000).unwrap();
        let mut foreign = o.clone();
        foreign.part_id = "foreign".into();
        assert!(s.begin("u", &foreign, 40_000).is_err());
        assert!(s.progress("u", &foreign).is_err());
        assert!(s.append("u", &foreign, 0, "x").is_err());
        assert!(s.seal("u", &foreign).is_err());
        assert!(s.read_chunk("u", &foreign, 0).is_err());
        assert!(s.begin("u", &o, 39_999).is_err());
        assert!(s.begin("too-large", &o, 50_001).is_err());
        assert!(s
            .append("u", &o, 0, &"x".repeat(MAX_CHUNK_BYTES + 1))
            .is_err());
        assert!(s.append("u", &o, u64::MAX, "x").is_err());
        assert_eq!(s.progress("u", &o).unwrap().committed_bytes, 0);
    }

    #[test]
    fn ten_million_bytes_are_written_and_read_one_chunk_at_a_time() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("u.db");
        let o = owner();
        let mut s = ContentUploadStore::open(&path, 10_000_000).unwrap();
        s.begin("u", &o, 10_000_000).unwrap();
        let mut offset = 0;
        while offset < 10_000_000 {
            let text = "x".repeat((10_000_000 - offset).min(MAX_CHUNK_BYTES as u64) as usize);
            offset = s.append("u", &o, offset, &text).unwrap().committed_bytes;
        }
        s.seal("u", &o).unwrap();
        drop(s);
        let s = ContentUploadStore::open(&path, 10_000_000).unwrap();
        let mut offset = 0;
        while let Some(text) = s.read_chunk("u", &o, offset).unwrap() {
            assert!(text.len() <= MAX_CHUNK_BYTES);
            offset += text.len() as u64;
        }
        assert_eq!(offset, 10_000_000);
    }
}
