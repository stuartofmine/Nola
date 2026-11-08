// SPDX-License-Identifier: Apache-2.0
use super::*;

// Deliberately separate directory and keyed version reads. A JOIN over all
// historical versions lets the optimizer revisit one Part thousands of times.
pub(super) const PART_DIRECTORY_SQL: &str = "SELECT part_id FROM assistant_journal_order WHERE owner=?1 AND (section,position,part_id)>(?2,?3,?4) ORDER BY section,position,part_id LIMIT ?5";
pub(super) const PART_VERSION_SQL: &str = "SELECT head_json FROM assistant_journal_part_versions WHERE owner=?1 AND part_id=?2 AND sequence<=?3 ORDER BY sequence DESC LIMIT 1";

fn limits(records: usize, bytes: usize) -> StoreResult<()> {
    if records == 0
        || records > ASSISTANT_PAGE_MAX_RECORDS
        || bytes == 0
        || bytes > ASSISTANT_FRAME_MAX_BYTES
    {
        return Err(invalid("journal page budget"));
    }
    Ok(())
}
pub(super) fn verify_cursor(c: &Connection, cursor: &AssistantJournalCursor) -> StoreResult<()> {
    let key = validate_read_owner(c, &cursor.identity)?;
    number(cursor.sequence)?;
    if cursor.sequence == 0 {
        if *cursor != AssistantJournalCursor::initial(cursor.identity.clone()).map_err(invalid)? {
            return Err(invalid("invalid journal genesis"));
        }
    } else {
        let encoded: String = c.query_row(
            "SELECT header_json FROM assistant_journal_records WHERE owner=?1 AND sequence=?2",
            params![key, number(cursor.sequence)?],
            |r| r.get(0),
        )?;
        let header: AssistantJournalRecordHeaderV1 = serde_json::from_str(&encoded)?;
        header.verify().map_err(invalid)?;
        if header.record_digest != cursor.record_digest {
            return Err(invalid("journal cursor digest mismatch"));
        }
    }
    Ok(())
}
pub(super) fn open(
    c: &Connection,
    after: &AssistantJournalCursor,
) -> StoreResult<AssistantJournalReadWindow> {
    let _snapshot = crate::read_snapshot::pin(c)?;
    verify_cursor(c, after)?;
    let through = read_head(c, &after.identity)?;
    if after.sequence > through.sequence {
        return Err(invalid("cursor beyond journal head"));
    }
    Ok(AssistantJournalReadWindow {
        after: after.clone(),
        through,
    })
}
pub(super) fn journal_page(
    c: &Connection,
    w: &AssistantJournalReadWindow,
    max_records: usize,
    max_bytes: usize,
) -> StoreResult<AssistantJournalPage> {
    limits(max_records, max_bytes)?;
    let _snapshot = crate::read_snapshot::pin(c)?;
    if w.after.identity != w.through.identity || w.after.sequence > w.through.sequence {
        return Err(invalid("journal read window mismatch"));
    }
    let key = validate_read_owner(c, &w.after.identity)?;
    let floor: u64 = c
        .query_row(
            "SELECT retained_floor FROM assistant_journal_heads WHERE owner=?1",
            [&key],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    if w.after.sequence < floor {
        return Ok(AssistantJournalPage::SnapshotRequired);
    }
    verify_cursor(c, &w.after)?;
    verify_cursor(c, &w.through)?;
    let mut statement=c.prepare("SELECT header_json,batch FROM assistant_journal_records WHERE owner=?1 AND sequence>?2 AND sequence<=?3 ORDER BY sequence LIMIT ?4")?;
    let mut rows = statement.query(params![
        key,
        number(w.after.sequence)?,
        number(w.through.sequence)?,
        max_records as i64
    ])?;
    let (mut used, mut records, mut next) = (0usize, Vec::new(), w.after.clone());
    while let Some(row) = rows.next()? {
        let encoded: String = row.get(0)?;
        let bytes: Vec<u8> = row.get(1)?;
        let record_size = encoded.len() + bytes.len();
        if used + record_size > max_bytes {
            if records.is_empty() {
                return Err(invalid("page budget cannot fit next record"));
            }
            break;
        }
        let header: AssistantJournalRecordHeaderV1 = serde_json::from_str(&encoded)?;
        let batch = AssistantOperationBatchV1::decode_canonical(&bytes).map_err(invalid)?;
        batch.verify_header(&header).map_err(invalid)?;
        if header.sequence != next.sequence + 1
            || header.previous_record_digest != next.record_digest
        {
            return Err(invalid("journal stored gap or digest mismatch"));
        }
        used += record_size;
        next.sequence = header.sequence;
        next.record_digest = header.record_digest;
        records.push(StoredAssistantJournalRecord { header, batch });
    }
    if records.is_empty() && next.sequence < w.through.sequence {
        return Ok(AssistantJournalPage::SnapshotRequired);
    }
    Ok(AssistantJournalPage::Records {
        complete: next == w.through,
        records,
        next,
    })
}
fn snapshot_key(c: &Connection, s: &AssistantJournalSnapshot) -> StoreResult<String> {
    snapshot_lifecycle::active_key(c, s)
}
pub(super) fn parts(
    c: &Connection,
    s: &AssistantJournalSnapshot,
    after: Option<&str>,
    max_records: usize,
    max_bytes: usize,
) -> StoreResult<AssistantPartPage> {
    limits(max_records, max_bytes)?;
    let _read = crate::read_snapshot::pin(c)?;
    let key = snapshot_key(c, s)?;
    let (section, position) = match after {
        Some(part_id) => c.query_row("SELECT section,position FROM assistant_journal_order WHERE owner=?1 AND part_id=?2",
            params![key,part_id], |r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?,
        None => (-1, -1),
    };
    let mut query = c.prepare(PART_DIRECTORY_SQL)?;
    let mut rows = query.query(params![key, section, position, after.unwrap_or(""), (max_records + 1) as i64])?;
    let mut version = c.prepare(PART_VERSION_SQL)?;
    let (mut parts, mut used, mut last) = (Vec::new(), 0usize, None);
    let mut examined = 0;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        if examined == max_records {
            return Ok(AssistantPartPage {
                parts,
                next_part_id: last,
            });
        }
        examined += 1;
        let encoded: Option<String> = version
            .query_row(params![key, id, number(s.at.sequence)?], |r| r.get(0))
            .optional()?
            .flatten();
        if let Some(encoded) = encoded {
            if encoded.len() > max_bytes {
                return Err(invalid("part page budget cannot fit next head"));
            }
            if used + encoded.len() > max_bytes {
                return Ok(AssistantPartPage {
                    parts,
                    next_part_id: last,
                });
            }
            used += encoded.len();
            parts.push(serde_json::from_str(&encoded)?);
        }
        // Directory keys, not returned Part count, define progress. A page may
        // be empty when all inspected IDs are retired or postdate this snapshot.
        last = Some(id);
    }
    Ok(AssistantPartPage {
        parts,
        next_part_id: None,
    })
}
pub(super) fn part_changes(
    c: &Connection, snapshot: &AssistantJournalSnapshot, base: &AssistantJournalCursor,
    after: Option<&str>, max_records: usize, max_bytes: usize,
) -> StoreResult<AssistantPartChangePage> {
    limits(max_records, max_bytes)?;
    let _read = crate::read_snapshot::pin(c)?;
    let key = snapshot_key(c, snapshot)?;
    if base.identity != snapshot.at.identity || base.sequence > snapshot.at.sequence {
        return Err(invalid("Part changes require an ordered same-owner cursor window"));
    }
    verify_cursor(c, base)?;
    let floor: i64 = c.query_row("SELECT retained_floor FROM assistant_journal_heads WHERE owner=?1",
        params![key], |r| r.get(0))?;
    if number(base.sequence)? < floor {
        return Err(invalid("Part changes require a retained base snapshot"));
    }
    let mut query = c.prepare("SELECT DISTINCT part_id FROM assistant_journal_part_versions INDEXED BY assistant_journal_versions_by_sequence
        WHERE owner=?1 AND sequence>?2 AND sequence<=?3 AND part_id>?4 ORDER BY part_id LIMIT ?5")?;
    let mut rows = query.query(params![key, number(base.sequence)?, number(snapshot.at.sequence)?,
        after.unwrap_or(""), (max_records + 1) as i64])?;
    // Every row resolves the same fixed-snapshot query. Prepare once per page,
    // rather than compiling identical SQL for each changed Part.
    let mut version = c.prepare(PART_VERSION_SQL)?;
    let mut changes = Vec::new();
    let mut used = 0usize;
    let mut last = None;
    while let Some(row) = rows.next()? {
        let part_id: String = row.get(0)?;
        if changes.len() == max_records {
            return Ok(AssistantPartChangePage { changes, next_part_id: last });
        }
        let encoded: Option<String> = version.query_row(
            params![key, part_id, number(snapshot.at.sequence)?], |r| r.get(0))?;
        let size = part_id.len() + encoded.as_ref().map_or(4, String::len);
        if size > max_bytes { return Err(invalid("Part change page cannot fit next head")); }
        if used + size > max_bytes {
            return Ok(AssistantPartChangePage { changes, next_part_id: last });
        }
        used += size;
        let head = encoded.as_deref().map(serde_json::from_str).transpose()?;
        last = Some(part_id.clone());
        changes.push(AssistantPartChange { part_id, head });
    }
    Ok(AssistantPartChangePage { changes, next_part_id: None })
}

pub(super) fn authorize_content(
    c: &Connection,
    s: &AssistantJournalSnapshot,
    part: &str,
    r: &JournalContentRef,
) -> StoreResult<String> {
    let key = snapshot_key(c, s)?;
    let encoded: Option<String> = c
        .query_row(
            PART_VERSION_SQL,
            params![key, part, number(s.at.sequence)?],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let part_head: JournalPartHead =
        serde_json::from_str(&encoded.ok_or_else(|| invalid("Part not visible in snapshot"))?)?;
    if !part_head.bodies.contains(r) {
        return Err(invalid("content reference not reachable in snapshot"));
    }
    content::verify_reference(c, &s.at.identity, part, r)?;
    Ok(key)
}
pub(super) fn manifest(
    c: &Connection,
    s: &AssistantJournalSnapshot,
    part: &str,
    r: &JournalContentRef,
    after: u64,
    max_records: usize,
) -> StoreResult<AssistantManifestPage> {
    limits(max_records, ASSISTANT_FRAME_MAX_BYTES)?;
    let _read = crate::read_snapshot::pin(c)?;
    let key = authorize_content(c, s, part, r)?;
    content::manifest_page(c, &key, part, r, after, max_records)
}
pub(super) fn chunk(
    c: &Connection,
    s: &AssistantJournalSnapshot,
    part: &str,
    r: &JournalContentRef,
    index: u64,
) -> StoreResult<AssistantTextChunkV1> {
    let _read = crate::read_snapshot::pin(c)?;
    let page = manifest(c, s, part, r, index, 1)?;
    verified_chunk(c, page)
}

/// Operation replay can need an intermediate content root overwritten later
/// in the SAME record. This is not a relaxation of visible snapshot reads:
/// only references explicitly introduced by the pinned, verified record qualify.
pub(super) fn record_chunk(
    c: &Connection,
    s: &AssistantJournalSnapshot,
    part: &str,
    reference: &JournalContentRef,
    index: u64,
) -> StoreResult<AssistantTextChunkV1> {
    let _read = crate::read_snapshot::pin(c)?;
    let key = snapshot_key(c, s)?;
    let (header, bytes): (String, Vec<u8>) = c.query_row(
        "SELECT header_json,batch FROM assistant_journal_records WHERE owner=?1 AND sequence=?2",
        params![key, number(s.at.sequence)?], |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let header: AssistantJournalRecordHeaderV1 = serde_json::from_str(&header)?;
    let batch = AssistantOperationBatchV1::decode_canonical(&bytes).map_err(invalid)?;
    batch.verify_header(&header).map_err(invalid)?;
    if header.identity != s.at.identity || header.sequence != s.at.sequence
        || header.record_digest != s.at.record_digest {
        return Err(invalid("record content snapshot mismatch"));
    }
    receive_content::content_base(&StoredAssistantJournalRecord { header, batch }, part, reference)?;
    content::verify_reference(c, &s.at.identity, part, reference)?;
    verified_chunk(c, content::manifest_page(c, &key, part, reference, index, 1)?)
}

fn verified_chunk(c: &Connection, page: AssistantManifestPage) -> StoreResult<AssistantTextChunkV1> {
    let expected = page
        .chunks
        .first()
        .ok_or_else(|| invalid("no chunk at manifest index"))?;
    let bytes: Vec<u8> = c.query_row(
        "SELECT bytes FROM assistant_journal_chunks WHERE digest=?1",
        [expected.digest.to_hex()],
        |r| r.get(0),
    )?;
    let chunk =
        AssistantTextChunkV1::new(String::from_utf8(bytes).map_err(invalid)?).map_err(invalid)?;
    if chunk.digest() != expected.digest || chunk.byte_length() as u64 != expected.utf8_bytes.0 {
        return Err(invalid("stored chunk integrity failure"));
    }
    Ok(chunk)
}
