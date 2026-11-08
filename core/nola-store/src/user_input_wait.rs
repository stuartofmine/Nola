// SPDX-License-Identifier: Apache-2.0
//! Durable question indices. Original Core events own question bodies; the
//! existing command receipt and checkpoint/outbox transaction own answer replay.
use crate::{params, Connection, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_domain::user_input_wait::{RespondToUserInputWait, UserInputWait, UserInputWaitState};
use nola_store_port::user_input_wait::{
    OwnedTaskUserInputWait, StoredUserInputWait, UserInputWaitRepository,
};
use nola_domain::core_event::CoreEventSource;
use sha2::{Digest, Sha256};
#[cfg(test)]
#[path = "user_input_wait_tests.rs"]
mod tests;

fn conflict() -> StoreError {
    StoreError::Conflict("user input wait identity, revision or state conflict".into())
}
fn invalid() -> StoreError {
    StoreError::Invariant("invalid user input wait provenance".into())
}
fn digest(event: &serde_json::Value) -> StoreResult<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(event)?)))
}
fn state(wait: &UserInputWait) -> &'static str {
    match wait.state {
        UserInputWaitState::Open => "open",
        UserInputWaitState::Resolved { .. } => "resolved",
        UserInputWaitState::Withdrawn { .. } => "withdrawn",
    }
}

fn load(c: &Connection, conversation: &str, id: &str) -> StoreResult<Option<StoredUserInputWait>> {
    let row:Option<(String,String,String,String,i64,String)>=c.query_row("SELECT h.conversation_id,h.run_id,e.payload_json,h.source_digest,h.revision,h.state
        FROM user_input_wait_heads h JOIN run_events e ON e.event_id=h.source_event_id AND e.run_id=h.run_id
        WHERE h.conversation_id=?1 AND h.wait_id=?2",params![conversation,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let Some((conversation_id, run_id, json, expected, revision, status)) = row else {
        return Ok(None);
    };
    let event: serde_json::Value = serde_json::from_str(&json)?;
    let wait: UserInputWait =
        serde_json::from_value(event.get("wait").cloned().ok_or_else(invalid)?)?;
    wait.validate().map_err(|_| invalid())?;
    if wait.wait_id != id
        || i64::try_from(wait.revision).ok() != Some(revision)
        || state(&wait) != status
        || digest(&event)? != expected
    {
        return Err(invalid());
    }
    Ok(Some(StoredUserInputWait {
        conversation_id,
        run_id,
        wait,
    }))
}

