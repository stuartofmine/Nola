// SPDX-License-Identifier: Apache-2.0
//! Durable bounded archive staging and one atomic canonical history replacement.
use super::*;
use nola_domain::archive_restore::*;
use nola_domain::CanonicalBodyReference;
use nola_store_port::archive_restore::ArchiveRestoreRepository;

fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(message.into())
}
fn conflict(message: &str) -> StoreError {
    StoreError::Conflict(message.into())
}
fn id(value: &str) -> StoreResult<()> {
    if value.is_empty()
        || value.len() > 128
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(invalid("invalid archive identity"));
    }
    Ok(())
}
fn number(value: u64) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| invalid("archive integer overflow"))
}
fn initial_digest() -> String {
    format!("{:x}", Sha256::digest(b"nola-archive-restore-v1"))
}
fn advance_digest(previous: &str, entry: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(previous.as_bytes());
    digest.update((entry.len() as u64).to_be_bytes());
    digest.update(entry.as_bytes());
    format!("{:x}", digest.finalize())
}
pub(super) fn directory_revision(connection: &Connection) -> StoreResult<u64> {
    let revision: i64 = connection.query_row(
        "SELECT revision FROM archive_directory_revision WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    u64::try_from(revision).map_err(|_| invalid("invalid canonical directory revision"))
}
pub(super) fn progress_on_connection(
    connection: &Connection,
    restore_id: &str,
) -> StoreResult<ArchiveRestoreProgress> {
    id(restore_id)?;
    connection.query_row("SELECT next_sequence,manifest_digest,state FROM archive_restore_sessions WHERE restore_id=?1",params![restore_id],|row|Ok(ArchiveRestoreProgress {
        restore_id:restore_id.into(),next_sequence:row.get(0)?,manifest_digest:row.get(1)?,completed:row.get::<_,String>(2)?=="committed",
    })).optional()?.ok_or_else(||conflict("unknown archive restore session"))
}
pub(super) fn begin_on_connection(
    connection: &Connection,
    input: &BeginArchiveRestore,
    replica: bool,
) -> StoreResult<ArchiveRestoreProgress> {
    id(&input.restore_id)?;
    number(input.expected_directory_revision)?;
    let encoded = serde_json::to_string(input)?;
    let previous: Option<String> = connection
        .query_row(
            "SELECT begin_json FROM archive_restore_sessions WHERE restore_id=?1",
            params![input.restore_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(previous) = previous {
        if previous != encoded {
            return Err(conflict("archive begin replay differs"));
        }
        return progress_on_connection(connection, &input.restore_id);
    }
    let current = directory_revision(connection)?;
    if !replica && current != input.expected_directory_revision {
        return Err(conflict("archive directory revision changed before begin"));
    }
    connection.execute(
        "INSERT INTO archive_restore_sessions VALUES(?1,?2,?3,0,?4,'staging',NULL,NULL)",
        params![
            input.restore_id,
            encoded,
            number(current)?,
            initial_digest()
        ],
    )?;
    progress_on_connection(connection, &input.restore_id)
}

fn validate_entry(entry: &ArchiveRestoreEntry) -> StoreResult<String> {
    // Bound allocation before serializing untrusted inline payloads.
    match entry {
        ArchiveRestoreEntry::ToolSource { claim } => {
            claim.validate().map_err(invalid_owned)?;
        }
        ArchiveRestoreEntry::PartDerivation { record } => {
            crate::part_derivation::validate_shape(record)?;
            if serde_json::to_vec(record)?.len() > 16384 {
                return Err(invalid(
                    "archive Part derivation record exceeds byte budget",
                ));
            }
        }
        ArchiveRestoreEntry::Profile { profile } => {
            id(&profile.id)?;
            if profile.title.len() > 4096
                || profile
                    .settings_json
                    .as_ref()
                    .is_some_and(|s| s.len() > 65536)
                || profile.updated_at_ms < profile.created_at_ms
            {
                return Err(invalid("invalid archive profile"));
            }
            if let Some(settings) = &profile.settings_json {
                let value: serde_json::Value = serde_json::from_str(settings)?;
                if !value.is_object() {
                    return Err(invalid(
                        "archive conversation settings must be a JSON object",
                    ));
                }
            }
        }
        ArchiveRestoreEntry::Message {
            conversation_id,
            message,
            references,
        } => {
            id(conversation_id)?;
            id(&message.id)?;
            if let Some(run) = &message.run_id {
                id(run)?;
            }
            if !matches!(message.role.as_str(), "user" | "assistant" | "system")
                || message.status != "completed"
                || message.ordinal < 1
                || !message
                    .completed_at_ms
                    .is_some_and(|at| at >= message.created_at_ms)
                || message.parts.is_empty()
                || message.parts.len() > 512
                || references.len() > 512
            {
                return Err(invalid("archive message must be bounded finalized history"));
            }
            let mut bytes = 0usize;
            let mut parts = std::collections::HashMap::new();
            for (offset, part) in message.parts.iter().enumerate() {
                id(&part.id)?;
                if part.ordinal != offset as i64 + 1
                    || part.state != "completed"
                    || part.updated_at_ms < part.created_at_ms
                    || parts.insert(part.id.as_str(), part).is_some()
                {
                    return Err(invalid("invalid archive Part identity, order or state"));
                }
                bytes = bytes
                    .saturating_add(part.content_text.len())
                    .saturating_add(part.content_json.as_ref().map_or(0, String::len))
                    .saturating_add(part.content_digest.as_ref().map_or(0, String::len));
                if bytes > 128 * 1024 {
                    return Err(invalid("archive inline body exceeds manifest budget"));
                }
                if let Some(json) = &part.content_json {
                    validate_json("archive Part JSON", json)?;
                }
            }
            crate::body_binding::validate_field_bindings(
                conversation_id,
                message
                    .parts
                    .iter()
                    .map(|part| (message.id.as_str(), message.status.as_str(), part)),
                references,
            )?;
        }
    }
    let encoded = serde_json::to_string(entry)?;
    if encoded.len() > 128 * 1024 {
        return Err(invalid("archive entry exceeds encoded manifest budget"));
    }
    Ok(encoded)
}

pub(super) fn stage_on_connection(
    connection: &Connection,
    restore_id: &str,
    sequence: u64,
    entry: &ArchiveRestoreEntry,
    require_prepared_binding: bool,
) -> StoreResult<ArchiveRestoreProgress> {
    id(restore_id)?;
    let sequence_i64 = number(sequence)?;
    let encoded = validate_entry(entry)?;
    let progress = progress_on_connection(connection, restore_id)?;
    if sequence < progress.next_sequence {
        let previous: String = connection.query_row(
            "SELECT entry_json FROM archive_restore_entries WHERE restore_id=?1 AND sequence=?2",
            params![restore_id, sequence_i64],
            |row| row.get(0),
        )?;
        if previous != encoded {
            return Err(conflict("archive stage replay differs"));
        }
        return Ok(progress);
    }
    if progress.completed || sequence != progress.next_sequence {
        return Err(conflict(
            "archive stage sequence is not the durable next offset",
        ));
    }
    match entry {
        ArchiveRestoreEntry::ToolSource { claim } => {
            validate_staged_tool_source(connection, restore_id, claim)?;
            let duplicate: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM archive_restore_entries WHERE restore_id=?1
                 AND json_extract(entry_json,'$.kind')='tool_source'
                 AND ((json_extract(entry_json,'$.claim.owner.conversation_id')=?2
                       AND json_extract(entry_json,'$.claim.source_item_id')=?3)
                      OR (json_extract(entry_json,'$.claim.owner.part_id')=?4
                          AND json_extract(entry_json,'$.claim.kind')=?5)))",
                params![restore_id, claim.owner.conversation_id(), claim.source_item_id,
                    claim.owner.part_id(), match claim.kind {
                        nola_domain::tool_source::ToolSourceKind::ToolCall => "tool_call",
                        nola_domain::tool_source::ToolSourceKind::ToolResult => "tool_result",
                    }],
                |row| row.get(0),
            )?;
            if duplicate {
                return Err(conflict("archive tool source identity or Part kind is already staged"));
            }
            if require_prepared_binding {
                let prepared: Option<String> = connection.query_row(
                    "SELECT reference_json FROM archive_restore_prepared WHERE restore_id=?1 AND content_id=?2",
                    params![restore_id, claim.body.content_id()], |row| row.get(0),
                ).optional()?;
                if prepared.as_deref().map(serde_json::from_str::<CanonicalBodyReference>)
                    .transpose()?.as_ref() != Some(&claim.body)
                {
                    return Err(conflict("archive tool source body is not prepared for this restore"));
                }
                crate::tool_source::validate_imported_tool_source_body(connection, claim)?;
            }
        }
        ArchiveRestoreEntry::PartDerivation { record } => {
            let owner = &record.target.owner;
            let declared: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM archive_restore_parts p JOIN archive_restore_messages m
                 ON m.restore_id=p.restore_id AND m.message_id=p.message_id
                 WHERE p.restore_id=?1 AND p.part_id=?2 AND m.message_id=?3 AND m.conversation_id=?4)",
                params![restore_id, owner.part_id(), owner.message_id(), owner.conversation_id()], |row| row.get(0))?;
            if !declared {
                return Err(conflict(
                    "archive Part derivation requires its previously staged target owner",
                ));
            }
            connection.execute("INSERT INTO archive_restore_derivations(restore_id,target_part_id,sequence) VALUES(?1,?2,?3)",
                params![restore_id, owner.part_id(), sequence_i64])?;
        }
        ArchiveRestoreEntry::Profile { profile } => {
            connection.execute(
                "INSERT INTO archive_restore_profiles VALUES(?1,?2,?3)",
                params![restore_id, profile.id, serde_json::to_string(profile)?],
            )?;
        }
        ArchiveRestoreEntry::Message {
            conversation_id,
            message,
            references,
        } => {
            let profile:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM archive_restore_profiles WHERE restore_id=?1 AND conversation_id=?2)",params![restore_id,conversation_id],|row|row.get(0))?;
            if !profile {
                return Err(conflict("archive profile must precede its messages"));
            }
            let count:i64=connection.query_row("SELECT COUNT(*) FROM archive_restore_messages WHERE restore_id=?1 AND conversation_id=?2",params![restore_id,conversation_id],|row|row.get(0))?;
            if message.ordinal != count + 1 {
                return Err(conflict("archive message ordinals must be contiguous"));
            }
            for reference in references {
                if require_prepared_binding {
                    let prepared:Option<String>=connection.query_row("SELECT reference_json FROM archive_restore_prepared WHERE restore_id=?1 AND content_id=?2",params![restore_id,reference.content_id()],|row|row.get(0)).optional()?;
                    if prepared
                        .as_deref()
                        .map(serde_json::from_str::<CanonicalBodyReference>)
                        .transpose()?
                        .as_ref()
                        != Some(reference)
                    {
                        return Err(conflict(
                            "archive body is not prepared for this restore session",
                        ));
                    }
                    canonical_text_replica::require_complete(connection, reference)?;
                }
            }
            connection.execute(
                "INSERT INTO archive_restore_messages VALUES(?1,?2,?3,?4,?5)",
                params![
                    restore_id,
                    message.id,
                    conversation_id,
                    message.ordinal,
                    sequence_i64
                ],
            )?;
            for part in &message.parts {
                connection.execute(
                    "INSERT INTO archive_restore_parts VALUES(?1,?2,?3)",
                    params![restore_id, part.id, message.id],
                )?;
            }
        }
    }
    connection.execute(
        "INSERT INTO archive_restore_entries VALUES(?1,?2,?3)",
        params![restore_id, sequence_i64, encoded],
    )?;
    let next = sequence
        .checked_add(1)
        .ok_or_else(|| invalid("archive sequence overflow"))?;
    let digest = advance_digest(&progress.manifest_digest, &encoded);
    connection.execute("UPDATE archive_restore_sessions SET next_sequence=?2,manifest_digest=?3 WHERE restore_id=?1",params![restore_id,number(next)?,digest])?;
    progress_on_connection(connection, restore_id)
}

