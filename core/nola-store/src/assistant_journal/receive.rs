// SPDX-License-Identifier: Apache-2.0
//! Remote records preserve source sequence/digest; they never become new local publications.
use super::*;

pub(super) fn reception_state(
    c: &Connection, source_host_id: &str, through: &AssistantJournalCursor,
) -> StoreResult<AssistantJournalReceptionState> {
    let _read = crate::read_snapshot::pin(c)?;
    let key = owner(&through.identity)?;
    number(through.sequence)?;
    authorize(c, source_host_id, &through.identity)?;
    let after = head_for_owner(c, &through.identity, &key)?;
    let complete = after.sequence >= through.sequence;
    if complete {
        if through.sequence == 0 {
            if *through != AssistantJournalCursor::initial(through.identity.clone()).map_err(invalid)? {
                return Err(invalid("invalid journal genesis"));
            }
        } else {
            reads::verify_cursor(c, through)?;
        }
    }
    Ok(AssistantJournalReceptionState { after, complete })
}

pub(super) fn has_bound_read_owner(c: &Connection, id: &AssistantJournalIdentity) -> StoreResult<bool> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM remote_assistant_journal_sources j
         JOIN remote_run_sources s ON s.run_id=j.run_id AND s.source_host_id=j.source_host_id
         JOIN runs r ON r.id=j.run_id JOIN messages m ON m.id=j.assistant_message_id
         WHERE j.run_id=?1 AND j.source_store_epoch=?2 AND j.assistant_message_id=?3
         AND r.conversation_id=?4 AND m.conversation_id=?4 AND m.run_id=r.id AND m.role='assistant')",
        params![id.run_id,id.source_store_epoch,id.assistant_message_id,id.conversation_id], |row|row.get(0),
    ).map_err(Into::into)
}

pub(super) fn receive(
    store: &mut NolaStore, source_host_id: &str, record: &StoredAssistantJournalRecord,
    chunks: &[AssistantTextChunkV1],
) -> StoreResult<AssistantJournalCommit> {
    require_uow(store)?;
    crate::require_id("authenticated source Host",source_host_id)?;
    record.batch.verify_header(&record.header).map_err(invalid)?;
    let id=&record.header.identity;
    let key=owner(id)?;
    let bytes=chunks.iter().try_fold(0usize,|total,chunk| {
        chunk.verify().map_err(invalid)?;
        total.checked_add(chunk.byte_length()).ok_or_else(||invalid("received chunk budget overflow"))
    })?;
    if chunks.len()>ASSISTANT_PAGE_MAX_RECORDS || bytes>ASSISTANT_FRAME_MAX_BYTES {
        return Err(invalid("received chunk budget"));
    }
    let tx=store.write_transaction()?;
    authorize(&tx,source_host_id,id)?;
    let current=head_for_owner(&tx,id,&key)?;
    if record.header.sequence<=current.sequence {
        let stored:Option<String>=tx.query_row(
            "SELECT header_json FROM assistant_journal_records WHERE owner=?1 AND sequence=?2",
            params![key,number(record.header.sequence)?],|row|row.get(0),
        ).optional()?;
        let stored=stored.ok_or_else(||invalid("remote journal replay record unavailable"))?;
        if serde_json::from_str::<AssistantJournalRecordHeaderV1>(&stored)?!=record.header {
            return Err(invalid("remote journal replay conflicts with committed record"));
        }
        tx.commit()?;
        return Ok(AssistantJournalCommit::Replay(AssistantJournalCursor {
            identity:id.clone(),sequence:record.header.sequence,record_digest:record.header.record_digest,
        }));
    }
    if record.header.sequence!=current.sequence.checked_add(1).ok_or_else(||invalid("journal sequence overflow"))?
        || record.header.previous_record_digest!=current.record_digest {
        return Err(StoreError::Conflict("remote journal gap or previous digest mismatch".into()));
    }
    tx.execute("INSERT OR IGNORE INTO remote_assistant_journal_sources(run_id,source_host_id,source_store_epoch,assistant_message_id) VALUES(?1,?2,?3,?4)",
        params![id.run_id,source_host_id,id.source_store_epoch,id.assistant_message_id])?;
    receive_content::activate(&tx,record)?;
    let next=write::apply_owned_frame(&tx,&current,&AssistantJournalFrame {
        batch:record.batch.clone(),supplied_chunks:chunks.to_vec(),
    },record.header.operation_id.clone(),record.header.source_commit_revision,&key)?;
    if next.record_digest!=record.header.record_digest {return Err(invalid("received record digest changed during application"));}
    tx.execute("DELETE FROM remote_assistant_content_stages WHERE owner=?1 AND record_digest=?2",params![key,record.header.record_digest.to_hex()])?;
    tx.execute("UPDATE conversations SET revision=revision+1 WHERE id=?1",[&id.conversation_id])?;
    tx.commit()?;
    Ok(AssistantJournalCommit::Applied(next))
}

pub(super) fn authorize(c: &Connection, source_host_id: &str, id: &AssistantJournalIdentity) -> StoreResult<()> {
    crate::require_id("authenticated source Host", source_host_id)?;
    let authorized:bool=c.query_row(
        "SELECT EXISTS(SELECT 1 FROM runs r JOIN remote_run_sources s ON s.run_id=r.id
         JOIN messages m ON m.id=?3 WHERE r.id=?1 AND r.conversation_id=?2 AND r.kind='conversation'
         AND (r.assistant_message_id=m.id OR EXISTS(SELECT 1 FROM remote_assistant_journal_sources j
             WHERE j.run_id=r.id AND j.assistant_message_id=m.id AND j.source_host_id=?4 AND j.source_store_epoch=?5))
         AND m.conversation_id=?2 AND m.run_id=r.id
         AND m.role='assistant' AND s.source_host_id=?4)",
        params![id.run_id,id.conversation_id,id.assistant_message_id,source_host_id,id.source_store_epoch],|row|row.get(0),
    )?;
    if !authorized {return Err(invalid("remote journal has no exact authenticated Run/assistant binding"));}
    let binding:Option<(String,String,String)>=c.query_row(
        "SELECT source_host_id,source_store_epoch,assistant_message_id FROM remote_assistant_journal_sources WHERE run_id=?1 AND assistant_message_id=?2",
        params![id.run_id,id.assistant_message_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional()?;
    if binding.as_ref().is_some_and(|(host,epoch,message)|host!=source_host_id || epoch!=&id.source_store_epoch || message!=&id.assistant_message_id) {
        return Err(invalid("remote journal source incarnation changed"));
    }
    Ok(())
}
