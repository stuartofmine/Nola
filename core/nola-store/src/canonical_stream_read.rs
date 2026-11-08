// SPDX-License-Identifier: Apache-2.0
//! Read-only private stream access. Range checks local hashes and geometry in a
//! fixed snapshot with trusted transaction layout; it is not an ancestry proof
//! against arbitrary self-consistent database forks. require_complete audits the
//! entire manifest ancestry. No tables or production writers are created here.
use crate::canonical_text_store::CanonicalTextRange;
use crate::{params, Connection, OptionalExtension, StoreError, StoreResult};
use nola_domain::{apply_canonical_stream, CanonicalStreamHead, CanonicalStreamOperation};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}
fn integer(value: u64) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| invalid("stream integer exceeds SQLite range"))
}
fn decode(encoded: Option<String>) -> StoreResult<CanonicalStreamHead> {
    let encoded = encoded.ok_or_else(|| invalid("oversized stream head"))?;
    let head: CanonicalStreamHead = serde_json::from_str(&encoded)?;
    head.validate()
        .map_err(|_| invalid("invalid stream head"))?;
    for value in [
        head.generation,
        head.revision,
        head.chunk_count,
        head.total_utf8_bytes,
    ] {
        integer(value)?;
    }
    Ok(head)
}

pub(super) fn resolve(
    connection: &Connection,
    conversation: &str,
    message: &str,
    part: &str,
) -> StoreResult<Option<CanonicalStreamHead>> {
    let _snapshot = crate::read_snapshot::pin(connection)?;
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='canonical_stream_heads')",
        [], |row| row.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let raw: Option<Option<String>> = connection
        .query_row(
            "SELECT CASE WHEN length(CAST(head_json AS BLOB))<=4096 THEN head_json END
         FROM canonical_stream_heads WHERE part_id=?1",
            [part],
            |row| row.get(0),
        )
        .optional()?;
    let Some(raw) = raw else { return Ok(None) };
    let head = decode(raw)?;
    if head.owner.conversation_id != conversation
        || head.owner.message_id != message
        || head.owner.part_id != part
    {
        return Err(invalid("stream requested owner differs"));
    }
    let valid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM message_parts p JOIN messages m ON m.id=p.message_id
         WHERE p.id=?1 AND m.id=?2 AND m.conversation_id=?3
         AND m.role='assistant' AND p.kind='text' AND p.content_text=''
         AND COALESCE((SELECT e.visible FROM message_projection_events e
             JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
             WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
             ORDER BY o.resulting_revision DESC LIMIT 1),1)=1
         AND NOT EXISTS(SELECT 1 FROM message_part_body_references b
             WHERE b.part_id=p.id AND b.field='content_text'))",
        params![part, message, conversation],
        |row| row.get(0),
    )?;
    if !valid
        || !crate::canonical_message_ownership::matches_read_owner(
            connection,
            conversation,
            message,
            &head.owner.run_id,
        )?
    {
        return Err(invalid("stream persisted owner or text backing conflicts"));
    }
    verify_manifest(connection, &head)?;
    check_tail(connection, &head)?;
    Ok(Some(head))
}

fn current(connection: &Connection, head: &CanonicalStreamHead) -> StoreResult<()> {
    let actual = resolve(
        connection,
        &head.owner.conversation_id,
        &head.owner.message_id,
        &head.owner.part_id,
    )?;
    if actual.as_ref() != Some(head) {
        return Err(invalid("stream head changed or is missing"));
    }
    Ok(())
}

