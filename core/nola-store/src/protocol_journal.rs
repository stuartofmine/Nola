// SPDX-License-Identifier: Apache-2.0
//! Durable command receipts and ordered projection event storage.

use rusqlite::{params, OptionalExtension, Row};

use super::{NolaStore, StoreError, StoreResult};

const MAX_PROJECTION_PAGE_ITEMS: usize = 512;

pub use nola_store_port::protocol_journal::*;

impl NolaStore {
    pub fn begin_command_receipt(
        &mut self,
        input: &BeginCommandReceiptInput,
    ) -> StoreResult<BeginCommandReceiptResult> {
        validate_receipt_input(input)?;
        let transaction = self.write_transaction()?;
        if let Some(existing) = command_receipt(&transaction, &input.client_id, &input.request_id)?
        {
            ensure_same_command(&existing, input)?;
            transaction.commit()?;
            return Ok(BeginCommandReceiptResult::Existing(existing));
        }
        transaction.execute(
            "INSERT INTO command_receipts(
                client_id,request_id,command_type,command_json,expected_revision,status,received_at
             ) VALUES (?1,?2,?3,?4,?5,'accepted',?6)",
            params![
                input.client_id,
                input.request_id,
                input.command_type,
                input.command_json,
                input.expected_revision,
                input.received_at
            ],
        )?;
        let stored = command_receipt(&transaction, &input.client_id, &input.request_id)?
            .ok_or_else(|| StoreError::Invariant("new command receipt was not readable".into()))?;
        transaction.commit()?;
        Ok(BeginCommandReceiptResult::Started(stored))
    }

    pub fn command_receipt(
        &self,
        client_id: &str,
        request_id: &str,
    ) -> StoreResult<Option<StoredCommandReceipt>> {
        validate_required("client_id", client_id)?;
        validate_required("request_id", request_id)?;
        command_receipt(&self.connection, client_id, request_id)
    }

    pub fn complete_command_receipt(
        &mut self,
        client_id: &str,
        request_id: &str,
        completed_at: &str,
        outcome: &CompleteCommandReceiptOutcome,
    ) -> StoreResult<StoredCommandReceipt> {
        validate_required("client_id", client_id)?;
        validate_required("request_id", request_id)?;
        validate_required("completed_at", completed_at)?;
        let (status, committed_revision, result_json, error_json, conflict_json) =
            validate_completion(outcome)?;
        let transaction = self.write_transaction()?;
        let existing = command_receipt(&transaction, client_id, request_id)?.ok_or_else(|| {
            StoreError::Invariant("cannot complete an unknown command receipt".into())
        })?;
        if existing.status != "accepted" {
            if existing.status == status
                && existing.committed_revision == committed_revision
                && existing.result_json.as_deref() == result_json.as_deref()
                && existing.error_json.as_deref() == error_json.as_deref()
                && existing.conflict_json.as_deref() == conflict_json.as_deref()
            {
                transaction.commit()?;
                return Ok(existing);
            }
            return Err(StoreError::Conflict(format!(
                "command receipt {client_id}/{request_id} already has terminal status {}",
                existing.status
            )));
        }
        transaction.execute(
            "UPDATE command_receipts SET
                status=?3,committed_revision=?4,result_json=?5,error_json=?6,
                conflict_json=?7,completed_at=?8
             WHERE client_id=?1 AND request_id=?2 AND status='accepted'",
            params![
                client_id,
                request_id,
                status,
                committed_revision,
                result_json,
                error_json,
                conflict_json,
                completed_at
            ],
        )?;
        let stored = command_receipt(&transaction, client_id, request_id)?.ok_or_else(|| {
            StoreError::Invariant("completed command receipt was not readable".into())
        })?;
        transaction.commit()?;
        Ok(stored)
    }

    pub fn append_projection_event(
        &mut self,
        input: &AppendProjectionEventInput,
    ) -> StoreResult<StoredProjectionEvent> {
        validate_projection_input(input)?;
        let expected = to_i64("expected_after_sequence", input.expected_after_sequence)?;
        let transaction = self.write_transaction()?;
        if let Some(existing) = projection_event_by_id(&transaction, &input.event_id)? {
            if existing.scope == input.scope
                && existing.sequence == input.expected_after_sequence.saturating_add(1)
                && existing.payload_json == input.payload_json
                && existing.occurred_at == input.occurred_at
            {
                transaction.commit()?;
                return Ok(existing);
            }
            return Err(StoreError::Conflict(format!(
                "projection event {} was reused with different content",
                input.event_id
            )));
        }
        let current: i64 = transaction
            .query_row(
                "SELECT current_sequence FROM projection_cursors
                 WHERE scope_type=?1 AND scope_id=?2",
                params![input.scope.scope_type, input.scope.scope_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if current != expected {
            return Err(StoreError::Conflict(format!(
                "projection cursor mismatch for {}/{}: expected {}, found {current}",
                input.scope.scope_type, input.scope.scope_id, expected
            )));
        }
        let sequence = current
            .checked_add(1)
            .ok_or_else(|| StoreError::Invariant("projection sequence overflow".into()))?;
        transaction.execute(
            "INSERT INTO projection_cursors(scope_type,scope_id,current_sequence,revision,updated_at)
             VALUES (?1,?2,?3,1,?4)
             ON CONFLICT(scope_type,scope_id) DO UPDATE SET
                current_sequence=excluded.current_sequence,
                revision=projection_cursors.revision+1,
                updated_at=excluded.updated_at",
            params![
                input.scope.scope_type,
                input.scope.scope_id,
                sequence,
                input.occurred_at
            ],
        )?;
        transaction.execute(
            "INSERT INTO projection_events(
                event_id,scope_type,scope_id,sequence,payload_json,occurred_at
             ) VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                input.event_id,
                input.scope.scope_type,
                input.scope.scope_id,
                sequence,
                input.payload_json,
                input.occurred_at
            ],
        )?;
        let stored = projection_event_by_id(&transaction, &input.event_id)?
            .ok_or_else(|| StoreError::Invariant("new projection event was not readable".into()))?;
        transaction.commit()?;
        Ok(stored)
    }

    pub fn projection_cursor(
        &self,
        scope: &ProjectionScopeKey,
    ) -> StoreResult<Option<StoredProjectionCursor>> {
        validate_scope(scope)?;
        self.connection
            .query_row(
                "SELECT current_sequence,revision,updated_at FROM projection_cursors
                 WHERE scope_type=?1 AND scope_id=?2",
                params![scope.scope_type, scope.scope_id],
                |row| {
                    Ok(StoredProjectionCursor {
                        scope: scope.clone(),
                        current_sequence: from_i64(row.get(0)?)?,
                        revision: from_i64(row.get(1)?)?,
                        updated_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn projection_events_after(
        &self,
        scope: &ProjectionScopeKey,
        after_sequence: u64,
        limit: usize,
    ) -> StoreResult<Vec<StoredProjectionEvent>> {
        validate_scope(scope)?;
        if limit == 0 || limit > MAX_PROJECTION_PAGE_ITEMS {
            return Err(StoreError::Invariant(format!(
                "projection event limit must be between 1 and {MAX_PROJECTION_PAGE_ITEMS}"
            )));
        }
        let after = to_i64("after_sequence", after_sequence)?;
        let limit = i64::try_from(limit)
            .map_err(|_| StoreError::Invariant("projection event limit overflow".into()))?;
        let mut statement = self.connection.prepare(
            "SELECT event_id,scope_type,scope_id,sequence,payload_json,occurred_at
             FROM projection_events
             WHERE scope_type=?1 AND scope_id=?2 AND sequence>?3
             ORDER BY sequence ASC LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![scope.scope_type, scope.scope_id, after, limit],
            projection_event_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

fn command_receipt(
    connection: &rusqlite::Connection,
    client_id: &str,
    request_id: &str,
) -> StoreResult<Option<StoredCommandReceipt>> {
    connection
        .query_row(
            "SELECT client_id,request_id,command_type,command_json,expected_revision,status,
                    committed_revision,result_json,error_json,conflict_json,received_at,completed_at
             FROM command_receipts WHERE client_id=?1 AND request_id=?2",
            params![client_id, request_id],
            command_receipt_row,
        )
        .optional()
        .map_err(StoreError::from)
}

fn command_receipt_row(row: &Row<'_>) -> rusqlite::Result<StoredCommandReceipt> {
    Ok(StoredCommandReceipt {
        client_id: row.get(0)?,
        request_id: row.get(1)?,
        command_type: row.get(2)?,
        command_json: row.get(3)?,
        expected_revision: row.get(4)?,
        status: row.get(5)?,
        committed_revision: row.get(6)?,
        result_json: row.get(7)?,
        error_json: row.get(8)?,
        conflict_json: row.get(9)?,
        received_at: row.get(10)?,
        completed_at: row.get(11)?,
    })
}

fn projection_event_by_id(
    connection: &rusqlite::Connection,
    event_id: &str,
) -> StoreResult<Option<StoredProjectionEvent>> {
    connection
        .query_row(
            "SELECT event_id,scope_type,scope_id,sequence,payload_json,occurred_at
             FROM projection_events WHERE event_id=?1",
            params![event_id],
            projection_event_row,
        )
        .optional()
        .map_err(StoreError::from)
}

fn projection_event_row(row: &Row<'_>) -> rusqlite::Result<StoredProjectionEvent> {
    Ok(StoredProjectionEvent {
        event_id: row.get(0)?,
        scope: ProjectionScopeKey {
            scope_type: row.get(1)?,
            scope_id: row.get(2)?,
        },
        sequence: from_i64(row.get(3)?)?,
        payload_json: row.get(4)?,
        occurred_at: row.get(5)?,
    })
}

fn validate_receipt_input(input: &BeginCommandReceiptInput) -> StoreResult<()> {
    validate_required("client_id", &input.client_id)?;
    validate_required("request_id", &input.request_id)?;
    validate_type_tag("command_type", &input.command_type)?;
    validate_json_object("command_json", &input.command_json)?;
    validate_required("received_at", &input.received_at)?;
    if input.expected_revision.is_some_and(|revision| revision < 0) {
        return Err(StoreError::Invariant(
            "expected_revision must not be negative".into(),
        ));
    }
    Ok(())
}

fn ensure_same_command(
    stored: &StoredCommandReceipt,
    input: &BeginCommandReceiptInput,
) -> StoreResult<()> {
    if stored.command_type == input.command_type
        && stored.command_json == input.command_json
        && stored.expected_revision == input.expected_revision
    {
        return Ok(());
    }
    Err(StoreError::Conflict(format!(
        "request identity {}/{} was reused for another command",
        input.client_id, input.request_id
    )))
}

type CompletionColumns = (
    &'static str,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn validate_completion(outcome: &CompleteCommandReceiptOutcome) -> StoreResult<CompletionColumns> {
    match outcome {
        CompleteCommandReceiptOutcome::Committed {
            committed_revision,
            result_json,
        } => {
            if committed_revision.is_some_and(|revision| revision <= 0) {
                return Err(StoreError::Invariant(
                    "committed_revision must be greater than zero when present".into(),
                ));
            }
            validate_json("result_json", result_json)?;
            Ok((
                "committed",
                *committed_revision,
                Some(result_json.clone()),
                None,
                None,
            ))
        }
        CompleteCommandReceiptOutcome::Rejected { error_json } => {
            validate_json_object("error_json", error_json)?;
            Ok(("rejected", None, None, Some(error_json.clone()), None))
        }
        CompleteCommandReceiptOutcome::Conflict { conflict_json } => {
            validate_json_object("conflict_json", conflict_json)?;
            Ok(("conflict", None, None, None, Some(conflict_json.clone())))
        }
    }
}

fn validate_projection_input(input: &AppendProjectionEventInput) -> StoreResult<()> {
    validate_required("event_id", &input.event_id)?;
    validate_scope(&input.scope)?;
    validate_json_object("payload_json", &input.payload_json)?;
    validate_required("occurred_at", &input.occurred_at)
}

fn validate_scope(scope: &ProjectionScopeKey) -> StoreResult<()> {
    validate_type_tag("scope_type", &scope.scope_type)?;
    if scope.scope_type != "conversations" {
        validate_required("scope_id", &scope.scope_id)?;
    } else if !scope.scope_id.is_empty() {
        return Err(StoreError::Invariant(
            "the conversations scope must not have a scope_id".into(),
        ));
    }
    Ok(())
}

fn validate_required(field: &str, value: &str) -> StoreResult<()> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(StoreError::Invariant(format!(
            "{field} must be non-empty and contain no control characters"
        )));
    }
    Ok(())
}

fn validate_type_tag(field: &str, value: &str) -> StoreResult<()> {
    validate_required(field, value)?;
    let mut characters = value.chars();
    if !characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
        || !characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(StoreError::Invariant(format!(
            "{field} must use lower snake_case"
        )));
    }
    Ok(())
}

fn validate_json(field: &str, value: &str) -> StoreResult<()> {
    crate::structured_json_validation::validate_json_str(value)
        .map_err(|error| StoreError::Invariant(format!("{field} is invalid JSON: {error}")))
}

fn validate_json_object(field: &str, value: &str) -> StoreResult<()> {
    let decoded = serde_json::from_str::<serde_json::Value>(value)
        .map_err(|error| StoreError::Invariant(format!("{field} is invalid JSON: {error}")))?;
    if !decoded.is_object() {
        return Err(StoreError::Invariant(format!(
            "{field} must be a JSON object"
        )));
    }
    Ok(())
}

fn to_i64(field: &str, value: u64) -> StoreResult<i64> {
    i64::try_from(value)
        .map_err(|_| StoreError::Invariant(format!("{field} exceeds SQLite integer range")))
}

fn from_i64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn begin_input() -> BeginCommandReceiptInput {
        BeginCommandReceiptInput {
            client_id: "mac-1".into(),
            request_id: "request-1".into(),
            command_type: "create_conversation".into(),
            command_json: r#"{"input":{"id":"conversation-1"}}"#.into(),
            expected_revision: None,
            received_at: "2026-09-04T00:00:00Z".into(),
        }
    }

    #[test]
    fn command_receipt_survives_reopen_and_prevents_identity_reuse() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state.db");
        let input = begin_input();
        {
            let mut store = NolaStore::open(&path).unwrap();
            assert!(matches!(
                store.begin_command_receipt(&input).unwrap(),
                BeginCommandReceiptResult::Started(_)
            ));
            store
                .complete_command_receipt(
                    &input.client_id,
                    &input.request_id,
                    "2026-09-04T00:00:01Z",
                    &CompleteCommandReceiptOutcome::Committed {
                        committed_revision: Some(1),
                        result_json: r#"{"id":"conversation-1"}"#.into(),
                    },
                )
                .unwrap();
        }
        let mut reopened = NolaStore::open(&path).unwrap();
        let replay = reopened.begin_command_receipt(&input).unwrap();
        assert!(
            matches!(replay, BeginCommandReceiptResult::Existing(ref receipt) if receipt.status == "committed")
        );
        let mut changed = input;
        changed.command_type = "delete_conversation".into();
        assert!(matches!(
            reopened.begin_command_receipt(&changed),
            Err(StoreError::Conflict(_))
        ));
    }

    #[test]
    fn projection_events_are_contiguous_and_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("state.db")).unwrap();
        let first = AppendProjectionEventInput {
            event_id: "event-1".into(),
            scope: ProjectionScopeKey {
                scope_type: "conversation".into(),
                scope_id: "conversation-1".into(),
            },
            expected_after_sequence: 0,
            payload_json: r#"{"type":"message_upserted"}"#.into(),
            occurred_at: "2026-09-04T00:00:00Z".into(),
        };
        assert_eq!(store.append_projection_event(&first).unwrap().sequence, 1);
        assert_eq!(store.append_projection_event(&first).unwrap().sequence, 1);
        let mut gap = first.clone();
        gap.event_id = "event-2".into();
        gap.expected_after_sequence = 4;
        assert!(matches!(
            store.append_projection_event(&gap),
            Err(StoreError::Conflict(_))
        ));
        let events = store.projection_events_after(&first.scope, 0, 32).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "event-1");
    }
}
