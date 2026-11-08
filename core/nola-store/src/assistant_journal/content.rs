// SPDX-License-Identifier: Apache-2.0
use super::*;

pub(super) fn field(value: JournalContentField) -> StoreResult<String> {
    Ok(serde_json::to_string(&value)?)
}
pub(super) fn verify_reference(
    c: &Connection,
    id: &AssistantJournalIdentity,
    part: &str,
    r: &JournalContentRef,
) -> StoreResult<()> {
    number(r.utf8_bytes.0)?;
    number(r.chunk_count.0)?;
    number(r.content_generation.0)?;
    if r.chunk_count.0 == 0 {
        if *r != empty_journal_content(id, part, r.field, r.content_generation).map_err(invalid)? {
            return Err(invalid("invalid empty content root"));
        }
        return Ok(());
    }
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM assistant_journal_manifests WHERE root=?1 AND owner=?2 AND part_id=?3 AND field=?4 AND generation=?5 AND byte_count=?6 AND chunk_count=?7)",params![r.content_root.to_hex(),owner(id)?,part,field(r.field)?,number(r.content_generation.0)?,number(r.utf8_bytes.0)?,number(r.chunk_count.0)?],|r|r.get(0))?;
    if !valid {
        return Err(invalid("content manifest unavailable for exact owner"));
    }
    Ok(())
}
/// The caller has already authorized the exact ref against either a real fixed
/// snapshot or the current published head. Only indexed manifest facts are read.
pub(super) fn manifest_page(
    c: &Connection,
    key: &str,
    part: &str,
    r: &JournalContentRef,
    after: u64,
    max_records: usize,
) -> StoreResult<AssistantManifestPage> {
    if max_records == 0 || max_records > ASSISTANT_PAGE_MAX_RECORDS {
        return Err(invalid("manifest page budget"));
    }
    if after > r.chunk_count.0 {
        return Err(invalid("chunk cursor beyond manifest"));
    }
    let mut query = c.prepare(
        "SELECT digest,byte_count,chunk_index FROM assistant_journal_manifest_chunks
         WHERE owner=?1 AND part_id=?2 AND field=?3 AND generation=?4
         AND chunk_index>=?5 AND chunk_index<?6 ORDER BY chunk_index LIMIT ?7",
    )?;
    let mut rows = query.query(params![
        key,
        part,
        field(r.field)?,
        number(r.content_generation.0)?,
        number(after)?,
        number(r.chunk_count.0)?,
        max_records as i64,
    ])?;
    let (mut chunks, mut next) = (Vec::new(), after);
    while let Some(row) = rows.next()? {
        let index: u64 = row.get(2)?;
        if index != next {
            return Err(invalid("manifest chunk gap"));
        }
        let utf8_bytes: u64 = row.get(1)?;
        if utf8_bytes == 0 || utf8_bytes > ASSISTANT_CHUNK_MAX_BYTES as u64 {
            return Err(invalid("invalid manifest chunk byte length"));
        }
        chunks.push(JournalChunkRef {
            digest: AssistantDigest::parse_hex(&row.get::<_, String>(0)?).map_err(invalid)?,
            utf8_bytes: JournalNumber(utf8_bytes),
        });
        next += 1;
    }
    if next == after && after < r.chunk_count.0 {
        return Err(invalid("incomplete manifest"));
    }
    Ok(AssistantManifestPage {
        chunks,
        next_chunk: next,
        complete: next == r.chunk_count.0,
    })
}
pub(super) fn supply(
    c: &Connection,
    chunks: &[AssistantTextChunkV1],
) -> StoreResult<Vec<JournalChunkRef>> {
    let total = chunks.iter().try_fold(0usize, |a, c| {
        a.checked_add(c.byte_length())
            .ok_or_else(|| invalid("chunk budget overflow"))
    })?;
    if total > ASSISTANT_FRAME_MAX_BYTES || chunks.len() > ASSISTANT_PAGE_MAX_RECORDS {
        return Err(invalid("supplied chunk budget"));
    }
    let mut refs = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        chunk.verify().map_err(invalid)?;
        c.execute(
            "INSERT OR IGNORE INTO assistant_journal_chunks(digest,bytes) VALUES(?1,?2)",
            params![chunk.digest().to_hex(), chunk.text().as_bytes()],
        )?;
        let stored: Vec<u8> = c.query_row(
            "SELECT bytes FROM assistant_journal_chunks WHERE digest=?1",
            [chunk.digest().to_hex()],
            |r| r.get(0),
        )?;
        if stored != chunk.text().as_bytes() {
            return Err(invalid("chunk digest collision"));
        }
        refs.push(JournalChunkRef {
            digest: chunk.digest(),
            utf8_bytes: JournalNumber(chunk.byte_length() as u64),
        });
    }
    Ok(refs)
}
pub(super) fn append(
    c: &Connection,
    key: &str,
    part: &str,
    previous: &JournalContentRef,
    chunks: &[JournalChunkRef],
) -> StoreResult<JournalContentRef> {
    let mut cursor = previous.clone();
    for chunk in chunks {
        let next = append_journal_content(&cursor, std::slice::from_ref(chunk)).map_err(invalid)?;
        c.execute("INSERT OR IGNORE INTO assistant_journal_manifests(root,owner,part_id,field,generation,byte_count,chunk_count,previous_root,chunk_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![next.content_root.to_hex(),key,part,field(next.field)?,number(next.content_generation.0)?,number(next.utf8_bytes.0)?,number(next.chunk_count.0)?,cursor.content_root.to_hex(),chunk.digest.to_hex()])?;
        c.execute("INSERT OR IGNORE INTO assistant_journal_manifest_chunks(owner,part_id,field,generation,chunk_index,digest,byte_count,result_root) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![key,part,field(next.field)?,number(next.content_generation.0)?,number(cursor.chunk_count.0)?,chunk.digest.to_hex(),number(chunk.utf8_bytes.0)?,next.content_root.to_hex()])?;
        let root:String=c.query_row("SELECT result_root FROM assistant_journal_manifest_chunks WHERE owner=?1 AND part_id=?2 AND field=?3 AND generation=?4 AND chunk_index=?5",params![key,part,field(next.field)?,number(next.content_generation.0)?,number(cursor.chunk_count.0)?],|r|r.get(0))?;
        if root != next.content_root.to_hex() {
            return Err(StoreError::Conflict("content generation fork".into()));
        }
        cursor = next;
    }
    Ok(cursor)
}

pub(super) struct Reader<'a> {
    pub c: &'a Connection,
    pub id: &'a AssistantJournalIdentity,
    pub supplied: &'a [JournalChunkRef],
}
impl AssistantJournalStateReader for Reader<'_> {
    fn load_part_head(&self, part: &str) -> Result<Option<JournalPartHead>, OperationError> {
        let key = owner(self.id).map_err(|_| OperationError::ManifestUnavailable)?;
        let value: Option<String> = self
            .c
            .query_row(
                "SELECT head_json FROM assistant_journal_parts WHERE owner=?1 AND part_id=?2",
                params![key, part],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| OperationError::ManifestUnavailable)?;
        if value.as_deref() == Some("null") {
            return Err(OperationError::RetiredPart);
        }
        value
            .map(|v| serde_json::from_str(&v).map_err(|_| OperationError::ManifestUnavailable))
            .transpose()
    }
    fn verify_content_reference(
        &self,
        id: &AssistantJournalIdentity,
        part: &str,
        r: &JournalContentRef,
    ) -> Result<(), OperationError> {
        verify_reference(self.c, id, part, r).map_err(|_| OperationError::ManifestUnavailable)
    }
    fn verify_chunk_reference(&self, r: &JournalChunkRef) -> Result<(), OperationError> {
        if self.supplied.contains(r) {
            return Ok(());
        }
        let valid:bool=self.c.query_row("SELECT EXISTS(SELECT 1 FROM assistant_journal_manifest_chunks WHERE owner=?1 AND digest=?2 AND byte_count=?3)",params![owner(self.id).map_err(|_|OperationError::ManifestUnavailable)?,r.digest.to_hex(),i64::try_from(r.utf8_bytes.0).map_err(|_|OperationError::InvalidContent)?],|row|row.get(0)).map_err(|_|OperationError::ManifestUnavailable)?;
        if valid {
            Ok(())
        } else {
            Err(OperationError::ManifestUnavailable)
        }
    }
}
