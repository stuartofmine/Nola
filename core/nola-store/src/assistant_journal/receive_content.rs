// SPDX-License-Identifier: Apache-2.0
//! Record-scoped content preparation. Public manifests change only with the record.
use super::*;

pub(super) fn content_base(record: &StoredAssistantJournalRecord, part: &str, target: &JournalContentRef) -> StoreResult<JournalContentRef> {
    for op in &record.batch.operations {
        let introduced=match op {
            AssistantJournalOperationV1::Declare(v) => v.part.part_id==part && v.part.bodies.contains(target),
            AssistantJournalOperationV1::Replace(v) => v.part_id==part && v.part.bodies.contains(target),
            AssistantJournalOperationV1::ReplaceContent(v) => v.part_id==part && v.resulting.as_ref()==Some(target),
            AssistantJournalOperationV1::Append(v) if v.part_id==part && &v.resulting==target => return Ok(v.expected.clone()),
            _=>false,
        };
        if introduced {return empty_journal_content(&record.header.identity,part,target.field,target.content_generation).map_err(invalid);}
    }
    Err(invalid("prepared content is not introduced by this exact record"))
}

fn stage_id(record: &StoredAssistantJournalRecord, part: &str, target: &JournalContentRef) -> StoreResult<String> {
    Ok(serde_json::to_string(&(&record.header.identity,record.header.record_digest,part,target))?)
}

