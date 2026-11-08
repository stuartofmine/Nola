// SPDX-License-Identifier: Apache-2.0
//! Canonical transcript append transaction: ordinal admission, Part insertion,
//! conversation revision and sync publication commit together. Application owns
//! finalized-message and idle-Run policy checks before invoking this repository.
use super::{
    conversation_append_sync, params, require_id, validate_json, AppendMessagesInput,
    AppendMessagesResult, NolaStore, OptionalExtension, StoreError, StoreResult,
};

#[cfg(test)]
mod field_publication_tests {
    use super::*;
    use crate::{
        ApplyRemoteSyncPageInput, CreateConversationInput, MessageInput, MessagePartInput,
        RemoteSyncChangeInput, StoredSyncChange,
    };
    use nola_domain::{CanonicalBodyReference, PartBodyField};

    fn create(store: &mut NolaStore) {
        store
            .create_conversation(&CreateConversationInput {
                id: "c".into(),
                title: "Fields".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
    }
    fn input() -> AppendMessagesInput {
        AppendMessagesInput {
            conversation_id: "c".into(),
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
                    kind: "tool_result".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: String::new(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            }],
        }
    }
    fn prepare(
        connection: &super::super::Connection,
        field: PartBodyField,
        raw: &str,
        owner: &str,
        missing: bool,
    ) -> CanonicalBodyReference {
        let id = if field == PartBodyField::ContentText {
            "text-body"
        } else {
            "json-body"
        };
        let reference = CanonicalBodyReference::for_field(
            id.into(),
            owner.into(),
            "m".into(),
            "p".into(),
            field,
            raw.len() as u64,
            1,
        )
        .unwrap();
        connection
            .execute(
                "INSERT INTO canonical_text_objects VALUES(?1,?2)",
                params![id, serde_json::to_string(&reference).unwrap()],
            )
            .unwrap();
        if !missing {
            connection
                .execute(
                    "INSERT INTO canonical_text_chunks VALUES(?1,0,?2)",
                    params![id, raw],
                )
                .unwrap();
        }
        reference
    }
    fn page(entries: &[StoredSyncChange], cursor: i64) -> ApplyRemoteSyncPageInput {
        ApplyRemoteSyncPageInput {
            replica_id: "fixture".into(),
            next_cursor: cursor + entries.len() as i64,
            applied_at_ms: 3,
            changes: entries
                .iter()
                .enumerate()
                .map(|(index, entry)| RemoteSyncChangeInput {
                    cursor: cursor + index as i64 + 1,
                    operation_id: entry.operation_id.clone(),
                    entity_type: entry.entity_type.clone(),
                    entity_id: entry.entity_id.clone(),
                    revision: entry.revision,
                    deleted: entry.deleted,
                    payload_json: entry.payload_json.clone(),
                    created_at_ms: entry.created_at_ms,
                })
                .collect(),
        }
    }
    fn assert_fields(store: &NolaStore) {
        use crate::canonical_field_reader::{CanonicalFieldReader, CanonicalJsonRead};
        let text = CanonicalFieldReader::open(
            &store.connection,
            "c",
            "m",
            "p",
            PartBodyField::ContentText,
        )
        .unwrap();
        assert_eq!(text.read_text(0).unwrap(), "original 文本");
        drop(text);
        let json = CanonicalFieldReader::open(
            &store.connection,
            "c",
            "m",
            "p",
            PartBodyField::ContentJson,
        )
        .unwrap();
        assert_eq!(
            json.read_json(None).unwrap(),
            CanonicalJsonRead::Complete(" {\"seed\":9007199254740993} ".into())
        );
    }

    #[test]
    fn dual_field_append_replicates_and_reopens_without_cross_field_aliasing() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = NolaStore::open(dir.path().join("source")).unwrap();
        create(&mut source);
        {
            let tx = source.connection.transaction().unwrap();
            let refs = [
                prepare(&tx, PartBodyField::ContentText, "original 文本", "c", false),
                prepare(
                    &tx,
                    PartBodyField::ContentJson,
                    " {\"seed\":9007199254740993} ",
                    "c",
                    false,
                ),
            ];
            append_in_transaction(&tx, &input(), Some(1), &refs).unwrap();
            tx.commit().unwrap();
        }
        assert_fields(&source);
        let destination_path = dir.path().join("destination");
        let mut destination = NolaStore::open(&destination_path).unwrap();
        let mut cursor = 0;
        loop {
            let entries = source.pending_sync_changes(100).unwrap();
            if entries.is_empty() {
                break;
            }
            let request = page(&entries, cursor);
            cursor = destination.apply_remote_sync_page(&request).unwrap().cursor;
            for entry in &entries {
                source
                    .resolve_sync_change(&entry.operation_id, "acknowledged", Some(cursor), 3)
                    .unwrap();
            }
        }
        assert_fields(&destination);
        drop(destination);
        assert_fields(&NolaStore::open(&destination_path).unwrap());
    }

