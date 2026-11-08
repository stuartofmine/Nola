// SPDX-License-Identifier: Apache-2.0
//! Transactional message ownership declarations; never execution authority.
use crate::{params, Connection, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_domain::assistant_journal::AssistantJournalIdentity;
use nola_domain::assistant_journal_operations::JournalNumber;
use nola_domain::run_message_admission::*;
use nola_domain::{MessagePartInput, ReplicaMessageDeclaration, TransitionMessageInput};
use nola_store_port::run_message_admission::RunMessageAdmissionRepository;

fn invalid(s: impl ToString) -> StoreError {
    StoreError::Invariant(s.to_string())
}
fn key(id: &AssistantJournalIdentity) -> StoreResult<String> {
    id.validate().map_err(invalid)?;
    Ok(serde_json::to_string(id)?)
}
fn number(n: u64) -> StoreResult<i64> {
    i64::try_from(n).map_err(invalid)
}
fn root(c: &Connection, conversation: &str, run: &str) -> StoreResult<AssistantJournalIdentity> {
    let (message,epoch):(String,String)=c.query_row("SELECT r.assistant_message_id,(SELECT value FROM store_metadata WHERE key='assistant_source_epoch') FROM runs r WHERE r.id=?1 AND r.conversation_id=?2 AND r.kind='conversation' AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id)",params![run,conversation],|r|Ok((r.get(0)?,r.get(1)?)))?;
    Ok(AssistantJournalIdentity {
        source_store_epoch: epoch,
        conversation_id: conversation.into(),
        run_id: run.into(),
        assistant_message_id: message,
    })
}
pub(crate) fn local_head(c: &Connection, conversation: &str, run: &str) -> StoreResult<RunMessageAdmissionCursor> {
    head(c, &root(c, conversation, run)?)
}
fn authorize(c: &Connection, host: &str, id: &AssistantJournalIdentity) -> StoreResult<()> {
    crate::require_id("source Host", host)?;
    key(id)?;
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM runs r JOIN remote_run_sources s ON s.run_id=r.id JOIN messages m ON m.id=r.assistant_message_id WHERE r.id=?1 AND r.conversation_id=?2 AND r.assistant_message_id=?3 AND r.kind='conversation' AND s.source_host_id=?4 AND m.conversation_id=r.conversation_id AND m.run_id=r.id AND m.role='assistant')",params![id.run_id,id.conversation_id,id.assistant_message_id,host],|r|r.get(0))?;
    if !valid {
        return Err(invalid(
            "message admission has no exact authenticated root Run",
        ));
    }
    let changed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM remote_assistant_journal_sources WHERE run_id=?1 AND (source_host_id<>?2 OR source_store_epoch<>?3))",params![id.run_id,host,id.source_store_epoch],|r|r.get(0))?;
    if changed {
        return Err(invalid("message admission source incarnation changed"));
    }
    Ok(())
}
fn head(c: &Connection, id: &AssistantJournalIdentity) -> StoreResult<RunMessageAdmissionCursor> {
    let raw:Option<String>=c.query_row("SELECT record_json FROM run_message_admission_records WHERE owner=?1 ORDER BY sequence DESC LIMIT 1",[key(id)?],|r|r.get(0)).optional()?;
    raw.map(|s| {
        serde_json::from_str::<RunMessageAdmissionRecord>(&s)
            .map(|r| r.cursor)
            .map_err(Into::into)
    })
    .unwrap_or_else(|| RunMessageAdmissionCursor::initial(id.clone()).map_err(invalid))
}
fn verify(c: &Connection, cursor: &RunMessageAdmissionCursor) -> StoreResult<()> {
    let actual = if cursor.sequence.0 == 0 {
        RunMessageAdmissionCursor::initial(cursor.identity.clone()).map_err(invalid)?
    } else {
        let raw: String = c.query_row(
            "SELECT record_json FROM run_message_admission_records WHERE owner=?1 AND sequence=?2",
            params![key(&cursor.identity)?, number(cursor.sequence.0)?],
            |r| r.get(0),
        )?;
        serde_json::from_str::<RunMessageAdmissionRecord>(&raw)?.cursor
    };
    if actual != *cursor {
        return Err(invalid("message admission cursor digest mismatch"));
    }
    Ok(())
}
fn insert_record(c: &Connection, record: &RunMessageAdmissionRecord) -> StoreResult<()> {
    c.execute(
        "INSERT INTO run_message_admission_records VALUES(?1,?2,?3)",
        params![
            key(&record.cursor.identity)?,
            number(record.cursor.sequence.0)?,
            serde_json::to_string(record)?
        ],
    )?;
    Ok(())
}
/// Called inside the actual admission/transition writer transaction. Only new
/// exact-Run rows beyond the persisted source ordinal are inspected.
pub(crate) fn publish(c: &Connection, conversation: &str, run: &str) -> StoreResult<()> {
    let local:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM runs r WHERE id=?1 AND kind='conversation' AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id))",[run],|r|r.get(0))?;
    if !local {
        return Ok(());
    }
    let id = root(c, conversation, run)?;
    let owner = key(&id)?;
    let mut cursor = head(c, &id)?;
    let last: i64 = c.query_row(
        "SELECT COALESCE(MAX(source_ordinal),-1) FROM run_message_admission_owners WHERE owner=?1",
        [&owner],
        |r| r.get(0),
    )?;
    let mut q=c.prepare("SELECT id,ordinal,role,created_at_ms,completed_at_ms FROM messages WHERE conversation_id=?1 AND ordinal>?2 AND run_id=?3 ORDER BY ordinal")?;
    let rows = q
        .query_map(params![conversation, last, run], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (message, ordinal, role, created, completed) in rows {
        if !matches!(role.as_str(), "user" | "assistant") {
            return Err(invalid("unsupported Run declaration role"));
        }
        let mut parts = Vec::new();
        if role == "user" {
            let mut q=c.prepare("SELECT id,kind,ordinal,state,content_text,content_json,content_digest,created_at_ms,updated_at_ms FROM message_parts WHERE message_id=?1 ORDER BY ordinal")?;
            parts = q
                .query_map([&message], |r| {
                    Ok(MessagePartInput {
                        id: r.get(0)?,
                        kind: r.get(1)?,
                        ordinal: r.get(2)?,
                        state: r.get(3)?,
                        content_text: r.get(4)?,
                        content_json: r.get(5)?,
                        content_digest: r.get(6)?,
                        created_at_ms: r.get(7)?,
                        updated_at_ms: r.get(8)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
        }
        let declaration = ReplicaMessageDeclaration {
            execution_ownership: crate::conversation_history_port::source_execution_ownership(c, conversation)?,
            ordinal,
            input: TransitionMessageInput {
                id: message.clone(),
                run_id: Some(run.into()),
                role: role.clone(),
                status: if role == "user" {
                    "completed"
                } else {
                    "streaming"
                }
                .into(),
                created_at_ms: created,
                completed_at_ms: if role == "user" {
                    Some(completed.unwrap_or(created))
                } else {
                    None
                },
                initial_parts: parts,
            },
        };
        let raw = serde_json::to_string(&declaration)?;
        let digest = admission_digest(raw.as_bytes());
        let mut offset = 0;
        while offset < raw.len() {
            let mut end = (offset + ADMISSION_CHUNK_BYTES).min(raw.len());
            while !raw.is_char_boundary(end) {
                end -= 1;
            }
            let mut record = RunMessageAdmissionRecord {
                cursor: RunMessageAdmissionCursor {
                    identity: id.clone(),
                    sequence: JournalNumber(
                        cursor
                            .sequence
                            .0
                            .checked_add(1)
                            .ok_or_else(|| invalid("admission sequence overflow"))?,
                    ),
                    record_digest: cursor.record_digest,
                },
                previous_digest: cursor.record_digest,
                message_id: message.clone(),
                offset: JournalNumber(offset as u64),
                total_utf8_bytes: JournalNumber(raw.len() as u64),
                declaration_digest: digest,
                text: raw[offset..end].into(),
            };
            record.cursor.record_digest = record.computed_digest().map_err(invalid)?;
            insert_record(c, &record)?;
            cursor = record.cursor;
            offset = end;
        }
        c.execute(
            "INSERT INTO run_message_admission_owners VALUES(?1,?2,?3,?3,?4,?5)",
            params![owner, message, ordinal, digest.to_hex(), created],
        )?;
    }
    Ok(())
}

pub(crate) fn require_frontier(
    c: &Connection,
    host: &str,
    through: &RunMessageAdmissionCursor,
) -> StoreResult<()> {
    authorize(c, host, &through.identity)?;
    verify(c, through)?;
    let staged: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM run_message_admission_stages WHERE owner=?1)",
        [key(&through.identity)?],
        |r| r.get(0),
    )?;
    if staged {
        return Err(invalid("message admission declaration incomplete"));
    }
    Ok(())
}
impl RunMessageAdmissionRepository for NolaStore {
    fn local_run_context_usage(&self, identity: &AssistantJournalIdentity) -> StoreResult<Option<nola_domain::committed_run_context_usage::CommittedRunContextUsage>> {
        NolaStore::local_run_context_usage(self, identity)
    }
    fn accept_remote_run_context_usage(&mut self, host: &str, record: &nola_domain::committed_run_context_usage::CommittedRunContextUsage) -> StoreResult<nola_domain::committed_run_context_usage::CommittedRunContextUsage> {
        NolaStore::accept_remote_run_context_usage(self, host, record)
    }
    type Error = StoreError;
    fn local_run_message_admission_head(
        &self,
        conversation: &str,
        run: &str,
    ) -> StoreResult<RunMessageAdmissionCursor> {
        let _read = crate::read_snapshot::pin(&self.connection)?;
        head(
            &self.connection,
            &root(&self.connection, conversation, run)?,
        )
    }
    fn read_run_message_admissions(
        &self,
        after: &RunMessageAdmissionCursor,
        through: &RunMessageAdmissionCursor,
        max_records: usize,
        max_encoded_bytes: usize,
    ) -> StoreResult<RunMessageAdmissionPage> {
        let c = &self.connection;
        let _read = crate::read_snapshot::pin(c)?;
        if after.identity != through.identity
            || after.sequence.0 > through.sequence.0
            || max_records == 0
            || max_records > 64
            || !(1024..=262144).contains(&max_encoded_bytes)
            || root(
                c,
                &through.identity.conversation_id,
                &through.identity.run_id,
            )? != through.identity
        {
            return Err(invalid("invalid local admission read scope/budget"));
        }
        verify(c, after)?;
        verify(c, through)?;
        let mut page = RunMessageAdmissionPage {
            after: after.clone(),
            through: through.clone(),
            next: after.clone(),
            complete: after == through,
            records: Vec::new(),
        };
        let mut q=c.prepare("SELECT record_json FROM run_message_admission_records WHERE owner=?1 AND sequence>?2 AND sequence<=?3 ORDER BY sequence LIMIT ?4")?;
        let rows = q.query_map(
            params![
                key(&through.identity)?,
                number(after.sequence.0)?,
                number(through.sequence.0)?,
                max_records as i64
            ],
            |r| r.get::<_, String>(0),
        )?;
        for raw in rows {
            let record: RunMessageAdmissionRecord = serde_json::from_str(&raw?)?;
            let old = page.next.clone();
            page.next = record.cursor.clone();
            page.complete = page.next == *through;
            page.records.push(record);
            if serde_json::to_vec(&page)?.len() > max_encoded_bytes {
                page.records.pop();
                page.next = old;
                page.complete = page.next == *through;
                break;
            }
        }
        if page.records.is_empty() && !page.complete {
            return Err(invalid("admission page byte budget too small"));
        }
        Ok(page)
    }
    fn run_message_admission_reception_state(
        &self,
        host: &str,
        through: &RunMessageAdmissionCursor,
    ) -> StoreResult<RunMessageAdmissionCursor> {
        let c = &self.connection;
        let _read = crate::read_snapshot::pin(c)?;
        authorize(c, host, &through.identity)?;
        let current = head(c, &through.identity)?;
        if current.sequence.0 >= through.sequence.0 {
            verify(c, through)?;
        }
        Ok(current)
    }
    fn receive_run_message_admissions(
        &mut self,
        host: &str,
        page: &RunMessageAdmissionPage,
    ) -> StoreResult<RunMessageAdmissionCursor> {
        if !self.has_write_unit_of_work() {
            return Err(invalid("admission receive requires UoW"));
        }
        if page.records.is_empty()
            || page.records.len() > 64
            || serde_json::to_vec(page)?.len() > 262144
            || page.after.identity != page.through.identity
            || page.next.identity != page.through.identity
            || page.next.sequence.0 > page.through.sequence.0
            || page.complete != (page.next == page.through)
        {
            return Err(invalid("invalid admission page"));
        }
        let tx = self.write_transaction()?;
        authorize(&tx, host, &page.through.identity)?;
        tx.execute(
            "INSERT OR IGNORE INTO remote_assistant_journal_sources VALUES(?1,?2,?3,?4)",
            params![
                page.through.identity.run_id,
                host,
                page.through.identity.source_store_epoch,
                page.through.identity.assistant_message_id
            ],
        )?;
        verify(&tx, &page.after)?;
        let mut previous = page.after.clone();
        let mut current = head(&tx, &page.through.identity)?;
        for record in &page.records {
            record.validate().map_err(invalid)?;
            if record.cursor.identity != page.through.identity
                || record.cursor.sequence.0
                    != previous
                        .sequence
                        .0
                        .checked_add(1)
                        .ok_or_else(|| invalid("admission sequence overflow"))?
                || record.previous_digest != previous.record_digest
            {
                return Err(invalid("admission page chain gap"));
            }
            if record.cursor.sequence.0 <= current.sequence.0 {
                verify(&tx, &record.cursor)?;
            } else {
                if current != previous {
                    return Err(invalid("admission receive missing prefix"));
                }
                apply_record(&tx, host, record)?;
                insert_record(&tx, record)?;
                current = record.cursor.clone();
            }
            previous = record.cursor.clone();
        }
        if previous != page.next {
            return Err(invalid("admission page tail differs"));
        }
        tx.commit()?;
        Ok(current)
    }
}

fn apply_record(c: &Connection, host: &str, r: &RunMessageAdmissionRecord) -> StoreResult<()> {
    let owner = key(&r.cursor.identity)?;
    let stage:Option<(String,i64,String,i64)>=c.query_row("SELECT message_id,total_bytes,declaration_digest,next_offset FROM run_message_admission_stages WHERE owner=?1",[&owner],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional()?;
    if let Some((message, total, digest, offset)) = stage {
        if message != r.message_id
            || total != number(r.total_utf8_bytes.0)?
            || digest != r.declaration_digest.to_hex()
            || offset != number(r.offset.0)?
        {
            return Err(invalid("admission chunk changes declaration"));
        }
    } else {
        if r.offset.0 != 0 {
            return Err(invalid("admission declaration missing prefix"));
        }
        c.execute(
            "INSERT INTO run_message_admission_stages VALUES(?1,?2,?3,?4,0)",
            params![
                owner,
                r.message_id,
                number(r.total_utf8_bytes.0)?,
                r.declaration_digest.to_hex()
            ],
        )?;
    }
    let next = r
        .offset
        .0
        .checked_add(r.text.len() as u64)
        .ok_or_else(|| invalid("admission offset overflow"))?;
    c.execute(
        "INSERT INTO run_message_admission_chunks VALUES(?1,?2,?3)",
        params![owner, number(r.offset.0)?, r.text],
    )?;
    c.execute(
        "UPDATE run_message_admission_stages SET next_offset=?2 WHERE owner=?1",
        params![owner, number(next)?],
    )?;
    if next < r.total_utf8_bytes.0 {
        return Ok(());
    }
    let mut raw = String::new();
    let mut query = c.prepare(
        "SELECT offset,text FROM run_message_admission_chunks WHERE owner=?1 ORDER BY offset",
    )?;
    for chunk in query.query_map([&owner], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })? {
        let (offset, text) = chunk?;
        if offset != raw.len() as i64 {
            return Err(invalid("admission persisted chunks have a gap"));
        }
        raw.push_str(&text);
    }
    if raw.len() as u64 != r.total_utf8_bytes.0 {
        return Err(invalid("admission declared byte length differs"));
    }
    if admission_digest(raw.as_bytes()) != r.declaration_digest {
        return Err(invalid("admission declaration digest mismatch"));
    }
    let declaration: ReplicaMessageDeclaration = serde_json::from_str(&raw)?;
    let m = &declaration.input;
    let id = &r.cursor.identity;
    match &declaration.execution_ownership {
        nola_domain::ConversationExecutionOwnership::TaskChild { parent_conversation_id, parent_run_id, task_id, child_conversation_id } => {
            for value in [parent_conversation_id, parent_run_id, task_id, child_conversation_id] { crate::require_id("task ownership", value)?; }
            if child_conversation_id != &id.conversation_id || parent_conversation_id == child_conversation_id {
                return Err(invalid("admitted task ownership differs from exact conversation"));
            }
        }
        nola_domain::ConversationExecutionOwnership::Unavailable => return Err(invalid("source message admission cannot assert unknown ownership")),
        nola_domain::ConversationExecutionOwnership::User => {}
    }
    let ownership_json = serde_json::to_string(&declaration.execution_ownership)?;
    let existing_owner: Option<(String,String,String)> = c.query_row(
        "SELECT source_host_id,source_store_epoch,ownership_json FROM remote_conversation_execution_owners WHERE run_id=?1",
        [&id.run_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    if existing_owner.as_ref().is_some_and(|(old_host,epoch,value)| old_host != host || epoch != &id.source_store_epoch || value != &ownership_json) {
        return Err(invalid("remote conversation execution ownership conflicts with admitted source"));
    }
    c.execute("INSERT OR IGNORE INTO remote_conversation_execution_owners(run_id,source_host_id,source_store_epoch,ownership_json) VALUES(?1,?2,?3,?4)",
        params![id.run_id,host,id.source_store_epoch,ownership_json])?;

    if m.id != r.message_id
        || m.run_id.as_deref() != Some(&id.run_id)
        || declaration.ordinal < 0
        || !matches!(m.role.as_str(), "assistant" | "user")
        || (m.role == "assistant"
            && (!m.initial_parts.is_empty()
                || m.status != "streaming"
                || m.completed_at_ms.is_some()))
        || (m.role == "user" && m.status != "completed")
    {
        return Err(invalid("invalid admitted message identity/content"));
    }
    let last: Option<i64> = c.query_row(
        "SELECT MAX(source_ordinal) FROM run_message_admission_owners WHERE owner=?1",
        [&owner],
        |r| r.get(0),
    )?;
    if last.is_some_and(|last| last >= declaration.ordinal) {
        return Err(invalid("admission source message order regressed"));
    }
    let existing: Option<(String, Option<String>, String, i64)> = c
        .query_row(
            "SELECT conversation_id,run_id,role,ordinal FROM messages WHERE id=?1",
            [&m.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let ordinal = if let Some((conversation, run, role, ordinal)) = existing {
        if conversation != id.conversation_id
            || run.as_deref() != Some(&id.run_id)
            || role != m.role
        {
            return Err(invalid("admitted message conflicts with local owner"));
        }
        ordinal
    } else {
        let last: i64 = c.query_row(
            "SELECT COALESCE(MAX(ordinal),0) FROM messages WHERE conversation_id=?1",
            [&id.conversation_id],
            |r| r.get(0),
        )?;
        let ordinal = last
            .checked_add(1)
            .ok_or_else(|| invalid("local ordinal exhausted"))?;
        c.execute("INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms,completed_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![m.id,id.conversation_id,id.run_id,m.role,m.status,ordinal,m.created_at_ms,m.completed_at_ms])?;
        ordinal
    };
    if m.role == "assistant" {
        c.execute("UPDATE messages SET created_at_ms=?2 WHERE id=?1", params![m.id,m.created_at_ms])?;
        c.execute(
            "INSERT OR IGNORE INTO remote_assistant_journal_sources VALUES(?1,?2,?3,?4)",
            params![id.run_id, host, id.source_store_epoch, m.id],
        )?;
    } else {
        c.execute("UPDATE messages SET created_at_ms=?2,completed_at_ms=?3,status=?4 WHERE id=?1", params![m.id,m.created_at_ms,m.completed_at_ms,m.status])?;
        c.execute("UPDATE runs SET created_at_ms=?3,updated_at_ms=MAX(updated_at_ms,?3) WHERE id=?1 AND user_message_id=?2", params![id.run_id,m.id,m.created_at_ms])?;
        let mut existing_parts=c.prepare("SELECT id,kind,content_text,content_json FROM message_parts WHERE message_id=?1 ORDER BY ordinal")?;
        let parts = existing_parts
            .query_map([&m.id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (part, kind, text, json) in parts {
            let source = m
                .initial_parts
                .iter()
                .find(|p| p.id == part)
                .ok_or_else(|| invalid("initial user Part identity differs from source"))?;
            let decode = |raw: &Option<String>| -> StoreResult<Option<serde_json::Value>> {
                raw.as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(Into::into)
            };
            if source.kind != kind
                || source.content_text != text
                || decode(&source.content_json)? != decode(&json)?
            {
                return Err(invalid("initial user public content differs from source"));
            }
        }
        // The admission is the source's first immutable declaration, not a
        // mutation of an already admitted user message. Replace only the
        // recipient's provisional Parts of this exact authenticated owner.
        for p in &m.initial_parts {
            let other: Option<String> = c
                .query_row(
                    "SELECT message_id FROM message_parts WHERE id=?1",
                    [&p.id],
                    |r| r.get(0),
                )
                .optional()?;
            if other.as_deref().is_some_and(|v| v != m.id) {
                return Err(invalid("user Part belongs to another message"));
            }
        }
        c.execute("DELETE FROM message_parts WHERE message_id=?1", [&m.id])?;
        for (i, p) in m.initial_parts.iter().enumerate() {
            crate::require_id("admitted user part", &p.id)?;
            if p.ordinal != i as i64 + 1 {
                return Err(invalid("user part order differs"));
            }
            let other: Option<String> = c
                .query_row(
                    "SELECT message_id FROM message_parts WHERE id=?1",
                    [&p.id],
                    |r| r.get(0),
                )
                .optional()?;
            if other.as_deref().is_some_and(|v| v != m.id) {
                return Err(invalid("user Part belongs to another message"));
            }
            if let Some(json) = &p.content_json {
                crate::validate_json("admitted user content", json)?;
            }
            c.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,content_digest,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind,ordinal=excluded.ordinal,state=excluded.state,content_text=excluded.content_text,content_json=excluded.content_json,content_digest=excluded.content_digest,created_at_ms=excluded.created_at_ms,updated_at_ms=excluded.updated_at_ms",params![p.id,m.id,p.kind,p.ordinal,p.state,p.content_text,p.content_json,p.content_digest,p.created_at_ms,p.updated_at_ms])?;
        }
    }
    c.execute(
        "INSERT INTO run_message_admission_owners VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            owner,
            m.id,
            declaration.ordinal,
            ordinal,
            r.declaration_digest.to_hex(),
            m.created_at_ms
        ],
    )?;
    c.execute(
        "DELETE FROM run_message_admission_stages WHERE owner=?1",
        [owner],
    )?;
    c.execute(
        "UPDATE conversations SET revision=revision+1,updated_at_ms=MAX(updated_at_ms,?2) WHERE id=?1",
        params![id.conversation_id,m.created_at_ms],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_unicode_user_is_chunked_durable_and_not_visible_until_complete() {
        use nola_store_port::conversation_history::ConversationProjectionRepository;
        for execution_ownership in [nola_domain::ConversationExecutionOwnership::User,
            nola_domain::ConversationExecutionOwnership::TaskChild { parent_conversation_id:"parent".into(),
                parent_run_id:"parent-run".into(), task_id:"task".into(), child_conversation_id:"c".into() }] {

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("admissions.sqlite");
        let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
        store
            .admit_remote_submission_replica(&nola_store_port::remote_session::AdmittedReplicaTurnInput {
                user_part_kind: "text".into(),
                source_host_id: "host".into(),
                conversation_id: "c".into(),
                conversation_title: "c".into(),
                user_message_id: "u".into(),
                user_part_id: "up".into(),
                user_text: "first".into(),
                user_content_json: None,
                assistant_message_id: "a".into(),
                assistant_part_id: "ap".into(),
                run_id: "r".into(),
                context_checkpoint_id: None,
                created_at_ms: 1,
            })
            .unwrap();
        let id = AssistantJournalIdentity {
            source_store_epoch: "epoch".into(),
            conversation_id: "c".into(),
            run_id: "r".into(),
            assistant_message_id: "a".into(),
        };
        let text = "下一步🙂".repeat(10000);
        let declaration = ReplicaMessageDeclaration {
            execution_ownership: execution_ownership.clone(),
            ordinal: 100,
            input: TransitionMessageInput {
                id: "steering".into(),
                run_id: Some("r".into()),
                role: "user".into(),
                status: "completed".into(),
                created_at_ms: 9,
                completed_at_ms: Some(9),
                initial_parts: vec![MessagePartInput {
                    id: "steering-body".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: text.clone(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 9,
                    updated_at_ms: 9,
                }],
            },
        };
        let raw = serde_json::to_string(&declaration).unwrap();
        let digest = admission_digest(raw.as_bytes());
        let initial = RunMessageAdmissionCursor::initial(id.clone()).unwrap();
        let mut cursor = initial.clone();
        let mut records = vec![];
        let mut offset = 0;
        while offset < raw.len() {
            let mut end = (offset + ADMISSION_CHUNK_BYTES).min(raw.len());
            while !raw.is_char_boundary(end) {
                end -= 1;
            }
            let mut r = RunMessageAdmissionRecord {
                cursor: RunMessageAdmissionCursor {
                    identity: id.clone(),
                    sequence: JournalNumber(cursor.sequence.0 + 1),
                    record_digest: cursor.record_digest,
                },
                previous_digest: cursor.record_digest,
                message_id: "steering".into(),
                offset: JournalNumber(offset as u64),
                total_utf8_bytes: JournalNumber(raw.len() as u64),
                declaration_digest: digest,
                text: raw[offset..end].into(),
            };
            r.cursor.record_digest = r.computed_digest().unwrap();
            cursor = r.cursor.clone();
            records.push(r);
            offset = end;
        }
        assert!(records.len() > 4);
        let through = cursor;
        let mut after = initial;
        for record in records {
            let complete = record.cursor == through;
            let page = RunMessageAdmissionPage {
                after: after.clone(),
                through: through.clone(),
                next: record.cursor.clone(),
                complete,
                records: vec![record],
            };
            assert!(serde_json::to_vec(&page).unwrap().len() < 262144);
            store.begin_write_unit_of_work().unwrap();
            after = store.receive_run_message_admissions("host", &page).unwrap();
            store.commit_write_unit_of_work().unwrap();
            drop(store);
            store = NolaStore::open_archive_mode(&path, false).unwrap();
            let present: bool = store
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE id='steering')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(present, complete);
            if !complete {
                assert_eq!(store.conversation_execution_ownership("c").unwrap(), nola_domain::ConversationExecutionOwnership::Unavailable);
                assert!(require_frontier(&store.connection, "host", &through).is_err());
            }
        }
        let actual: String = store
            .connection
            .query_row(
                "SELECT content_text FROM message_parts WHERE id='steering-body'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(actual, text);
        assert_eq!(store.conversation_execution_ownership("c").unwrap(), execution_ownership);
        // Even a newly checksummed declaration from the same source cannot
        // rewrite the already admitted execution owner of this Run.
        let mut conflicting = declaration.clone();
        conflicting.ordinal += 1;
        conflicting.input.id = "conflicting-owner-message".into();
        conflicting.execution_ownership = nola_domain::ConversationExecutionOwnership::TaskChild {
            parent_conversation_id:"foreign-parent".into(), parent_run_id:"foreign-run".into(),
            task_id:"foreign-task".into(), child_conversation_id:"c".into(),
        };
        conflicting.input.initial_parts.clear();
        let raw = serde_json::to_string(&conflicting).unwrap();
        let mut record = RunMessageAdmissionRecord {
            cursor: RunMessageAdmissionCursor { identity: through.identity.clone(), sequence: JournalNumber(through.sequence.0+1), record_digest: through.record_digest },
            previous_digest: through.record_digest, message_id: conflicting.input.id,
            offset: JournalNumber(0), total_utf8_bytes: JournalNumber(raw.len() as u64), declaration_digest: admission_digest(raw.as_bytes()), text: raw,
        };
        record.cursor.record_digest = record.computed_digest().unwrap();
        let page = RunMessageAdmissionPage { after: through.clone(), through: record.cursor.clone(), next: record.cursor.clone(), complete:true, records:vec![record] };
        store.begin_write_unit_of_work().unwrap();
        assert!(store.receive_run_message_admissions("host", &page).is_err());
        store.rollback_write_unit_of_work().unwrap();
        assert_eq!(store.conversation_execution_ownership("c").unwrap(), execution_ownership);

        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT count(*) FROM run_message_admission_chunks",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        require_frontier(&store.connection, "host", &through).unwrap();
        drop(store);
        let store = NolaStore::open_archive_mode(&path, false).unwrap();
        assert_eq!(store.schema_version().unwrap(), crate::STORE_SCHEMA_VERSION);
        assert_eq!(store.conversation_execution_ownership("c").unwrap(), execution_ownership);
        assert_eq!(
            store
                .load_conversation("c")
                .unwrap()
                .unwrap()
                .messages
                .len(),
            3
        );
        }
}
}
