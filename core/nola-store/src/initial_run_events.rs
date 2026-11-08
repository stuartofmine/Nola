// SPDX-License-Identifier: Apache-2.0
//! Admission control facts share the Run's creation transaction and sequence.
use crate::{params, Connection, StoreError, StoreResult};
use nola_store_port::run_creation::InitialCoreRunEvent;

pub(super) fn append(
    connection: &Connection,
    run_id: &str,
    events: &[InitialCoreRunEvent],
    at_ms: i64,
    checkpoint_cursor: i64,
    checkpoint_revision: i64,
) -> StoreResult<()> {
    if events.is_empty() {
        return Ok(());
    }
    if connection.is_autocommit() {
        return Err(StoreError::Invariant(
            "initial control facts require their admission unit of work".into(),
        ));
    }
    // Admission currently starts its Store clock at the Core baseline revision.
    // Do not infer Core identity from that convention: subsequent Store commit
    // revisions and Core reduction revisions are independent clocks.
    let (session, state_session, core_revision, core_cursor): (String, String, i64, i64) =
        connection.query_row(
            "SELECT json_extract(b.checkpoint_json,'$.session_id'),
                json_extract(b.checkpoint_json,'$.state.session_id'),
                json_extract(b.checkpoint_json,'$.revision'),
                json_extract(b.checkpoint_json,'$.last_event_cursor.sequence')
             FROM runs r JOIN core_checkpoints h ON h.conversation_id=r.conversation_id
             JOIN core_checkpoint_bodies b ON b.conversation_id=h.conversation_id
             WHERE r.id=?1 AND h.revision=?2 AND h.event_cursor=?3",
            params![run_id, checkpoint_revision, checkpoint_cursor],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    if session != state_session || core_revision < 0 || core_cursor != checkpoint_cursor {
        return Err(StoreError::Invariant(
            "initial control facts have no exact Core baseline".into(),
        ));
    }
    let mut sequence: i64 = connection.query_row(
        "SELECT next_event_sequence FROM runs WHERE id=?1",
        [run_id],
        |row| row.get(0),
    )?;
    let mut previous = None;
    for event in events {
        crate::require_id("initial Core session", &event.core_session_id)?;
        if let Some(turn) = &event.core_turn_id {
            crate::require_id("initial Core turn", turn)?;
        }
        if event.core_cursor == 0
            || i64::try_from(event.core_cursor).map_or(true, |cursor| cursor > checkpoint_cursor)
            || i64::try_from(event.core_revision).map_or(true, |revision| revision > core_revision)
            || event.core_session_id != session
            || previous.is_some_and(|cursor| cursor >= event.core_cursor)
            || events
                .first()
                .is_some_and(|first| first.core_session_id != event.core_session_id)
            || event.payload_json.len() > 16 * 1024
            || !matches!(
                event.event_kind.as_str(),
                "assistant_segment_started"
                    | "assistant_effect_bound"
                    | "assistant_activity_bound"
                    | "session_started"
                    | "turn_started"
                    | "host_catalog_updated"
                    | "session_configuration_updated"
                    | "context_updated"
                    | "compaction_requested"
                    | "compaction_completed"
                    | "turn_completed"
                    | "input_accepted"
                    | "input_applied"
                    | "input_withdrawn"
                    | "user_input_wait_changed"
                    | "requirement_scope_changed"
                    | "requirement_assessed"
                    | "run_result_decided"
                    | "runtime_decision_fact_observed"
            )
        {
            return Err(StoreError::Invariant(
                "invalid initial Core control fact".into(),
            ));
        }
        let mut payload: serde_json::Value = serde_json::from_str(&event.payload_json)
            .map_err(|error| StoreError::Invariant(error.to_string()))?;
        if matches!(event.event_kind.as_str(), "compaction_completed" | "turn_completed") {
            let maintenance_noop: bool = connection.query_row(
                "SELECT kind='maintenance' AND status='sealed' FROM runs WHERE id=?1",
                [run_id], |row| row.get(0),
            )?;
            let immediate_result: bool = if event.event_kind == "turn_completed"
                && payload.get("turn_id").and_then(serde_json::Value::as_str) == Some(run_id)
                && event.core_turn_id.as_deref() == Some(run_id)
            {
                connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM runs r JOIN core_checkpoint_bodies b ON b.conversation_id=r.conversation_id
                     WHERE r.id=?1 AND r.kind='conversation'
                     AND json_extract(b.checkpoint_json,'$.state.execution.active_turn.turn_id')=?1
                     AND json_extract(b.checkpoint_json,'$.state.execution.active_turn.status')='completed'
                     AND EXISTS(SELECT 1 FROM run_events e WHERE e.run_id=r.id AND e.event_kind='run_result_decided'))",
                    [run_id], |row| row.get(0),
                )?
            } else { false };
            if !maintenance_noop && !immediate_result {
                return Err(StoreError::Invariant("initial terminal facts require verified maintenance no-op admission".into()));
            }
        }
        if payload.get("type").and_then(serde_json::Value::as_str)
            != Some(event.event_kind.as_str())
        {
            return Err(StoreError::Invariant(
                "initial Core event kind mismatch".into(),
            ));
        }
        if event.event_kind == "turn_started"
            && (payload
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                != Some(event.core_session_id.as_str())
                || payload.get("turn_id").and_then(serde_json::Value::as_str) != Some(run_id)
                || event.core_turn_id.as_deref() != Some(run_id))
        {
            return Err(StoreError::Invariant(
                "initial TurnStarted owner mismatch".into(),
            ));
        }
        if event.event_kind == "input_applied" {
            let applications = payload.get("applications").and_then(serde_json::Value::as_array)
                .ok_or_else(|| StoreError::Invariant("initial input applications are missing".into()))?;
            if payload.get("turn_id").and_then(serde_json::Value::as_str) != Some(run_id)
                || event.core_turn_id.as_deref() != Some(run_id)
                || applications.is_empty()
                || applications.iter().any(|application| application.get("destination_turn_id")
                    .and_then(serde_json::Value::as_str) != Some(run_id)) {
                return Err(StoreError::Invariant("initial input application owner mismatch".into()));
            }
        }
        payload["core_envelope"] = serde_json::to_value(nola_domain::core_event::CoreEventSource {
            session_id: event.core_session_id.clone(), turn_id: event.core_turn_id.clone(),
            cursor: nola_domain::core_event::EventCursor { sequence: event.core_cursor }, revision: event.core_revision,
        })?;
        connection.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6)",
            params![format!("core:{run_id}:{}",event.core_cursor),run_id,sequence,event.event_kind,payload.to_string(),at_ms])?;
        crate::input_facts::index_event(connection,&format!("core:{run_id}:{}",event.core_cursor))?;
        crate::user_input_wait::index_event(connection,&format!("core:{run_id}:{}",event.core_cursor),None)?;
        crate::run_requirements::index_event(connection,&format!("core:{run_id}:{}",event.core_cursor),None)?;
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| StoreError::Invariant("Run event sequence overflow".into()))?;
        previous = Some(event.core_cursor);
    }
    connection.execute(
        "UPDATE runs SET next_event_sequence=?2 WHERE id=?1",
        params![run_id, sequence],
    )?;
    crate::user_input_wait::ensure_not_sealed_with_open_waits(connection,run_id)?;
    Ok(())
}
