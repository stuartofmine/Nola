// SPDX-License-Identifier: Apache-2.0
//! Snapshot-authorized public journal ranges. Does not reinterpret canonical fields.
use super::*;
use nola_domain::assistant_journal::{ASSISTANT_RANGE_MAX_BYTES, ASSISTANT_RANGE_MIN_BYTES};

pub(super) const RANGE_SQL: &str =
    "SELECT m.byte_count,m.chunk_count,m.root,m.previous_root,m.chunk_digest,c.bytes
 FROM assistant_journal_manifests m INDEXED BY assistant_journal_manifest_byte_range
 JOIN assistant_journal_chunks c ON c.digest=m.chunk_digest
 WHERE m.owner=?1 AND m.part_id=?2 AND m.field=?3 AND m.generation=?4
 AND m.byte_count>?5 AND m.byte_count<=?6
 ORDER BY m.byte_count LIMIT ?7";

pub(super) fn read(
    c: &Connection,
    s: &AssistantJournalSnapshot,
    part: &str,
    content: &JournalContentRef,
    offset: u64,
    limit: usize,
) -> StoreResult<AssistantContentRange> {
    if !(ASSISTANT_RANGE_MIN_BYTES..=ASSISTANT_RANGE_MAX_BYTES).contains(&limit) {
        return Err(invalid("journal range raw-byte budget must be 4..32768"));
    }
    number(offset)?;
    let _snapshot = crate::read_snapshot::pin(c)?;
    let key = reads::authorize_content(c, s, part, content)?;
    read_verified(c, &key, part, content, offset, limit)
}

// Caller must establish the exact published owner and reachable content first.
pub(super) fn read_verified(c: &Connection, key: &str, part: &str,
    content: &JournalContentRef, offset: u64, limit: usize,
) -> StoreResult<AssistantContentRange> {
    if offset > content.utf8_bytes.0 {
        return Err(invalid("range starts beyond content"));
    }
    let mut result = AssistantContentRange {
        offset_bytes: offset,
        next_offset_bytes: offset,
        total_utf8_bytes: content.utf8_bytes.0,
        text: String::new(),
        end_of_content: offset == content.utf8_bytes.0,
    };
    if result.end_of_content {
        return Ok(result);
    }
    let mut statement = c.prepare(RANGE_SQL)?;
    // Each stored chunk contains at least one byte; no range can require more
    // than limit chunks. The extra row permits scalar-boundary lookahead.
    let mut rows = statement.query(params![
        key,
        part,
        content::field(content.field)?,
        number(content.content_generation.0)?,
        number(offset)?,
        number(content.utf8_bytes.0)?,
        (limit + 1) as i64
    ])?;
    let mut previous: Option<(u64, u64, String)> = None;
    let mut budget_stopped = false;
    while let Some(row) = rows.next()? {
        let end: u64 = row.get(0)?;
        let count: u64 = row.get(1)?;
        let root: String = row.get(2)?;
        let preceding: Option<String> = row.get(3)?;
        let digest: String = row.get(4)?;
        let bytes: Vec<u8> = row.get(5)?;
        let chunk = AssistantTextChunkV1::new(String::from_utf8(bytes).map_err(invalid)?)
            .map_err(invalid)?;
        if chunk.digest().to_hex() != digest {
            return Err(invalid("range chunk digest mismatch"));
        }
        let start = end
            .checked_sub(chunk.byte_length() as u64)
            .ok_or_else(|| invalid("manifest byte endpoint underflow"))?;
        if count == 0 || count > content.chunk_count.0 {
            return Err(invalid("manifest chunk count mismatch"));
        }
        if let Some((prior_end, prior_count, prior_root)) = &previous {
            if *prior_end != start
                || *prior_count + 1 != count
                || preceding.as_deref() != Some(prior_root.as_str())
            {
                return Err(invalid("manifest range gap or chain mismatch"));
            }
        } else if start > offset || end <= offset {
            return Err(invalid("manifest does not cover requested byte"));
        }
        if end == content.utf8_bytes.0
            && (count != content.chunk_count.0 || root != content.content_root.to_hex())
        {
            return Err(invalid("manifest endpoint differs from fixed reference"));
        }
        let begin = usize::try_from(result.next_offset_bytes - start).map_err(invalid)?;
        if !chunk.text().is_char_boundary(begin) {
            return Err(invalid("range offset is not a UTF-8 boundary"));
        }
        let remaining = limit - result.text.len();
        let mut stop = (begin + remaining).min(chunk.byte_length());
        while !chunk.text().is_char_boundary(stop) {
            stop -= 1;
        }
        result.text.push_str(&chunk.text()[begin..stop]);
        result.next_offset_bytes += (stop - begin) as u64;
        if stop < chunk.byte_length() || result.text.len() == limit {
            budget_stopped = true;
            break;
        }
        previous = Some((end, count, root));
        if result.next_offset_bytes == content.utf8_bytes.0 {
            break;
        }
    }
    if result.next_offset_bytes == offset {
        return Err(invalid("range cannot make UTF-8 progress"));
    }
    if !budget_stopped && result.next_offset_bytes < content.utf8_bytes.0 {
        return Err(invalid("manifest range ended before fixed reference"));
    }
    result.end_of_content = result.next_offset_bytes == content.utf8_bytes.0;
    Ok(result)
}
