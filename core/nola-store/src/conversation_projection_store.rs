// SPDX-License-Identifier: Apache-2.0
//! Idempotent conversation and checkpoint projection commit transactions.

use super::*;

impl NolaStore {
    pub fn apply_conversation_projection(
        &mut self,
        input: &ApplyConversationProjectionInput,
    ) -> StoreResult<ApplyConversationProjectionResult> {
        require_id("projection.operation_id", &input.operation_id)?;
        require_id("projection.conversation_id", &input.conversation_id)?;
        let transaction = self.write_transaction()?;
        if let Some((revision, count)) = transaction
            .query_row(
                "SELECT resulting_revision, ledger_message_count
                 FROM conversation_projection_operations WHERE operation_id=?1",
                params![input.operation_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
        {
            return Ok(ApplyConversationProjectionResult {
                applied: false,
                conversation_revision: revision,
                ledger_message_count: count.max(0) as usize,
            });
        }
        let (current_revision, ledger_count): (i64, i64) = transaction
            .query_row(
                "SELECT c.revision, COUNT(m.id) FROM conversations c
                 LEFT JOIN messages m ON m.conversation_id=c.id
                 WHERE c.id=?1 GROUP BY c.id",
                params![input.conversation_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::Invariant(format!("unknown conversation {}", input.conversation_id))
            })?;
        if current_revision != input.expected_revision {
            return Err(StoreError::Conflict(format!(
                "conversation revision mismatch: expected {}, found {current_revision}",
                input.expected_revision
            )));
        }
        // Admission is rechecked under the same write transaction as the CAS.
        if matches!(input.operation_kind.as_str(), "edit" | "delete") {
            let active: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1 AND status NOT IN ('sealed'))",
                params![input.conversation_id], |row| row.get(0),
            )?;
            if active {
                return Err(StoreError::Invariant(
                    "cannot edit conversation history while its Run is active".into(),
                ));
            }
            for run_id in input
                .appended_messages
                .iter()
                .filter_map(|message| message.run_id.as_deref())
            {
                let foreign: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND conversation_id<>?2)",
                    params![run_id, input.conversation_id],
                    |row| row.get(0),
                )?;
                if foreign {
                    return Err(StoreError::Invariant(
                        "replacement Run belongs to another conversation".into(),
                    ));
                }
            }
        }
        for mutation in &input.mutations {
            require_id("projection.event_id", &mutation.event_id)?;
            require_id("projection.message_id", &mutation.message_id)?;
            let belongs: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND conversation_id=?2)",
                params![mutation.message_id, input.conversation_id],
                |row| row.get(0),
            )?;
            if !belongs {
                return Err(StoreError::Invariant(format!(
                    "message {} does not belong to conversation {}",
                    mutation.message_id, input.conversation_id
                )));
            }
        }
        for (offset, message) in input.appended_messages.iter().enumerate() {
            require_id("projection.appended_message.id", &message.id)?;
            let expected_ordinal = ledger_count + offset as i64 + 1;
            if message.ordinal != expected_ordinal {
                return Err(StoreError::Invariant(format!(
                    "message ordinal conflict: expected {expected_ordinal}, found {}",
                    message.ordinal
                )));
            }
            transaction.execute(
                "INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms,completed_at_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![message.id,input.conversation_id,message.run_id,message.role,message.status,
                    message.ordinal,message.created_at_ms,message.completed_at_ms],
            )?;
            for (part_offset, part) in message.parts.iter().enumerate() {
                require_id("projection.appended_part.id", &part.id)?;
                if part.ordinal != part_offset as i64 + 1 {
                    return Err(StoreError::Invariant(format!(
                        "part ordinal conflict: expected {}, found {}",
                        part_offset + 1,
                        part.ordinal
                    )));
                }
                if let Some(content_json) = &part.content_json {
                    validate_json("projection.appended_part.content_json", content_json)?;
                }
                transaction.execute(
                    "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,
                     content_json,content_digest,created_at_ms,updated_at_ms)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
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
        let next_revision = current_revision + 1;
        let next_count = ledger_count.max(0) as usize + input.appended_messages.len();
        transaction.execute(
            "INSERT INTO conversation_projection_operations(
                operation_id,conversation_id,operation_kind,parent_message_id,
                resulting_revision,ledger_message_count,created_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                input.operation_id,
                input.conversation_id,
                input.operation_kind,
                input.parent_message_id,
                next_revision,
                next_count as i64,
                input.created_at_ms
            ],
        )?;
        for mutation in &input.mutations {
            transaction.execute(
                "INSERT INTO message_projection_events(
                    event_id,operation_id,conversation_id,message_id,visible,reason,created_at_ms
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    mutation.event_id,
                    input.operation_id,
                    input.conversation_id,
                    mutation.message_id,
                    i64::from(mutation.visible),
                    mutation.reason,
                    input.created_at_ms
                ],
            )?;
        }
        transaction.execute(
            "UPDATE conversations SET revision=?2,updated_at_ms=?3 WHERE id=?1",
            params![input.conversation_id, next_revision, input.created_at_ms],
        )?;
        transaction.commit()?;
        Ok(ApplyConversationProjectionResult {
            applied: true,
            conversation_revision: next_revision,
            ledger_message_count: next_count,
        })
    }

    pub fn commit_checkpoint_projection(
        &mut self,
        input: &CommitCheckpointProjectionInput,
    ) -> StoreResult<CommitCheckpointProjectionResult> {
        let checkpoint = &input.checkpoint;
        let projection = &input.projection;
        require_id("checkpoint.conversation_id", &checkpoint.conversation_id)?;
        require_id("projection.operation_id", &projection.operation_id)?;
        if checkpoint.conversation_id != projection.conversation_id {
            return Err(StoreError::Invariant(
                "checkpoint and projection conversation ids differ".to_string(),
            ));
        }
        if !projection.appended_messages.is_empty() {
            return Err(StoreError::Invariant(
                "atomic idle projection cannot append messages".to_string(),
            ));
        }
        validate_json("checkpoint.checkpoint_json", &checkpoint.checkpoint_json)?;
        if checkpoint.revision <= checkpoint.expected_revision {
            return Err(StoreError::Invariant(
                "checkpoint revision must advance".to_string(),
            ));
        }
        let transaction = self.write_transaction()?;
        if let Some((revision, count)) = transaction
            .query_row(
                "SELECT resulting_revision,ledger_message_count
                 FROM conversation_projection_operations WHERE operation_id=?1",
                params![projection.operation_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
        {
            let stored = transaction.query_row(
                "SELECT revision,event_cursor,checkpoint_json,updated_at_ms
                 FROM core_checkpoints LEFT JOIN core_checkpoint_bodies USING(conversation_id) WHERE conversation_id=?1",
                params![checkpoint.conversation_id],
                |row| {
                    Ok(StoredCheckpoint {
                        conversation_id: checkpoint.conversation_id.clone(),
                        revision: row.get(0)?,
                        event_cursor: row.get(1)?,
                        checkpoint_json: row.get(2)?,
                        updated_at_ms: row.get(3)?,
                    })
                },
            )?;
            return Ok(CommitCheckpointProjectionResult {
                checkpoint: stored,
                projection: ApplyConversationProjectionResult {
                    applied: false,
                    conversation_revision: revision,
                    ledger_message_count: count.max(0) as usize,
                },
            });
        }
        let stored_checkpoint_revision = transaction
            .query_row(
                "SELECT revision FROM core_checkpoints WHERE conversation_id=?1",
                params![checkpoint.conversation_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        if stored_checkpoint_revision != checkpoint.expected_revision {
            return Err(StoreError::Conflict(format!(
                "checkpoint revision mismatch: expected {}, found {stored_checkpoint_revision}",
                checkpoint.expected_revision
            )));
        }
        let (conversation_revision, ledger_count): (i64, i64) = transaction
            .query_row(
                "SELECT c.revision,COUNT(m.id) FROM conversations c
                 LEFT JOIN messages m ON m.conversation_id=c.id
                 WHERE c.id=?1 GROUP BY c.id",
                params![projection.conversation_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::Invariant(format!(
                    "unknown conversation {}",
                    projection.conversation_id
                ))
            })?;
        if conversation_revision != projection.expected_revision {
            return Err(StoreError::Conflict(format!(
                "conversation revision mismatch: expected {}, found {conversation_revision}",
                projection.expected_revision
            )));
        }
        for mutation in &projection.mutations {
            require_id("projection.event_id", &mutation.event_id)?;
            require_id("projection.message_id", &mutation.message_id)?;
            let belongs: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND conversation_id=?2)",
                params![mutation.message_id, projection.conversation_id],
                |row| row.get(0),
            )?;
            if !belongs {
                return Err(StoreError::Invariant(format!(
                    "message {} does not belong to conversation {}",
                    mutation.message_id, projection.conversation_id
                )));
            }
        }
        checkpoint_store::write_baseline(&transaction, &checkpoint.conversation_id,
            checkpoint.revision, checkpoint.event_cursor,
            &checkpoint.checkpoint_json, checkpoint.updated_at_ms)?;
        let next_revision = conversation_revision + 1;
        transaction.execute(
            "INSERT INTO conversation_projection_operations(
                operation_id,conversation_id,operation_kind,parent_message_id,
                resulting_revision,ledger_message_count,created_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                projection.operation_id,
                projection.conversation_id,
                projection.operation_kind,
                projection.parent_message_id,
                next_revision,
                ledger_count,
                projection.created_at_ms
            ],
        )?;
        for mutation in &projection.mutations {
            transaction.execute(
                "INSERT INTO message_projection_events(
                    event_id,operation_id,conversation_id,message_id,visible,reason,created_at_ms
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    mutation.event_id,
                    projection.operation_id,
                    projection.conversation_id,
                    mutation.message_id,
                    i64::from(mutation.visible),
                    mutation.reason,
                    projection.created_at_ms
                ],
            )?;
        }
        transaction.execute(
            "UPDATE conversations SET revision=?2,updated_at_ms=?3 WHERE id=?1",
            params![
                projection.conversation_id,
                next_revision,
                projection.created_at_ms
            ],
        )?;
        transaction.commit()?;
        Ok(CommitCheckpointProjectionResult {
            checkpoint: StoredCheckpoint {
                conversation_id: checkpoint.conversation_id.clone(),
                revision: checkpoint.revision,
                event_cursor: checkpoint.event_cursor,
                checkpoint_json: checkpoint.checkpoint_json.clone(),
                updated_at_ms: checkpoint.updated_at_ms,
            },
            projection: ApplyConversationProjectionResult {
                applied: true,
                conversation_revision: next_revision,
                ledger_message_count: ledger_count.max(0) as usize,
            },
        })
    }
}
