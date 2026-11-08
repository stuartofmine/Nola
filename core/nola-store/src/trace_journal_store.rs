// SPDX-License-Identifier: Apache-2.0
//! Append-only execution trace persistence and conversation/run queries.

use super::*;

impl NolaStore {
    pub fn append_trace_journal_event(
        &mut self,
        input: &AppendTraceJournalEventInput,
    ) -> StoreResult<bool> {
        require_id("trace.event_id", &input.event_id)?;
        require_id("trace.trace_id", &input.trace_id)?;
        require_id("trace.run_id", &input.run_id)?;
        require_id("trace.conversation_id", &input.conversation_id)?;
        require_id("trace.turn_id", &input.turn_id)?;
        if input.sequence <= 0 {
            return Err(StoreError::Invariant(
                "trace sequence must be positive".into(),
            ));
        }
        let _: serde_json::Value = serde_json::from_str(&input.event_json)?;
        if let Some(envelope) = input.envelope_json.as_deref() {
            let value: serde_json::Value = serde_json::from_str(envelope)?;
            if !value.is_object() {
                return Err(StoreError::Invariant(
                    "trace transport envelope must be a JSON object".into(),
                ));
            }
        }
        let transaction = self.write_transaction()?;
        transaction.execute(
            "INSERT OR IGNORE INTO trace_journal_runs(
                trace_id,run_id,conversation_id,message_id,turn_id,status,last_sequence,
                started_at,updated_at
             ) VALUES(?1,?2,?3,?4,?5,'running',0,?6,?6)",
            params![
                input.trace_id,
                input.run_id,
                input.conversation_id,
                input.message_id,
                input.turn_id,
                input.emitted_at
            ],
        )?;
        let already_terminal: bool = transaction.query_row(
            "SELECT finished_at IS NOT NULL FROM trace_journal_runs WHERE trace_id=?1",
            params![input.trace_id],
            |row| row.get(0),
        )?;
        let inserted = if already_terminal {
            false
        } else {
            transaction.execute(
                "INSERT OR IGNORE INTO trace_journal_events(
                    event_id,trace_id,conversation_id,turn_id,sequence,kind,status,visibility,
                    span_id,parent_span_id,emitted_at,event_json,envelope_json
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                params![
                    input.event_id,
                    input.trace_id,
                    input.conversation_id,
                    input.turn_id,
                    input.sequence,
                    input.kind,
                    input.status,
                    input.visibility,
                    input.span_id,
                    input.parent_span_id,
                    input.emitted_at,
                    input.event_json,
                    input.envelope_json
                ],
            )? == 1
        };
        if !already_terminal {
            if let Some(envelope) = input.envelope_json.as_deref() {
                transaction.execute(
                    "UPDATE trace_journal_events SET envelope_json=?2
                     WHERE event_id=?1 AND trace_id=?3",
                    params![input.event_id, envelope, input.trace_id],
                )?;
            }
        }
        if inserted {
            transaction.execute(
                "UPDATE trace_journal_runs
                 SET status=COALESCE(?1,status),last_sequence=MAX(last_sequence,?2),
                     terminal_reason=CASE WHEN ?3 THEN ?4 ELSE terminal_reason END,
                     updated_at=?5,finished_at=CASE WHEN ?3 THEN ?5 ELSE finished_at END
                 WHERE trace_id=?6",
                params![
                    input.run_status,
                    input.sequence,
                    input.is_terminal,
                    input.terminal_reason,
                    input.emitted_at,
                    input.trace_id
                ],
            )?;
        }
        transaction.commit()?;
        Ok(inserted)
    }

    pub fn exact_run_trace_replay(
        &self, conversation_id: &str, run_id: &str, assistant_message_id: &str,
        after_sequence: i64, through_sequence: Option<i64>, limit: usize,
    ) -> StoreResult<nola_store_port::trace_journal::StoredExactTraceReplay> {
        use nola_store_port::trace_journal::StoredExactTraceReplay;
        let _snapshot = read_snapshot::pin(&self.connection)?;
        if after_sequence < 0 || !(1..=256).contains(&limit)
            || through_sequence.is_some_and(|end| end < after_sequence) {
            return Err(StoreError::Invariant("invalid exact trace replay window".into()));
        }
        let owner: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs r JOIN messages m ON m.id=r.assistant_message_id
             AND m.conversation_id=r.conversation_id AND m.role='assistant'
             JOIN assistant_turn_message_bindings b ON b.run_id=r.id AND b.conversation_id=r.conversation_id
                AND b.message_id=r.assistant_message_id AND b.admission=1
             WHERE r.id=?1 AND r.conversation_id=?2 AND r.assistant_message_id=?3)",
            params![run_id, conversation_id, assistant_message_id], |row| row.get(0))?;
        if !owner { return Err(StoreError::Invariant("exact trace replay owner mismatch".into())); }
        let mut query = self.connection.prepare(&format!(
            "SELECT {TRACE_JOURNAL_RUN_COLUMNS} FROM trace_journal_runs WHERE run_id=?1 LIMIT 2"))?;
        let runs = query.query_map([run_id], trace_journal_run_row)?.collect::<Result<Vec<_>,_>>()?;
        if runs.len() > 1 { return Err(StoreError::Invariant("exact Run has ambiguous traces".into())); }
        let Some(run) = runs.first() else {
            if after_sequence != 0 || through_sequence.is_some_and(|end| end != 0) {
                return Err(StoreError::Invariant("exact trace replay cursor has no journal".into()));
            }
            return Ok(StoredExactTraceReplay { trace_id: None, through_sequence: 0, next_sequence: 0, records: vec![] });
        };
        if run.conversation_id != conversation_id || run.message_id.as_deref() != Some(assistant_message_id)
            || run.turn_id != run_id || run.last_sequence < after_sequence {
            return Err(StoreError::Invariant("exact trace journal identity or cursor mismatch".into()));
        }
        let through = through_sequence.unwrap_or(run.last_sequence);
        if through > run.last_sequence { return Err(StoreError::Invariant("exact trace replay frontier is ahead of journal".into())); }
        let mut statement = self.connection.prepare(&format!(
            "SELECT {TRACE_JOURNAL_EVENT_COLUMNS} FROM trace_journal_events
             WHERE trace_id=?1 AND sequence>?2 AND sequence<=?3 ORDER BY sequence LIMIT ?4"))?;
        let records = statement.query_map(params![run.trace_id,after_sequence,through,limit as i64],trace_journal_event_row)?
            .collect::<Result<Vec<_>,_>>()?;
        let expected_count = (through-after_sequence).min(limit as i64) as usize;
        if records.len() != expected_count { return Err(StoreError::Invariant("exact trace journal has a gap".into())); }
        for (index, record) in records.iter().enumerate() {
            if record.sequence != after_sequence + index as i64 + 1 || record.trace_id != run.trace_id
                || record.conversation_id != conversation_id || record.turn_id != run_id {
                return Err(StoreError::Invariant("exact trace replay record mismatch".into()));
            }
        }
        let next_sequence = records.last().map_or(after_sequence, |record|record.sequence);
        Ok(StoredExactTraceReplay { trace_id: Some(run.trace_id.clone()), through_sequence: through, next_sequence, records })
    }

    pub fn trace_journal_events_after(
        &self,
        trace_id: &str,
        sequence: i64,
    ) -> StoreResult<Vec<StoredTraceJournalEvent>> {
        require_id("trace.trace_id", trace_id)?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {TRACE_JOURNAL_EVENT_COLUMNS} FROM trace_journal_events
             WHERE trace_id=?1 AND sequence>?2 ORDER BY sequence"
        ))?;
        let rows = statement.query_map(params![trace_id, sequence], trace_journal_event_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Formal execution history requires a canonical Run and its exact root
    /// message. Pre-admission diagnostics remain readable by trace ID, but
    /// their trace status is not evidence that a conversation Run exists.
    pub fn trace_journal_runs_for_conversation(
        &self,
        conversation_id: &str,
    ) -> StoreResult<Vec<StoredTraceJournalRun>> {
        require_id("trace.conversation_id", conversation_id)?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {TRACE_JOURNAL_RUN_COLUMNS} FROM trace_journal_runs t
             WHERE conversation_id=?1 AND EXISTS(
                SELECT 1 FROM runs r JOIN messages m ON m.id=r.assistant_message_id
                    AND m.conversation_id=r.conversation_id AND m.role='assistant'
             JOIN assistant_turn_message_bindings b ON b.run_id=r.id AND b.conversation_id=r.conversation_id
                AND b.message_id=r.assistant_message_id AND b.admission=1
                WHERE r.id=t.run_id AND r.conversation_id=t.conversation_id
                    AND r.assistant_message_id=t.message_id AND t.turn_id=r.id
             ) ORDER BY started_at,trace_id"
        ))?;
        let rows = statement.query_map(params![conversation_id], trace_journal_run_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn latest_trace_journal_id(&self, conversation_id: &str) -> StoreResult<Option<String>> {
        require_id("trace.conversation_id", conversation_id)?;
        self.connection
            .query_row(
                "SELECT trace_id FROM trace_journal_runs WHERE conversation_id=?1
                 ORDER BY updated_at DESC,trace_id DESC LIMIT 1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
    }
}