pub(super) fn stage(store: &mut NolaStore, input: &StageAssistantJournalContent) -> StoreResult<JournalContentRef> {
    require_uow(store)?;
    input.record.batch.verify_header(&input.record.header).map_err(invalid)?;
    let id=&input.record.header.identity;
    let key=owner(id)?;
    let base=content_base(&input.record,&input.part_id,&input.content)?;
    if input.expected_prefix.field!=base.field || input.expected_prefix.content_generation!=base.content_generation
        || input.chunks.is_empty() || input.chunks.len()>ASSISTANT_PAGE_MAX_RECORDS {
        return Err(invalid("invalid remote content preparation scope or budget"));
    }
    let mut size=0usize;
    let mut references=Vec::with_capacity(input.chunks.len());
    for chunk in &input.chunks {
        chunk.verify().map_err(invalid)?;
        size=size.checked_add(chunk.byte_length()).ok_or_else(||invalid("chunk budget overflow"))?;
        references.push(JournalChunkRef {digest:chunk.digest(),utf8_bytes:JournalNumber(chunk.byte_length() as u64)});
    }
    if size>ASSISTANT_FRAME_MAX_BYTES {return Err(invalid("preparation chunk budget"));}
    let result=append_journal_content(&input.expected_prefix,&references).map_err(invalid)?;
    if result.chunk_count.0>input.content.chunk_count.0 || result.utf8_bytes.0>input.content.utf8_bytes.0
        || (result.chunk_count==input.content.chunk_count && result!=input.content) {
        return Err(invalid("prepared content does not match committed manifest"));
    }
    let tx=store.write_transaction()?;
    receive::authorize(&tx,&input.source_host_id,id)?;
    let head=head_for_owner(&tx,id,&key)?;
    if input.record.header.sequence <= head.sequence {
        // Another receiver may publish this exact record while a chunk is in
        // flight. Acknowledge only content already committed under that owner;
        // never recreate preparation or accept a different record/prefix.
        let stored: String = tx.query_row(
            "SELECT header_json FROM assistant_journal_records WHERE owner=?1 AND sequence=?2",
            params![key, number(input.record.header.sequence)?], |row| row.get(0),
        )?;
        if serde_json::from_str::<AssistantJournalRecordHeaderV1>(&stored)? != input.record.header {
            return Err(invalid("remote journal replay conflicts with committed record"));
        }
        if input.restart && input.expected_prefix != base {
            return Err(invalid("preparation restart requires exact base"));
        }
        content::verify_reference(&tx, id, &input.part_id, &input.expected_prefix)?;
        content::verify_reference(&tx, id, &input.part_id, &result)?;
        content::verify_reference(&tx, id, &input.part_id, &input.content)?;
        tx.commit()?;
        return Ok(result);
    }
    if input.record.header.sequence!=head.sequence.checked_add(1).ok_or_else(||invalid("sequence overflow"))?
        || input.record.header.previous_record_digest!=head.record_digest {
        return Err(StoreError::Conflict("content preparation requires the exact next record".into()));
    }
    if content::verify_reference(&tx,id,&input.part_id,&base).is_err() {
        // A preceding operation in the same record may introduce this base.
        // Accept only its complete exact preparation, never a guessed digest.
        let prior=stage_id(&input.record,&input.part_id,&base)?;
        let prepared:bool=tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM remote_assistant_content_stages WHERE id=?1 AND content_json=prefix_json)",
            [prior],|row|row.get(0),
        )?;
        if !prepared {return Err(invalid("preparation base is neither committed nor fully prepared for this record"));}
    }
    let stage=stage_id(&input.record,&input.part_id,&input.content)?;
    if input.restart {
        if input.expected_prefix!=base {return Err(invalid("preparation restart requires exact base"));}
        tx.execute("DELETE FROM remote_assistant_content_stages WHERE id=?1",[&stage])?;
    }
    let previous:Option<String>=tx.query_row("SELECT prefix_json FROM remote_assistant_content_stages WHERE id=?1",[&stage],|r|r.get(0)).optional()?;
    if let Some(previous)=previous {
        let previous:JournalContentRef=serde_json::from_str(&previous)?;
        if previous!=input.expected_prefix {
            // Exact repeated page, including a lost ACK before later pages.
            if result.chunk_count.0>previous.chunk_count.0 {return Err(StoreError::Conflict("preparation prefix mismatch".into()));}
            if input.expected_prefix!=base {
                let root:Option<String>=tx.query_row("SELECT result_root FROM remote_assistant_content_stage_chunks WHERE stage_id=?1 AND chunk_index=?2",
                    params![stage,number(input.expected_prefix.chunk_count.0.checked_sub(1).ok_or_else(||invalid("prefix underflow"))?)?],|r|r.get(0)).optional()?;
                if root.as_deref()!=Some(&input.expected_prefix.content_root.to_hex()) {return Err(invalid("replayed preparation prefix mismatch"));}
            }
            for (offset,chunk) in input.chunks.iter().enumerate() {
                let bytes:Option<Vec<u8>>=tx.query_row("SELECT bytes FROM remote_assistant_content_stage_chunks WHERE stage_id=?1 AND chunk_index=?2",
                    params![stage,number(input.expected_prefix.chunk_count.0+offset as u64)?],|r|r.get(0)).optional()?;
                if bytes.as_deref()!=Some(chunk.text().as_bytes()) {return Err(invalid("replayed preparation bytes differ"));}
            }
            tx.commit()?;return Ok(result);
        }
    } else {
        if input.expected_prefix!=base {return Err(invalid("preparation must begin at exact content base"));}
        tx.execute("INSERT INTO remote_assistant_content_stages(id,owner,record_digest,part_id,base_json,content_json,prefix_json) VALUES(?1,?2,?3,?4,?5,?6,?5)",
            params![stage,key,input.record.header.record_digest.to_hex(),input.part_id,serde_json::to_string(&base)?,serde_json::to_string(&input.content)?])?;
    }
    tx.execute("INSERT OR IGNORE INTO remote_assistant_journal_sources(run_id,source_host_id,source_store_epoch,assistant_message_id) VALUES(?1,?2,?3,?4)",
        params![id.run_id,input.source_host_id,id.source_store_epoch,id.assistant_message_id])?;
    let mut prefix=input.expected_prefix.clone();
    for (chunk,reference) in input.chunks.iter().zip(&references) {
        let next=append_journal_content(&prefix,std::slice::from_ref(reference)).map_err(invalid)?;
        tx.execute("INSERT INTO remote_assistant_content_stage_chunks(stage_id,chunk_index,digest,result_root,bytes) VALUES(?1,?2,?3,?4,?5)",
            params![stage,number(prefix.chunk_count.0)?,chunk.digest().to_hex(),next.content_root.to_hex(),chunk.text().as_bytes()])?;
        prefix=next;
    }
    tx.execute("UPDATE remote_assistant_content_stages SET prefix_json=?2 WHERE id=?1",params![stage,serde_json::to_string(&prefix)?])?;
    tx.commit()?;
    Ok(result)
}

