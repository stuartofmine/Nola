// SPDX-License-Identifier: Apache-2.0
//! Committed journal persistence with separate local and remote admission gates.
mod content;
mod publication;
mod range;
mod reads;
pub(crate) mod order;
mod snapshot_lifecycle;
#[cfg(test)]
mod tests;
mod write;
mod receive;
mod receive_content;
mod terminal;
mod terminal_receive;
use crate::{params, Connection, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_domain::assistant_journal::*;
use nola_domain::assistant_journal_operations::*;
use nola_store_port::assistant_journal::*;

const LOCAL_RUN_HEADS_SQL: &str = "SELECT owner FROM assistant_journal_heads
    WHERE json_extract(owner,'$.source_store_epoch')=?1
      AND json_extract(owner,'$.conversation_id')=?2
      AND json_extract(owner,'$.run_id')=?3
      AND json_extract(owner,'$.assistant_message_id')>?4
    ORDER BY json_extract(owner,'$.assistant_message_id') LIMIT ?5";

/// Read-side provenance uses the same source binding as journal reception;
/// a self-consistent digest alone does not establish local/remote ownership.
pub(crate) fn validate_terminal_source(
    store: &NolaStore,
    seal: &nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
    source_host: Option<&str>,
) -> StoreResult<()> {
    if let Some(host) = source_host {
        let reception = terminal_receive::state(&store.connection, host, seal)?;
        if !reception.committed || !reception.manifest_complete
            || reception.next_entry != seal.entry_count {
            return Err(invalid("terminal source reception is incomplete"));
        }
    } else {
        let stored = terminal::load(store, &seal.identity.conversation_id, &seal.identity.run_id)?;
        if stored.as_ref() != Some(seal) {
            return Err(invalid("terminal source differs from its local seal"));
        }
    }
    Ok(())
}

pub(crate) fn initialize_owner_index(c: &Connection) -> StoreResult<()> {
    c.execute_batch("CREATE INDEX IF NOT EXISTS assistant_journal_run_owners ON assistant_journal_heads(
        json_extract(owner,'$.source_store_epoch'), json_extract(owner,'$.conversation_id'),
        json_extract(owner,'$.run_id'), json_extract(owner,'$.assistant_message_id'));")?;
    Ok(())
}

fn invalid(message: impl ToString) -> StoreError {
    StoreError::Invariant(message.to_string())
}
fn number(value: u64) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| invalid("journal number exceeds SQLite range"))
}
fn owner(identity: &AssistantJournalIdentity) -> StoreResult<String> {
    identity.validate().map_err(invalid)?;
    Ok(serde_json::to_string(identity)?)
}
fn require_uow(store: &NolaStore) -> StoreResult<()> {
    if !store.has_write_unit_of_work() {
        return Err(invalid("assistant journal requires write unit of work"));
    }
    Ok(())
}
fn validate_owner(c: &Connection, id: &AssistantJournalIdentity) -> StoreResult<String> {
    let key = owner(id)?;
    let valid: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM runs r JOIN messages m ON m.run_id=r.id WHERE r.id=?1 AND r.conversation_id=?2 AND m.id=?3 AND m.conversation_id=?2 AND m.role='assistant' AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id) AND ?4=(SELECT value FROM store_metadata WHERE key='assistant_source_epoch'))",params![id.run_id,id.conversation_id,id.assistant_message_id,id.source_store_epoch],|r|r.get(0))?;
    if !valid {
        return Err(invalid("journal source owner or epoch mismatch"));
    }
    Ok(key)
}
pub(super) fn planning_owner_key(
    store: &NolaStore,
    stamp: &AssistantSourceCommitStamp,
    id: &AssistantJournalIdentity,
) -> StoreResult<String> {
    require_uow(store)?;
    publication::validate_applied_planning_source(store, stamp, id)?;
    validate_owner(&store.connection, id)
}
/// A trusted execution handoff preserves published journal provenance. It
/// grants access to earlier immutable versions, not permission to publish for
/// the previous Run. Authoring still uses `validate_owner` below.
fn validate_read_owner(c: &Connection, id: &AssistantJournalIdentity) -> StoreResult<String> {
    let key = owner(id)?;
    let source: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM runs r WHERE r.id=?1 AND r.conversation_id=?2
         AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id)
         AND ?3=(SELECT value FROM store_metadata WHERE key='assistant_source_epoch'))",
        params![id.run_id, id.conversation_id, id.source_store_epoch],
        |row| row.get(0),
    )?;
    let remote = !source && receive::has_bound_read_owner(c, id)?;
    if !(source || remote)
        || !crate::canonical_message_ownership::matches_read_owner(
            c,
            &id.conversation_id,
            &id.assistant_message_id,
            &id.run_id,
        )?
    {
        return Err(invalid("journal read owner or epoch mismatch"));
    }
    Ok(key)
}
fn head(c: &Connection, id: &AssistantJournalIdentity) -> StoreResult<AssistantJournalCursor> {
    let key = validate_owner(c, id)?;
    head_for_owner(c, id, &key)
}
fn read_head(c: &Connection, id: &AssistantJournalIdentity) -> StoreResult<AssistantJournalCursor> {
    let key = validate_read_owner(c, id)?;
    head_for_owner(c, id, &key)
}
/// Resolve only the current canonical owners in an already bounded message
/// window. Imported epochs come from admitted provenance, never client input.
pub(crate) fn window_heads(
    c: &Connection, conversation: &nola_store_port::conversation_history::StoredConversation,
) -> StoreResult<Vec<AssistantJournalCursor>> {
    let mut heads = Vec::new();
    for message in &conversation.messages {
        if message.role != "assistant" { continue; }
        let Some(run_id) = &message.run_id else { continue; };
        let epoch: Option<String> = c.query_row(
            "SELECT CASE WHEN EXISTS(SELECT 1 FROM remote_run_sources WHERE run_id=?1)
             THEN (SELECT j.source_store_epoch FROM remote_assistant_journal_sources j
                   JOIN remote_run_sources s ON s.run_id=j.run_id AND s.source_host_id=j.source_host_id
                   WHERE j.run_id=?1 AND j.assistant_message_id=?2)
             ELSE (SELECT value FROM store_metadata WHERE key='assistant_source_epoch') END",
            params![run_id, message.id], |row| row.get(0))?;
        let Some(source_store_epoch) = epoch else { continue; };
        let identity = AssistantJournalIdentity {
            source_store_epoch, conversation_id: conversation.id.clone(),
            run_id: run_id.clone(), assistant_message_id: message.id.clone(),
        };
        // An imported transcript may name a historical Run without admitting
        // an execution or publishing any journal. Discover actual heads only;
        // an existing head still goes through the full provenance gate below.
        let exists: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM assistant_journal_heads WHERE owner=?1)",
            params![owner(&identity)?], |row| row.get(0))?;
        if !exists {
            // Normal local admission intentionally publishes its genesis
            // cursor before any visible Part exists. A historical transcript's
            // run label is not such an admission: require the actual Run root
            // and the immutable admission binding, not just message.run_id.
            let admitted: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM runs r
                 JOIN assistant_turn_message_bindings b ON b.run_id=r.id
                   AND b.conversation_id=r.conversation_id AND b.message_id=?3
                 WHERE r.id=?1 AND r.conversation_id=?2
                   AND ((r.assistant_message_id=?3 AND b.admission=1) OR EXISTS(
                       SELECT 1 FROM assistant_message_segments segment WHERE segment.run_id=r.id
                       AND segment.turn_id=b.turn_id AND segment.message_id=b.message_id))
                   AND r.kind='conversation'
                   AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id))",
                params![run_id, conversation.id, message.id], |row| row.get(0))?;
            if !admitted { continue; }
        }
        let cursor = read_head(c, &identity)?;
        heads.push(cursor);
    }
    Ok(heads)
}
fn head_for_owner(
    c: &Connection,
    id: &AssistantJournalIdentity,
    key: &str,
) -> StoreResult<AssistantJournalCursor> {
    let stored: Option<(u64, String)> = c
        .query_row(
            "SELECT sequence,digest FROM assistant_journal_heads WHERE owner=?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match stored {
        Some((sequence, digest)) => Ok(AssistantJournalCursor {
            identity: id.clone(),
            sequence,
            record_digest: AssistantDigest::parse_hex(&digest).map_err(invalid)?,
        }),
        None => AssistantJournalCursor::initial(id.clone()).map_err(invalid),
    }
}