struct Manifest {
    head: CanonicalStreamHead,
    previous: Option<String>,
    operation: String,
}
fn manifest(connection: &Connection, part: &str, root: &str) -> StoreResult<Manifest> {
    let row: Option<(Option<String>, Option<String>, String)> = connection
        .query_row(
            "SELECT CASE WHEN length(CAST(head_json AS BLOB))<=4096 THEN head_json END,
         previous_root,operation FROM canonical_stream_manifests WHERE part_id=?1 AND root=?2",
            params![part, root],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (encoded, previous, operation) = row.ok_or_else(|| invalid("missing stream manifest"))?;
    let head = decode(encoded)?;
    if head.root != root || head.owner.part_id != part {
        return Err(invalid("stream manifest identity differs"));
    }
    Ok(Manifest {
        head,
        previous,
        operation,
    })
}

struct Chunk {
    index: u64,
    offset: u64,
    text: String,
    root: String,
}
fn chunk_at_index(
    connection: &Connection,
    head: &CanonicalStreamHead,
    index: u64,
) -> StoreResult<Chunk> {
    let row: Option<(i64,Option<String>,String)> = connection.query_row(
        "SELECT offset_bytes,CASE WHEN length(CAST(text AS BLOB)) BETWEEN 1 AND 32768 THEN text END,
         manifest_root FROM canonical_stream_chunks WHERE part_id=?1 AND generation=?2 AND chunk_index=?3",
        params![head.owner.part_id,integer(head.generation)?,integer(index)?],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional()?;
    let (offset, text, root) = row.ok_or_else(|| invalid("missing stream chunk"))?;
    Ok(Chunk {
        index,
        offset: u64::try_from(offset).map_err(|_| invalid("negative chunk offset"))?,
        text: text.ok_or_else(|| invalid("invalid stream chunk size"))?,
        root,
    })
}

/// Recompute one transition from its predecessor header and bounded payload.
/// The predecessor's own ancestry is intentionally left to require_complete.
fn verify_manifest(
    connection: &Connection,
    head: &CanonicalStreamHead,
) -> StoreResult<Option<CanonicalStreamHead>> {
    let record = manifest(connection, &head.owner.part_id, &head.root)?;
    if record.head != *head {
        return Err(invalid("stream head and manifest differ"));
    }
    let old = record
        .previous
        .as_ref()
        .map(|root| manifest(connection, &head.owner.part_id, root).map(|m| m.head))
        .transpose()?;
    let operation = if record.operation == "declare" {
        if old.is_some() {
            return Err(invalid("declare has previous stream head"));
        }
        CanonicalStreamOperation::Declare {
            owner: head.owner.clone(),
        }
    } else {
        let expected = old
            .clone()
            .ok_or_else(|| invalid("stream predecessor missing"))?;
        match record.operation.as_str() {
            "append" | "replace" => {
                let text = if head.chunk_count == 0 {
                    if record.operation != "replace" {
                        return Err(invalid("empty stream append"));
                    }
                    String::new()
                } else {
                    let chunk = chunk_at_index(connection, head, head.chunk_count - 1)?;
                    let offset = if record.operation == "replace" {
                        0
                    } else {
                        expected.total_utf8_bytes
                    };
                    if chunk.root != head.root || chunk.offset != offset {
                        return Err(invalid("stream chunk manifest or offset differs"));
                    }
                    chunk.text
                };
                if record.operation == "append" {
                    CanonicalStreamOperation::Append {
                        owner: head.owner.clone(),
                        expected_head: expected,
                        delta: text,
                    }
                } else {
                    CanonicalStreamOperation::Replace {
                        owner: head.owner.clone(),
                        expected_head: expected,
                        text,
                    }
                }
            }
            "close" => CanonicalStreamOperation::Close {
                owner: head.owner.clone(),
                expected_head: expected,
            },
            _ => return Err(invalid("unknown stream manifest operation")),
        }
    };
    let actual = apply_canonical_stream(old.as_ref(), &operation)
        .map_err(|_| invalid("invalid stream manifest transition"))?;
    if actual != *head {
        return Err(invalid("stream manifest content hash differs"));
    }
    Ok(old)
}

const CHECK_TAIL_SQL: &str =
    "SELECT EXISTS(SELECT 1 FROM canonical_stream_chunks WHERE part_id=?1 AND generation=?2
         AND chunk_index>=?3) OR EXISTS(SELECT 1 FROM canonical_stream_chunks
         WHERE part_id=?1 AND generation=?2 AND offset_bytes>=?4)";

fn check_tail(connection: &Connection, head: &CanonicalStreamHead) -> StoreResult<()> {
    let extra: bool = connection.query_row(
        CHECK_TAIL_SQL,
        params![
            head.owner.part_id,
            integer(head.generation)?,
            integer(head.chunk_count)?,
            integer(head.total_utf8_bytes)?
        ],
        |row| row.get(0),
    )?;
    if extra {
        return Err(invalid("stream contains extra tail chunks"));
    }
    if head.chunk_count > 0 {
        let last = chunk_at_index(connection, head, head.chunk_count - 1)?;
        if last.offset.checked_add(last.text.len() as u64) != Some(head.total_utf8_bytes) {
            return Err(invalid("stream last chunk length differs"));
        }
    }
    Ok(())
}

pub(super) fn range(
    connection: &Connection,
    head: &CanonicalStreamHead,
    offset: u64,
    limit: usize,
) -> StoreResult<CanonicalTextRange> {
    let _snapshot = crate::read_snapshot::pin(connection)?;
    if !(4..=32768).contains(&limit) || offset > head.total_utf8_bytes {
        return Err(invalid("invalid stream range"));
    }
    current(connection, head)?;
    let mut result = String::with_capacity(limit);
    let mut cursor = offset;
    if offset < head.total_utf8_bytes {
        let index: Option<i64> = connection
            .query_row(
                "SELECT chunk_index FROM canonical_stream_chunks WHERE part_id=?1 AND generation=?2
             AND offset_bytes<=?3 ORDER BY offset_bytes DESC LIMIT 1",
                params![
                    head.owner.part_id,
                    integer(head.generation)?,
                    integer(offset)?
                ],
                |row| row.get(0),
            )
            .optional()?;
        let mut index =
            u64::try_from(index.ok_or_else(|| invalid("stream range missing first chunk"))?)
                .map_err(|_| invalid("negative stream index"))?;
        loop {
            let chunk = chunk_at_index(connection, head, index)?;
            verify_chunk(connection, head, &chunk)?;
            let skip = cursor
                .checked_sub(chunk.offset)
                .and_then(|v| usize::try_from(v).ok())
                .ok_or_else(|| invalid("stream range gap"))?;
            if skip >= chunk.text.len() || !chunk.text.is_char_boundary(skip) {
                return Err(invalid("invalid stream UTF-8 offset"));
            }
            let mut end = (skip + limit - result.len()).min(chunk.text.len());
            while !chunk.text.is_char_boundary(end) {
                end -= 1;
            }
            result.push_str(&chunk.text[skip..end]);
            cursor += (end - skip) as u64;
            if cursor > head.total_utf8_bytes {
                return Err(invalid("stream exceeds head"));
            }
            if end < chunk.text.len() || result.len() == limit || cursor == head.total_utf8_bytes {
                break;
            }
            index = index
                .checked_add(1)
                .ok_or_else(|| invalid("stream index overflow"))?;
        }
        if result.is_empty() {
            return Err(invalid("stream range made no progress"));
        }
    }
    Ok(CanonicalTextRange {
        offset_bytes: offset,
        next_offset_bytes: cursor,
        text: result,
        end_of_content: cursor == head.total_utf8_bytes,
    })
}

fn verify_chunk(
    connection: &Connection,
    current: &CanonicalStreamHead,
    chunk: &Chunk,
) -> StoreResult<()> {
    if chunk.index >= current.chunk_count {
        return Err(invalid("stream chunk exceeds head count"));
    }
    let record = manifest(connection, &current.owner.part_id, &chunk.root)?;
    if record.head.owner != current.owner
        || record.head.generation != current.generation
        || record.head.chunk_count != chunk.index + 1
        || record.head.revision > current.revision
        || record.head.total_utf8_bytes != chunk.offset + chunk.text.len() as u64
    {
        return Err(invalid("stream chunk owner or geometry differs"));
    }
    verify_manifest(connection, &record.head)?;
    if chunk.index == 0 {
        if chunk.offset != 0 {
            return Err(invalid("stream first offset is not zero"));
        }
    } else {
        let previous = chunk_at_index(connection, current, chunk.index - 1)?;
        if previous.offset.checked_add(previous.text.len() as u64) != Some(chunk.offset)
            || record.previous.as_deref() != Some(previous.root.as_str())
        {
            return Err(invalid("stream chunk predecessor differs"));
        }
    }
    Ok(())
}

/// Character tail with local integrity checks only, not an ancestry audit.
/// Visit the newest indexed chunks until one extra scalar proves truncation.
pub(super) fn tail(
    connection: &Connection,
    head: &CanonicalStreamHead,
    characters: usize,
) -> StoreResult<String> {
    let _snapshot = crate::read_snapshot::pin(connection)?;
    let wanted = characters
        .checked_add(1)
        .filter(|_| characters > 0)
        .ok_or_else(|| invalid("invalid stream character tail budget"))?;
    current(connection, head)?;
    let mut reversed = Vec::new();
    for index in (0..head.chunk_count).rev() {
        let chunk = chunk_at_index(connection, head, index)?;
        verify_chunk(connection, head, &chunk)?;
        for character in chunk.text.chars().rev() {
            reversed.push(character);
            if reversed.len() == wanted {
                break;
            }
        }
        if reversed.len() == wanted {
            break;
        }
    }
    let truncated = reversed.len() > characters;
    if truncated {
        reversed.pop();
    }
    let mut result = String::new();
    if truncated {
        result.push('…');
    }
    result.extend(reversed.into_iter().rev());
    Ok(result)
}

pub(super) fn require_complete(
    connection: &Connection,
    head: &CanonicalStreamHead,
) -> StoreResult<()> {
    audit_complete(connection, head, None)
}

/// Complete ancestry/geometry validation and a sequential content SHA in one
/// fixed snapshot. No validation certificate survives this function call.
pub(super) fn complete_digest(
    connection: &Connection,
    head: &CanonicalStreamHead,
) -> StoreResult<String> {
    let mut digest = Sha256::new();
    audit_complete(connection, head, Some(&mut digest))?;
    Ok(format!("{:x}", digest.finalize()))
}

fn audit_complete(
    connection: &Connection,
    head: &CanonicalStreamHead,
    mut content_digest: Option<&mut Sha256>,
) -> StoreResult<()> {
    let _snapshot = crate::read_snapshot::pin(connection)?;
    current(connection, head)?;
    let mut cursor = Some(head.clone());
    let mut last_generation = None;
    // Cache only already verified current-generation manifest metadata, not
    // text or a cross-transaction "trusted" flag. Older generations are still
    // audited, but need not remain in memory for the current content pass.
    let mut verified = HashMap::new();
    while let Some(value) = cursor {
        // The pure transition verifies strict revision descent, preventing cycles.
        if last_generation != Some(value.generation) {
            check_tail(connection, &value)?;
            last_generation = Some(value.generation);
        }
        let previous = verify_manifest(connection, &value)?;
        if value.generation == head.generation {
            verified.insert(
                value.root.clone(),
                (value, previous.as_ref().map(|old| old.root.clone())),
            );
        }
        cursor = previous;
    }
    let mut next_offset = 0u64;
    let mut previous_root: Option<String> = None;
    for index in 0..head.chunk_count {
        let chunk = chunk_at_index(connection, head, index)?;
        // Membership is required, not just a locally self-consistent chunk:
        // a valid-looking fork must not replace a chunk of the audited chain.
        let (manifest, predecessor) = verified
            .get(&chunk.root)
            .ok_or_else(|| invalid("stream chunk is outside the audited ancestry"))?;
        let end = chunk
            .offset
            .checked_add(chunk.text.len() as u64)
            .ok_or_else(|| invalid("stream chunk offset overflow"))?;
        if chunk.index != index
            || chunk.offset != next_offset
            || manifest.owner != head.owner
            || manifest.generation != head.generation
            || manifest.chunk_count != index + 1
            || manifest.revision > head.revision
            || manifest.total_utf8_bytes != end
            || (index > 0 && predecessor.as_deref() != previous_root.as_deref())
        {
            return Err(invalid(
                "stream audited chunk geometry or predecessor differs",
            ));
        }
        // The exact transition payload was hashed above and cannot change
        // within this snapshot. Do not repeat verify_manifest for each range.
        if let Some(digest) = content_digest.as_deref_mut() {
            digest.update(chunk.text.as_bytes());
        }
        next_offset = end;
        previous_root = Some(chunk.root);
    }
    if next_offset != head.total_utf8_bytes {
        return Err(invalid("stream audited content length differs"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "canonical_stream_read_tests.rs"]
mod tests;