impl ArchiveRestoreRepository for NolaStore {
    type Error = StoreError;
    fn archive_directory_revision(&self) -> StoreResult<u64> {
        directory_revision(&self.connection)
    }
    fn archive_restore_progress(&self, restore_id: &str) -> StoreResult<ArchiveRestoreProgress> {
        progress_on_connection(&self.connection, restore_id)
    }
    fn begin_archive_restore(
        &mut self,
        input: &BeginArchiveRestore,
    ) -> StoreResult<ArchiveRestoreProgress> {
        let tx = self.write_transaction()?;
        let result = begin_on_connection(&tx, input, false)?;
        tx.commit()?;
        Ok(result)
    }
    fn archive_restore_session_exists(&self, restore_id: &str) -> StoreResult<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM archive_restore_sessions WHERE restore_id=?1)",
            params![restore_id],
            |row| row.get(0),
        )?)
    }
    fn archive_restore_accepts_upload(
        &self,
        restore_id: &str,
        conversation_id: &str,
    ) -> StoreResult<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM archive_restore_sessions s JOIN archive_restore_profiles p ON p.restore_id=s.restore_id WHERE s.restore_id=?1 AND s.state='staging' AND p.conversation_id=?2)",params![restore_id,conversation_id],|row|row.get(0))?)
    }
    fn prepared_archive_reference(
        &self,
        restore_id: &str,
        content_id: &str,
    ) -> StoreResult<Option<CanonicalBodyReference>> {
        let encoded:Option<String>=self.connection.query_row("SELECT reference_json FROM archive_restore_prepared WHERE restore_id=?1 AND content_id=?2",params![restore_id,content_id],|row|row.get(0)).optional()?;
        encoded
            .map(|value| serde_json::from_str(&value).map_err(StoreError::from))
            .transpose()
    }
    fn prepare_archive_content(
        &mut self,
        restore_id: &str,
        reference: &CanonicalBodyReference,
        chunks: &mut dyn Iterator<Item = Result<(u64, String), String>>,
        maximum_bytes: u64,
    ) -> StoreResult<()> {
        let tx = self.write_transaction()?;
        let accepts:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM archive_restore_sessions s JOIN archive_restore_profiles p ON p.restore_id=s.restore_id WHERE s.restore_id=?1 AND s.state='staging' AND p.conversation_id=?2)",params![restore_id,reference.conversation_id()],|row|row.get(0))?;
        if !accepts {
            return Err(conflict("archive body owner has no staged profile"));
        }
        canonical_text_store::put_chunks(
            &tx,
            reference,
            chunks.map(|chunk| chunk.map_err(invalid_owned)),
            maximum_bytes,
        )?;
        let encoded = serde_json::to_string(reference)?;
        let previous:Option<String>=tx.query_row("SELECT reference_json FROM archive_restore_prepared WHERE restore_id=?1 AND content_id=?2",params![restore_id,reference.content_id()],|row|row.get(0)).optional()?;
        if previous.as_ref().is_some_and(|old| old != &encoded) {
            return Err(conflict("archive prepared reference replay differs"));
        }
        tx.execute(
            "INSERT OR IGNORE INTO archive_restore_prepared VALUES(?1,?2,?3)",
            params![restore_id, reference.content_id(), encoded],
        )?;
        tx.commit()?;
        Ok(())
    }
    fn stage_archive_entry(
        &mut self,
        restore_id: &str,
        sequence: u64,
        entry: &ArchiveRestoreEntry,
    ) -> StoreResult<ArchiveRestoreProgress> {
        let tx = self.write_transaction()?;
        let result = stage_on_connection(&tx, restore_id, sequence, entry, true)?;
        tx.commit()?;
        Ok(result)
    }
    fn commit_archive_restore(
        &mut self,
        input: &CommitArchiveRestore,
    ) -> StoreResult<ArchiveRestoreResult> {
        if !self.has_write_unit_of_work() {
            return Err(invalid(
                "archive final commit requires the command receipt write unit",
            ));
        }
        let tx = self.write_transaction()?;
        let result = commit_on_connection(&tx, input, false)?;
        tx.commit()?;
        Ok(result)
    }
}
fn invalid_owned(message: String) -> StoreError {
    StoreError::Invariant(message)
}

