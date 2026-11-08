// SPDX-License-Identifier: Apache-2.0
//! Terminal delivery facts are guarded by the existing task progress CAS.
//! Receipts never grant input authority. The durable queue contains raw input
//! and must not be projected into model-visible task metadata or diagnostics.
use super::*;
use nola_domain::terminal_input::{TerminalInputReceipt, TERMINAL_INPUT_RECEIPTS_METADATA_KEY as KEY};
use nola_store_port::runtime_task::RuntimeTaskProgressMutation;
use std::collections::BTreeMap;
use nola_domain::terminal_input::{TerminalInputRequest, TerminalInputPhase,
    TERMINAL_INPUT_QUEUE_METADATA_KEY as QUEUE};

impl NolaStore {
    pub fn submit_terminal_input(&mut self, input: &TerminalInputRequest, now_ms: i64) -> StoreResult<TerminalInputReceipt> {
        let receipt = input.prepared_receipt().map_err(StoreError::Invariant)?;
        if now_ms < 0 { return Err(StoreError::Invariant("invalid terminal input time".into())); }
        let tx = self.write_transaction()?;
        let raw: Option<String> = tx.query_row(
            "SELECT t.metadata_json FROM durable_tasks t JOIN runtime_task_attempts a ON a.task_id=t.id
             WHERE t.id=?1 AND t.kind='shell_process' AND t.owner_session_id=?2 AND t.status='running'
               AND t.cancel_requested=0 AND t.updated_at_ms<=?5 AND t.lease_until_ms>?5
               AND a.id=?3 AND a.worker_generation=?4 AND a.state='leased'
               AND a.worker_id=t.worker_id AND a.lease_token=t.ownership_token
               AND a.lease_expires_at_ms=t.lease_until_ms",
            params![input.identity.task_id,input.identity.owner_session_id,input.identity.attempt_id,
                input.identity.worker_generation,now_ms], |row| row.get(0)).optional()?;
        let raw = raw.ok_or_else(|| StoreError::Invariant("terminal input requires the current live owned attempt".into()))?;
        let mut metadata: serde_json::Value = serde_json::from_str(&raw)?;
        if metadata.get("terminal").is_none_or(serde_json::Value::is_null) {
            return Err(StoreError::Invariant("task has no interactive terminal".into()));
        }
        let terminal: nola_domain::terminal_input::TerminalSessionConfiguration =
            serde_json::from_value(metadata["terminal"].clone())?;
        terminal.validate().map_err(StoreError::Invariant)?;
        if terminal.host_id != input.identity.host_id {
            return Err(StoreError::Invariant("terminal input belongs to another Host".into()));
        }
        let mut queue: BTreeMap<String,TerminalInputRequest> = metadata.get(QUEUE).cloned()
            .map(serde_json::from_value).transpose()?.unwrap_or_default();
        let mut history = receipts(&raw)?;
        if let Some(stored) = history.get(&input.identity.request_id) {
            if !stored.matches_input(&input.identity,input.text.as_bytes()) {
                return Err(StoreError::Invariant("terminal input receipt content differs".into()));
            }
            stored.validate().map_err(StoreError::Invariant)?;
            if stored.phase != TerminalInputPhase::Delivered
                && queue.get(&input.identity.request_id) != Some(input) {
                return Err(StoreError::Invariant("pending terminal input payload differs or is missing".into()));
            }
            return Ok(stored.clone());
        }
        if queue.contains_key(&input.identity.request_id) {
            return Err(StoreError::Invariant("terminal input has no delivery receipt".into()));
        }
        // A terminal has one ordered input stream. Do not let independent
        // callers reorder chunks or append to a delivery of unknown outcome.
        if history.values().any(|entry| !matches!(entry.phase,TerminalInputPhase::Delivered)) {
            return Err(StoreError::Invariant("terminal has unresolved input; inspect its receipt before submitting more".into()));
        }
        queue.insert(input.identity.request_id.clone(),input.clone());
        history.insert(input.identity.request_id.clone(),receipt.clone());
        metadata[QUEUE] = serde_json::to_value(queue)?;
        metadata[KEY] = serde_json::to_value(history)?;
        tx.execute("UPDATE durable_tasks SET metadata_json=?2,updated_at_ms=?3 WHERE id=?1",
            params![input.identity.task_id,metadata.to_string(),now_ms])?;
        tx.commit()?;
        Ok(receipt)
    }
}