pub(super) fn index_event(
    c: &Connection,
    event_id: &str,
    response: Option<&RespondToUserInputWait>,
) -> StoreResult<()> {
    if c.is_autocommit() {
        return Err(invalid());
    }
    let (run,conversation,kind,json):(String,String,String,String)=c.query_row("SELECT e.run_id,r.conversation_id,e.event_kind,e.payload_json FROM run_events e JOIN runs r ON r.id=e.run_id WHERE e.event_id=?1",[event_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    if kind != "user_input_wait_changed" {
        return Ok(());
    }
    let payload: serde_json::Value = serde_json::from_str(&json)?;
    if payload.get("type").and_then(serde_json::Value::as_str) != Some(kind.as_str()) {
        return Err(invalid());
    }
    let wait: UserInputWait =
        serde_json::from_value(payload.get("wait").cloned().ok_or_else(invalid)?)?;
    wait.validate().map_err(|_| invalid())?;
    let envelope: CoreEventSource =
        serde_json::from_value(payload.get("core_envelope").cloned().ok_or_else(invalid)?)?;
    let revision = i64::try_from(envelope.revision).map_err(|_| invalid())?;
    let cursor = i64::try_from(envelope.cursor.sequence).map_err(|_| invalid())?;
    if cursor <= 0
        || envelope.session_id != wait.core_session_id
        || envelope.turn_id.as_deref() != Some(wait.turn_id.as_str())
        || wait.created_at_revision > envelope.revision
    {
        return Err(invalid());
    }
    let owned:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM assistant_turn_message_bindings WHERE conversation_id=?1 AND run_id=?2 AND turn_id=?3)",params![conversation,run,wait.turn_id],|r|r.get(0))?;
    let frontier:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM core_delivery_witnesses WHERE conversation_id=?1 AND json_extract(witness_json,'$.session_id')=?2
        AND json_extract(witness_json,'$.state.session_id')=?2 AND event_cursor>=?3 AND json_extract(witness_json,'$.core_revision')>=?4)",params![conversation,wait.core_session_id,cursor,revision],|r|r.get(0))?;
    if !owned || !frontier {
        return Err(invalid());
    }
    let old = load(c, &conversation, &wait.wait_id)?;
    let new_digest = digest(&payload)?;
    if let Some(old) = old {
        let old_event: String = c.query_row(
            "SELECT source_event_id FROM user_input_wait_heads WHERE wait_id=?1",
            [&wait.wait_id],
            |r| r.get(0),
        )?;
        if old_event == event_id && old.wait == wait {
            return Ok(());
        }
        if old.run_id != run
            || !old.wait.is_open()
            || wait.revision != old.wait.revision.checked_add(1).ok_or_else(invalid)?
            || wait.is_open()
        {
            return Err(conflict());
        }
        let mut fixed = wait.clone();
        fixed.revision = old.wait.revision;
        fixed.state = old.wait.state.clone();
        if fixed != old.wait {
            return Err(conflict());
        }
        if let UserInputWaitState::Resolved {
            command_id,
            response_digest,
        } = &wait.state
        {
            let answer = response.ok_or_else(invalid)?;
            if answer.command_id != *command_id
                || answer.wait_id != wait.wait_id
                || answer.expected_revision != old.wait.revision
                || answer.response_schema_digest != wait.response_schema_digest
                || answer.digest().map_err(|_| invalid())? != *response_digest
            {
                return Err(conflict());
            }
            old.wait
                .question_set
                .validate_response(&answer.response)
                .map_err(|_| invalid())?;
            c.execute(
                "INSERT INTO user_input_wait_responses VALUES(?1,?2,?3,?4,?5)",
                params![
                    wait.wait_id,
                    command_id,
                    response_digest,
                    wait.revision,
                    event_id
                ],
            )?;
        }
        let changed=c.execute("UPDATE user_input_wait_heads SET revision=?2,state=?3,source_event_id=?4,source_digest=?5,core_revision=?6,core_cursor=?7 WHERE wait_id=?1 AND revision=?8 AND state='open' AND core_cursor<?7",
            params![wait.wait_id,wait.revision,state(&wait),event_id,new_digest,revision,cursor,old.wait.revision])?;
        if changed != 1 {
            return Err(conflict());
        }
    } else {
        if !wait.is_open() || wait.revision != 1 {
            return Err(conflict());
        }
        c.execute(
            "INSERT INTO user_input_wait_heads VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                wait.wait_id,
                wait.core_session_id,
                conversation,
                run,
                wait.turn_id,
                wait.revision,
                state(&wait),
                wait.response_schema_digest,
                event_id,
                new_digest,
                revision,
                cursor
            ],
        )?;
    }
    Ok(())
}

pub(super) fn ensure_not_sealed_with_open_waits(c: &Connection, run: &str) -> StoreResult<()> {
    let forbidden:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM runs r JOIN user_input_wait_heads w ON w.run_id=r.id WHERE r.id=?1 AND w.state='open' AND r.status='sealed')",[run],|r|r.get(0))?;
    if forbidden {
        return Err(conflict());
    }
    Ok(())
}

pub(super) fn response_from_reduction(
    json: Option<&str>,
) -> StoreResult<Option<RespondToUserInputWait>> {
    let Some(json) = json else {
        return Ok(None);
    };
    let reduction: nola_store_port::run_effect::SubmissionReplayReduction =
        serde_json::from_str(json)?;
    let submission: serde_json::Value = serde_json::from_str(&reduction.submission_json)?;
    if submission.get("type").and_then(serde_json::Value::as_str)
        != Some("respond_to_user_input_wait")
    {
        return Ok(None);
    }
    Ok(Some(serde_json::from_value(
        submission.get("input").cloned().ok_or_else(invalid)?,
    )?))
}

