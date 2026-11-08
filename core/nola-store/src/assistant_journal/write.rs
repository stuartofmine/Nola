// SPDX-License-Identifier: Apache-2.0
use super::*;

pub(super) fn apply_frame(
    tx: &Connection,
    current: &AssistantJournalCursor,
    input: &AssistantJournalFrame,
    operation_id: String,
    revision: u64,
) -> StoreResult<AssistantJournalCursor> {
    let id = &input.batch.identity;
    let key = validate_owner(tx, id)?;
    apply_owned_frame(tx, current, input, operation_id, revision, &key)
}

/// The caller must prove local authoring or the exact authenticated remote Run
/// before entering this shared atomic mutation kernel.
pub(super) fn apply_owned_frame(
    tx: &Connection, current: &AssistantJournalCursor, input: &AssistantJournalFrame,
    operation_id: String, revision: u64, key: &str,
) -> StoreResult<AssistantJournalCursor> {
    let id = &input.batch.identity;
    if current.identity != *id {
        return Err(invalid("journal cursor owner mismatch"));
    }
    let bytes = input.batch.canonical_bytes().map_err(invalid)?;
    if head_for_owner(tx, id, key)? != *current {
        return Err(StoreError::Conflict("journal head CAS mismatch".into()));
    }
    // Validate supplied bytes before any public head mutation. The savepoint rolls
    // these preparation rows back if typed validation or any later write fails.
    let supplied = content::supply(tx, &input.supplied_chunks)?;
    let reader = content::Reader {
        c: tx,
        id,
        supplied: &supplied,
    };
    let changes = validate_assistant_batch(&reader, &input.batch).map_err(invalid)?;
    // The current Domain batch requires operations. A producer with unchanged
    // visible state must not submit a batch; never trust a caller NoChange flag.
    if changes.is_empty() {
        return Err(invalid("journal batch contains no validated changes"));
    }
    let sequence = current
        .sequence
        .checked_add(1)
        .ok_or_else(|| invalid("journal sequence overflow"))?;
    number(sequence)?;
    let header = input
        .batch
        .seal(sequence, current.record_digest, operation_id, revision)
        .map_err(invalid)?;
    order::record(tx, key, &input.batch)?;
    for operation in &input.batch.operations {
        if let AssistantJournalOperationV1::Append(v) = operation {
            let resulting = content::append(tx, &key, &v.part_id, &v.expected, &v.chunks)?;
            if resulting != v.resulting {
                return Err(invalid("append manifest differs from validated result"));
            }
        }
    }
    for change in &changes {
        // Canonical Part IDs are globally owned, even if a projection uses a
        // synthetic ID that is not itself a raw Core message_parts row.
        let foreign: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM message_parts WHERE id=?1 AND message_id!=?2)",
            params![change.part_id, id.assistant_message_id],
            |r| r.get(0),
        )?;
        if foreign {
            return Err(invalid("journal Part belongs to another assistant"));
        }
        let encoded = change
            .replacement
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        tx.execute("INSERT INTO assistant_journal_parts(owner,part_id,head_json) VALUES(?1,?2,?3) ON CONFLICT(owner,part_id) DO UPDATE SET head_json=excluded.head_json",params![key,change.part_id,encoded.as_deref().unwrap_or("null")])?;
        tx.execute("INSERT INTO assistant_journal_part_versions(owner,part_id,sequence,head_json) VALUES(?1,?2,?3,?4)",params![key,change.part_id,number(sequence)?,encoded])?;
    }
    tx.execute("INSERT INTO assistant_journal_records(owner,sequence,operation_id,source_revision,header_json,batch) VALUES(?1,?2,?3,?4,?5,?6)",params![key,number(sequence)?,header.operation_id,number(revision)?,serde_json::to_string(&header)?,bytes])?;
    tx.execute("INSERT INTO assistant_journal_heads(owner,sequence,digest) VALUES(?1,?2,?3) ON CONFLICT(owner) DO UPDATE SET sequence=excluded.sequence,digest=excluded.digest",params![key,number(sequence)?,header.record_digest.to_hex()])?;
    Ok(AssistantJournalCursor {
        identity: id.clone(),
        sequence,
        record_digest: header.record_digest,
    })
}
