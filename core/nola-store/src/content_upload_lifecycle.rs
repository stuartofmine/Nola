// SPDX-License-Identifier: Apache-2.0
//! Retire duplicate staging bytes only after an independent connection observes
//! committed canonical ownership. Upload identities remain immutable and replayable.
use super::*;
use nola_domain::CanonicalBodyReference;

pub(super) fn is_retired(connection: &Connection, id: &str) -> StoreResult<bool> {
    let exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='upload_retirements')", [], |r| r.get(0))?;
    if !exists {
        return Ok(false);
    }
    let encoded: Option<String> = connection
        .query_row(
            "SELECT reference_json FROM upload_retirements WHERE upload_id=?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(encoded) = encoded {
        let reference: CanonicalBodyReference = serde_json::from_str(&encoded)?;
        if reference.content_id() != id {
            return Err(conflict("retired upload reference identity differs"));
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

impl ContentUploadStore {
    pub(super) fn initialize_retirement_schema(&self) -> StoreResult<()> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS upload_retirements(upload_id TEXT PRIMARY KEY REFERENCES uploads(id) ON DELETE CASCADE, reference_json TEXT NOT NULL CHECK(length(CAST(reference_json AS BLOB))<=4096))")?;
        Ok(())
    }

    pub(crate) fn retire_durable_uploads(&mut self, canonical_path: &Path) -> StoreResult<()> {
        self.initialize_retirement_schema()?;
        // A separate :memory: connection is a different database. In-memory
        // adapters retain staging; production file databases use durable evidence.
        if canonical_path == Path::new(":memory:")
            || !canonical_path
                .try_exists()
                .map_err(|error| StoreError::Invariant(error.to_string()))?
        {
            return Ok(());
        }
        let canonical = Connection::open_with_flags(
            canonical_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        canonical.busy_timeout(std::time::Duration::from_secs(5))?;
        let exists: bool = canonical.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='canonical_text_objects')", [], |r|r.get(0))?;
        if !exists {
            return Ok(());
        }
        let candidates = {
            let mut statement = self.connection.prepare("SELECT id,conversation_id,import_id,part_id FROM uploads WHERE sealed=1 AND NOT EXISTS(SELECT 1 FROM upload_retirements r WHERE r.upload_id=uploads.id) LIMIT 128")?;
            let rows = statement.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    UploadOwner {
                        conversation_id: r.get(1)?,
                        import_id: r.get(2)?,
                        part_id: r.get(3)?,
                    },
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (id, owner) in candidates {
            let reference: Option<String> = canonical.query_row(
                "SELECT reference_json FROM canonical_text_objects o WHERE id=?1 AND (
                EXISTS(SELECT 1 FROM archive_restore_prepared p WHERE p.restore_id=?2 AND p.content_id=o.id AND p.reference_json=o.reference_json)
                OR EXISTS(SELECT 1 FROM transcript_import_publications p,json_each(p.manifest_json,'$.uploads') u WHERE p.import_id=?2 AND p.conversation_id=?3 AND json_extract(u.value,'$.upload_id')=o.id AND json_extract(u.value,'$.part_id')=?4))",
                params![id,owner.import_id,owner.conversation_id,owner.part_id], |r|r.get(0)).optional()?;
            let Some(encoded) = reference else {
                continue;
            };
            let reference: CanonicalBodyReference = serde_json::from_str(&encoded)?;
            if reference.content_id() != id
                || reference.conversation_id() != owner.conversation_id
                || reference.part_id() != owner.part_id
            {
                return Err(conflict("committed upload retirement owner differs"));
            }
            crate::canonical_text_replica::require_complete(&canonical, &reference)?;
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            let progress = load(&tx, &id, &owner)?;
            // Another adapter may have retired this candidate after our scan.
            if is_retired(&tx, &id)? {
                tx.commit()?;
                continue;
            }
            if !progress.sealed
                || progress.cancelled
                || progress.expected_bytes != reference.total_utf8_bytes()
            {
                return Err(conflict("committed upload retirement length differs"));
            }
            // Verify exact original chunk geometry and bytes before switching the
            // read source, so offset-based upload readers keep the same contract.
            let mut chunks=tx.prepare("SELECT offset_bytes,text FROM upload_chunks WHERE upload_id=?1 ORDER BY offset_bytes")?;
            let mut rows = chunks.query(params![id])?;
            let mut count = 0u32;
            while let Some(row) = rows.next()? {
                let offset: i64 = row.get(0)?;
                let text: String = row.get(1)?;
                let same:bool=canonical.query_row("SELECT EXISTS(SELECT 1 FROM canonical_text_chunks WHERE content_id=?1 AND offset_bytes=?2 AND text=?3)",params![id,offset,text],|r|r.get(0))?;
                if !same {
                    return Err(conflict("canonical upload retirement bytes differ"));
                }
                count += 1;
            }
            if count != reference.chunk_count() {
                return Err(conflict("canonical upload retirement chunks differ"));
            }
            drop(rows);
            drop(chunks);
            tx.execute(
                "INSERT INTO upload_retirements(upload_id,reference_json) VALUES(?1,?2)",
                params![id, encoded],
            )?;
            tx.execute("DELETE FROM upload_chunks WHERE upload_id=?1", params![id])?;
            tx.commit()?;
        }
        Ok(())
    }

    pub(crate) fn read_chunk_with_canonical(
        &self,
        id: &str,
        owner: &UploadOwner,
        offset: u64,
        canonical_path: &Path,
    ) -> StoreResult<Option<String>> {
        let progress = self.progress(id, owner)?;
        if !is_retired(&self.connection, id)? {
            return self.read_chunk(id, owner, offset);
        }
        if offset > progress.expected_bytes {
            return Err(invalid("read offset exceeds committed length"));
        }
        if offset == progress.expected_bytes {
            return Ok(None);
        }
        let encoded: String = self.connection.query_row(
            "SELECT reference_json FROM upload_retirements WHERE upload_id=?1",
            params![id],
            |r| r.get(0),
        )?;
        let reference: CanonicalBodyReference = serde_json::from_str(&encoded)?;
        if reference.content_id() != id
            || reference.conversation_id() != owner.conversation_id
            || reference.part_id() != owner.part_id
            || reference.total_utf8_bytes() != progress.expected_bytes
        {
            return Err(conflict("retired upload identity differs"));
        }
        let canonical = Connection::open_with_flags(
            canonical_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        crate::canonical_text_store::validate_reference_from_connection(&canonical, &reference)?;
        let text:Option<String>=canonical.query_row("SELECT text FROM canonical_text_chunks WHERE content_id=?1 AND offset_bytes=?2 AND length(CAST(text AS BLOB)) BETWEEN 1 AND 32768",params![reference.content_id(),offset as i64],|r|r.get(0)).optional()?;
        let text =
            text.ok_or_else(|| invalid("read offset has no valid bounded canonical chunk"))?;
        if offset + text.len() as u64 > progress.expected_bytes {
            return Err(invalid("canonical upload chunk exceeds expected length"));
        }
        Ok(Some(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{content_upload_port::LazyContentUploadStore, NolaStore};
    use nola_store_port::{archive_restore::*, content_upload::ContentUploadRepository};

    fn setup(path: &Path) -> NolaStore {
        let mut store = NolaStore::open(path).unwrap();
        store
            .begin_archive_restore(&BeginArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: store.archive_directory_revision().unwrap(),
                created_at_ms: 1,
            })
            .unwrap();
        store
            .stage_archive_entry(
                "restore",
                0,
                &ArchiveRestoreEntry::Profile {
                    profile: ArchiveConversationProfile {
                        id: "c".into(),
                        title: "staged".into(),
                        settings_json: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    },
                },
            )
            .unwrap();
        store
    }
    fn owner(index: usize) -> UploadOwner {
        UploadOwner {
            conversation_id: "c".into(),
            import_id: "restore".into(),
            part_id: format!("p-{index}"),
        }
    }
    fn reference(index: usize) -> CanonicalBodyReference {
        CanonicalBodyReference::for_field(
            format!("u-{index}"),
            "c".into(),
            format!("m-{index}"),
            format!("p-{index}"),
            nola_domain::PartBodyField::ContentText,
            6,
            2,
        )
        .unwrap()
    }
    fn prepare(store: &mut NolaStore, index: usize) {
        store
            .prepare_archive_content(
                "restore",
                &reference(index),
                &mut vec![Ok((0, "中".into())), Ok((3, "文".into()))].into_iter(),
                10_000,
            )
            .unwrap();
    }
    fn upload(store: &mut impl ContentUploadRepository<Error = String>, index: usize) {
        let id = format!("u-{index}");
        let owner = owner(index);
        store.begin(&id, &owner, 6).unwrap();
        store.append(&id, &owner, 0, "中").unwrap();
        store.append(&id, &owner, 3, "文").unwrap();
        store.seal(&id, &owner).unwrap();
    }

    #[test]
    fn prepared_json_upload_retires_reopens_and_keeps_exact_field_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("json.db");
        let uploads_path = directory.path().join("json.db.uploads.sqlite");
        let mut main = setup(&path);
        let raw = " {\"text\":\"你好🙂\",\"n\":9007199254740993} ";
        let owner = owner(0);
        let mut uploads = LazyContentUploadStore::for_application(&path, 1000);
        uploads
            .begin("json-upload", &owner, raw.len() as u64)
            .unwrap();
        uploads.append("json-upload", &owner, 0, raw).unwrap();
        uploads.seal("json-upload", &owner).unwrap();
        let reference = CanonicalBodyReference::for_field(
            "json-upload".into(),
            "c".into(),
            "m-0".into(),
            "p-0".into(),
            nola_domain::PartBodyField::ContentJson,
            raw.len() as u64,
            1,
        )
        .unwrap();
        main.prepare_archive_content(
            "restore",
            &reference,
            &mut std::iter::once(Ok((0, raw.to_string()))),
            1000,
        )
        .unwrap();
        uploads.begin("trigger", &self::owner(1), 1).unwrap();
        drop(uploads);
        let retired = Connection::open(&uploads_path).unwrap();
        let remaining: i64 = retired
            .query_row(
                "SELECT count(*) FROM upload_chunks WHERE upload_id='json-upload'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
        let recorded: String = retired
            .query_row(
                "SELECT reference_json FROM upload_retirements WHERE upload_id='json-upload'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<CanonicalBodyReference>(&recorded).unwrap(),
            reference
        );
        let uploads = LazyContentUploadStore::for_application(&path, 1000);
        assert_eq!(
            uploads
                .read_chunk("json-upload", &owner, 0)
                .unwrap()
                .as_deref(),
            Some(raw)
        );
        assert_eq!(
            uploads
                .read_chunk("json-upload", &owner, raw.len() as u64)
                .unwrap(),
            None
        );
        let changed = CanonicalBodyReference::for_field(
            "json-upload".into(),
            "c".into(),
            "m-0".into(),
            "p-0".into(),
            nola_domain::PartBodyField::ContentText,
            raw.len() as u64,
            1,
        )
        .unwrap();
        main.connection
            .execute(
                "UPDATE canonical_text_objects SET reference_json=?1 WHERE id='json-upload'",
                [serde_json::to_string(&changed).unwrap()],
            )
            .unwrap();
        assert!(
            uploads.read_chunk("json-upload", &owner, 0).is_err(),
            "retirement cannot reinterpret JSON as Text"
        );
    }

    #[test]
    fn changed_json_bytes_cannot_retire_original_upload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("changed.db");
        let mut main = setup(&path);
        let owner = owner(0);
        let uploads_path = directory.path().join("changed.uploads");
        let mut uploads = ContentUploadStore::open(&uploads_path, 1000).unwrap();
        uploads.begin("changed", &owner, 7).unwrap();
        uploads.append("changed", &owner, 0, "{\"n\":1}").unwrap();
        uploads.seal("changed", &owner).unwrap();
        let reference = CanonicalBodyReference::for_field(
            "changed".into(),
            "c".into(),
            "m".into(),
            "p-0".into(),
            nola_domain::PartBodyField::ContentJson,
            7,
            1,
        )
        .unwrap();
        main.prepare_archive_content(
            "restore",
            &reference,
            &mut std::iter::once(Ok((0, "{\"n\":2}".into()))),
            1000,
        )
        .unwrap();
        assert!(uploads.retire_durable_uploads(&path).is_err());
        assert!(!is_retired(&uploads.connection, "changed").unwrap());
        assert_eq!(
            uploads.read_chunk("changed", &owner, 0).unwrap().as_deref(),
            Some("{\"n\":1}")
        );
    }

    #[test]
    fn two_hundred_durable_uploads_release_active_quota_and_reopen_reads_exact_original_boundaries()
    {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("main.db");
        let mut main = setup(&path);
        let mut uploads = LazyContentUploadStore::for_application(&path, 100);
        for index in 0..200 {
            upload(&mut uploads, index);
            prepare(&mut main, index);
            if index % 25 == 0 {
                drop(uploads);
                uploads = LazyContentUploadStore::for_application(&path, 100);
            }
        }
        uploads.begin("trigger", &owner(200), 1).unwrap();
        drop(uploads);
        let mut uploads = LazyContentUploadStore::for_application(&path, 100);
        for index in [0, 127, 199] {
            let id = format!("u-{index}");
            let owner = owner(index);
            assert!(uploads.begin(&id, &owner, 6).unwrap().sealed);
            assert!(uploads.seal(&id, &owner).unwrap().sealed);
            assert_eq!(
                uploads.read_chunk(&id, &owner, 0).unwrap().as_deref(),
                Some("中")
            );
            assert_eq!(
                uploads.read_chunk(&id, &owner, 3).unwrap().as_deref(),
                Some("文")
            );
            assert!(uploads.read_chunk(&id, &owner, 1).is_err());
            assert_eq!(uploads.read_chunk(&id, &owner, 6).unwrap(), None);
            assert!(uploads.begin(&id, &owner, 7).is_err());
            assert!(uploads.read_chunk(&id, &self::owner(999), 0).is_err());
            assert!(uploads.cancel(&id, &owner).is_err());
        }
        let db = Connection::open(directory.path().join("main.db.uploads.sqlite")).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM upload_retirements", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            200
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM upload_chunks", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn uncommitted_or_rolled_back_canonical_preparation_never_retires_original_upload() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("main.db");
        let mut main = setup(&path);
        let mut uploads = LazyContentUploadStore::for_application(&path, 100);
        upload(&mut uploads, 0);
        main.begin_write_unit_of_work().unwrap();
        prepare(&mut main, 0);
        uploads.begin("during", &owner(1), 1).unwrap();
        let db = Connection::open(directory.path().join("main.db.uploads.sqlite")).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM upload_retirements", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        main.rollback_write_unit_of_work().unwrap();
        drop(main);
        drop(uploads);
        let mut main = NolaStore::open(&path).unwrap();
        let mut uploads = LazyContentUploadStore::for_application(&path, 100);
        uploads.begin("after", &owner(2), 1).unwrap();
        assert_eq!(
            uploads.read_chunk("u-0", &owner(0), 3).unwrap().as_deref(),
            Some("文")
        );
        prepare(&mut main, 0);
        uploads.begin("committed", &owner(3), 1).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM upload_retirements", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn unbound_or_differently_chunked_canonical_content_cannot_release_staging() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("main.db");
        let mut main = setup(&path);
        let mut uploads = LazyContentUploadStore::for_application(&path, 100);
        upload(&mut uploads, 0);
        let reference =
            CanonicalBodyReference::for_field("u-0".into(), "c".into(), "m-0".into(), "p-0".into(), nola_domain::PartBodyField::ContentText, 6, 1)
                .unwrap();
        crate::canonical_text_store::put_chunks(
            &main.connection,
            &reference,
            vec![Ok((0, "中文".into()))],
            100,
        )
        .unwrap();
        uploads.begin("unbound", &owner(1), 1).unwrap();
        main.prepare_archive_content(
            "restore",
            &reference,
            &mut vec![Ok((0, "中文".into()))].into_iter(),
            100,
        )
        .unwrap();
        assert!(uploads.begin("geometry", &owner(2), 1).is_err());
        assert_eq!(
            uploads.read_chunk("u-0", &owner(0), 3).unwrap().as_deref(),
            Some("文")
        );
    }
    #[test]
    fn committed_publication_retires_only_the_matching_import_owner() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("main.db");
        let main = setup(&path);
        main.connection.execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','published',1,1,1)",[]).unwrap();
        let mut uploads = LazyContentUploadStore::for_application(&path, 100);
        upload(&mut uploads, 0);
        crate::canonical_text_store::put_chunks(
            &main.connection,
            &reference(0),
            vec![Ok((0, "中".into())), Ok((3, "文".into()))],
            100,
        )
        .unwrap();
        let manifest=serde_json::json!({"uploads":[{"upload_id":"u-0","part_id":"p-0","message_id":"m-0","expected_utf8_bytes":6}]}).to_string();
        main.connection
            .execute(
                "INSERT INTO transcript_import_publications VALUES('foreign','c',?1,'{}')",
                params![manifest],
            )
            .unwrap();
        uploads.begin("foreign", &owner(1), 1).unwrap();
        let db = Connection::open(directory.path().join("main.db.uploads.sqlite")).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM upload_retirements", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        main.connection
            .execute(
                "INSERT INTO transcript_import_publications VALUES('restore','c',?1,'{}')",
                params![manifest],
            )
            .unwrap();
        uploads.begin("matching", &owner(2), 1).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM upload_retirements", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            uploads.read_chunk("u-0", &owner(0), 3).unwrap().as_deref(),
            Some("文")
        );
    }
}