    #[test]
    fn second_field_missing_invalid_or_foreign_rolls_back_entire_append() {
        let dir = tempfile::tempdir().unwrap();
        for mode in ["missing", "invalid-json", "foreign"] {
            let mut store = NolaStore::open(dir.path().join(mode)).unwrap();
            create(&mut store);
            let pending_before = store.pending_sync_changes(100).unwrap();
            {
                let tx = store.connection.transaction().unwrap();
                let refs = [
                    prepare(&tx, PartBodyField::ContentText, "original 文本", "c", false),
                    prepare(
                        &tx,
                        PartBodyField::ContentJson,
                        if mode == "invalid-json" {
                            "{broken"
                        } else {
                            "{}"
                        },
                        if mode == "foreign" { "other" } else { "c" },
                        mode == "missing",
                    ),
                ];
                assert!(append_in_transaction(&tx, &input(), Some(1), &refs).is_err());
            }
            for table in [
                "messages",
                "message_parts",
                "message_part_body_references",
                "canonical_text_objects",
                "canonical_text_chunks",
            ] {
                let count: i64 = store
                    .connection
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })
                    .unwrap();
                assert_eq!(count, 0, "{mode}: {table}");
            }
            assert_eq!(store.conversation_revision("c").unwrap(), Some(1));
            let pending_after = store.pending_sync_changes(100).unwrap();
            assert_eq!(
                pending_before
                    .iter()
                    .map(|entry| &entry.operation_id)
                    .collect::<Vec<_>>(),
                pending_after
                    .iter()
                    .map(|entry| &entry.operation_id)
                    .collect::<Vec<_>>()
            );
        }
    }
}

impl NolaStore {
    pub fn append_messages(
        &mut self,
        input: &AppendMessagesInput,
    ) -> StoreResult<AppendMessagesResult> {
        let transaction = self.write_transaction()?;
        let result = append_in_transaction(&transaction, input, None, &[])?;
        transaction.commit()?;
        Ok(result)
    }
}