fn receipts(raw: &str) -> StoreResult<BTreeMap<String, TerminalInputReceipt>> {
    let value: serde_json::Value = serde_json::from_str(raw)?;
    match value.get(KEY) {
        None => Ok(BTreeMap::new()),
        Some(value) => Ok(serde_json::from_value(value.clone())?),
    }
}

pub(super) fn validate_progress(connection: &Connection, mutation: &RuntimeTaskProgressMutation) -> StoreResult<()> {
    // Read authoritative predecessor inside the caller's write transaction;
    // expected_metadata_json remains part of the subsequent CAS, not evidence.
    let row: Option<(String, Option<String>, String)> = connection.query_row(
        "SELECT kind,owner_session_id,metadata_json FROM durable_tasks WHERE id=?1",
        [&mutation.task_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    let Some((kind, owner, raw)) = row else { return Ok(()); }; // Existing task CAS rejects absence.
    validate_queue_progress(&raw, &mutation.metadata_json)?;
    let previous = receipts(&raw)?;
    let next = receipts(&mutation.metadata_json)?;
    if previous.is_empty() && next.is_empty() { return Ok(()); }
    if kind != "shell_process" || owner.as_deref().is_none_or(str::is_empty) {
        return Err(StoreError::Invariant("terminal input requires an owned shell task".into()));
    }
    for request_id in previous.keys() {
        if !next.contains_key(request_id) {
            return Err(StoreError::Invariant("task progress cannot erase terminal input receipts".into()));
        }
    }
    for (request_id, receipt) in &next {
        receipt.validate().map_err(StoreError::Invariant)?;
        if request_id != &receipt.identity.request_id || receipt.identity.task_id != mutation.task_id
            || Some(receipt.identity.owner_session_id.as_str()) != owner.as_deref() {
            return Err(StoreError::Invariant("terminal input receipt belongs to another owner".into()));
        }
        let old = previous.get(request_id);
        if old.is_none() {
            return Err(StoreError::Invariant("only input submission may prepare a terminal receipt".into()));
        }
        receipt.follows(old).map_err(StoreError::Invariant)?;
        if old == Some(receipt) { continue; }
        if receipt.identity.attempt_id != mutation.attempt_id
            || receipt.identity.worker_generation != mutation.worker_generation {
            return Err(StoreError::Invariant("terminal input receipt belongs to another execution generation".into()));
        }
    }
    Ok(())
}

pub(super) fn require_unchanged(expected: &str, supplied: &str) -> StoreResult<()> {
    require_queue_unchanged(expected, supplied)?;
    if receipts(expected)? != receipts(supplied)? {
        return Err(StoreError::Invariant("final progress cannot rewrite terminal input receipts".into()));
    }
    Ok(())
}

fn require_queue_unchanged(expected: &str, supplied: &str) -> StoreResult<()> {
    let previous: serde_json::Value = serde_json::from_str(expected)?;
    let next: serde_json::Value = serde_json::from_str(supplied)?;
    if previous.get(QUEUE) != next.get(QUEUE) {
        return Err(StoreError::Invariant("worker progress cannot change admitted terminal input".into()));
    }
    Ok(())
}

fn validate_queue_progress(expected: &str, supplied: &str) -> StoreResult<()> {
    let before: serde_json::Value = serde_json::from_str(expected)?;
    let after: serde_json::Value = serde_json::from_str(supplied)?;
    let queue = |value: &serde_json::Value| -> StoreResult<BTreeMap<String, TerminalInputRequest>> {
        Ok(value.get(QUEUE).cloned().map(serde_json::from_value).transpose()?.unwrap_or_default())
    };
    let previous = queue(&before)?;
    let next = queue(&after)?;
    let next_receipts = receipts(supplied)?;
    for (id, input) in &next {
        if previous.get(id) != Some(input) {
            return Err(StoreError::Invariant("worker cannot add or replace terminal input".into()));
        }
    }
    for (id, input) in &previous {
        if next.contains_key(id) { continue; }
        if !next_receipts.get(id).is_some_and(|receipt| {
            receipt.phase == TerminalInputPhase::Delivered
                && receipt.matches_input(&input.identity, input.text.as_bytes())
        }) {
            return Err(StoreError::Invariant("only acknowledged terminal payloads may be removed".into()));
        }
    }
    Ok(())

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_expired_or_replaced_attempts_cannot_queue_terminal_input() {
        for fault in ["none", "cancel", "terminal", "expired", "generation", "owner", "host", "attempt"] {
            let dir = tempfile::tempdir().unwrap();
            let mut store = NolaStore::open_archive_mode(&dir.path().join("terminal.sqlite"), false).unwrap();
            let metadata = serde_json::json!({"terminal":{"size":{"rows":24,"columns":80},"host_id":"host"}}).to_string();
            store.connection.execute("INSERT INTO durable_tasks(id,kind,status,title,project_root,created_at_ms,updated_at_ms,
                owner_session_id,worker_id,ownership_token,lease_until_ms,metadata_json)
                VALUES('task','shell_process','running','fixture','/fixture',1,1,'session','worker','token',100,?1)", [&metadata]).unwrap();
            store.connection.execute_batch("INSERT INTO runtime_task_attempts VALUES('attempt','task',1,'worker','generation','token',100,'leased',1,1);").unwrap();
            let mut input = TerminalInputRequest { identity:nola_domain::terminal_input::TerminalInputIdentity {
                host_id:"host".into(),owner_session_id:"session".into(),task_id:"task".into(),attempt_id:"attempt".into(),
                worker_generation:"generation".into(),request_id:"request".into() },text:"private input\n".into() };
            match fault {
                "cancel" => { store.connection.execute("UPDATE durable_tasks SET cancel_requested=1", []).unwrap(); }
                "terminal" => { store.connection.execute("UPDATE durable_tasks SET status='cancelled'", []).unwrap(); }
                "generation" => input.identity.worker_generation="replacement".into(),
                "owner" => input.identity.owner_session_id="another-session".into(),
                "host" => input.identity.host_id="another-host".into(),
                "attempt" => input.identity.attempt_id="another-attempt".into(),
                _ => {}
            }
            let result = store.submit_terminal_input(&input, if fault == "expired" {100} else {2});
            if fault == "none" {
                assert_eq!(result.unwrap().phase, TerminalInputPhase::Prepared);
                assert_eq!(store.submit_terminal_input(&input, 3).unwrap().phase, TerminalInputPhase::Prepared);
            } else {
                assert!(result.is_err(), "{fault}");
                let after:String = store.connection.query_row("SELECT metadata_json FROM durable_tasks WHERE id='task'", [], |row|row.get(0)).unwrap();
                assert_eq!(after,metadata,"Rejected input mutated state: {fault}");
            }
        }
    }

    #[test]
    fn queue_payload_is_removed_only_after_matching_delivery() {
        let input = TerminalInputRequest {
            identity: nola_domain::terminal_input::TerminalInputIdentity {
                host_id: "host".into(), owner_session_id: "session".into(),
                task_id: "task".into(), attempt_id: "attempt".into(),
                worker_generation: "generation".into(), request_id: "request".into(),
            },
            text: "secret input\n".into(),
        };
        let mut receipt = input.prepared_receipt().unwrap();
        let before = serde_json::json!({(QUEUE): {"request": input}, (KEY): {"request": receipt}}).to_string();
        let pending = serde_json::json!({(QUEUE): {}, (KEY): {"request": receipt}}).to_string();
        assert!(validate_queue_progress(&before, &pending).is_err());
        receipt.phase = TerminalInputPhase::Delivered;
        receipt.acknowledged_bytes = receipt.size_bytes;
        let delivered = serde_json::json!({(QUEUE): {}, (KEY): {"request": receipt}}).to_string();
        validate_queue_progress(&before, &delivered).unwrap();
        assert!(validate_queue_progress(&delivered, &before).is_err());
        receipt.identity.request_id = "different".into();
        let wrong = serde_json::json!({(QUEUE): {}, (KEY): {"request": receipt}}).to_string();
        assert!(validate_queue_progress(&before, &wrong).is_err());
    }

    #[test]
    fn progress_uses_database_predecessor_and_rejects_foreign_execution() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE durable_tasks(id TEXT,kind TEXT,owner_session_id TEXT,metadata_json TEXT);
            INSERT INTO durable_tasks VALUES('task','shell_process','session','{}');").unwrap();
        let identity = nola_domain::terminal_input::TerminalInputIdentity {
            host_id:"host".into(), owner_session_id:"session".into(), task_id:"task".into(),
            attempt_id:"attempt".into(), worker_generation:"generation".into(), request_id:"request".into(),
        };
        let receipt = TerminalInputReceipt::prepare(identity, b"input\n").unwrap();
        let stored = serde_json::json!({(KEY):{"request":receipt}}).to_string();
        let mut mutation = RuntimeTaskProgressMutation {
            task_id:"task".into(), attempt_id:"attempt".into(), expected_attempt_revision:1,
            worker_generation:"generation".into(), lease_token:"token".into(), current_expiry_ms:100,
            expected_updated_at_ms:1, expected_metadata_json:"{}".into(), progress_summary:None,
            last_activity:None, activity:None, metadata_json:stored.clone(), now_ms:2,
        };
        assert!(validate_progress(&connection, &mutation).is_err());
        connection.execute("UPDATE durable_tasks SET metadata_json=?1", [&stored]).unwrap();
        let mut first_write = receipt.clone();
        first_write.phase = nola_domain::terminal_input::TerminalInputPhase::Writing;
        mutation.metadata_json = serde_json::json!({(KEY):{"request":first_write}}).to_string();
        validate_progress(&connection, &mutation).unwrap();
        mutation.worker_generation = "other".into();
        assert!(validate_progress(&connection, &mutation).is_err());
        mutation.worker_generation = "generation".into();
        connection.execute("UPDATE durable_tasks SET metadata_json=?1", [&stored]).unwrap();
        // A fabricated empty predecessor cannot erase an already committed receipt.
        mutation.metadata_json = "{}".into();
        assert!(validate_progress(&connection, &mutation).is_err());
        let mut writing = receipt.clone();
        writing.phase = nola_domain::terminal_input::TerminalInputPhase::Writing;
        mutation.metadata_json = serde_json::json!({(KEY):{"request":writing}}).to_string();
        validate_progress(&connection, &mutation).unwrap();
        connection.execute("UPDATE durable_tasks SET owner_session_id='foreign'", []).unwrap();
        assert!(validate_progress(&connection, &mutation).is_err());
    }

    #[test]
    fn malformed_or_erased_receipts_are_not_empty_state() {
        assert!(receipts("{}").unwrap().is_empty());
        for value in [serde_json::Value::Null, serde_json::json!([]), serde_json::json!({"request":{}})] {
            assert!(receipts(&serde_json::json!({(KEY):value}).to_string()).is_err());
        }
        let identity = nola_domain::terminal_input::TerminalInputIdentity {
            host_id:"host".into(), owner_session_id:"session".into(), task_id:"task".into(),
            attempt_id:"attempt".into(), worker_generation:"generation".into(), request_id:"request".into(),
        };
        let receipt = TerminalInputReceipt::prepare(identity, b"input\n").unwrap();
        let stored = serde_json::json!({(KEY):{"request":receipt}}).to_string();
        require_unchanged(&stored, &stored).unwrap();
        assert!(require_unchanged(&stored, "{}").is_err());
    }
}
