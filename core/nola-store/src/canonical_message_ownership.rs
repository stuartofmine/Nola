// SPDX-License-Identifier: Apache-2.0
//! Trusted message execution handoff facts; historical membership grants read
//! provenance only, never write authority to an earlier Run.
use crate::{params, Connection, OptionalExtension, StoreError, StoreResult};

pub(super) fn has_trusted_resumption(
    connection: &Connection,
    conversation: &str,
    message: &str,
) -> StoreResult<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages m
         JOIN canonical_message_run_owners current ON current.conversation_id=m.conversation_id AND current.message_id=m.id AND current.run_id=m.run_id
         JOIN canonical_message_run_owners previous ON previous.conversation_id=m.conversation_id AND previous.message_id=m.id AND previous.run_id<>m.run_id
         WHERE m.id=?1 AND m.conversation_id=?2 AND m.role='assistant')",
        params![message,conversation], |r|r.get(0),
    ).map_err(StoreError::from)
}

pub(super) fn matches_read_owner(
    connection: &Connection,
    conversation: &str,
    message: &str,
    run: &str,
) -> StoreResult<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages m WHERE m.id=?1 AND m.conversation_id=?2 AND m.role='assistant'
         AND (m.run_id=?3 OR (EXISTS(SELECT 1 FROM canonical_message_run_owners current
             WHERE current.conversation_id=m.conversation_id AND current.message_id=m.id AND current.run_id=m.run_id)
          AND EXISTS(SELECT 1 FROM canonical_message_run_owners historical
             WHERE historical.conversation_id=m.conversation_id AND historical.message_id=m.id AND historical.run_id=?3))))",
        params![message,conversation,run], |r|r.get(0),
    ).map_err(StoreError::from)
}

/// Local execution readers also require the current peer to remain a real
/// local conversation Run. Replica body readers have their own authority and
/// deliberately do not use this local-only boundary.
pub(super) fn matches_local_read_owner(
    connection: &Connection, conversation: &str, message: &str, run: &str,
) -> StoreResult<bool> {
    if !matches_read_owner(connection, conversation, message, run)? {
        return Ok(false);
    }
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages m JOIN runs current ON current.id=m.run_id
            JOIN runs source ON source.id=?3
            WHERE m.id=?1 AND m.conversation_id=?2
            AND current.conversation_id=?2 AND current.kind='conversation'
            AND source.conversation_id=?2 AND source.kind='conversation'
            AND NOT EXISTS(SELECT 1 FROM remote_run_sources remote
                WHERE remote.run_id=current.id OR remote.run_id=source.id))",
        params![message,conversation,run], |row|row.get(0),
    ).map_err(StoreError::from)
}

/// Call only from the local internal admission or authenticated replica resume
/// transaction, before changing the current message owner. Never from a head.
pub(super) fn record_resumption(
    connection: &Connection,
    conversation: &str,
    message: &str,
    previous: &str,
    next: &str,
) -> StoreResult<()> {
    if connection.is_autocommit()
        || previous == next
        || previous.trim().is_empty()
        || next.trim().is_empty()
    {
        return Err(StoreError::Invariant(
            "invalid message ownership handoff transaction".into(),
        ));
    }
    let current: Option<String> = connection
        .query_row(
            "SELECT run_id FROM messages WHERE id=?1 AND conversation_id=?2 AND role='assistant'",
            params![message, conversation],
            |r| r.get(0),
        )
        .optional()?;
    if current.as_deref() != Some(previous) {
        return Err(StoreError::Conflict(
            "message ownership handoff does not match its current owner".into(),
        ));
    }
    let (has_history, current_known):(bool,bool)=connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM canonical_message_run_owners WHERE conversation_id=?1 AND message_id=?2),
         EXISTS(SELECT 1 FROM canonical_message_run_owners WHERE conversation_id=?1 AND message_id=?2 AND run_id=?3)",
        params![conversation,message,previous],|r|Ok((r.get(0)?,r.get(1)?)),
    )?;
    if has_history && !current_known {
        return Err(StoreError::Conflict(
            "current message owner has no trusted handoff provenance".into(),
        ));
    }
    // Prove pre-existing immutable bindings were readable before granting any
    // new handoff membership. A queued illegal Sealed publication cannot gain
    // authorization merely by adding itself to this history.
    let mut statement = connection.prepare(
        "SELECT b.part_id,b.field FROM message_part_body_references b
         JOIN message_parts p ON p.id=b.part_id WHERE p.message_id=?1",
    )?;
    let mut rows = statement.query([message])?;
    while let Some(row) = rows.next()? {
        let part: String = row.get(0)?;
        let field: String = row.get(1)?;
        let field = match field.as_str() {
            "content_text" => nola_domain::PartBodyField::ContentText,
            "content_json" => nola_domain::PartBodyField::ContentJson,
            _ => return Err(StoreError::Invariant("unknown canonical body field".into())),
        };
        crate::published_part_body::resolve_field(connection, conversation, message, &part, field)?;
    }
    for run in [previous, next] {
        connection.execute(
            "INSERT OR IGNORE INTO canonical_message_run_owners VALUES(?1,?2,?3)",
            params![conversation, message, run],
        )?;
    }
    Ok(())
}