pub(super) fn append_in_transaction(
    transaction: &super::Connection,
    input: &AppendMessagesInput,
    expected_revision: Option<i64>,
    references: &[nola_domain::CanonicalBodyReference],
) -> StoreResult<AppendMessagesResult> {
    crate::body_binding::validate_field_bindings(
        &input.conversation_id,
        input.messages.iter().flat_map(|message| {
            message
                .parts
                .iter()
                .map(move |part| (message.id.as_str(), message.status.as_str(), part))
        }),
        references,
    )?;
    require_id("append.conversation_id", &input.conversation_id)?;
    input
        .expected_message_count
        .checked_add(input.messages.len())
        .and_then(|count| i64::try_from(count).ok())
        .ok_or_else(|| {
            StoreError::Invariant("append message count exceeds canonical ordinal range".into())
        })?;
    // Recheck under the writer transaction. Upload preparation or another
    // caller can separate Application admission from the eventual commit.
    let active: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1
             AND status NOT IN ('sealed'))",
        params![input.conversation_id],
        |row| row.get(0),
    )?;
    if active {
        return Err(StoreError::Conflict(
            "cannot append imported history while its Run is active".into(),
        ));
    }
    let mut checked_runs = std::collections::HashSet::new();
    for run_id in input
        .messages
        .iter()
        .filter_map(|message| message.run_id.as_deref())
    {
        if !checked_runs.insert(run_id) {
            continue;
        }
        let foreign: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND conversation_id<>?2)",
            params![run_id, input.conversation_id],
            |row| row.get(0),
        )?;
        if foreign {
            return Err(StoreError::Conflict(
                "imported transcript Run belongs to another conversation".into(),
            ));
        }
    }
    let (existing_count, current_revision): (i64, i64) = transaction
        .query_row(
            "SELECT COUNT(m.id), c.revision
                 FROM conversations c
                 LEFT JOIN messages m ON m.conversation_id = c.id
                 WHERE c.id = ?1
                 GROUP BY c.id",
            params![input.conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::Invariant(format!("unknown conversation {}", input.conversation_id))
        })?;
    if expected_revision.is_some_and(|expected| expected != current_revision) {
        return Err(StoreError::Conflict(
            "conversation revision changed before transcript publication".into(),
        ));
    }
    if existing_count.max(0) as usize != input.expected_message_count {
        return Err(StoreError::Invariant(format!(
            "message count conflict: expected {}, found {}",
            input.expected_message_count,
            existing_count.max(0)
        )));
    }

    for (offset, message) in input.messages.iter().enumerate() {
        require_id("append.message.id", &message.id)?;
        let expected_ordinal = input.expected_message_count as i64 + offset as i64 + 1;
        if message.ordinal != expected_ordinal {
            return Err(StoreError::Invariant(format!(
                "message ordinal conflict: expected {expected_ordinal}, found {}",
                message.ordinal
            )));
        }
        transaction.execute(
            "INSERT INTO messages(
                    id, conversation_id, run_id, role, status, ordinal,
                    created_at_ms, completed_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                message.id,
                input.conversation_id,
                message.run_id,
                message.role,
                message.status,
                message.ordinal,
                message.created_at_ms,
                message.completed_at_ms
            ],
        )?;
        for (part_offset, part) in message.parts.iter().enumerate() {
            require_id("append.part.id", &part.id)?;
            if part.ordinal != part_offset as i64 + 1 {
                return Err(StoreError::Invariant(format!(
                    "part ordinal conflict: expected {}, found {}",
                    part_offset + 1,
                    part.ordinal
                )));
            }
            if let Some(content_json) = &part.content_json {
                validate_json("append.part.content_json", content_json)?;
            }
            transaction.execute(
                "INSERT INTO message_parts(
                        id, message_id, kind, ordinal, state, content_text,
                        content_json, content_digest, created_at_ms, updated_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    part.id,
                    message.id,
                    part.kind,
                    part.ordinal,
                    part.state,
                    part.content_text,
                    part.content_json,
                    part.content_digest,
                    part.created_at_ms,
                    part.updated_at_ms
                ],
            )?;
        }
    }

    for reference in references {
        super::canonical_text_replica::require_complete(transaction, reference)?;
        transaction.execute(
                "INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES(?1,?2,?3)",
                params![reference.part_id(), match reference.field() {
                    nola_domain::PartBodyField::ContentText => "content_text",
                    nola_domain::PartBodyField::ContentJson => "content_json",
                }, serde_json::to_string(reference)?],
            )?;
        super::canonical_text_replica::enqueue(transaction, reference, input.updated_at_ms)?;
    }
    let next_revision = current_revision
        .checked_add(1)
        .ok_or_else(|| StoreError::Invariant("conversation revision overflow".into()))?;
    transaction.execute(
        "UPDATE conversations
             SET revision = ?2, updated_at_ms = ?3
             WHERE id = ?1",
        params![input.conversation_id, next_revision, input.updated_at_ms],
    )?;
    if references.is_empty() {
        conversation_append_sync::enqueue(transaction, input, next_revision)?;
    } else {
        conversation_append_sync::enqueue_with_references(
            transaction,
            input,
            next_revision,
            references,
        )?;
    }
    Ok(AppendMessagesResult {
        appended_message_count: input.messages.len(),
        message_count: input.expected_message_count + input.messages.len(),
        conversation_revision: next_revision,
    })
}
