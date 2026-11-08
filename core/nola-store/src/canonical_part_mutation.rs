// SPDX-License-Identifier: Apache-2.0
//! Canonical Part SQL mutation entry shared by local commits and replica apply.
//! The caller owns the transaction, Run/message ownership checks and receipt
//! ordering. Legacy and private stream mutations share admission here; no
//! mutation independently commits any writes.

use crate::{require_id, validate_json, StoreError, StoreResult};
use nola_domain::PartMutation;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

pub(super) fn apply_part_mutation(
    transaction: &Connection,
    assistant_message_id: &str,
    mutation: &PartMutation,
) -> StoreResult<()> {
    let part_id = part_mutation_id(mutation);
    let stored: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT message_id,kind,state FROM message_parts WHERE id=?1",
            params![part_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let identity =
        stored.as_ref().map(
            |(message, kind, state)| nola_kernel::CanonicalPartIdentity {
                part_id,
                message_id: message,
                kind,
                state,
            },
        );
    let decision =
        nola_kernel::evaluate_part_mutation(assistant_message_id, mutation, identity.as_ref());
    if decision != nola_kernel::PartAdmission::Apply {
        return Err(StoreError::Invariant(format!(
            "part mutation did not target exactly one eligible Part: {decision:?}",
        )));
    }
    let has_stream: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM canonical_stream_heads WHERE part_id=?1)",
        [part_id],
        |row| row.get(0),
    )?;
    if has_stream
        && matches!(
            mutation,
            PartMutation::Upsert { .. } | PartMutation::AppendText { .. }
        )
    {
        return Err(StoreError::Conflict(
            "legacy mutation cannot overwrite a canonical stream".into(),
        ));
    }
    let changed = match mutation {
        PartMutation::Stream {
            operation,
            updated_at_ms,
        } => {
            if operation.owner().message_id != assistant_message_id {
                return Err(StoreError::Invariant(
                    "stream mutation assistant owner mismatch".into(),
                ));
            }
            crate::canonical_stream_store::apply(transaction, operation)?;
            transaction.execute(
                "UPDATE message_parts SET updated_at_ms=?2 WHERE id=?1 AND message_id=?3",
                params![part_id, updated_at_ms, assistant_message_id],
            )?
        }
        PartMutation::Metadata {
            part_id,
            state,
            content_json,
            updated_at_ms,
        } => {
            if transaction.is_autocommit() {
                return Err(StoreError::Invariant(
                    "metadata mutation requires an outer transaction".into(),
                ));
            }
            let previous_state = &stored
                .as_ref()
                .ok_or_else(|| StoreError::Invariant("metadata Part is missing".into()))?
                .2;
            if state != previous_state {
                return Err(StoreError::Conflict(
                    "metadata mutation cannot change Part lifecycle; use Close".into(),
                ));
            }
            if let Some(json) = content_json {
                validate_json("part.content_json", json)?;
            }
            let sealed_json: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM message_part_body_references WHERE part_id=?1 AND field='content_json')",
                [part_id], |row| row.get(0),
            )?;
            if sealed_json && content_json.is_some() {
                return Err(StoreError::Conflict(
                    "metadata mutation cannot overwrite sealed JSON".into(),
                ));
            }
            transaction.execute(
                "UPDATE message_parts SET content_json=COALESCE(?2,content_json),updated_at_ms=?3 WHERE id=?1 AND message_id=?4",
                params![part_id, content_json, updated_at_ms, assistant_message_id],
            )?
        }
        PartMutation::Upsert {
            part_id,
            kind,
            ordinal: _,
            state,
            content_text,
            content_json,
            content_digest,
            created_at_ms,
            updated_at_ms,
        } => {
            require_id("part.part_id", part_id)?;
            if let Some(value) = content_json {
                validate_json("part.content_json", value)?;
            }
            transaction.execute(
                "INSERT INTO message_parts(
                    id, message_id, kind, ordinal, state, content_text,
                    content_json, content_digest, created_at_ms, updated_at_ms
                 ) VALUES (
                    ?1, ?2, ?3,
                    COALESCE(
                        (SELECT ordinal FROM message_parts
                         WHERE id = ?1 AND message_id = ?2 AND kind = ?3),
                        (SELECT COALESCE(MAX(ordinal), 0) + 1
                         FROM message_parts WHERE message_id = ?2)
                    ),
                    ?4, ?5, ?6, ?7, ?8, ?9
                 )
                 ON CONFLICT(id) DO UPDATE SET
                    state = excluded.state,
                    content_text = excluded.content_text,
                    content_json = excluded.content_json,
                    content_digest = excluded.content_digest,
                    updated_at_ms = excluded.updated_at_ms
                 WHERE message_parts.message_id = excluded.message_id
                   AND message_parts.kind = excluded.kind",
                params![
                    part_id,
                    assistant_message_id,
                    kind,
                    state,
                    content_text,
                    content_json,
                    content_digest,
                    created_at_ms,
                    updated_at_ms
                ],
            )?
        }
        PartMutation::AppendText {
            part_id,
            expected_content_digest,
            text,
            updated_at_ms,
        } => {
            let content_text: String = transaction
                .query_row(
                    "SELECT content_text FROM message_parts
                     WHERE id = ?1 AND message_id = ?2 AND state = 'open'",
                    params![part_id, assistant_message_id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| {
                    StoreError::Invariant(format!("open Part {part_id} does not exist"))
                })?;
            let actual_digest = format!("{:x}", Sha256::digest(content_text.as_bytes()));
            if actual_digest != *expected_content_digest {
                return Err(StoreError::Conflict(format!(
                    "Part {part_id} append prefix mismatch: stored={actual_digest}, expected={expected_content_digest}"
                )));
            }
            transaction.execute(
                "UPDATE message_parts SET content_text = content_text || ?2, updated_at_ms = ?3
                 WHERE id = ?1 AND message_id = ?4 AND state = 'open'",
                params![part_id, text, updated_at_ms, assistant_message_id],
            )?
        }
        PartMutation::Close {
            part_id,
            state,
            content_json,
            content_digest,
            updated_at_ms,
        } => {
            if let Some(value) = content_json {
                validate_json("part.content_json", value)?;
            }
            let stream = if has_stream {
                let conversation: String = transaction.query_row(
                    "SELECT conversation_id FROM messages WHERE id=?1",
                    [assistant_message_id],
                    |row| row.get(0),
                )?;
                let head = crate::canonical_stream_read::resolve(
                    transaction,
                    &conversation,
                    assistant_message_id,
                    part_id,
                )?
                .ok_or_else(|| StoreError::Invariant("stream disappeared during Close".into()))?;
                if !head.closed {
                    return Err(StoreError::Conflict(
                        "Close requires an already closed stream head".into(),
                    ));
                }
                Some(head)
            } else {
                None
            };
            if let Some(expected_digest) = content_digest {
                let actual_digest = if let Some(head) = stream.as_ref() {
                    crate::canonical_stream_read::complete_digest(transaction, head)?
                } else {
                    let content_text: String = transaction
                        .query_row(
                            "SELECT content_text FROM message_parts
                         WHERE id = ?1 AND message_id = ?2",
                            params![part_id, assistant_message_id],
                            |row| row.get(0),
                        )
                        .optional()?
                        .ok_or_else(|| StoreError::Invariant(format!("unknown Part {part_id}")))?;
                    format!("{:x}", Sha256::digest(content_text.as_bytes()))
                };
                if actual_digest != *expected_digest {
                    return Err(StoreError::Conflict(format!(
                        "Part {part_id} content digest mismatch: stored={actual_digest}, expected={expected_digest}"
                    )));
                }
            }
            transaction.execute(
                "UPDATE message_parts SET state = ?2,
                    content_json = COALESCE(?3, content_json),
                    content_digest = COALESCE(?4, content_digest), updated_at_ms = ?5
                 WHERE id = ?1 AND message_id = ?6",
                params![
                    part_id,
                    state,
                    content_json,
                    content_digest,
                    updated_at_ms,
                    assistant_message_id
                ],
            )?
        }
    };
    if changed != 1 {
        return Err(StoreError::Invariant(
            "part mutation did not target exactly one Part".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn part_mutation_id(mutation: &PartMutation) -> &str {
    match mutation {
        PartMutation::Upsert { part_id, .. }
        | PartMutation::AppendText { part_id, .. }
        | PartMutation::Close { part_id, .. }
        | PartMutation::Metadata { part_id, .. } => part_id,
        PartMutation::Stream { operation, .. } => &operation.owner().part_id,
    }
}