fn validate_staged_tool_source(
    connection: &Connection,
    restore_id: &str,
    claim: &nola_domain::tool_source::ToolSourceClaim,
) -> StoreResult<()> {
    claim.validate().map_err(invalid_owned)?;
    let encoded: Option<String> = connection.query_row(
        "SELECT e.entry_json FROM archive_restore_parts p
         JOIN archive_restore_messages m ON m.restore_id=p.restore_id AND m.message_id=p.message_id
         JOIN archive_restore_entries e ON e.restore_id=m.restore_id AND e.sequence=m.sequence
         WHERE p.restore_id=?1 AND p.part_id=?2 AND m.message_id=?3 AND m.conversation_id=?4",
        params![restore_id, claim.owner.part_id(), claim.owner.message_id(), claim.owner.conversation_id()],
        |row| row.get(0),
    ).optional()?;
    let entry: ArchiveRestoreEntry = serde_json::from_str(
        encoded.as_deref().ok_or_else(|| conflict("archive tool source requires its previously staged target"))?
    )?;
    let ArchiveRestoreEntry::Message { message, .. } = entry else {
        return Err(invalid("archive tool source target index does not name a message"));
    };
    if message.role != "assistant" || !message.parts.iter().any(|part|
        part.id == claim.owner.part_id() && part.kind == "tool" && part.state == "completed")
    {
        return Err(conflict("archive tool source target is not a finalized assistant Tool Part"));
    }
    Ok(())
}

