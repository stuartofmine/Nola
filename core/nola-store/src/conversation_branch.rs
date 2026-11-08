// SPDX-License-Identifier: Apache-2.0
//! Metadata-only branch: canonical copy, lineage and sync facts share one commit.
use super::*;
use nola_domain::{CanonicalBranchIntent, CanonicalBranchResult, CanonicalBodyReference};

pub(super) fn identity(operation: &str, kind: &str, source: &str) -> String {
    let encoded = serde_json::to_vec(&(operation, kind, source)).expect("string tuple encodes");
    format!("branch-{:x}", Sha256::digest(encoded))
}

impl NolaStore {
    pub fn conversation_branch_lineage(
        &self,
        conversation_id: &str,
    ) -> StoreResult<Option<CanonicalBranchIntent>> {
        require_id("branch conversation", conversation_id)?;
        let encoded:Option<String>=self.connection.query_row(
            "SELECT intent_json FROM conversation_branch_operations WHERE json_extract(intent_json,'$.conversation.id')=?1",
            params![conversation_id],|row|row.get(0)).optional()?;
        encoded
            .map(|value| serde_json::from_str(&value).map_err(StoreError::from))
            .transpose()
    }

    pub fn create_conversation_branch(
        &mut self,
        input: &CanonicalBranchIntent,
    ) -> StoreResult<CanonicalBranchResult> {
        for id in [
            &input.operation_id,
            &input.source_conversation_id,
            &input.through_message_id,
            &input.conversation.id,
        ] {
            require_id("branch identity", id)?;
            if id.len() > 128 || id.trim() != id || id.chars().any(char::is_control) {
                return Err(StoreError::Invariant(
                    "branch identity exceeds 128 bytes".into(),
                ));
            }
        }
        if input.conversation.id == input.source_conversation_id
            || input.expected_source_revision < 1
            || input.conversation.title.len() > 4096
            || input
                .conversation
                .settings_json
                .as_ref()
                .is_some_and(|s| s.len() > 32768)
        {
            return Err(StoreError::Invariant(
                "invalid branch metadata or source revision".into(),
            ));
        }
        if let Some(settings) = &input.conversation.settings_json {
            validate_json("branch settings", settings)?;
        }
        let encoded = serde_json::to_string(input)?;
        let tx = self.write_transaction()?;
        let existing: Option<(String,String)> = tx.query_row(
            "SELECT intent_json,result_json FROM conversation_branch_operations WHERE operation_id=?1",
            params![input.operation_id],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((intent, result)) = existing {
            if intent != encoded {
                return Err(StoreError::Conflict(
                    "branch operation replay differs".into(),
                ));
            }
            let mut result: CanonicalBranchResult = serde_json::from_str(&result)?;
            result.created = false;
            return Ok(result);
        }
        let revision: Option<i64> = tx
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                params![input.source_conversation_id],
                |row| row.get(0),
            )
            .optional()?;
        if revision != Some(input.expected_source_revision) {
            return Err(StoreError::Conflict(
                "branch source revision changed".into(),
            ));
        }
        let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1 AND status NOT IN ('sealed'))",
            params![input.source_conversation_id],|row|row.get(0))?;
        if active {
            return Err(StoreError::Conflict(
                "cannot branch a conversation with an active Run".into(),
            ));
        }
        let boundary: Option<i64> = tx.query_row(
            "SELECT m.ordinal FROM messages m WHERE m.conversation_id=?1 AND m.id=?2 AND m.status='completed'
             AND COALESCE((SELECT e.visible FROM message_projection_events e JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
               WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id ORDER BY o.resulting_revision DESC LIMIT 1),1)=1",
            params![input.source_conversation_id,input.through_message_id],|row|row.get(0)).optional()?;
        let boundary = boundary.ok_or_else(|| {
            StoreError::Conflict("branch boundary is not visible completed history".into())
        })?;
        let target_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            params![input.conversation.id],
            |row| row.get(0),
        )?;
        if target_exists {
            return Err(StoreError::Conflict(
                "branch target already exists without this operation".into(),
            ));
        }
        tx.execute(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES(?1,?2,1,?3,?4)",
            params![
                input.conversation.id,
                input.conversation.title,
                input.conversation.created_at_ms,
                input.conversation.updated_at_ms
            ],
        )?;
        if let Some(settings) = &input.conversation.settings_json {
            tx.execute(
                "INSERT INTO conversation_settings VALUES(?1,?2,?3)",
                params![
                    input.conversation.id,
                    settings,
                    input.conversation.updated_at_ms
                ],
            )?;
        }
        publish_conversation_content(&tx, &input.conversation.id, input.committed_at_ms)?;
        let mut messages = tx.prepare(
            "SELECT m.id,m.role,m.status,m.created_at_ms,m.completed_at_ms FROM messages m
             WHERE m.conversation_id=?1 AND m.ordinal<=?2
             AND COALESCE((SELECT e.visible FROM message_projection_events e JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
               WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id ORDER BY o.resulting_revision DESC LIMIT 1),1)=1 ORDER BY m.ordinal")?;
        let mut rows = messages.query(params![input.source_conversation_id, boundary])?;
        let mut count = 0;
        let mut revision = 1;
        while let Some(row) = rows.next()? {
            let source_id: String = row.get(0)?;
            let status: String = row.get(2)?;
            if status != "completed" {
                return Err(StoreError::Conflict(
                    "branch prefix contains unfinished history".into(),
                ));
            }
            let message_id = identity(&input.operation_id, "message", &source_id);
            let (parts, references) = copy_parts(&tx, input, &source_id, &message_id)?;
            let message = MessageInput {
                id: message_id,
                run_id: None,
                role: row.get(1)?,
                status,
                ordinal: count as i64 + 1,
                created_at_ms: row.get(3)?,
                completed_at_ms: row.get(4)?,
                parts,
            };
            let append = AppendMessagesInput {
                conversation_id: input.conversation.id.clone(),
                expected_message_count: count,
                messages: vec![message],
                updated_at_ms: input.committed_at_ms,
            };
            // One bounded message manifest per replica fact; text is chunked above.
            if serde_json::to_vec(&append)?.len() > 256 * 1024 {
                return Err(StoreError::Invariant(
                    "branch message metadata exceeds bounded manifest".into(),
                ));
            }
            let result = conversation_append_store::append_in_transaction(
                &tx,
                &append,
                Some(revision),
                &references,
            )?;
            crate::part_derivation::record_branch_message(
                &tx,
                &input.operation_id,
                &source_id,
                &append.messages[0].id,
            )?;
            crate::tool_source::copy_branch_sources(&tx,input,&append.messages[0].id,result.conversation_revision)?;
            crate::part_derivation_sync::enqueue_message(
                &tx,
                &input.operation_id,
                &append.messages[0].id,
                input.committed_at_ms,
            )?;
            count = result.message_count;
            revision = result.conversation_revision;
        }
        drop(rows);
        drop(messages);
        let result = CanonicalBranchResult {
            created: true,
            message_count: count,
            conversation_revision: revision,
            conversation_id: input.conversation.id.clone(),
            source_conversation_id: input.source_conversation_id.clone(),
            through_message_id: input.through_message_id.clone(),
        };
        tx.execute(
            "INSERT INTO conversation_branch_operations VALUES(?1,?2,?3)",
            params![input.operation_id, encoded, serde_json::to_string(&result)?],
        )?;
        let lineage_payload = serde_json::to_string(&BranchLineage {
            intent: input.clone(),
            result: result.clone(),
            derivation_count: tx.query_row(
                "SELECT COUNT(*) FROM message_part_derivations WHERE operation_id=?1",
                params![input.operation_id],
                |row| row.get(0),
            )?,
        })?;
        enqueue_sync_change(
            &tx,
            "conversation_branch_lineage",
            &input.operation_id,
            1,
            false,
            &lineage_payload,
            input.committed_at_ms,
        )?;
        crate::part_derivation_sync::remember_local(
            &tx,
            "conversation_branch_lineage",
            &input.operation_id,
            &lineage_payload,
        )?;
        tx.commit()?;
        Ok(result)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BranchLineage {
    pub(super) intent: CanonicalBranchIntent,
    pub(super) result: CanonicalBranchResult,
    pub(super) derivation_count: u64,
}

pub(super) fn apply_lineage(
    connection: &Connection,
    change: &RemoteSyncChangeInput,
) -> StoreResult<()> {
    if change.deleted || change.revision != 1 || change.payload_json.len() > 64 * 1024 {
        return Err(StoreError::Invariant("invalid branch lineage fact".into()));
    }
    if crate::part_derivation_sync::accepted_replay(connection, change)? {
        return Ok(());
    }
    let lineage: BranchLineage = serde_json::from_str(&change.payload_json)?;
    let input = &lineage.intent;
    let result = &lineage.result;
    let declared_count = i64::try_from(result.message_count)
        .map_err(|_| StoreError::Invariant("branch lineage count overflow".into()))?;
    let expected_revision = declared_count
        .checked_add(1)
        .ok_or_else(|| StoreError::Invariant("branch lineage revision overflow".into()))?;
    if input.operation_id != change.entity_id
        || input.source_conversation_id == input.conversation.id
        || input.expected_source_revision < 1
        || !result.created
        || result.message_count == 0
        || result.conversation_revision != expected_revision
        || result.conversation_id != input.conversation.id
        || result.source_conversation_id != input.source_conversation_id
        || result.through_message_id != input.through_message_id
    {
        return Err(StoreError::Conflict(
            "branch lineage owner or boundary mismatch".into(),
        ));
    }
    for id in [
        &input.operation_id,
        &input.source_conversation_id,
        &input.through_message_id,
        &input.conversation.id,
    ] {
        if id.is_empty() || id.len() > 128 || id.trim() != id || id.chars().any(char::is_control) {
            return Err(StoreError::Invariant(
                "invalid branch lineage identity".into(),
            ));
        }
    }
    if crate::part_derivation_sync::target_is_tombstoned(connection, &input.conversation.id)? {
        crate::part_derivation_sync::remember_remote(connection, change)?;
        return Ok(());
    }
    let target:Option<(i64,i64)>=connection.query_row("SELECT c.revision,COUNT(m.id) FROM conversations c LEFT JOIN messages m ON m.conversation_id=c.id WHERE c.id=?1 GROUP BY c.id",params![input.conversation.id],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    if target.is_none_or(|(revision, count)| {
        revision < result.conversation_revision || count < declared_count
    }) {
        return Err(StoreError::Conflict(
            "branch lineage awaits canonical target history".into(),
        ));
    }
    crate::part_derivation_sync::verify_lineage_coverage(connection, &lineage)?;
    let intent = serde_json::to_string(input)?;
    let result = serde_json::to_string(result)?;
    let existing:Option<(String,String)>=connection.query_row("SELECT intent_json,result_json FROM conversation_branch_operations WHERE operation_id=?1",params![input.operation_id],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    if let Some(existing) = existing {
        if existing != (intent, result) {
            return Err(StoreError::Conflict("branch lineage replay differs".into()));
        }
    } else {
        let other:bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM conversation_branch_operations WHERE json_extract(intent_json,'$.conversation.id')=?1)",params![input.conversation.id],|row|row.get(0))?;
        if other {
            return Err(StoreError::Conflict(
                "branch target already has different lineage".into(),
            ));
        }
        connection.execute(
            "INSERT INTO conversation_branch_operations VALUES(?1,?2,?3)",
            params![input.operation_id, intent, result],
        )?;
    }
    crate::part_derivation_sync::remember_remote(connection, change)?;
    Ok(())
}

fn copy_parts(
    connection: &Connection,
    input: &CanonicalBranchIntent,
    source_message: &str,
    target_message: &str,
) -> StoreResult<(Vec<MessagePartInput>, Vec<CanonicalBodyReference>)> {
    let mut statement = connection.prepare(
        "SELECT id,kind,state,created_at_ms,updated_at_ms
         FROM message_parts WHERE message_id=?1 ORDER BY ordinal LIMIT 513",
    )?;
    let metadata = statement
        .query_map(params![source_message], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if metadata.len() > 512 {
        return Err(StoreError::Invariant(
            "branch message has too many Parts".into(),
        ));
    }
    let mut parts = Vec::new();
    let mut references = Vec::new();
    let mut inline_bytes = 0_u64;
    for (source_id, kind, state, created_at_ms, updated_at_ms) in metadata {
        if state != "completed" {
            return Err(StoreError::Conflict(
                "branch prefix contains unfinished Part".into(),
            ));
        }
        let part_id = identity(&input.operation_id, "part", &source_id);
        let content_digest: Option<String> = connection.query_row(
            "SELECT CASE WHEN length(CAST(content_digest AS BLOB))<=4096 THEN content_digest END,
             COALESCE(length(CAST(content_digest AS BLOB)),0) FROM message_parts WHERE id=?1",
            params![source_id],
            |row| {
                if row.get::<_, i64>(1)? > 4096 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                row.get(0)
            },
        )?;
        inline_bytes = inline_bytes.saturating_add(
            content_digest
                .as_ref()
                .map_or(0, |value| value.len() as u64),
        );
        let mut content_text = String::new();
        let mut content_json = None;
        for field in [
            nola_domain::PartBodyField::ContentText,
            nola_domain::PartBodyField::ContentJson,
        ] {
            let reader = crate::canonical_field_reader::CanonicalFieldReader::open(
                connection,
                &input.source_conversation_id,
                source_message,
                &source_id,
                field,
            )?;
            let Some(bytes) = reader.total_utf8_bytes() else {
                continue;
            };
            if let crate::canonical_field_reader::CanonicalFieldBacking::Chunked(head) =
                reader.backing()
            {
                if !head.closed {
                    return Err(StoreError::Conflict(
                        "branch prefix contains an open stream".into(),
                    ));
                }
                // Even a tiny stream copied inline must prove the complete
                // source layout/chain before creating an independent target.
                crate::canonical_stream_read::require_complete(connection, head)?;
            }
            if reader.sealed_reference().is_none() && bytes <= 16384 {
                inline_bytes = inline_bytes.saturating_add(bytes);
                if inline_bytes > 256 * 1024 {
                    return Err(StoreError::Invariant(
                        "branch message exceeds bounded inline manifest budget".into(),
                    ));
                }
                let text = if bytes == 0 {
                    String::new()
                } else {
                    let range = reader.read_range(0, 16384)?;
                    if range.next_offset_bytes != bytes || !range.end_of_content {
                        return Err(StoreError::Invariant(
                            "branch inline field is incomplete".into(),
                        ));
                    }
                    range.text
                };
                match field {
                    nola_domain::PartBodyField::ContentText => content_text = text,
                    nola_domain::PartBodyField::ContentJson => content_json = Some(text),
                }
            } else {
                references.push(copy_field(
                    connection,
                    input,
                    &source_id,
                    target_message,
                    &part_id,
                    field,
                    &reader,
                    bytes,
                )?);
            }
        }
        if inline_bytes > 256 * 1024 {
            return Err(StoreError::Invariant(
                "branch Part metadata exceeds manifest budget".into(),
            ));
        }
        parts.push(MessagePartInput {
            id: part_id,
            kind,
            ordinal: parts.len() as i64 + 1,
            state,
            content_text,
            content_json,
            content_digest,
            created_at_ms,
            updated_at_ms,
        });
    }
    Ok((parts, references))
}

/// Copy original bytes into a new owner/field object, never a source reference.
/// Both passes are bounded and share the outer branch write transaction.
fn copy_field(
    connection: &Connection,
    input: &CanonicalBranchIntent,
    source_part: &str,
    target_message: &str,
    target_part: &str,
    field: nola_domain::PartBodyField,
    reader: &crate::canonical_field_reader::CanonicalFieldReader<'_>,
    bytes: u64,
) -> StoreResult<CanonicalBodyReference> {
    if let Some(source) = reader.sealed_reference() {
        // Range reads validate only the visited span. A new publication must
        // also prove the source's exact declared count and absence of tail data.
        canonical_text_replica::require_complete(connection, source)?;
    }
    if bytes > 256 * 1024 * 1024 {
        return Err(StoreError::Invariant(
            "branch field exceeds content quota".into(),
        ));
    }
    let mut offset = 0;
    let mut count = 0_u32;
    while offset < bytes {
        let range = reader.read_range(offset, 32768)?;
        if range.next_offset_bytes <= offset || range.next_offset_bytes > bytes {
            return Err(StoreError::Invariant(
                "branch field copy made no progress".into(),
            ));
        }
        offset = range.next_offset_bytes;
        count += 1;
    }
    let kind = match field {
        nola_domain::PartBodyField::ContentText => "body",
        nola_domain::PartBodyField::ContentJson => "body-json",
    };
    let reference = CanonicalBodyReference::for_field(
        identity(&input.operation_id, kind, source_part),
        input.conversation.id.clone(),
        target_message.into(),
        target_part.into(),
        field,
        bytes,
        count,
    )
    .map_err(|error| StoreError::Invariant(error.to_string()))?;
    let mut offset = 0;
    let chunks = std::iter::from_fn(|| {
        if offset >= bytes {
            return None;
        }
        let start = offset;
        match reader.read_range(start, 32768) {
            Ok(range) if range.next_offset_bytes > start && range.next_offset_bytes <= bytes => {
                offset = range.next_offset_bytes;
                Some(Ok((start, range.text)))
            }
            Ok(_) => {
                offset = bytes;
                Some(Err(StoreError::Invariant(
                    "branch field copy made no progress".into(),
                )))
            }
            Err(error) => {
                offset = bytes;
                Some(Err(error))
            }
        }
    });
    // JSON objects are syntax-validated by the same production put_chunks path.
    canonical_text_store::put_chunks(connection, &reference, chunks, 256 * 1024 * 1024)?;
    Ok(reference)
}
#[cfg(test)]
#[path = "conversation_branch_field_tests.rs"]
mod field_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (NolaStore, CanonicalBranchIntent) {
        let mut store = NolaStore::open(":memory:").unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "source".into(),
                title: "Source".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        store
            .append_messages(&AppendMessagesInput {
                conversation_id: "source".into(),
                expected_message_count: 0,
                updated_at_ms: 2,
                messages: vec![MessageInput {
                    id: "m".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(2),
                    parts: vec![MessagePartInput {
                        id: "p".into(),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: "中😀".repeat(10_000),
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    }],
                }],
            })
            .unwrap();
        let input = CanonicalBranchIntent {
            operation_id: "operation".into(),
            source_conversation_id: "source".into(),
            expected_source_revision: 2,
            through_message_id: "m".into(),
            conversation: nola_domain::CanonicalBranchTarget {
                id: "target".into(),
                title: "Branch".into(),
                settings_json: None,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            committed_at_ms: 3,
        };
        (store, input)
    }

    #[test]
    fn final_lineage_failure_rolls_back_copied_content_messages_and_sync_then_identical_retry_succeeds(
    ) {
        let (mut store, input) = fixture();
        let count = || {
            store
                .connection
                .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        };
        let before = count();
        store.connection.execute_batch("CREATE TRIGGER fail_branch_lineage BEFORE INSERT ON conversation_branch_operations BEGIN SELECT RAISE(ABORT,'injected lineage failure'); END").unwrap();
        assert!(store.create_conversation_branch(&input).is_err());
        assert!(store.load_conversation("target").unwrap().is_none());
        let objects: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM canonical_text_objects", [], |row| {
                row.get(0)
            })
            .unwrap();
        let after: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
            .unwrap();
        assert_eq!(objects, 0);
        assert_eq!(after, before);
        assert_eq!(
            store.load_conversation("source").unwrap().unwrap().revision,
            2
        );
        store
            .connection
            .execute_batch("DROP TRIGGER fail_branch_lineage")
            .unwrap();
        let result = store.create_conversation_branch(&input).unwrap();
        assert!(result.created);
        let source = store.load_conversation("source").unwrap().unwrap();
        let target = store.load_conversation("target").unwrap().unwrap();
        assert_eq!(
            source.messages[0].parts[0].content_text,
            target.messages[0].parts[0].content_text
        );
        assert!(store.load_checkpoint("target").unwrap().is_none());
    }

    #[test]
    fn unfinished_part_and_active_source_run_refuse_branch_without_target_visibility() {
        let (mut store, input) = fixture();
        store
            .connection
            .execute(
                "UPDATE message_parts SET state='streaming' WHERE id='p'",
                [],
            )
            .unwrap();
        assert!(store
            .create_conversation_branch(&input)
            .unwrap_err()
            .to_string()
            .contains("unfinished Part"));
        assert!(store.load_conversation("target").unwrap().is_none());
        store
            .connection
            .execute(
                "UPDATE message_parts SET state='completed' WHERE id='p'",
                [],
            )
            .unwrap();
        store.connection.execute("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES('run','source','conversation','running',1,1,1)",[]).unwrap();
        assert!(store
            .create_conversation_branch(&input)
            .unwrap_err()
            .to_string()
            .contains("active Run"));
        assert!(store.load_conversation("target").unwrap().is_none());
    }
    #[test]
    fn bounded_structured_payload_survives_branch_without_losing_its_json() {
        let (mut store, input) = fixture();
        let structured =
            serde_json::json!({"type":"tool_result","output":"x".repeat(20_000)}).to_string();
        store.connection.execute("UPDATE message_parts SET kind='tool_result',content_text='tool output',content_json=?1 WHERE id='p'",params![structured]).unwrap();
        store.create_conversation_branch(&input).unwrap();
        let target = store.load_conversation("target").unwrap().unwrap();
        assert_eq!(
            target.messages[0].parts[0].content_json.as_deref(),
            Some(structured.as_str())
        );
        assert_eq!(target.messages[0].parts[0].content_text, "tool output");
    }
    #[test]
    fn inline_and_referenced_branch_generations_preserve_part_metadata() {
        let (mut store, input) = fixture();
        store.connection.execute("UPDATE message_parts SET content_digest='original',created_at_ms=11,updated_at_ms=22 WHERE id='p'", []).unwrap();
        store.create_conversation_branch(&input).unwrap();
        let first = store.load_conversation("target").unwrap().unwrap();
        let part = &first.messages[0].parts[0];
        assert_eq!(part.content_digest.as_deref(), Some("original"));
        assert_eq!((part.created_at_ms, part.updated_at_ms), (11, 22));
        let mut next = input;
        next.operation_id = "next".into();
        next.source_conversation_id = "target".into();
        next.expected_source_revision = first.revision;
        next.through_message_id = first.messages[0].id.clone();
        next.conversation.id = "third".into();
        store.create_conversation_branch(&next).unwrap();
        let second = store.load_conversation("third").unwrap().unwrap();
        let part = &second.messages[0].parts[0];
        assert_eq!(part.content_digest.as_deref(), Some("original"));
        assert_eq!((part.created_at_ms, part.updated_at_ms), (11, 22));
        assert_eq!(part.content_text, first.messages[0].parts[0].content_text);
    }

    #[test]
    fn oversized_digest_rejects_branch_without_publishing_a_target() {
        let (mut store, input) = fixture();
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_digest=?1 WHERE id='p'",
                params!["x".repeat(4097)],
            )
            .unwrap();
        assert!(store.create_conversation_branch(&input).is_err());
        assert!(store.load_conversation("target").unwrap().is_none());
        let objects: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM canonical_text_objects", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(objects, 0);
    }
}
