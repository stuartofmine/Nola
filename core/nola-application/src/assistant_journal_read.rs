// SPDX-License-Identifier: Apache-2.0
//! Fixed-watermark public journal reads. No checkpoint or latest-message lookup.
use nola_domain::assistant_journal::{
    AssistantJournalCursor, AssistantJournalRecordHeaderV1,
    ASSISTANT_FRAME_MAX_BYTES, ASSISTANT_PAGE_MAX_RECORDS,
};
use nola_domain::assistant_journal_operations::AssistantOperationBatchV1;
use nola_store_port::assistant_journal::{
    AssistantJournalPage, AssistantJournalReadWindow, AssistantJournalRepository,
};
use serde::{Deserialize, Serialize};
use base64::Engine;
use nola_domain::assistant_journal_operations::{JournalContentRef, JournalNumber};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalManifestPage {
    pub chunks: Vec<nola_domain::assistant_journal_operations::JournalChunkRef>,
    pub next_chunk: JournalNumber,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalChunk {
    pub digest: nola_domain::assistant_journal::AssistantDigest,
    pub utf8_bytes: JournalNumber,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalReceiveResult {
    pub applied: bool,
    pub cursor: AssistantJournalCursor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageAssistantJournalContent {
    pub source_host_id: String,
    pub record: AssistantJournalRecord,
    pub part_id: String,
    pub content: JournalContentRef,
    pub expected_prefix: JournalContentRef,
    pub chunks: Vec<AssistantJournalChunk>,
    pub restart: bool,
}

pub(crate) fn stage_content<R: AssistantJournalRepository
    + nola_store_port::protocol_journal::CommandProtocolRepositoryProvider + ?Sized>(
    store: &mut R, input: StageAssistantJournalContent,
) -> Result<JournalContentRef,String> {
    check_wire_budget(&input)?;
    let chunks=decode_chunks(input.chunks)?;
    crate::assistant_publication_transaction::with_publication_transaction(store,|store| {
        store.stage_assistant_journal_content(&nola_store_port::assistant_journal::StageAssistantJournalContent {
            source_host_id:input.source_host_id,
            record:nola_store_port::assistant_journal::StoredAssistantJournalRecord {header:input.record.header,batch:input.record.batch},
            part_id:input.part_id,content:input.content,expected_prefix:input.expected_prefix,chunks,restart:input.restart,
        }).map_err(|e|e.to_string())
    })
}

pub(crate) fn next_content<R: AssistantJournalRepository + ?Sized>(
    store: &R, source_host_id: &str, record: AssistantJournalRecord,
) -> Result<Option<nola_store_port::assistant_journal::AssistantJournalContentNeed>, String> {
    check_wire_budget(&record)?;
    let result = store.next_assistant_journal_content(source_host_id,
        &nola_store_port::assistant_journal::StoredAssistantJournalRecord {header:record.header,batch:record.batch},
    ).map_err(|e|e.to_string())?;
    check_wire_budget(&result)?;
    Ok(result)
}

pub(crate) fn receive_record<R: AssistantJournalRepository
    + nola_store_port::protocol_journal::CommandProtocolRepositoryProvider + ?Sized>(
    store: &mut R, source_host_id: &str, record: AssistantJournalRecord,
    chunks: Vec<AssistantJournalChunk>,
) -> Result<AssistantJournalReceiveResult,String> {
    // Admission bounds the actual wire bundle before decoding or staging data.
    check_wire_budget(&(&record,&chunks))?;
    let verified=decode_chunks(chunks)?;
    crate::assistant_publication_transaction::with_publication_transaction(store,|store| {
        let result=store.receive_assistant_journal_record(source_host_id,
            &nola_store_port::assistant_journal::StoredAssistantJournalRecord {header:record.header,batch:record.batch},
            &verified,
        ).map_err(|e|e.to_string())?;
        let (applied,cursor)=match result {
            nola_store_port::assistant_journal::AssistantJournalCommit::Applied(cursor)=>(true,cursor),
            nola_store_port::assistant_journal::AssistantJournalCommit::Replay(cursor)=>(false,cursor),
        };
        Ok(AssistantJournalReceiveResult {applied,cursor})
    })
}

pub(crate) fn read_manifest<R: AssistantJournalRepository + ?Sized>(
    store: &R, snapshot: AssistantJournalSnapshotHandle, part_id: &str,
    content: &JournalContentRef, after_chunk: JournalNumber, max_records: usize,
) -> Result<AssistantJournalManifestPage, String> {
    let page=store.read_assistant_snapshot_manifest(
        &snapshot.stored(),part_id,content,after_chunk.0,max_records,
    ).map_err(|e|e.to_string())?;
    let result=AssistantJournalManifestPage {
        chunks:page.chunks,next_chunk:JournalNumber(page.next_chunk),complete:page.complete,
    };
    check_wire_budget(&result)?;
    Ok(result)
}

pub(crate) fn read_record_chunk<R: AssistantJournalRepository + ?Sized>(
    store: &R, snapshot: AssistantJournalSnapshotHandle, part_id: &str,
    content: &JournalContentRef, chunk_index: JournalNumber,
) -> Result<AssistantJournalChunk, String> {
    encode_chunk(store.read_assistant_record_chunk(
        &snapshot.stored(), part_id, content, chunk_index.0,
    ).map_err(|e|e.to_string())?)
}

pub(crate) fn read_chunk<R: AssistantJournalRepository + ?Sized>(
    store: &R, snapshot: AssistantJournalSnapshotHandle, part_id: &str,
    content: &JournalContentRef, chunk_index: JournalNumber,
) -> Result<AssistantJournalChunk, String> {
    let chunk=store.read_assistant_snapshot_chunk(
        &snapshot.stored(),part_id,content,chunk_index.0,
    ).map_err(|e|e.to_string())?;
    encode_chunk(chunk)
}

fn encode_chunk(chunk: nola_domain::assistant_journal::AssistantTextChunkV1) -> Result<AssistantJournalChunk,String> {
    chunk.verify().map_err(|e|e.to_string())?;
    let result=AssistantJournalChunk {
        digest:chunk.digest(),utf8_bytes:JournalNumber(chunk.byte_length() as u64),
        data_base64:base64::engine::general_purpose::STANDARD.encode(chunk.text().as_bytes()),
    };
    check_wire_budget(&result)?;
    Ok(result)
}

fn check_wire_budget(value: &impl Serialize) -> Result<(),String> {
    if serde_json::to_vec(value).map_err(|e|e.to_string())?.len()>ASSISTANT_FRAME_MAX_BYTES {
        return Err("assistant journal result exceeds transport budget".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maximum_control_character_chunk_keeps_original_digest_with_bounded_wire() {
        let original=nola_domain::assistant_journal::AssistantTextChunkV1::new("\u{1}".repeat(65536)).unwrap();
        assert!(serde_json::to_vec(&original).unwrap().len()>ASSISTANT_FRAME_MAX_BYTES);
        let encoded=encode_chunk(original.clone()).unwrap();
        assert!(serde_json::to_vec(&encoded).unwrap().len()<90_000);
        let bytes=base64::engine::general_purpose::STANDARD.decode(encoded.data_base64).unwrap();
        assert_eq!(bytes.len() as u64,encoded.utf8_bytes.0);
        let restored=nola_domain::assistant_journal::AssistantTextChunkV1::new(String::from_utf8(bytes).unwrap()).unwrap();
        assert_eq!(restored.digest(),encoded.digest);
        assert_eq!(restored,original);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalPartPage {
    pub at: AssistantJournalCursor,
    pub parts: Vec<nola_domain::assistant_journal_operations::JournalPartHead>,
    /// Directory continuation, including retired entries; empty is not EOF.
    pub next_part_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalPartChange {
    pub part_id: String,
    #[serde(deserialize_with = "required_change_option")]
    pub head: Option<nola_domain::assistant_journal_operations::JournalPartHead>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalPartChangePage {
    pub base: AssistantJournalCursor,
    pub at: AssistantJournalCursor,
    pub changes: Vec<AssistantJournalPartChange>,
    #[serde(deserialize_with = "required_change_option")]
    pub next_part_id: Option<String>,
}

fn required_change_option<'de, D, T>(decoder: D) -> Result<Option<T>, D::Error>
where D: serde::Deserializer<'de>, T: Deserialize<'de> {
    Option::<T>::deserialize(decoder)
}

#[test]
fn part_change_requires_explicit_retirement_not_missing_head() {
    let retired: AssistantJournalPartChange = serde_json::from_value(
        serde_json::json!({"part_id":"gone","head":null}),
    ).unwrap();
    assert!(retired.head.is_none());
    assert!(serde_json::from_value::<AssistantJournalPartChange>(
        serde_json::json!({"part_id":"gone"}),
    ).is_err());
}

pub(crate) fn read_part_changes<R: AssistantJournalRepository + ?Sized>(
    store: &R, snapshot: AssistantJournalSnapshotHandle, base: AssistantJournalCursor,
    after_part_id: Option<&str>, max_records: usize, max_encoded_bytes: usize,
) -> Result<AssistantJournalPartChangePage, String> {
    if max_records == 0 || max_records > ASSISTANT_PAGE_MAX_RECORDS
        || !(1024..=ASSISTANT_FRAME_MAX_BYTES).contains(&max_encoded_bytes) {
        return Err("invalid assistant change transport page budget".into());
    }
    let page = store.read_assistant_snapshot_changes(&snapshot.stored(), &base,
        after_part_id, max_records, ASSISTANT_FRAME_MAX_BYTES).map_err(|e| e.to_string())?;
    let mut result = AssistantJournalPartChangePage {
        base, at: snapshot.at,
        changes: page.changes.into_iter().map(|change| AssistantJournalPartChange {
            part_id: change.part_id, head: change.head,
        }).collect(),
        next_part_id: page.next_part_id,
    };
    while serde_json::to_vec(&result).map_err(|e| e.to_string())?.len() > max_encoded_bytes {
        if result.changes.len() <= 1 {
            return Err("assistant change transport page cannot fit next entry".into());
        }
        result.changes.pop();
        result.next_part_id = Some(result.changes.last().expect("nonempty change page").part_id.clone());
    }
    Ok(result)
}

pub(crate) fn read_parts<R: AssistantJournalRepository + ?Sized>(
    store: &R, snapshot: AssistantJournalSnapshotHandle, after_part_id: Option<&str>,
    max_records: usize, max_encoded_bytes: usize,
) -> Result<AssistantJournalPartPage, String> {
    if max_records == 0 || max_records > ASSISTANT_PAGE_MAX_RECORDS
        || !(1024..=ASSISTANT_FRAME_MAX_BYTES).contains(&max_encoded_bytes) {
        return Err("invalid assistant snapshot transport page budget".into());
    }
    let page=store.read_assistant_snapshot_parts(
        &snapshot.stored(),after_part_id,max_records,ASSISTANT_FRAME_MAX_BYTES,
    ).map_err(|e|e.to_string())?;
    let mut result=AssistantJournalPartPage {
        at:snapshot.at,parts:page.parts,next_part_id:page.next_part_id,
    };
    loop {
        if serde_json::to_vec(&result).map_err(|e|e.to_string())?.len()<=max_encoded_bytes {
            return Ok(result);
        }
        if result.parts.len()<=1 {
            return Err("assistant snapshot transport page cannot fit next head".into());
        }
        result.parts.pop();
        // Retrying after the last retained visible key can revisit tombstones,
        // but must never skip a head removed solely for this wire budget.
        result.next_part_id=Some(result.parts.last().expect("nonempty page").part_id.clone());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalSnapshotHandle {
    pub snapshot_id: String,
    pub at: AssistantJournalCursor,
}
impl AssistantJournalSnapshotHandle {
    fn stored(&self) -> nola_store_port::assistant_journal::AssistantJournalSnapshot {
        nola_store_port::assistant_journal::AssistantJournalSnapshot {
            snapshot_id: self.snapshot_id.clone(), at: self.at.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalTextRange {
    pub offset_bytes: JournalNumber,
    pub next_offset_bytes: JournalNumber,
    pub total_utf8_bytes: JournalNumber,
    pub text: String,
    pub end_of_content: bool,
}

pub(crate) fn open_snapshot<R: AssistantJournalRepository
    + nola_store_port::protocol_journal::CommandProtocolRepositoryProvider + ?Sized>(
    store: &mut R, at: AssistantJournalCursor,
) -> Result<AssistantJournalSnapshotHandle, String> {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        let snapshot=store.open_assistant_journal_snapshot_at(&at).map_err(|e|e.to_string())?;
        Ok(AssistantJournalSnapshotHandle {snapshot_id:snapshot.snapshot_id,at:snapshot.at})
    })
}

pub(crate) fn release_snapshot<R: AssistantJournalRepository
    + nola_store_port::protocol_journal::CommandProtocolRepositoryProvider + ?Sized>(
    store: &mut R, snapshot: AssistantJournalSnapshotHandle,
) -> Result<(), String> {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        // Both first release and exact replay succeed. A forged handle fails.
        store.release_assistant_journal_snapshot(&snapshot.stored()).map_err(|e|e.to_string())?;
        Ok(())
    })
}

pub(crate) fn read_range<R: AssistantJournalRepository + ?Sized>(
    store: &R, snapshot: AssistantJournalSnapshotHandle, part_id: &str,
    content: &JournalContentRef, offset_bytes: JournalNumber, limit_bytes: u32,
) -> Result<AssistantJournalTextRange, String> {
    let range=store.read_assistant_snapshot_range(
        &snapshot.stored(),part_id,content,offset_bytes.0,limit_bytes as usize,
    ).map_err(|e|e.to_string())?;
    let result=AssistantJournalTextRange {
        offset_bytes:JournalNumber(range.offset_bytes),next_offset_bytes:JournalNumber(range.next_offset_bytes),
        total_utf8_bytes:JournalNumber(range.total_utf8_bytes),text:range.text,end_of_content:range.end_of_content,
    };
    if serde_json::to_vec(&result).map_err(|e|e.to_string())?.len()>ASSISTANT_FRAME_MAX_BYTES {
        return Err("assistant journal content range exceeds transport budget".into());
    }
    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalRecord {
    pub header: AssistantJournalRecordHeaderV1,
    pub batch: AssistantOperationBatchV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssistantJournalReadResult {
    Records {
        after: AssistantJournalCursor,
        through: AssistantJournalCursor,
        records: Vec<AssistantJournalRecord>,
        next: AssistantJournalCursor,
        complete: bool,
    },
    SnapshotRequired,
}

pub(crate) fn read<R: AssistantJournalRepository + ?Sized>(
    store: &R,
    after: AssistantJournalCursor,
    through: AssistantJournalCursor,
    max_records: usize,
    max_encoded_bytes: usize,
) -> Result<AssistantJournalReadResult, String> {
    if max_records == 0 || max_records > ASSISTANT_PAGE_MAX_RECORDS
        || !(1024..=ASSISTANT_FRAME_MAX_BYTES).contains(&max_encoded_bytes) {
        return Err("invalid assistant journal transport page budget".into());
    }
    // Store verifies both owner-bound cursors and the complete digest chain.
    // Its byte budget is binary storage size, not the transport JSON budget.
    let page = store.read_assistant_journal_page(
        &AssistantJournalReadWindow { after: after.clone(), through: through.clone() },
        max_records, ASSISTANT_FRAME_MAX_BYTES,
    ).map_err(|error| error.to_string())?;
    let AssistantJournalPage::Records { records, next, complete } = page else {
        return Ok(AssistantJournalReadResult::SnapshotRequired);
    };
    let mut result = AssistantJournalReadResult::Records {
        after: after.clone(), through,
        records: records.into_iter().map(|record| AssistantJournalRecord {
            header: record.header, batch: record.batch,
        }).collect(), next, complete,
    };
    loop {
        // Bound the actual encoded result, including cursors and metadata.
        if serde_json::to_vec(&result).map_err(|error| error.to_string())?.len() <= max_encoded_bytes {
            return Ok(result);
        }
        let AssistantJournalReadResult::Records { records, next, complete, .. } = &mut result else {
            unreachable!()
        };
        if records.len() <= 1 {
            return Err("assistant journal transport page cannot fit next record".into());
        }
        records.pop();
        let header = &records.last().expect("nonempty bounded page").header;
        *next = AssistantJournalCursor {
            identity: after.identity.clone(), sequence: header.sequence,
            record_digest: header.record_digest.clone(),
        };
        *complete = false;
    }
}

fn decode_chunks(chunks: Vec<AssistantJournalChunk>) -> Result<Vec<nola_domain::assistant_journal::AssistantTextChunkV1>,String> {
    if chunks.len()>ASSISTANT_PAGE_MAX_RECORDS {return Err("received chunk count exceeds budget".into());}
    let mut verified=Vec::with_capacity(chunks.len());
    for chunk in chunks {
        if chunk.utf8_bytes.0==0 || chunk.utf8_bytes.0>65536 || chunk.data_base64.len()>87384 {
            return Err("invalid received chunk size".into());
        }
        let bytes=base64::engine::general_purpose::STANDARD.decode(&chunk.data_base64).map_err(|e|e.to_string())?;
        if bytes.len() as u64!=chunk.utf8_bytes.0 {return Err("received chunk length mismatch".into());}
        let text=String::from_utf8(bytes).map_err(|e|e.to_string())?;
        let value=nola_domain::assistant_journal::AssistantTextChunkV1::new(text).map_err(|e|e.to_string())?;
        if value.digest()!=chunk.digest {return Err("received chunk digest mismatch".into());}
        verified.push(value);
    }
    Ok(verified)
}