fn validate_commit(
    connection: &Connection,
    input: &CommitArchiveRestore,
    replica: bool,
) -> StoreResult<Option<ArchiveRestoreResult>> {
    id(&input.restore_id)?;
    number(input.expected_directory_revision)?;
    number(input.expected_entry_count)?;
    let (begin,expected,sequence,digest,state,old_commit,result):(String,i64,i64,String,String,Option<String>,Option<String>)=connection.query_row(
        "SELECT begin_json,expected_directory_revision,next_sequence,manifest_digest,state,commit_json,result_json FROM archive_restore_sessions WHERE restore_id=?1",params![input.restore_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)))?;
    let encoded = serde_json::to_string(input)?;
    if state == "committed" {
        if old_commit.as_deref() != Some(encoded.as_str()) {
            return Err(conflict("archive final commit replay differs"));
        }
        return Ok(Some(serde_json::from_str(
            result
                .as_deref()
                .ok_or_else(|| invalid("archive receipt is missing"))?,
        )?));
    }
    let original: BeginArchiveRestore = serde_json::from_str(&begin)?;
    if input.expected_directory_revision != original.expected_directory_revision
        || sequence != number(input.expected_entry_count)?
        || digest != input.manifest_digest
    {
        return Err(conflict(
            "archive final manifest does not match staged content",
        ));
    }
    if directory_revision(connection)? != expected as u64 {
        return Err(conflict(
            "archive canonical directory changed during staging",
        ));
    }
    let active: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM runs WHERE status NOT IN ('sealed'))",
        [],
        |row| row.get(0),
    )?;
    if active {
        return Err(conflict("archive restore cannot overlap any active Run"));
    }
    // Recompute from durable entries: the digest is evidence of exact staged
    // bytes, not merely a client claim or an independently mutable counter.
    let mut statement=connection.prepare("SELECT sequence,CASE WHEN length(CAST(entry_json AS BLOB))<=131072 THEN entry_json END FROM archive_restore_entries WHERE restore_id=?1 ORDER BY sequence")?;
    let mut rows = statement.query(params![input.restore_id])?;
    let mut expected_sequence = 0;
    let mut computed = initial_digest();
    while let Some(row) = rows.next()? {
        let seq: i64 = row.get(0)?;
        let encoded: String = row.get(1)?;
        if seq != expected_sequence {
            return Err(conflict("archive staged entries are incomplete"));
        }
        let entry: ArchiveRestoreEntry = serde_json::from_str(&encoded)?;
        if validate_entry(&entry)? != encoded {
            return Err(conflict("archive staged entry encoding changed"));
        }
        if let ArchiveRestoreEntry::Message { references, .. } = &entry {
            for reference in references {
                canonical_text_replica::require_complete(connection, reference)?;
            }
        }
        if let ArchiveRestoreEntry::ToolSource { claim } = &entry {
            validate_staged_tool_source(connection, &input.restore_id, claim)?;
            crate::tool_source::validate_imported_tool_source_body(connection, claim)?;
        }
        if let ArchiveRestoreEntry::PartDerivation { record } = &entry {
            let indexed: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM archive_restore_derivations WHERE restore_id=?1 AND target_part_id=?2 AND sequence=?3)",
                params![input.restore_id,record.target.owner.part_id(),seq], |row| row.get(0))?;
            if !indexed {
                return Err(conflict("archive derivation index is incomplete"));
            }
        }
        computed = advance_digest(&computed, &encoded);
        expected_sequence += 1;
    }
    if computed != digest || expected_sequence != sequence {
        return Err(conflict("archive staged digest or count mismatch"));
    }
    if !replica && expected as u64 != input.expected_directory_revision {
        return Err(conflict("archive local CAS scope mismatch"));
    }
    Ok(None)
}

fn prepare_targets(connection: &Connection, restore_id: &str) -> StoreResult<()> {
    let mut statement=connection.prepare("SELECT id FROM conversations UNION SELECT conversation_id FROM archive_restore_profiles WHERE restore_id=?1 ORDER BY 1")?;
    let mut rows = statement.query(params![restore_id])?;
    while let Some(row) = rows.next()? {
        let conversation_id: String = row.get(0)?;
        let base_revision: Option<i64> = connection
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()?;
        let marker:Option<(i64,bool)>=connection.query_row("SELECT revision,deleted FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id=?1",params![conversation_id],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let retained:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM archive_restore_profiles WHERE restore_id=?1 AND conversation_id=?2)",params![restore_id,conversation_id],|row|row.get(0))?;
        if marker.is_some_and(|(_, deleted)| deleted) {
            return Err(conflict(
                "archive restore cannot resurrect a tombstoned conversation identity",
            ));
        }
        let resulting_revision = base_revision
            .unwrap_or(0)
            .max(marker.map_or(0, |(revision, _)| revision))
            .checked_add(1)
            .ok_or_else(|| invalid("archive conversation revision overflow"))?;
        let target = ArchiveRestoreTarget {
            conversation_id: conversation_id.clone(),
            base_revision,
            base_content_revision: marker.map(|(revision, _)| revision),
            resulting_revision,
            deleted: !retained,
        };
        connection.execute(
            "INSERT INTO archive_restore_targets VALUES(?1,?2,?3)",
            params![restore_id, conversation_id, serde_json::to_string(&target)?],
        )?;
    }
    Ok(())
}

fn validate_targets(connection: &Connection, restore_id: &str) -> StoreResult<()> {
    let missing:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM conversations c WHERE NOT EXISTS(SELECT 1 FROM archive_restore_targets t WHERE t.restore_id=?1 AND t.conversation_id=c.id)) OR EXISTS(SELECT 1 FROM archive_restore_profiles p WHERE p.restore_id=?1 AND NOT EXISTS(SELECT 1 FROM archive_restore_targets t WHERE t.restore_id=?1 AND t.conversation_id=p.conversation_id))",params![restore_id],|row|row.get(0))?;
    if missing {
        return Err(conflict(
            "archive target facts do not cover the complete canonical directory",
        ));
    }
    let mut statement=connection.prepare("SELECT conversation_id,target_json FROM archive_restore_targets WHERE restore_id=?1 ORDER BY conversation_id")?;
    let mut rows = statement.query(params![restore_id])?;
    while let Some(row) = rows.next()? {
        let identity: String = row.get(0)?;
        crate::agent_room_admission::require_unowned_context(connection, &identity)?;
        let target: ArchiveRestoreTarget = serde_json::from_str(&row.get::<_, String>(1)?)?;
        id(&target.conversation_id)?;
        let actual: Option<i64> = connection
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                params![identity],
                |row| row.get(0),
            )
            .optional()?;
        let marker:Option<(i64,bool)>=connection.query_row("SELECT revision,deleted FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id=?1",params![identity],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        let retained:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM archive_restore_profiles WHERE restore_id=?1 AND conversation_id=?2)",params![restore_id,identity],|row|row.get(0))?;
        let expected = actual
            .unwrap_or(0)
            .max(marker.map_or(0, |(revision, _)| revision))
            .checked_add(1)
            .ok_or_else(|| invalid("archive target revision overflow"))?;
        if identity != target.conversation_id
            || actual != target.base_revision
            || marker.map(|(revision, _)| revision) != target.base_content_revision
            || marker.is_some_and(|(_, deleted)| deleted)
            || target.deleted == retained
            || target.resulting_revision != expected
            || (target.deleted && actual.is_none())
        {
            return Err(conflict(
                "archive target base revision or ownership changed",
            ));
        }
    }
    Ok(())
}