fn introduced_content(record: &StoredAssistantJournalRecord) -> Vec<(&String, &JournalContentRef)> {
    let mut ordered=Vec::new();
    for op in &record.batch.operations {
        match op {
            AssistantJournalOperationV1::Declare(v)=>ordered.extend(v.part.bodies.iter().map(|r|(&v.part.part_id,r))),
            AssistantJournalOperationV1::Replace(v)=>ordered.extend(v.part.bodies.iter().map(|r|(&v.part_id,r))),
            AssistantJournalOperationV1::Append(v)=>ordered.push((&v.part_id,&v.resulting)),
            AssistantJournalOperationV1::ReplaceContent(v)=>ordered.extend(v.resulting.iter().map(|r|(&v.part_id,r))),
            _=>{},
        }
    }
    ordered
}

/// Client adapters must not reconstruct empty roots, infer operation ordering,
/// or keep an in-memory download cursor as recovery authority.
pub(super) fn next_content(
    c: &Connection, source_host_id: &str, record: &StoredAssistantJournalRecord,
) -> StoreResult<Option<AssistantJournalContentNeed>> {
    let _read = crate::read_snapshot::pin(c)?;
    record.batch.verify_header(&record.header).map_err(invalid)?;
    let id = &record.header.identity;
    let key = owner(id)?;
    receive::authorize(c, source_host_id, id)?;
    let current = head_for_owner(c, id, &key)?;
    if record.header.sequence <= current.sequence {
        let stored: String = c.query_row(
            "SELECT header_json FROM assistant_journal_records WHERE owner=?1 AND sequence=?2",
            params![key, number(record.header.sequence)?], |row| row.get(0),
        )?;
        if serde_json::from_str::<AssistantJournalRecordHeaderV1>(&stored)? != record.header {
            return Err(invalid("remote journal replay conflicts with committed record"));
        }
        return Ok(None);
    }
    if record.header.sequence != current.sequence.checked_add(1).ok_or_else(|| invalid("sequence overflow"))?
        || record.header.previous_record_digest != current.record_digest {
        return Err(StoreError::Conflict("content preparation requires the exact next record".into()));
    }
    for (part, target) in introduced_content(record) {
        let base = content_base(record, part, target)?;
        if target == &base || content::verify_reference(c, id, part, target).is_ok() { continue; }
        let stage = stage_id(record, part, target)?;
        let prepared: Option<String> = c.query_row(
            "SELECT prefix_json FROM remote_assistant_content_stages WHERE id=?1",
            [stage], |row| row.get(0),
        ).optional()?;
        let prefix = prepared.map(|value| serde_json::from_str::<JournalContentRef>(&value)).transpose()?.unwrap_or(base);
        if &prefix == target { continue; }
        return Ok(Some(AssistantJournalContentNeed {
            part_id: part.clone(), content: target.clone(), expected_prefix: prefix,
        }));
    }
    Ok(None)
}

pub(super) fn activate(c: &Connection, record: &StoredAssistantJournalRecord) -> StoreResult<()> {
    let id=&record.header.identity;let key=owner(id)?;
    for (part,target) in introduced_content(record) {
        let stage=stage_id(record,part,target)?;
        let row:Option<(String,String)>=c.query_row("SELECT base_json,prefix_json FROM remote_assistant_content_stages WHERE id=?1",[&stage],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((base,prepared))=row else {continue};
        let mut prefix:JournalContentRef=serde_json::from_str(&base)?;
        let prepared:JournalContentRef=serde_json::from_str(&prepared)?;
        if &prepared!=target || content_base(record,part,target)?!=prefix {return Err(invalid("remote manifest preparation incomplete"));}
        content::verify_reference(c,id,&part,&prefix)?;
        let mut chunks=c.prepare("SELECT chunk_index,digest,bytes FROM remote_assistant_content_stage_chunks WHERE stage_id=?1 ORDER BY chunk_index")?;
        let mut pages=chunks.query([stage])?;
        while let Some(page)=pages.next()? {
            if page.get::<_,u64>(0)?!=prefix.chunk_count.0 {return Err(invalid("prepared chunk sequence gap"));}
            let bytes:Vec<u8>=page.get(2)?;
            let chunk=AssistantTextChunkV1::new(String::from_utf8(bytes).map_err(invalid)?).map_err(invalid)?;
            if chunk.digest().to_hex()!=page.get::<_,String>(1)? {return Err(invalid("prepared chunk digest mismatch"));}
            let refs=content::supply(c,&[chunk])?;
            prefix=content::append(c,&key,&part,&prefix,&refs)?;
        }
        if &prefix!=target {return Err(invalid("prepared manifest root mismatch"));}
    }
    Ok(())
}
