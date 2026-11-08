// SPDX-License-Identifier: Apache-2.0
//! Canonical Attempt persistence. All transitions are supplied by Application.
use super::*;
use nola_domain::{ModelAttemptMutation, ModelAttemptRecord};

fn read(connection: &Connection, id: &str) -> StoreResult<Option<ModelAttemptRecord>> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT record_json FROM run_attempts WHERE id=?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|raw| {
        serde_json::from_str(&raw)
            .map_err(|e| StoreError::Invariant(format!("invalid Attempt record: {e}")))
    })
    .transpose()
}
pub(super) fn tail(
    connection: &Connection,
    effect: &str,
) -> StoreResult<Option<ModelAttemptRecord>> {
    let mut q=connection.prepare("SELECT a.record_json FROM run_attempts a WHERE effect_id=?1 AND NOT EXISTS(SELECT 1 FROM run_attempts child WHERE child.predecessor_id=a.id) LIMIT 2")?;
    let rows = q
        .query_map(params![effect], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() > 1 {
        return Err(StoreError::Invariant(
            "Attempt effect has multiple chain tails".into(),
        ));
    }
    rows.first()
        .map(|raw| {
            serde_json::from_str(raw)
                .map_err(|e| StoreError::Invariant(format!("invalid Attempt record: {e}")))
        })
        .transpose()
}
impl NolaStore {
    pub fn model_attempt(&self, id: &str) -> StoreResult<Option<ModelAttemptRecord>> {
        require_id("attempt.id", id)?;
        read(&self.connection, id)
    }
    /// Includes a terminal tail, so an existing effect cannot silently resurrect.
    pub fn active_model_attempt_for_effect(
        &self,
        id: &str,
    ) -> StoreResult<Option<ModelAttemptRecord>> {
        require_id("attempt.effect_id", id)?;
        tail(&self.connection, id)
    }
}

pub(super) fn apply(
    connection: &Connection,
    run_id: &str,
    mutations: &[ModelAttemptMutation],
) -> StoreResult<()> {
    for mutation in mutations {
        let record = &mutation.record;
        for (name, id) in [
            ("id", &record.id),
            ("run", &record.run_id),
            ("invocation", &record.invocation_id),
            ("effect", &record.effect_id),
            ("text_part", &record.text_part_id),
            ("reasoning_part", &record.reasoning_part_id),
        ] {
            require_id(name, id)?;
        }
        if record.run_id != run_id {
            return Err(StoreError::Conflict(
                "Attempt belongs to another Run".into(),
            ));
        }
        let raw =
            serde_json::to_string(record).map_err(|e| StoreError::Invariant(e.to_string()))?;
        if raw.len() > 65536 {
            return Err(StoreError::Invariant(
                "Attempt metadata exceeds 64KiB".into(),
            ));
        }
        let state =
            serde_json::to_value(record.state).map_err(|e| StoreError::Invariant(e.to_string()))?;
        let state = state.as_str().expect("Attempt state serializes as string");
        match mutation.expected_revision {
            None => {
                if record.revision != 1 {
                    return Err(StoreError::Invariant(
                        "new Attempt revision must be one".into(),
                    ));
                }
                if let Some(existing) = read(connection, &record.id)? {
                    if existing == *record {
                        continue;
                    }
                    return Err(StoreError::Conflict(
                        "Attempt identity already exists with different facts".into(),
                    ));
                }
                if let Some(previous) = &record.predecessor_id {
                    let previous = read(connection, previous)?.ok_or_else(|| {
                        StoreError::Conflict("Attempt predecessor is missing".into())
                    })?;
                    if previous.run_id != record.run_id
                        || previous.effect_id != record.effect_id
                        || previous.invocation_id != record.invocation_id
                        || !previous.state.is_terminal()
                    {
                        return Err(StoreError::Conflict(
                            "Attempt predecessor ownership or terminal state mismatch".into(),
                        ));
                    }
                } else if tail(connection, &record.effect_id)?.is_some() {
                    return Err(StoreError::Conflict(
                        "existing effect requires an explicit Attempt predecessor".into(),
                    ));
                }
                connection.execute("INSERT INTO run_attempts(id,run_id,invocation_id,effect_id,text_part_id,reasoning_part_id,predecessor_id,state,revision,record_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![record.id,record.run_id,record.invocation_id,record.effect_id,record.text_part_id,record.reasoning_part_id,record.predecessor_id,state,record.revision,raw])?;
            }
            Some(expected) => {
                let previous = read(connection, &record.id)?
                    .ok_or_else(|| StoreError::Conflict("Attempt is missing".into()))?;
                if previous.revision != expected || expected.checked_add(1) != Some(record.revision)
                {
                    return Err(StoreError::Conflict("Attempt revision changed".into()));
                }
                if previous.run_id != record.run_id
                    || previous.invocation_id != record.invocation_id
                    || previous.effect_id != record.effect_id
                    || previous.text_part_id != record.text_part_id
                    || previous.reasoning_part_id != record.reasoning_part_id
                    || previous.predecessor_id != record.predecessor_id
                    || previous.created_at_ms != record.created_at_ms
                {
                    return Err(StoreError::Conflict(
                        "Attempt immutable ownership changed".into(),
                    ));
                }
                connection.execute("UPDATE run_attempts SET state=?2,revision=?3,record_json=?4 WHERE id=?1 AND revision=?5",params![record.id,state,record.revision,raw,expected])?;
            }
        }
        if record.state.is_terminal() {
            connection.execute(
                "UPDATE runs SET active_attempt_id=NULL WHERE id=?1 AND active_attempt_id=?2",
                params![run_id, record.id],
            )?;
        } else {
            let changed=connection.execute("UPDATE runs SET active_attempt_id=?2 WHERE id=?1 AND (active_attempt_id IS NULL OR active_attempt_id=?2)",params![run_id,record.id])?;
            if changed != 1 {
                return Err(StoreError::Conflict(
                    "Run already has another active Attempt".into(),
                ));
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
    let Some(id) = &effect.model_attempt_id else {
        return Ok(());
    };
    let record = read(connection, id)?
        .ok_or_else(|| StoreError::Conflict("effect references missing Attempt".into()))?;
    if record.run_id != run_id
        || record.effect_id != effect.effect_id
        || tail(connection, &effect.effect_id)?
            .as_ref()
            .map(|r| r.id.as_str())
            != Some(id.as_str())
    {
        return Err(StoreError::Conflict(
            "effect Attempt ownership or chain tail mismatch".into(),
        ));
    }
    let existing: Option<String> = connection
        .query_row(
            "SELECT attempt_id FROM run_attempt_deliveries WHERE request_id=?1",
            params![effect.request_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(existing) = existing {
        let previous = read(connection, &existing)?
            .ok_or_else(|| StoreError::Invariant("delivery Attempt missing".into()))?;
        if previous.run_id != run_id || previous.effect_id != effect.effect_id {
            return Err(StoreError::Conflict(
                "delivery request belongs to another effect".into(),
            ));
        }
    }
    connection.execute("INSERT INTO run_attempt_deliveries(request_id,attempt_id) VALUES(?1,?2) ON CONFLICT(request_id) DO UPDATE SET attempt_id=excluded.attempt_id",params![effect.request_id,id])?;
    Ok(())
}

/// Render repository-owned initial attempt facts. Initial Core lifecycle
/// envelopes are persisted separately and do not duplicate these mutations.
pub(super) fn append_initial_events(
    connection: &Connection,
    run_id: &str,
    mutations: &[ModelAttemptMutation],
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
        connection.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES(?1,?2,?3,'model_attempt_changed',?4,?5)",params![format!("model-attempt:{}:{}",record.id,record.revision),run_id,sequence,payload,at_ms])?;
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
