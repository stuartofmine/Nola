// SPDX-License-Identifier: Apache-2.0
//! Durable ownership of opaque delegated executions. Application supplies transitions.
use super::*;
use nola_domain::{DelegatedInvocationMutation, DelegatedInvocationRecord};

fn decode(raw: String) -> StoreResult<DelegatedInvocationRecord> {
    serde_json::from_str(&raw)
        .map_err(|e| StoreError::Invariant(format!("invalid delegated invocation: {e}")))
}
fn read(connection: &Connection, id: &str) -> StoreResult<Option<DelegatedInvocationRecord>> {
    let raw = connection
        .query_row(
            "SELECT record_json FROM delegated_invocations WHERE id=?1",
            params![id],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    raw.map(decode).transpose()
}
pub(super) fn for_effect(
    connection: &Connection,
    effect: &str,
) -> StoreResult<Option<DelegatedInvocationRecord>> {
    let raw = connection
        .query_row(
            "SELECT record_json FROM delegated_invocations WHERE effect_id=?1",
            params![effect],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    raw.map(decode).transpose()
}
pub(super) fn recovery_records(
    connection: &Connection,
    run_id: &str,
) -> StoreResult<Vec<DelegatedInvocationRecord>> {
    // Drive terminal owner lookup from pending deliveries, avoiding a scan of
    // every historical invocation in a long-lived Run. The target is a stored
    // protocol relationship; Application validates its cancellation semantics.
    let mut statement = connection.prepare("WITH wanted(id) AS (
        SELECT id FROM delegated_invocations WHERE run_id=?1 AND state IN ('prepared','running','waiting_permission','interrupted','cancelling')
        UNION SELECT d.id FROM effect_outbox e JOIN delegated_invocations d ON d.effect_id=e.effect_id WHERE e.run_id=?1 AND e.status='pending' AND d.run_id=?1
        UNION SELECT d.id FROM effect_outbox e JOIN delegated_invocations d ON d.effect_id=e.target_effect_id WHERE e.run_id=?1 AND e.status='pending' AND d.run_id=?1
        UNION SELECT p.invocation_id FROM delegated_permission_deliveries p WHERE p.run_id=?1 AND (p.state IN ('awaiting_decision','prepared','claimed','outcome_unknown') OR EXISTS(SELECT 1 FROM effect_outbox e WHERE e.run_id=p.run_id AND e.effect_id=p.control_effect_id AND e.status='pending'))
    ) SELECT d.record_json FROM wanted w JOIN delegated_invocations d ON d.id=w.id ORDER BY d.id")?;
    let raw = statement
        .query_map(params![run_id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    raw.into_iter().map(decode).collect()
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDelegatedRequestParts {
    pub run_id: String,
    pub invocation_id: String,
    pub request_id: String,
    pub text_part_id: String,
    pub reasoning_part_id: String,
}
impl NolaStore {
    pub fn delegated_invocation(&self, id: &str) -> StoreResult<Option<DelegatedInvocationRecord>> {
        require_id("delegated_invocation.id", id)?;
        read(&self.connection, id)
    }
    pub fn delegated_invocation_for_effect(
        &self,
        effect_id: &str,
    ) -> StoreResult<Option<DelegatedInvocationRecord>> {
        require_id("delegated_invocation.effect_id", effect_id)?;
        for_effect(&self.connection, effect_id)
    }
    pub fn delegated_text_part_owner(
        &self,
        conversation_id: &str,
        part_id: &str,
    ) -> StoreResult<Option<StoredDelegatedRequestParts>> {
        require_id("conversation_id", conversation_id)?;
        require_id("part_id", part_id)?;
        let mut statement = self.connection.prepare("SELECT d.run_id,d.id,COALESCE((SELECT request_id FROM delegated_invocation_deliveries WHERE invocation_id=d.id ORDER BY request_id LIMIT 1),''),d.text_part_id,d.reasoning_part_id FROM delegated_invocations d JOIN runs r ON r.id=d.run_id WHERE r.conversation_id=?1 AND d.text_part_id=?2 LIMIT 2")?;
        let rows = statement
            .query_map(params![conversation_id, part_id], parts_row)?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.len() > 1 {
            return Err(StoreError::Invariant(
                "ambiguous delegated Part ownership".into(),
            ));
        }
        Ok(rows.into_iter().next())
    }
    pub fn delegated_request_parts(
        &self,
        request_id: &str,
    ) -> StoreResult<Option<StoredDelegatedRequestParts>> {
        require_id("request_id", request_id)?;
        Ok(self.connection.query_row("SELECT d.run_id,d.id,b.request_id,d.text_part_id,d.reasoning_part_id FROM delegated_invocation_deliveries b JOIN delegated_invocations d ON d.id=b.invocation_id WHERE b.request_id=?1",params![request_id],parts_row).optional()?)
    }
}
fn parts_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredDelegatedRequestParts> {
    Ok(StoredDelegatedRequestParts {
        run_id: row.get(0)?,
        invocation_id: row.get(1)?,
        request_id: row.get(2)?,
        text_part_id: row.get(3)?,
        reasoning_part_id: row.get(4)?,
    })
}
pub(super) fn apply(
    connection: &Connection,
    run_id: &str,
    mutations: &[DelegatedInvocationMutation],
) -> StoreResult<()> {
    for mutation in mutations {
        let record = &mutation.record;
        for (name, id) in [
            ("id", &record.id),
            ("run_id", &record.run_id),
            ("effect_id", &record.effect_id),
            ("backend_id", &record.backend_id),
            ("text_part_id", &record.text_part_id),
            ("reasoning_part_id", &record.reasoning_part_id),
        ] {
            require_id(name, id)?;
        }
        if let Some(session) = &record.backend_session_id {
            require_id("backend_session_id", session)?;
        }
        if record.run_id != run_id {
            return Err(StoreError::Conflict(
                "delegated invocation belongs to another Run".into(),
            ));
        }
        let raw =
            serde_json::to_string(record).map_err(|e| StoreError::Invariant(e.to_string()))?;
        if raw.len() > 65536 {
            return Err(StoreError::Invariant(
                "delegated invocation metadata exceeds 64KiB".into(),
            ));
        }
        let state =
            serde_json::to_value(record.state).map_err(|e| StoreError::Invariant(e.to_string()))?;
        let state = state.as_str().ok_or_else(|| {
            StoreError::Invariant("delegated state must serialize as string".into())
        })?;
        match mutation.expected_revision {
            None => {
                if record.revision != 1 {
                    return Err(StoreError::Invariant(
                        "new delegated invocation revision must be one".into(),
                    ));
                }
                if let Some(existing) = read(connection, &record.id)? {
                    if existing == *record {
                        continue;
                    }
                    return Err(StoreError::Conflict(
                        "delegated invocation identity already exists".into(),
                    ));
                }
                if for_effect(connection, &record.effect_id)?.is_some() {
                    return Err(StoreError::Conflict(
                        "effect already owns a delegated invocation".into(),
                    ));
                }
                connection.execute("INSERT INTO delegated_invocations(id,run_id,effect_id,backend_id,backend_session_id,text_part_id,reasoning_part_id,state,revision,record_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![record.id,record.run_id,record.effect_id,record.backend_id,record.backend_session_id,record.text_part_id,record.reasoning_part_id,state,record.revision,raw])?;
            }
            Some(expected) => {
                let previous = read(connection, &record.id)?
                    .ok_or_else(|| StoreError::Conflict("delegated invocation missing".into()))?;
                if previous.revision != expected || expected.checked_add(1) != Some(record.revision)
                {
                    return Err(StoreError::Conflict(
                        "delegated invocation revision changed".into(),
                    ));
                }
                if previous.state.is_terminal() {
                    return Err(StoreError::Conflict(
                        "terminal delegated invocation is immutable".into(),
                    ));
                }
                if previous.run_id != record.run_id
                    || previous.effect_id != record.effect_id
                    || previous.backend_id != record.backend_id
                    || previous.text_part_id != record.text_part_id
                    || previous.reasoning_part_id != record.reasoning_part_id
                    || previous.created_at_ms != record.created_at_ms
                    || (previous.backend_session_id.is_some()
                        && previous.backend_session_id != record.backend_session_id)
                {
                    return Err(StoreError::Conflict(
                        "delegated invocation immutable ownership changed".into(),
                    ));
                }
                connection.execute("UPDATE delegated_invocations SET backend_session_id=?2,state=?3,revision=?4,record_json=?5 WHERE id=?1 AND revision=?6",params![record.id,record.backend_session_id,state,record.revision,raw,expected])?;
            }
        }
    }
    Ok(())
}
pub(super) fn bind_delivery(
    connection: &Connection,
    run_id: &str,
    effect: &OutboxEffectInput,
) -> StoreResult<()> {
    let Some(id) = &effect.delegated_invocation_id else {
        return Ok(());
    };
    if effect.model_attempt_id.is_some() {
        return Err(StoreError::Conflict(
            "delivery cannot own both Attempt and delegated invocation".into(),
        ));
    }
    let record = read(connection, id)?.ok_or_else(|| {
        StoreError::Conflict("delivery references missing delegated invocation".into())
    })?;
    if record.run_id != run_id || record.effect_id != effect.effect_id {
        return Err(StoreError::Conflict(
            "delegated delivery ownership mismatch".into(),
        ));
    }
    let existing: Option<String> = connection
        .query_row(
            "SELECT invocation_id FROM delegated_invocation_deliveries WHERE request_id=?1",
            params![effect.request_id],
            |r| r.get(0),
        )
        .optional()?;
    if existing.as_ref().is_some_and(|previous| previous != id) {
        return Err(StoreError::Conflict(
            "delegated delivery request already belongs to another invocation".into(),
        ));
    }
    connection.execute("INSERT INTO delegated_invocation_deliveries(request_id,invocation_id) VALUES(?1,?2) ON CONFLICT(request_id) DO NOTHING",params![effect.request_id,id])?;
    Ok(())
}
pub(super) fn append_initial_events(
    connection: &Connection,
    run_id: &str,
    mutations: &[DelegatedInvocationMutation],
    at_ms: i64,
) -> StoreResult<()> {
    let mut sequence: i64 = connection.query_row(
        "SELECT next_event_sequence FROM runs WHERE id=?1",
        params![run_id],
        |r| r.get(0),
    )?;
    for mutation in mutations {
        let record = &mutation.record;
        let payload =
            serde_json::to_string(record).map_err(|e| StoreError::Invariant(e.to_string()))?;
        connection.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES(?1,?2,?3,'delegated_invocation_changed',?4,?5)",params![format!("delegated-invocation:{}:{}",record.id,record.revision),run_id,sequence,payload,at_ms])?;
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| StoreError::Invariant("Run event sequence overflow".into()))?;
    }
    connection.execute(
        "UPDATE runs SET next_event_sequence=?2 WHERE id=?1",
        params![run_id, sequence],
    )?;
    Ok(())
}