impl UserInputWaitRepository for NolaStore {
    type Error = StoreError;
    fn user_input_wait_task_cancel_requested(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> StoreResult<bool> {
        crate::require_id("wait conversation", conversation_id)?;
        crate::require_id("wait run", run_id)?;
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM runtime_task_run_bindings b
            JOIN durable_tasks t ON t.id=b.task_id AND t.kind='agent'
                AND t.owner_session_id=b.owner_session_id AND t.child_session_id=b.child_session_id
            JOIN runs r ON r.id=b.run_id AND r.conversation_id=b.child_session_id
            WHERE b.child_session_id=?1 AND b.run_id=?2 AND t.cancel_requested=1)",
                params![conversation_id, run_id],
                |r| r.get(0),
            )
            .map_err(StoreError::from)
    }
    fn owned_task_user_input_waits(
        &self,
        owner_conversation_id: &str,
        task_id: Option<&str>,
    ) -> StoreResult<Vec<OwnedTaskUserInputWait>> {
        crate::require_id("wait task owner", owner_conversation_id)?;
        if let Some(task_id) = task_id {
            crate::require_id("wait task", task_id)?;
        }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let mut query = self.connection.prepare("SELECT t.id,w.conversation_id,w.wait_id FROM durable_tasks t
            JOIN runtime_task_run_bindings b ON b.task_id=t.id AND b.owner_session_id=t.owner_session_id
                AND b.child_session_id=t.child_session_id
            JOIN runs r ON r.id=b.run_id AND r.conversation_id=b.child_session_id
            JOIN user_input_wait_heads w ON w.run_id=b.run_id AND w.conversation_id=b.child_session_id
            WHERE t.kind='agent' AND t.owner_session_id=?1 AND (?2 IS NULL OR t.id=?2)
                AND t.cancel_requested=0 AND t.status NOT IN ('sealed','completed','failed','cancelled')
                AND r.status<>'sealed' AND w.state='open'
            ORDER BY t.created_at_ms,t.id,w.core_cursor,w.wait_id")?;
        let identities = query
            .query_map(params![owner_conversation_id, task_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        identities
            .into_iter()
            .map(|(task_id, conversation, id)| {
                let stored = load(&self.connection, &conversation, &id)?.ok_or_else(invalid)?;
                if !stored.wait.is_open() {
                    return Err(invalid());
                }
                Ok(OwnedTaskUserInputWait { task_id, stored })
            })
            .collect()
    }
    fn load_user_input_wait(
        &self,
        conversation_id: &str,
        wait_id: &str,
    ) -> StoreResult<Option<StoredUserInputWait>> {
        crate::require_id("wait conversation", conversation_id)?;
        crate::require_id("wait id", wait_id)?;
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        load(&self.connection, conversation_id, wait_id)
    }
    fn open_user_input_waits(
        &self,
        conversation_id: &str,
        run_id: Option<&str>,
    ) -> StoreResult<Vec<StoredUserInputWait>> {
        crate::require_id("wait conversation", conversation_id)?;
        if let Some(run) = run_id {
            crate::require_id("wait run", run)?;
        }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        open_waits(&self.connection, conversation_id, run_id)
    }
}

/// Caller owns the snapshot; used by both presentation and startup recovery.
pub(crate) fn open_waits(connection: &Connection, conversation_id: &str, run_id: Option<&str>) -> StoreResult<Vec<StoredUserInputWait>> {
        let mut query=connection.prepare("SELECT wait_id FROM user_input_wait_heads WHERE conversation_id=?1 AND (?2 IS NULL OR run_id=?2) AND state='open' ORDER BY core_cursor,wait_id")?;
        let ids = query
            .query_map(params![conversation_id, run_id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| load(connection, conversation_id, &id)?.ok_or_else(invalid))
            .collect()
}