impl AssistantJournalRepository for NolaStore {
    type Error = StoreError;
    fn begin_receive_assistant_run_terminal(&mut self,host:&str,seal:&nola_domain::assistant_run_terminal::AssistantRunTerminalSeal)->StoreResult<nola_domain::assistant_run_terminal::AssistantRunTerminalReception> { terminal_receive::begin(self,host,seal) }
    fn stage_assistant_run_terminal_manifest(&mut self,host:&str,page:&nola_domain::assistant_run_terminal::AssistantRunTerminalManifestPage)->StoreResult<nola_domain::assistant_run_terminal::AssistantRunTerminalReception> { terminal_receive::stage(self,host,page) }
    fn commit_received_assistant_run_terminal(&mut self,host:&str,seal:&nola_domain::assistant_run_terminal::AssistantRunTerminalSeal)->StoreResult<nola_domain::assistant_run_terminal::AssistantRunTerminalReception> { terminal_receive::commit(self,host,seal) }
    fn seal_assistant_run_terminal(&mut self, transition_id: &str) -> StoreResult<Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal>> {
        terminal::freeze(self, transition_id)
    }
    fn local_assistant_run_terminal_seal(&self, conversation: &str, run: &str) -> StoreResult<Option<nola_domain::assistant_run_terminal::AssistantRunTerminalSeal>> {
        terminal::load(self, conversation, run)
    }
    fn read_local_assistant_run_terminal_manifest(&self, seal: &nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
        after: JournalNumber, max_records: usize, max_bytes: usize,
    ) -> StoreResult<nola_domain::assistant_run_terminal::AssistantRunTerminalManifestPage> {
        terminal::page(self, seal, after, max_records, max_bytes)
    }
    fn assistant_journal_reception_state(
        &self, source_host_id: &str, through: &AssistantJournalCursor,
    ) -> StoreResult<AssistantJournalReceptionState> {
        receive::reception_state(&self.connection, source_host_id, through)
    }
    fn next_assistant_journal_content(
        &self, source_host_id: &str, record: &StoredAssistantJournalRecord,
    ) -> StoreResult<Option<AssistantJournalContentNeed>> {
        receive_content::next_content(&self.connection, source_host_id, record)
    }
    fn stage_assistant_journal_content(&mut self, input: &StageAssistantJournalContent) -> StoreResult<JournalContentRef> {
        receive_content::stage(self,input)
    }
    fn receive_assistant_journal_record(
        &mut self, source_host_id: &str, record: &StoredAssistantJournalRecord,
        chunks: &[AssistantTextChunkV1],
    ) -> StoreResult<AssistantJournalCommit> {
        receive::receive(self, source_host_id, record, chunks)
    }
    fn assistant_source_epoch(&self) -> StoreResult<String> {
        Ok(self.connection.query_row(
            "SELECT value FROM store_metadata WHERE key='assistant_source_epoch'",
            [],
            |r| r.get(0),
        )?)
    }
    fn local_run_assistant_journal_heads(
        &self, conversation_id: &str, run_id: &str, after_message_id: Option<&str>, limit: usize,
    ) -> StoreResult<Vec<AssistantJournalCursor>> {
        if limit == 0 || limit > 64 { return Err(invalid("journal owner page budget")); }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let local: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs r WHERE r.id=?1 AND r.conversation_id=?2
             AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id))",
            params![run_id, conversation_id], |r| r.get(0))?;
        if !local { return Err(invalid("journal Run is not authored by this source Host")); }
        let epoch = self.assistant_source_epoch()?;
        let mut query = self.connection.prepare(LOCAL_RUN_HEADS_SQL)?;
        let rows = query.query_map(params![epoch, conversation_id, run_id, after_message_id.unwrap_or(""), limit as i64],
            |r| r.get::<_,String>(0))?;
        let mut heads = Vec::new();
        for row in rows {
            let identity = serde_json::from_str::<AssistantJournalIdentity>(&row?)?;
            heads.push(read_head(&self.connection, &identity)?);
        }
        Ok(heads)
    }
    fn assistant_journal_head(
        &self,
        id: &AssistantJournalIdentity,
    ) -> StoreResult<AssistantJournalCursor> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        read_head(&self.connection, id)
    }
    fn local_assistant_journal_head(
        &self,
        id: &AssistantJournalIdentity,
    ) -> StoreResult<AssistantJournalCursor> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let local:bool=self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs r WHERE r.id=?1 AND r.conversation_id=?2
             AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id)
             AND ?3=(SELECT value FROM store_metadata WHERE key='assistant_source_epoch'))",
            params![id.run_id,id.conversation_id,id.source_store_epoch],|row|row.get(0),
        )?;
        if !local {return Err(invalid("journal is not authored by this source Host"));}
        read_head(&self.connection, id)
    }
    fn assistant_journal_part(
        &self,
        id: &AssistantJournalIdentity,
        part: &str,
    ) -> StoreResult<Option<JournalPartHead>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        validate_read_owner(&self.connection, id)?;
        content::Reader {
            c: &self.connection,
            id,
            supplied: &[],
        }
        .load_part_head(part)
        .map_err(invalid)
    }
    fn assistant_journal_part_state(
        &self,
        id: &AssistantJournalIdentity,
        part: &str,
    ) -> StoreResult<AssistantJournalPartState> {
        crate::require_id("journal_part.part_id", part)?;
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let key = validate_read_owner(&self.connection, id)?;
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT head_json FROM assistant_journal_parts WHERE owner=?1 AND part_id=?2",
                params![key, part],
                |row| row.get(0),
            )
            .optional()?;
        match value.as_deref() {
            None => Ok(AssistantJournalPartState::NeverDeclared),
            Some("null") => Ok(AssistantJournalPartState::Retired),
            Some(value) => {
                let head: JournalPartHead = serde_json::from_str(value)?;
                if head.identity != *id || head.part_id != part {
                    return Err(invalid("journal Part head identity differs"));
                }
                Ok(AssistantJournalPartState::Live(head))
            }
        }
    }
    fn commit_assistant_journal(
        &mut self,
        input: &CommitAssistantJournal,
    ) -> StoreResult<AssistantJournalCommit> {
        let set = CommitAssistantPublicationSet {
            stamp: input.stamp.clone(),
            groups: vec![AssistantPublicationGroup {
                expected: input.expected.clone(),
                frames: vec![AssistantJournalFrame {
                    batch: input.batch.clone(),
                    supplied_chunks: input.supplied_chunks.clone(),
                }],
            }],
        };
        match publication::commit(self, &set)? {
            AssistantPublicationCommit::Applied(mut cursors) => {
                Ok(AssistantJournalCommit::Applied(cursors.remove(0)))
            }
            AssistantPublicationCommit::Replay(mut cursors) => {
                Ok(AssistantJournalCommit::Replay(cursors.remove(0)))
            }
        }
    }
    fn commit_assistant_publication_set(
        &mut self,
        input: &CommitAssistantPublicationSet,
    ) -> StoreResult<AssistantPublicationCommit> {
        publication::commit(self, input)
    }
    fn assistant_journal_content_manifest(
        &self,
        id: &AssistantJournalIdentity,
        part: &str,
        reference: &JournalContentRef,
        after: u64,
        max_records: usize,
    ) -> StoreResult<AssistantManifestPage> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let key = validate_read_owner(&self.connection, id)?;
        let head = content::Reader {
            c: &self.connection,
            id,
            supplied: &[],
        }
        .load_part_head(part)
        .map_err(invalid)?
        .ok_or_else(|| invalid("published Part unavailable"))?;
        if head.identity != *id || head.part_id != part || !head.bodies.contains(reference) {
            return Err(invalid(
                "content reference not reachable from exact published head",
            ));
        }
        content::verify_reference(&self.connection, id, part, reference)?;
        content::manifest_page(&self.connection, &key, part, reference, after, max_records)
    }
    fn prepare_assistant_content(
        &mut self,
        id: &AssistantJournalIdentity,
        part: &str,
        previous: &JournalContentRef,
        chunks: &[AssistantTextChunkV1],
    ) -> StoreResult<JournalContentRef> {
        require_uow(self)?;
        let tx = self.write_transaction()?;
        let key = validate_owner(&tx, id)?;
        content::verify_reference(&tx, id, part, previous)?;
        let refs = content::supply(&tx, chunks)?;
        let result = content::append(&tx, &key, part, previous, &refs)?;
        tx.commit()?;
        Ok(result)
    }
    fn open_assistant_journal_read(
        &self,
        after: &AssistantJournalCursor,
    ) -> StoreResult<AssistantJournalReadWindow> {
        reads::open(&self.connection, after)
    }
    fn read_assistant_journal_page(
        &self,
        window: &AssistantJournalReadWindow,
        max_records: usize,
        max_bytes: usize,
    ) -> StoreResult<AssistantJournalPage> {
        reads::journal_page(&self.connection, window, max_records, max_bytes)
    }
    fn open_assistant_journal_snapshot(
        &mut self,
        id: &AssistantJournalIdentity,
    ) -> StoreResult<AssistantJournalSnapshot> {
        require_uow(self)?;
        let at = read_head(&self.connection, id)?;
        self.open_assistant_journal_snapshot_at(&at)
    }
    fn read_assistant_snapshot_parts(
        &self,
        snapshot: &AssistantJournalSnapshot,
        after: Option<&str>,
        max_records: usize,
        max_bytes: usize,
    ) -> StoreResult<AssistantPartPage> {
        reads::parts(&self.connection, snapshot, after, max_records, max_bytes)
    }
    fn read_assistant_snapshot_changes(
        &self, snapshot: &AssistantJournalSnapshot, base: &AssistantJournalCursor,
        after: Option<&str>, max_records: usize, max_bytes: usize,
    ) -> StoreResult<AssistantPartChangePage> {
        reads::part_changes(&self.connection, snapshot, base, after, max_records, max_bytes)
    }
    fn open_assistant_journal_snapshot_at(
        &mut self,
        at: &AssistantJournalCursor,
    ) -> StoreResult<AssistantJournalSnapshot> {
        require_uow(self)?;
        let tx = self.write_transaction()?;
        reads::verify_cursor(&tx, at)?;
        if matches!(reads::journal_page(&tx, &AssistantJournalReadWindow {
            after: at.clone(), through: at.clone(),
        }, 1, ASSISTANT_FRAME_MAX_BYTES)?, AssistantJournalPage::SnapshotRequired) {
            return Err(invalid("requested journal snapshot is no longer retained"));
        }
        let snapshot_id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO assistant_journal_snapshots(id,owner,sequence,digest) VALUES(?1,?2,?3,?4)",
            params![snapshot_id, owner(&at.identity)?, number(at.sequence)?, at.record_digest.to_hex()],
        )?;
        tx.commit()?;
        Ok(AssistantJournalSnapshot { snapshot_id, at: at.clone() })
    }
    fn release_assistant_journal_snapshot(
        &mut self,
        snapshot: &AssistantJournalSnapshot,
    ) -> StoreResult<AssistantSnapshotRelease> {
        snapshot_lifecycle::release(self, snapshot)
    }
    fn read_assistant_snapshot_manifest(
        &self,
        snapshot: &AssistantJournalSnapshot,
        part: &str,
        content: &JournalContentRef,
        after: u64,
        max_records: usize,
    ) -> StoreResult<AssistantManifestPage> {
        reads::manifest(
            &self.connection,
            snapshot,
            part,
            content,
            after,
            max_records,
        )
    }
    fn read_assistant_snapshot_chunk(
        &self,
        snapshot: &AssistantJournalSnapshot,
        part: &str,
        content: &JournalContentRef,
        index: u64,
    ) -> StoreResult<AssistantTextChunkV1> {
        reads::chunk(&self.connection, snapshot, part, content, index)
    }
    fn read_assistant_record_chunk(
        &self,
        snapshot: &AssistantJournalSnapshot,
        part: &str,
        content: &JournalContentRef,
        index: u64,
    ) -> StoreResult<AssistantTextChunkV1> {
        reads::record_chunk(&self.connection, snapshot, part, content, index)
    }
    fn read_assistant_snapshot_range(
        &self,
        snapshot: &AssistantJournalSnapshot,
        part: &str,
        content: &JournalContentRef,
        offset: u64,
        limit: usize,
    ) -> StoreResult<AssistantContentRange> {
        range::read(&self.connection, snapshot, part, content, offset, limit)
    }
}