pub(super) fn commit_replica_on_connection(
    connection: &Connection,
    input: &CommitArchiveRestore,
) -> StoreResult<ArchiveRestoreResult> {
    commit_on_connection(connection, input, true)
}
fn commit_on_connection(
    connection: &Connection,
    input: &CommitArchiveRestore,
    replica: bool,
) -> StoreResult<ArchiveRestoreResult> {
    if let Some(result) = validate_commit(connection, input, replica)? {
        return Ok(result);
    }
    if !replica {
        prepare_targets(connection, &input.restore_id)?;
    }
    validate_targets(connection, &input.restore_id)?;
    // History replacement preserves conversation identities/revision lineage.
    // Retire obsolete runtime caches before deleting messages referenced by Runs.
    for table in [
        "runs",
        "core_checkpoints",
        "context_checkpoints",
        "remote_session_mirrors",
        "remote_conversation_projections",
    ] {
        connection.execute(&format!("DELETE FROM {table} WHERE conversation_id IN(SELECT conversation_id FROM archive_restore_targets WHERE restore_id=?1)"),params![input.restore_id])?;
    }
    connection.execute("DELETE FROM messages WHERE conversation_id IN(SELECT conversation_id FROM archive_restore_targets WHERE restore_id=?1)",params![input.restore_id])?;
    connection.execute("DELETE FROM data_source_overrides WHERE conversation_id IN(SELECT conversation_id FROM archive_restore_targets WHERE restore_id=?1 AND json_extract(target_json,'$.deleted')=1)",params![input.restore_id])?;
    let mut statement=connection.prepare("SELECT target_json FROM archive_restore_targets WHERE restore_id=?1 ORDER BY conversation_id")?;
    let mut rows = statement.query(params![input.restore_id])?;
    while let Some(row) = rows.next()? {
        let target: ArchiveRestoreTarget = serde_json::from_str(&row.get::<_, String>(0)?)?;
        if target.deleted {
            connection.execute(
                "DELETE FROM conversations WHERE id=?1",
                params![target.conversation_id],
            )?;
        } else {
            let encoded:String=connection.query_row("SELECT profile_json FROM archive_restore_profiles WHERE restore_id=?1 AND conversation_id=?2",params![input.restore_id,target.conversation_id],|row|row.get(0))?;
            let profile: ArchiveConversationProfile = serde_json::from_str(&encoded)?;
            connection.execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET title=excluded.title,revision=excluded.revision,created_at_ms=excluded.created_at_ms,updated_at_ms=excluded.updated_at_ms",params![profile.id,profile.title,target.resulting_revision,profile.created_at_ms,profile.updated_at_ms])?;
            connection.execute(
                "DELETE FROM conversation_settings WHERE conversation_id=?1",
                params![profile.id],
            )?;
            if let Some(settings) = profile.settings_json {
                connection.execute(
                    "INSERT INTO conversation_settings VALUES(?1,?2,?3)",
                    params![profile.id, settings, profile.updated_at_ms],
                )?;
            }
        }
        connection.execute("INSERT INTO sync_entity_revisions(entity_type,entity_id,revision,deleted,operation_id,updated_at_ms) VALUES('conversation',?1,?2,?3,?4,?5) ON CONFLICT(entity_type,entity_id) DO UPDATE SET revision=excluded.revision,deleted=excluded.deleted,operation_id=excluded.operation_id,updated_at_ms=excluded.updated_at_ms",params![target.conversation_id,target.resulting_revision,target.deleted,format!("archive:{}",input.restore_id),input.committed_at_ms])?;
    }
    drop(rows);
    drop(statement);
    let mut statement=connection.prepare("SELECT e.entry_json FROM archive_restore_messages m JOIN archive_restore_entries e ON e.restore_id=m.restore_id AND e.sequence=m.sequence WHERE m.restore_id=?1 ORDER BY m.sequence")?;
    let mut rows = statement.query(params![input.restore_id])?;
    while let Some(row) = rows.next()? {
        let entry: ArchiveRestoreEntry = serde_json::from_str(&row.get::<_, String>(0)?)?;
        let ArchiveRestoreEntry::Message {
            conversation_id,
            message,
            references,
        } = entry
        else {
            return Err(invalid("archive message index does not name a message"));
        };
        connection.execute(
            "INSERT INTO messages VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                message.id,
                conversation_id,
                message.run_id,
                message.role,
                message.status,
                message.ordinal,
                message.created_at_ms,
                message.completed_at_ms
            ],
        )?;
        for part in message.parts {
            connection.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,content_digest,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![part.id,message.id,part.kind,part.ordinal,part.state,part.content_text,part.content_json,part.content_digest,part.created_at_ms,part.updated_at_ms])?;
        }
        for reference in references {
            let field = match reference.field() {
                nola_domain::PartBodyField::ContentText => "content_text",
                nola_domain::PartBodyField::ContentJson => "content_json",
            };
            connection.execute(
                "INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES(?1,?2,?3)",
                params![reference.part_id(), field, serde_json::to_string(&reference)?],
            )?;
        }
    }
    drop(rows);
    drop(statement);
    // Imported records remain untrusted claims. Validate against the final canonical
    // fields only after all Messages/references have been rebuilt in this transaction.
    let mut statement = connection.prepare(
        "SELECT CASE WHEN length(CAST(d.target_part_id AS BLOB))<=128 THEN d.target_part_id END,
         CASE WHEN length(CAST(e.entry_json AS BLOB))<=131072 THEN e.entry_json END
         FROM archive_restore_derivations d JOIN archive_restore_entries e ON e.restore_id=d.restore_id AND e.sequence=d.sequence
         WHERE d.restore_id=?1 ORDER BY d.sequence")?;
    let mut rows = statement.query(params![input.restore_id])?;
    while let Some(row) = rows.next()? {
        let part: String = row.get(0)?;
        let entry: ArchiveRestoreEntry = serde_json::from_str(&row.get::<_, String>(1)?)?;
        let ArchiveRestoreEntry::PartDerivation { record } = entry else {
            return Err(invalid(
                "archive derivation index does not name a Part derivation",
            ));
        };
        if part != record.target.owner.part_id() {
            return Err(invalid("archive derivation index owner differs"));
        }
        crate::part_derivation::import_archive_claim(connection, &record)?;
    }
    drop(rows);
    drop(statement);
    // Source claims are installed only after every final target and derivation
    // exists. This stays inside the same directory replacement transaction.
    let mut statement = connection.prepare(
        "SELECT sequence,entry_json FROM archive_restore_entries WHERE restore_id=?1
         AND json_extract(entry_json,'$.kind')='tool_source' ORDER BY sequence",
    )?;
    let mut rows = statement.query(params![input.restore_id])?;
    while let Some(row) = rows.next()? {
        let sequence: u64 = row.get(0)?;
        let entry: ArchiveRestoreEntry = serde_json::from_str(&row.get::<_, String>(1)?)?;
        let ArchiveRestoreEntry::ToolSource { claim } = entry else {
            return Err(invalid("archive source index does not name a tool source"));
        };
        crate::tool_source::install_imported_tool_source(connection, &claim,
            crate::tool_source::ToolSourceImport::Archive {
                restore_id: input.restore_id.clone(), sequence,
            })?;
    }
    drop(rows);
    drop(statement);
    let conversation_count: u64 = connection.query_row(
        "SELECT COUNT(*) FROM archive_restore_profiles WHERE restore_id=?1",
        params![input.restore_id],
        |row| row.get(0),
    )?;
    let message_count: u64 = connection.query_row(
        "SELECT COUNT(*) FROM archive_restore_messages WHERE restore_id=?1",
        params![input.restore_id],
        |row| row.get(0),
    )?;
    let result = ArchiveRestoreResult {
        restore_id: input.restore_id.clone(),
        conversation_count,
        message_count,
        directory_revision: directory_revision(connection)?,
    };
    connection.execute("UPDATE archive_restore_sessions SET state='committed',commit_json=?2,result_json=?3 WHERE restore_id=?1",params![input.restore_id,serde_json::to_string(input)?,serde_json::to_string(&result)?])?;
    if !replica {
        crate::archive_restore_sync::enqueue(connection, &input.restore_id, input.committed_at_ms)?;
    }
    Ok(result)
}