/// Bounded result preview from public answer Parts at the immutable completion frontier.
/// No private canonical body, tool payload, or snapshot handle is created.
pub(crate) fn activity_result_text(c:&Connection, conversation:&str, run:&str, parts:&[String])
    -> StoreResult<(Option<String>,bool)> {
    let _read = crate::read_snapshot::pin(c)?;
    let mut manifests=c.prepare("SELECT entry_json FROM assistant_run_terminal_manifest WHERE run_id=?1
        UNION ALL SELECT entry_json FROM remote_assistant_run_terminal_entries WHERE run_id=?1")?;
    let entries=manifests.query_map([run],|row|row.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    let mut heads=Vec::new();
    let mut versions=c.prepare("SELECT head_json FROM assistant_journal_part_versions v
        WHERE owner=?1 AND part_id IN (SELECT value FROM json_each(?2))
        AND sequence=(SELECT MAX(sequence) FROM assistant_journal_part_versions p
            WHERE p.owner=v.owner AND p.part_id=v.part_id AND p.sequence<=?3)
        AND head_json IS NOT NULL")?;
    let selected=serde_json::to_string(parts)?;
    for raw in entries {
        let entry:nola_domain::assistant_run_terminal::AssistantRunTerminalManifestEntry=serde_json::from_str(&raw)?;
        let nola_domain::assistant_run_terminal::AssistantRunTerminalManifestEntry::Assistant{through,..}=entry else {continue};
        if through.identity.conversation_id!=conversation || through.identity.run_id!=run {
            return Err(invalid("Activity result manifest owner mismatch"));
        }
        let key=validate_read_owner(c,&through.identity)?;
        for raw in versions.query_map(params![key,selected,number(through.sequence)?],|row|row.get::<_,String>(0))? {
            heads.push(serde_json::from_str::<JournalPartHead>(&raw?)?);
        }
    }
    let mut text=String::new();
    for (index,part) in parts.iter().enumerate() {
        let Some(head)=heads.iter().find(|head|&head.part_id==part) else {continue};
        let JournalPartMetadata::Text(metadata)=&head.metadata else {continue};
        if metadata.visibility!=JournalVisibility::Public || !metadata.is_final
            || metadata.review.as_ref().is_some_and(|review|*review!=nola_domain::TextReviewState::Approved) {continue;}
        let key=validate_read_owner(c,&head.identity)?;
        let Some(body)=head.bodies.iter().find(|body|body.field==JournalContentField::Markdown) else {continue};
        content::verify_reference(c,&head.identity,part,body)?;
        let remaining=32768usize.saturating_sub(text.len()+if text.is_empty(){0}else{2});
        if remaining<4 {return Ok((Some(text),true));}
        let range=range::read_verified(c,&key,part,body,0,remaining)?;
        if !text.is_empty(){text.push_str("\n\n");}
        text.push_str(&range.text);
        if !range.end_of_content {return Ok((Some(text),true));}
        if text.len()>=32764 && index+1<parts.len(){return Ok((Some(text),true));}
    }
    Ok(((!text.is_empty()).then_some(text),false))
}