impl NolaStore {
    pub fn archive_restore_conversation_ids(&self, restore_id: &str) -> StoreResult<Vec<String>> {
        id(restore_id)?;
        let mut statement=self.connection.prepare("SELECT id FROM conversations UNION SELECT conversation_id FROM archive_restore_profiles WHERE restore_id=?1 UNION SELECT conversation_id FROM archive_restore_targets WHERE restore_id=?1 ORDER BY 1")?;
        let ids = statement
            .query_map(params![restore_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile(id: &str, title: &str) -> ArchiveRestoreEntry {
        ArchiveRestoreEntry::Profile {
            profile: ArchiveConversationProfile {
                id: id.into(),
                title: title.into(),
                settings_json: Some("{}".into()),
                created_at_ms: 1,
                updated_at_ms: 3,
            },
        }
    }
    fn message(text: &str) -> ArchiveRestoreEntry {
        ArchiveRestoreEntry::Message{conversation_id:"c".into(),message:ArchiveMessage{id:"m".into(),run_id:Some("terminal-run".into()),role:"user".into(),status:"completed".into(),ordinal:1,created_at_ms:1,completed_at_ms:Some(2),parts:vec![MessagePartInput{id:"p".into(),kind:"text".into(),ordinal:1,state:"completed".into(),content_text:text.into(),content_json:None,content_digest:None,created_at_ms:1,updated_at_ms:2},MessagePartInput{id:"semantic".into(),kind:"tool_result".into(),ordinal:2,state:"completed".into(),content_text:"tool evidence".into(),content_json:Some(r#"{"source_uri":"file:///fixture/document","output":"original semantic evidence"}"#.into()),content_digest:None,created_at_ms:1,updated_at_ms:2}]},references:vec![]}
    }
    fn fixture(path: &std::path::Path) -> NolaStore {
        let mut store = NolaStore::open(path).unwrap();
        for id in ["c", "excluded"] {
            store
                .create_conversation(&CreateConversationInput {
                    id: id.into(),
                    title: "Old".into(),
                    revision: 1,
                    settings_json: Some("{}".into()),
                    created_at_ms: 1,
                    updated_at_ms: 1,
                })
                .unwrap();
        }
        if let ArchiveRestoreEntry::Message { message, .. } = message("original") {
            store
                .append_messages(&AppendMessagesInput {
                    conversation_id: "c".into(),
                    expected_message_count: 0,
                    updated_at_ms: 2,
                    messages: vec![MessageInput {
                        id: message.id,
                        run_id: message.run_id,
                        role: message.role,
                        status: message.status,
                        ordinal: message.ordinal,
                        created_at_ms: message.created_at_ms,
                        completed_at_ms: message.completed_at_ms,
                        parts: message.parts,
                    }],
                })
                .unwrap();
        }
        // Deliberately corrupt retained execution metadata: archive replacement
        // must remove it, not interpret a claimed lifecycle as a valid result.
        store.connection.execute_batch("INSERT INTO runs(id,conversation_id,kind,user_message_id,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms,completed_at_ms) VALUES('terminal-run','c','conversation','m','m','sealed',2,1,2,2);
          INSERT INTO run_events VALUES('event','terminal-run',1,'completed','{}',2);
          INSERT INTO effect_outbox(effect_id,run_id,request_id,effect_kind,binding_json,payload_digest,recovery_required,status,created_at_ms,acknowledged_at_ms)
            VALUES('effect','terminal-run','request','model',NULL,'retained-corrupt-history',1,'acknowledged',1,2);
          INSERT INTO effect_outbox_payloads VALUES('effect','{}');
          INSERT INTO run_attempts(id,run_id,invocation_id,effect_id,text_part_id,reasoning_part_id,predecessor_id,state,revision,record_json) VALUES('attempt','terminal-run','invocation','effect','p','reasoning',NULL,'succeeded',1,'{}');
          INSERT INTO run_attempt_deliveries VALUES('request','attempt');
          INSERT INTO delegated_invocations VALUES('delegated','terminal-run','delegated-effect','codex','session','delegated-text','delegated-reasoning','succeeded',1,'{}');
          INSERT INTO delegated_invocation_deliveries VALUES('delegated-request','delegated');
          INSERT INTO delegated_permission_deliveries VALUES('permission','terminal-run','delegated','delegated-effect','occurrence','control','control-request','transport_written',1,'{}');
          INSERT INTO attachments VALUES('attachment','m','Document','text/plain','fixture-digest',4,'file:///fixture/document',1);
          INSERT INTO artifacts VALUES('artifact','terminal-run','m','Artifact','text/plain','artifact-digest',4,'{}',1);
          INSERT INTO core_checkpoints VALUES('c',1,1,2);
          INSERT INTO core_checkpoint_bodies VALUES('c','{}');
          INSERT INTO remote_session_mirrors(conversation_id,source_host_id,core_revision,event_cursor,snapshot_json,updated_at_ms) VALUES('c','host',1,1,'{}',2);
          INSERT INTO remote_conversation_projections VALUES('c','host',1,1,1,'{}',2);
          INSERT INTO data_sources(id,name,kind,location,access_policy,validation_contract_json,capabilities_json,privacy,revision,created_at_ms,updated_at_ms) VALUES('source','Source','file','file:///fixture','local','{}','[]','private',1,1,1);
          INSERT INTO data_source_overrides VALUES('c','source',1,1),('excluded','source',1,1);
          INSERT INTO data_source_usage VALUES('usage','source','Source','context','test','c',0,'{}',1);").unwrap();
        store
    }
    fn staged(store: &mut NolaStore) -> CommitArchiveRestore {
        let revision = store.archive_directory_revision().unwrap();
        store
            .begin_archive_restore(&BeginArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: revision,
                created_at_ms: 3,
            })
            .unwrap();
        for (sequence, entry) in [
            profile("c", "New"),
            message("replacement"),
            profile("new", "New conversation"),
        ]
        .iter()
        .enumerate()
        {
            store
                .stage_archive_entry("restore", sequence as u64, entry)
                .unwrap();
        }
        let progress = store.archive_restore_progress("restore").unwrap();
        CommitArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: revision,
            expected_entry_count: progress.next_sequence,
            manifest_digest: progress.manifest_digest,
            committed_at_ms: 4,
        }
    }
    fn finish(
        store: &mut NolaStore,
        input: &CommitArchiveRestore,
    ) -> StoreResult<ArchiveRestoreResult> {
        store.begin_write_unit_of_work()?;
        let result = store.commit_archive_restore(input);
        match result {
            Ok(result) => {
                store.commit_write_unit_of_work()?;
                Ok(result)
            }
            Err(error) => {
                store.rollback_write_unit_of_work()?;
                Err(error)
            }
        }
    }

    #[test]
    fn room_owned_contexts_reject_archive_replacement_and_removal_before_any_history_changes() {
        use nola_domain::agent_room::{AgentRoom, RoomParticipant};
        for protected in ["c", "excluded"] {
            let mut store = fixture(std::path::Path::new(":memory:"));
            let room = AgentRoom::new("room".into(), "Shared".into(), vec![RoomParticipant {
                id: "member".into(), agent_id: "agent".to_string().try_into().unwrap(),
                agent_revision: 1, display_name: "Agent".into(), description: "Research".into(), context_conversation_id: protected.into(), active: false,
            }]).unwrap();
            store.put_product_setting("agent_chat_room/room", &serde_json::to_string(&room).unwrap(), 1).unwrap();
            let before = store.load_conversation("c").unwrap().unwrap();
            let excluded = store.load_conversation("excluded").unwrap().unwrap();
            let revision = store.archive_directory_revision().unwrap();
            let input = staged(&mut store);
            assert!(finish(&mut store, &input).unwrap_err().to_string().contains("room participant context"));
            assert_eq!(store.load_conversation("c").unwrap().unwrap(), before);
            assert_eq!(store.load_conversation("excluded").unwrap().unwrap(), excluded);
            assert!(store.load_conversation("new").unwrap().is_none());
            assert!(store.load_checkpoint("c").unwrap().is_some());
            assert_eq!(store.archive_directory_revision().unwrap(), revision);
            assert!(!store.archive_restore_progress("restore").unwrap().completed);
        }
    }

    #[test]
    fn same_identity_restore_is_atomically_visible_retires_terminal_runtime_and_preserves_semantic_source_fields(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.db");
        let mut store = fixture(&path);
        let before = store.load_conversation("c").unwrap().unwrap();
        let revision = store.archive_directory_revision().unwrap();
        let input = staged(&mut store);
        assert_eq!(store.load_conversation("c").unwrap().unwrap(), before);
        assert_eq!(store.archive_directory_revision().unwrap(), revision);
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        let observer = NolaStore::open(&path).unwrap();
        store.begin_write_unit_of_work().unwrap();
        let result = store.commit_archive_restore(&input).unwrap();
        assert_eq!(observer.load_conversation("c").unwrap().unwrap(), before);
        assert!(observer.load_conversation("new").unwrap().is_none());
        drop(observer);
        store.commit_write_unit_of_work().unwrap();
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        let after = store.load_conversation("c").unwrap().unwrap();
        assert_eq!(after.revision, before.revision + 1);
        assert_eq!(after.messages[0].id, "m");
        assert_eq!(after.messages[0].parts[0].id, "p");
        assert_eq!(after.messages[0].parts[0].content_text, "replacement");
        assert_eq!(after.messages[0].parts[1], before.messages[0].parts[1]);
        assert!(store.load_conversation("excluded").unwrap().is_none());
        assert!(store.load_checkpoint("c").unwrap().is_none());
        let check = store
            .connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .exists([])
            .unwrap();
        assert!(!check);
        for table in [
            "runs",
            "run_events",
            "effect_outbox",
            "run_attempts",
            "run_attempt_deliveries",
            "delegated_invocations",
            "delegated_invocation_deliveries",
            "delegated_permission_deliveries",
            "attachments",
            "artifacts",
            "core_checkpoints",
            "core_checkpoint_bodies",
            "remote_session_mirrors",
            "remote_conversation_projections",
        ] {
            let count: i64 = store
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
        let source_overrides: Vec<String> = store
            .connection
            .prepare("SELECT conversation_id FROM data_source_overrides")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(source_overrides, vec!["c"]);
        assert_eq!(finish(&mut store, &input).unwrap(), result);
        assert_eq!(store.load_conversation("c").unwrap().unwrap(), after);
        let marker:(i64,bool)=store.connection.query_row("SELECT revision,deleted FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id='c'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(marker, (3, false));
        let deleted:bool=store.connection.query_row("SELECT deleted FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id='excluded'",[],|row|row.get(0)).unwrap();
        assert!(deleted);
        assert!(store.connection.query_row("SELECT EXISTS(SELECT 1 FROM sync_outbox WHERE entity_type='conversation' AND entity_id='c' AND deleted=1)",[],|row|row.get::<_,bool>(0)).unwrap()==false);
    }

    #[test]
    fn final_sync_failure_rolls_back_all_histories_directory_runtime_and_keeps_exact_staging_for_retry(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollback.db");
        let mut store = fixture(&path);
        let before = store.load_conversation("c").unwrap().unwrap();
        let revision = store.archive_directory_revision().unwrap();
        let input = staged(&mut store);
        let outbox: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
            .unwrap();
        store.connection.execute_batch("CREATE TRIGGER archive_fail BEFORE INSERT ON sync_outbox WHEN NEW.entity_type='archive_restore_commit' BEGIN SELECT RAISE(ABORT,'injected final archive outbox failure'); END").unwrap();
        assert!(finish(&mut store, &input).is_err());
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        assert_eq!(store.load_conversation("c").unwrap().unwrap(), before);
        assert!(store.load_conversation("excluded").unwrap().is_some());
        assert!(store.load_conversation("new").unwrap().is_none());
        assert_eq!(store.archive_directory_revision().unwrap(), revision);
        assert!(store.load_checkpoint("c").unwrap().is_some());
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            outbox
        );
        assert!(!store.archive_restore_progress("restore").unwrap().completed);
        store
            .connection
            .execute_batch("DROP TRIGGER archive_fail")
            .unwrap();
        assert!(finish(&mut store, &input).is_ok());
    }

    #[test]
    fn invalid_settings_duplicate_identity_changed_replay_and_stale_or_active_directory_fail_without_visibility(
    ) {
        let mut store = fixture(std::path::Path::new(":memory:"));
        let before = store.load_conversation("c").unwrap().unwrap();
        let revision = store.archive_directory_revision().unwrap();
        store
            .begin_archive_restore(&BeginArchiveRestore {
                restore_id: "restore".into(),
                expected_directory_revision: revision,
                created_at_ms: 3,
            })
            .unwrap();
        for value in ["[]", "null", "42"] {
            let mut entry = profile("c", "New");
            if let ArchiveRestoreEntry::Profile { profile } = &mut entry {
                profile.settings_json = Some(value.into());
            }
            assert!(store.stage_archive_entry("restore", 0, &entry).is_err());
        }
        assert_eq!(
            store
                .archive_restore_progress("restore")
                .unwrap()
                .next_sequence,
            0
        );
        assert_eq!(store.load_conversation("c").unwrap().unwrap(), before);
        store
            .stage_archive_entry("restore", 0, &profile("c", "New"))
            .unwrap();
        assert!(store
            .stage_archive_entry("restore", 0, &profile("c", "Changed"))
            .is_err());
        assert!(store
            .stage_archive_entry("restore", 1, &profile("c", "New"))
            .is_err());
        let progress = store
            .stage_archive_entry("restore", 1, &message("replacement"))
            .unwrap();
        let mut input = CommitArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: revision,
            expected_entry_count: progress.next_sequence,
            manifest_digest: progress.manifest_digest,
            committed_at_ms: 4,
        };
        let correct = input.manifest_digest.clone();
        input.manifest_digest = "changed".into();
        assert!(finish(&mut store, &input).is_err());
        input.manifest_digest = correct;
        store
            .connection
            .execute(
                "UPDATE runs SET status='running' WHERE id='terminal-run'",
                [],
            )
            .unwrap();
        assert!(finish(&mut store, &input)
            .unwrap_err()
            .to_string()
            .contains("active Run"));
        store
            .connection
            .execute(
                "UPDATE runs SET status='sealed' WHERE id='terminal-run'",
                [],
            )
            .unwrap();
        store
            .rename_conversation("c", "Concurrent change", None, 5)
            .unwrap();
        assert!(finish(&mut store, &input)
            .unwrap_err()
            .to_string()
            .contains("directory changed"));
        assert_eq!(
            store.load_conversation("c").unwrap().unwrap().messages,
            before.messages
        );
    }
}

#[cfg(test)]
#[path = "archive_derivation_tests.rs"]
mod derivation_tests;
