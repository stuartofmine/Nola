// SPDX-License-Identifier: Apache-2.0
//! Durable runtime-task progress, suspension, completion, recovery, and event journal.

use super::*;
use crate::runtime_task_shell_delivery_store::{validate_shell_dispatch_progress, shell_dispatch_allows_expired_recovery};

#[cfg(test)]
mod permission_budget_cas_tests {
    use super::*;
    use nola_domain::runtime_task_scheduling::*;
    use nola_store_port::runtime_task::RuntimeTaskPermissionBudgetMutation;

    fn fixture() -> (tempfile::TempDir, NolaStore, RuntimeTaskPermissionBudgetMutation) {
        let dir = tempfile::tempdir().unwrap();
        let store = NolaStore::open_archive_mode(&dir.path().join("budget.sqlite"), false).unwrap();
        let mut policy = RuntimeTaskScheduling {
            priority: RuntimeTaskPriority::Normal, queued_at_ms: 1,
            resource_budget: RuntimeTaskResourceBudget { max_wall_time_ms: Some(10), ..Default::default() },
            output_bytes: 0, resource_usage: Default::default(), budget_exhaustion: None,
        };
        let before = serde_json::json!({(SCHEDULING_METADATA_KEY): policy, "host":"fixture"}).to_string();
        policy.budget_exhaustion = Some("wall_time_ms".into());
        let after = serde_json::json!({(SCHEDULING_METADATA_KEY): policy, "host":"fixture"}).to_string();
        store.connection.execute("INSERT INTO durable_tasks(id,kind,status,title,project_root,created_at_ms,started_at_ms,updated_at_ms,metadata_json)
            VALUES('task','shell','waiting_permission','fixture','/fixture',1,2,3,?1)", [&before]).unwrap();
        store.connection.execute_batch("INSERT INTO runtime_task_attempts VALUES('attempt','task',1,'worker','generation','token',100,'permission_suspended',2,2);
            INSERT INTO permission_requests(request_id,task_id,child_session_id,capability,input_json,scope_rule,status,created_at_ms)
            VALUES('permission','task','child','tool','{}','fixture','pending',3);
            INSERT INTO runtime_task_permission_suspensions VALUES('suspension','task','attempt','permission','continuation','{}','pending',1,3,NULL);").unwrap();
        (dir, store, RuntimeTaskPermissionBudgetMutation {
            task_id:"task".into(), suspension_id:"suspension".into(), expected_suspension_revision:1,
            attempt_id:"attempt".into(), expected_attempt_revision:2, expected_updated_at_ms:3,
            expected_started_at_ms:Some(2), expected_metadata_json:before, metadata_json:after, now_ms:12,
        })
    }

    #[test]
    fn final_progress_and_terminal_commit_are_atomic() {
        for fault in ["none", "metadata", "timestamp", "event"] {
            let (_dir, mut store, _) = fixture();
            store.connection.execute_batch("UPDATE durable_tasks SET kind='shell_process',status='running',worker_id='worker',ownership_token='token',lease_until_ms=100;
                UPDATE runtime_task_attempts SET state='leased';").unwrap();
            let before = store.runtime_task("task").unwrap().unwrap();
            let attempt = store.runtime_task_current_attempt("task").unwrap().unwrap();
            let mut metadata: serde_json::Value = serde_json::from_str(&before.metadata_json).unwrap();
            metadata["final_cpu_ms"] = serde_json::json!(7);
            let progress = nola_store_port::runtime_task::RuntimeTaskFinalProgress {
                expected_updated_at_ms: if fault == "timestamp" { 2 } else { before.updated_at_ms },
                expected_metadata_json: if fault == "metadata" { "{}".into() } else { before.metadata_json.clone() },
                metadata_json: metadata.to_string(),
            };
            if fault == "event" {
                store.connection.execute_batch("CREATE TRIGGER reject_final_event BEFORE INSERT ON durable_task_events BEGIN SELECT RAISE(ABORT,'injected event failure'); END;").unwrap();
            }
            let result = store.finish_runtime_task_detailed_with_event(
                &RuntimeTaskLeaseWitness::from(&attempt), "completed", None, None, None, None, None,
                false, "finished", "completed", None, "{}", Some(&progress), 10);
            if fault == "none" {
                assert!(result.unwrap());
                let after = store.runtime_task("task").unwrap().unwrap();
                assert_eq!(after.status,"completed");
                assert_eq!(serde_json::from_str::<serde_json::Value>(&after.metadata_json).unwrap()["final_cpu_ms"],7);
                assert!(store.runtime_task_terminal_receipt("task").unwrap().is_some());
            } else {
                if fault == "event" { assert!(result.is_err()); } else { assert!(!result.unwrap()); }
                assert_eq!(store.runtime_task("task").unwrap().unwrap(),before);
                assert_eq!(store.runtime_task_current_attempt("task").unwrap().unwrap(),attempt);
                assert!(store.runtime_task_terminal_receipt("task").unwrap().is_none());
                let events:i64 = store.connection.query_row("SELECT COUNT(*) FROM durable_task_events",[],|r|r.get(0)).unwrap();
                assert_eq!(events,0);
            }
        }
    }

    #[test]
    fn permission_budget_cas_only_records_first_exhaustion() {
        let (_dir, mut store, mutation) = fixture();
        assert!(store.observe_runtime_task_permission_budget(&mutation).unwrap());
        assert!(!store.observe_runtime_task_permission_budget(&mutation).unwrap());
        let state: (String, String, i64) = store.connection.query_row(
            "SELECT status,metadata_json,updated_at_ms FROM durable_tasks WHERE id='task'", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(state, ("waiting_permission".into(), mutation.metadata_json, 12));
        let untouched: (String,i64,String,i64) = store.connection.query_row(
            "SELECT a.state,a.revision,s.state,s.revision FROM runtime_task_attempts a JOIN runtime_task_permission_suspensions s ON s.attempt_id=a.id", [],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(untouched, ("permission_suspended".into(),2,"pending".into(),1));
        let events:i64 = store.connection.query_row("SELECT COUNT(*) FROM durable_task_events WHERE kind='budget_exhausted'", [], |r|r.get(0)).unwrap();
        assert_eq!(events,1);
    }

    #[test]
    fn permission_budget_cas_refuses_competing_state_without_writes() {
        for change in [
            "UPDATE durable_tasks SET cancel_requested=1",
            "UPDATE durable_tasks SET status='running'",
            "UPDATE runtime_task_permission_suspensions SET revision=2",
            "UPDATE runtime_task_permission_suspensions SET state='resumed'",
            "UPDATE runtime_task_attempts SET revision=3",
            "UPDATE durable_tasks SET metadata_json='{}'",
            "UPDATE durable_tasks SET updated_at_ms=4",
            "UPDATE durable_tasks SET started_at_ms=1",
            "UPDATE durable_tasks SET ownership_token='other'",
        ] {
            let (_dir, mut store, mutation) = fixture();
            store.connection.execute_batch(change).unwrap();
            assert!(!store.observe_runtime_task_permission_budget(&mutation).unwrap(), "{change}");
            let events:i64 = store.connection.query_row("SELECT COUNT(*) FROM durable_task_events", [], |r|r.get(0)).unwrap();
            assert_eq!(events,0,"{change}");
        }
    }
}

fn current_question_notification(
    connection: &Connection,
    delivery: &nola_store_port::runtime_task::RuntimeTaskNotificationDelivery,
) -> StoreResult<bool> {
    if !delivery.occurrence_id.starts_with("user-input:") { return Ok(false); }
    let mut query = connection.prepare(
        "SELECT w.run_id,w.wait_id,w.revision,w.response_schema_digest,w.source_digest,e.payload_json
         FROM durable_tasks t
         JOIN runtime_task_run_bindings b ON b.task_id=t.id AND b.owner_session_id=t.owner_session_id
           AND b.child_session_id=t.child_session_id
         JOIN runs r ON r.id=b.run_id AND r.conversation_id=b.child_session_id
         JOIN user_input_wait_heads w ON w.run_id=r.id AND w.conversation_id=r.conversation_id
         JOIN run_events e ON e.event_id=w.source_event_id AND e.run_id=w.run_id
         WHERE t.id=?1 AND t.owner_session_id=?2 AND t.kind='agent' AND t.cancel_requested=0
           AND t.status NOT IN ('sealed','completed','failed','cancelled')
           AND r.status <> 'sealed' AND w.state='open'
           AND e.event_kind='user_input_wait_changed'"
    )?;
    let rows = query.query_map(params![delivery.task_id, delivery.owner_conversation_id], |r| {
        Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,u64>(2)?,
            r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?))
    })?;
    for row in rows {
        let (run, id, revision, schema, source_digest, json) = row?;
        if nola_store_port::runtime_task::user_input_notification_occurrence_id(&run, &id, revision, &schema)
            != delivery.occurrence_id { continue; }
        let event: serde_json::Value = serde_json::from_str(&json)?;
        let wait: nola_domain::user_input_wait::UserInputWait = serde_json::from_value(
            event.get("wait").cloned().ok_or_else(|| StoreError::Invariant("question event lacks wait".into()))?
        )?;
        use sha2::{Digest, Sha256};
        let actual_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&event)?));
        if wait.validate().is_err() || !wait.is_open() || wait.wait_id != id
            || wait.revision != revision || wait.response_schema_digest != schema || actual_digest != source_digest {
            return Err(StoreError::Invariant("question notification provenance is invalid".into()));
        }
        return Ok(true);
    }
    Ok(false)
}

/// Recognize only committed historical occurrences of this exact task owner.
/// This is negative presentation authority: it cannot ACK, answer, or grant.
fn historical_question_notification(
    connection: &Connection,
    delivery: &nola_store_port::runtime_task::RuntimeTaskNotificationDelivery,
) -> StoreResult<bool> {
    let Some(identity) = delivery.occurrence_id.strip_prefix("user-input:") else { return Ok(false); };
    let Ok((run_id, wait_id, revision, schema)) = serde_json::from_str::<(String, String, u64, String)>(identity) else { return Ok(false); };
    if nola_store_port::runtime_task::user_input_notification_occurrence_id(
        &run_id, &wait_id, revision, &schema
    ) != delivery.occurrence_id { return Ok(false); }
    let mut statement = connection.prepare(
        "SELECT e.run_id,e.payload_json,h.core_session_id,h.turn_id,h.core_cursor,h.core_revision
         FROM durable_tasks t
         JOIN runtime_task_run_bindings b ON b.task_id=t.id
           AND b.owner_session_id=t.owner_session_id AND b.child_session_id=t.child_session_id
         JOIN runs r ON r.id=b.run_id AND r.conversation_id=b.child_session_id
         JOIN user_input_wait_heads h ON h.run_id=r.id AND h.conversation_id=r.conversation_id
           AND h.wait_id=?4 AND h.response_schema_digest=?6 AND h.revision>=?5
         JOIN assistant_turn_message_bindings a ON a.run_id=r.id AND a.conversation_id=r.conversation_id
           AND a.turn_id=h.turn_id
         JOIN run_events e ON e.run_id=r.id
         WHERE t.id=?1 AND t.owner_session_id=?2 AND t.kind='agent'
           AND e.run_id=?3 AND e.event_kind='user_input_wait_changed'
           AND json_extract(e.payload_json,'$.wait.wait_id')=?4
           AND json_extract(e.payload_json,'$.wait.revision')=?5
           AND json_extract(e.payload_json,'$.wait.response_schema_digest')=?6"
    )?;
    let rows = statement.query_map(params![delivery.task_id, delivery.owner_conversation_id, run_id, wait_id, revision, schema], |row| {
        Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?,
            row.get::<_,String>(3)?, row.get::<_,u64>(4)?, row.get::<_,u64>(5)?))
    })?;
    for row in rows {
        let (run_id, raw, core_session_id, turn_id, core_cursor, core_revision) = row?;
        let event: serde_json::Value = serde_json::from_str(&raw)?;
        let wait: nola_domain::user_input_wait::UserInputWait = serde_json::from_value(
            event.get("wait").cloned().ok_or_else(|| StoreError::Invariant("question event lacks wait".into()))?
        )?;
        if nola_store_port::runtime_task::user_input_notification_occurrence_id(
            &run_id, &wait.wait_id, wait.revision, &wait.response_schema_digest
        ) != delivery.occurrence_id { continue; }
        wait.validate().map_err(|error| StoreError::Invariant(error.to_string()))?;
        let source: nola_domain::core_event::CoreEventSource = serde_json::from_value(
            event.get("core_envelope").cloned().ok_or_else(|| StoreError::Invariant("question event lacks Core source".into()))?
        )?;
        if event.get("type").and_then(serde_json::Value::as_str) != Some("user_input_wait_changed")
            || wait.core_session_id != core_session_id || wait.turn_id != turn_id
            || source.session_id != core_session_id || source.turn_id.as_deref() != Some(turn_id.as_str())
            || source.cursor.sequence == 0 || source.cursor.sequence > core_cursor
            || source.revision > core_revision || wait.created_at_revision > source.revision
        {
            return Err(StoreError::Invariant("historical question notification provenance is invalid".into()));
        }
        if wait.is_open() { return Ok(true); }
    }
    Ok(false)
}

fn require_public_task_event_kind(kind: &str) -> StoreResult<()> {
    require_id("task_event.kind", kind)?;
    if matches!(kind, "notification_delivered" | "user_input_wait_opened" | "user_input_wait_resolved") {
        return Err(StoreError::Invariant("reserved task event requires its authoritative typed transition".into()));
    }
    Ok(())
}

fn revoke_runtime_task_lease(
    transaction: &crate::connection::WriteTransaction<'_>,
    lease: &RuntimeTaskLeaseWitness,
    allow_cancelled: bool,
    now_ms: i64,
) -> StoreResult<bool> {
    require_id("runtime_task_lease.task_id", &lease.task_id)?;
    require_runtime_task_claim_identity("attempt_id", &lease.attempt_id)?;
    require_runtime_task_claim_identity("worker_generation", &lease.worker_generation)?;
    require_runtime_task_claim_identity("lease_token", &lease.lease_token)?;
    if lease.expected_attempt_revision < 1 {
        return Err(StoreError::Invariant(
            "runtime task lease revision must be positive".into(),
        ));
    }
    Ok(transaction.execute(
        "UPDATE runtime_task_attempts SET state='superseded', revision=revision+1
         WHERE id=?1 AND task_id=?2 AND revision=?3 AND worker_generation=?4
           AND lease_token=?5 AND lease_expires_at_ms=?6
           AND state='leased' AND lease_expires_at_ms>?7 AND claimed_at_ms<=?7
           AND EXISTS (
             SELECT 1 FROM durable_tasks AS task
             WHERE task.id=runtime_task_attempts.task_id AND task.status='running'
               AND task.worker_id=runtime_task_attempts.worker_id
               AND task.ownership_token=runtime_task_attempts.lease_token
               AND task.lease_until_ms=runtime_task_attempts.lease_expires_at_ms
               AND task.lease_until_ms>?7 AND task.updated_at_ms<=?7
               AND (?8 OR task.cancel_requested=0)
           )",
        params![
            lease.attempt_id,
            lease.task_id,
            lease.expected_attempt_revision,
            lease.worker_generation,
            lease.lease_token,
            lease.current_expiry_ms,
            now_ms,
            allow_cancelled,
        ],
    )? == 1)
}

/// Recovery owns an expiry predicate, never a worker lease witness. Revoke old
/// leased attempts only while that predicate holds in the same transaction.
fn revoke_expired_runtime_task_attempts(
    transaction: &crate::connection::WriteTransaction<'_>,
    task_id: &str,
    now_ms: i64,
    include_suspended: bool,
) -> StoreResult<()> {
    transaction.execute(
        "UPDATE runtime_task_attempts SET state='superseded', revision=revision+1
         WHERE task_id=?1 AND state='leased'
           AND EXISTS (
             SELECT 1 FROM durable_tasks AS task WHERE task.id=?1
               AND (task.status='running' OR (?3 AND task.status IN ('waiting_permission','suspended')))
               AND (task.lease_until_ms IS NULL OR task.lease_until_ms<=?2)
           )",
        params![task_id, now_ms, include_suspended],
    )?;
    Ok(())
}

impl NolaStore {
    pub fn runtime_task_notification_delivered(&self, delivery: &nola_store_port::runtime_task::RuntimeTaskNotificationDelivery) -> StoreResult<bool> {
        self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_task_events WHERE task_id=?1 AND kind='notification_delivered'
             AND json_extract(detail_json,'$.owner_conversation_id') IS ?2
             AND json_extract(detail_json,'$.occurrence_id')=?3
             AND json_extract(detail_json,'$.consumer_id')=?4)",
            params![delivery.task_id, delivery.owner_conversation_id, delivery.occurrence_id, delivery.consumer_id],
            |r| r.get(0),
        ).map_err(StoreError::from)
    }

    pub fn acknowledge_runtime_task_delivery(&mut self, delivery: &nola_store_port::runtime_task::RuntimeTaskNotificationDelivery) -> StoreResult<nola_store_port::runtime_task::RuntimeTaskNotificationAcknowledgement> {
        use nola_store_port::runtime_task::RuntimeTaskNotificationAcknowledgement as Acknowledgement;
        if let Some(owner) = &delivery.owner_conversation_id { require_id("owner", owner)?; }
        for (name, value) in [("task", &delivery.task_id),
            ("occurrence", &delivery.occurrence_id), ("consumer", &delivery.consumer_id), ("command", &delivery.command_id)] {
            require_id(name, value)?;
        }
        let detail = serde_json::to_string(delivery)?;
        let tx = self.write_transaction()?;
        let owner_matches: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_tasks WHERE id=?1 AND owner_session_id IS ?2
             AND (owner_session_id IS NOT NULL OR kind='shell_process'))",
            params![delivery.task_id, delivery.owner_conversation_id], |row| row.get(0))?;
        if !owner_matches {
            return Err(StoreError::Invariant("notification belongs to an unknown task or another owner".into()));
        }
        let previous: Option<String> = tx.query_row(
            "SELECT detail_json FROM durable_task_events WHERE kind='notification_delivered'
             AND json_extract(detail_json,'$.consumer_id')=?1
             AND (json_extract(detail_json,'$.command_id')=?2 OR
                 (task_id=?3 AND json_extract(detail_json,'$.owner_conversation_id') IS ?4
                   AND json_extract(detail_json,'$.occurrence_id')=?5)) LIMIT 1",
            params![delivery.consumer_id, delivery.command_id, delivery.task_id, delivery.owner_conversation_id, delivery.occurrence_id], |r| r.get(0)).optional()?;
        if let Some(previous) = previous {
            if previous != detail { return Err(StoreError::Invariant("notification command identity conflict".into())); }
            return Ok(Acknowledgement::AlreadyDelivered);
        }
        let valid: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_tasks t WHERE t.id=?1 AND t.owner_session_id IS ?2
             AND (t.owner_session_id IS NOT NULL OR t.kind='shell_process')
             AND ((t.status IN ('sealed','completed','failed','cancelled') AND EXISTS(
                 SELECT 1 FROM runtime_task_terminal_receipts r WHERE r.task_id=t.id AND 'terminal:' || r.receipt_id=?3
                   AND json_extract(r.receipt_json,'$.task_id')=t.id
                   AND json_extract(r.receipt_json,'$.status')=t.status
                   AND json_extract(r.receipt_json,'$.owner_session_id')=COALESCE(t.owner_session_id,'')
                   AND json_extract(r.receipt_json,'$.ended_at_ms')=t.ended_at_ms))
              OR (t.status='waiting_permission' AND EXISTS(SELECT 1 FROM runtime_task_permission_suspensions s
                 JOIN permission_requests p ON p.request_id=s.permission_request_id WHERE s.task_id=t.id
                 AND s.state='pending' AND p.status='pending' AND 'permission:' || s.permission_request_id=?3))))",
            params![delivery.task_id, delivery.owner_conversation_id, delivery.occurrence_id], |r| r.get(0))?;
        if !valid && !current_question_notification(&tx, delivery)? {
            let historical_permission: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM runtime_task_permission_suspensions s
                 JOIN permission_requests p ON p.request_id=s.permission_request_id AND p.task_id=s.task_id
                 WHERE s.task_id=?1 AND 'permission:' || s.permission_request_id=?2)",
                params![delivery.task_id, delivery.occurrence_id], |row| row.get(0))?;
            if historical_permission || historical_question_notification(&tx, delivery)? {
                // A concurrent resolution invalidated a genuine observation.
                // Do not write a delivery record or mistake it for approval.
                return Ok(Acknowledgement::Superseded);
            }
            return Err(StoreError::Invariant("notification occurrence is no longer current or belongs to another owner".into()));
        }
        let changed = tx.execute(
            "INSERT INTO durable_task_events(task_id,kind,status,detail_json,created_at_ms)
             SELECT ?1,'notification_delivered','delivered',?2,CAST(strftime('%s','now') AS INTEGER)*1000
             WHERE NOT EXISTS(SELECT 1 FROM durable_task_events WHERE task_id=?1 AND kind='notification_delivered'
               AND json_extract(detail_json,'$.owner_conversation_id') IS ?3
               AND json_extract(detail_json,'$.occurrence_id')=?4 AND json_extract(detail_json,'$.consumer_id')=?5)",
            params![delivery.task_id, detail, delivery.owner_conversation_id, delivery.occurrence_id, delivery.consumer_id])?;
        tx.commit()?;
        Ok(if changed == 1 { Acknowledgement::Delivered } else { Acknowledgement::AlreadyDelivered })
    }
    /// Keep an uncertain command out of ordinary expiry/retry scheduling. The
    /// receipt identity is data for reconciliation, never retry authorization.
    pub fn suspend_runtime_task_for_command_reconciliation(
        &mut self,
        input: &nola_store_port::runtime_task::SuspendRuntimeTaskForCommandReconciliation,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskCommandReconciliation>> {
        use nola_store_port::runtime_task::RuntimeTaskCommandReconciliation;
        for (name, value) in [
            ("client_id", input.client_id.as_str()),
            ("request_id", input.request_id.as_str()),
            ("failure_code", input.failure_code.as_str()),
        ] {
            require_id(name, value)?;
        }
        if input.now_ms <= 0 || input.failure_code.len() > 128 {
            return Err(StoreError::Invariant(
                "invalid command reconciliation input".into(),
            ));
        }
        let transaction = self.write_transaction()?;
        let receipt: Option<String> = transaction
            .query_row(
                "SELECT status FROM command_receipts WHERE client_id=?1 AND request_id=?2",
                params![input.client_id, input.request_id],
                |row| row.get(0),
            )
            .optional()?;
        // Even a settled command still needs its worker continuation reconciled.
        // Settlement may race the lost-ack lookup; leaving the task running here
        // would let expiry replay an operation whose outcome is already durable.
        if receipt.as_deref().is_some_and(|status| {
            !matches!(status, "accepted" | "committed" | "rejected" | "conflict")
        }) {
            return Err(StoreError::Invariant(
                "invalid reconciliation receipt status".into(),
            ));
        }
        let metadata: Option<String> = transaction
            .query_row(
                "SELECT metadata_json FROM durable_tasks WHERE id=?1",
                [&input.lease.task_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(metadata) = metadata else {
            transaction.rollback()?;
            return Ok(None);
        };
        let mut metadata: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&metadata)?;
        if !revoke_runtime_task_lease(&transaction, &input.lease, true, input.now_ms)? {
            transaction.rollback()?;
            return Ok(None);
        }
        let pending = RuntimeTaskCommandReconciliation {
            task_id: input.lease.task_id.clone(),
            attempt_id: input.lease.attempt_id.clone(),
            client_id: input.client_id.clone(),
            request_id: input.request_id.clone(),
            failure_code: input.failure_code.clone(),
            state: "pending".into(),
            revision: 1,
            awaiting_at_ms: input.now_ms,
        };
        metadata.insert(
            "_nola_command_reconciliation".into(),
            serde_json::to_value(&pending)?,
        );
        let changed = transaction.execute(
            "UPDATE durable_tasks SET status='awaiting_reconciliation', worker_id=NULL,
             ownership_token=NULL,lease_until_ms=NULL,heartbeat_at_ms=NULL,
             progress_summary=?2,metadata_json=?3,updated_at_ms=?4,unread=1
             WHERE id=?1 AND status='running'",
            params![
                input.lease.task_id,
                input.progress_summary,
                serde_json::to_string(&metadata)?,
                input.now_ms
            ],
        )?;
        if changed != 1 {
            transaction.rollback()?;
            return Ok(None);
        }
        transaction.execute(
            "INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
             VALUES(?1,'command_reconciliation','awaiting_reconciliation',?2,?3,?4)",
            params![
                input.lease.task_id,
                input.progress_summary,
                serde_json::to_string(&pending)?,
                input.now_ms
            ],
        )?;
        transaction.commit()?;
        Ok(Some(pending))
    }

    pub fn pending_runtime_task_command_reconciliation(
        &self,
        task_id: &str,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskCommandReconciliation>> {
        require_id("command_reconciliation.task_id", task_id)?;
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let value: Option<String> = self.connection.query_row(
            "SELECT metadata_json FROM durable_tasks WHERE id=?1 AND status='awaiting_reconciliation'",
            [task_id], |row| row.get(0),
        ).optional()?;
        let Some(value) = value else {
            return Ok(None);
        };
        let metadata: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&value)?;
        let value = metadata
            .get("_nola_command_reconciliation")
            .ok_or_else(|| {
                StoreError::Invariant("awaiting task has no command reconciliation identity".into())
            })?;
        let pending: nola_store_port::runtime_task::RuntimeTaskCommandReconciliation =
            serde_json::from_value(value.clone())?;
        let exact: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runtime_task_attempts WHERE id=?1 AND task_id=?2 AND state='superseded')",
            params![pending.attempt_id,task_id], |row| row.get(0),
        )?;
        if pending.task_id != task_id
            || pending.state != "pending"
            || pending.revision != 1
            || pending.client_id.is_empty()
            || pending.request_id.is_empty()
            || !exact
        {
            return Err(StoreError::Invariant(
                "command reconciliation owner differs".into(),
            ));
        }
        Ok(Some(pending))
    }

    pub fn observe_runtime_task_permission_budget(
        &mut self,
        mutation: &nola_store_port::runtime_task::RuntimeTaskPermissionBudgetMutation,
    ) -> StoreResult<bool> {
        require_id("permission_budget.task_id", &mutation.task_id)?;
        require_id("permission_budget.suspension_id", &mutation.suspension_id)?;
        require_runtime_task_claim_identity("attempt_id", &mutation.attempt_id)?;
        if mutation.expected_suspension_revision < 1 || mutation.expected_attempt_revision < 1
            || mutation.expected_updated_at_ms < 0 || mutation.now_ms < mutation.expected_updated_at_ms
            || mutation.expected_started_at_ms.is_some_and(|start| start < 0 || start > mutation.now_ms)
        {
            return Err(StoreError::Invariant("invalid permission budget observation identity or time".into()));
        }
        let mut expected: serde_json::Value = serde_json::from_str(&mutation.expected_metadata_json)?;
        let supplied: serde_json::Value = serde_json::from_str(&mutation.metadata_json)?;
        let key = nola_domain::runtime_task_scheduling::SCHEDULING_METADATA_KEY;
        let mut policy: nola_domain::runtime_task_scheduling::RuntimeTaskScheduling =
            serde_json::from_value(expected.get(key).cloned()
                .ok_or_else(|| StoreError::Invariant("permission budget policy missing".into()))?)?;
        if policy.budget_exhaustion.is_some() {
            return Err(StoreError::Invariant("permission budget mutation must record first exhaustion".into()));
        }
        let failure = policy.execution_budget_failure(mutation.expected_started_at_ms, mutation.now_ms)
            .map_err(StoreError::Invariant)?
            .ok_or_else(|| StoreError::Invariant("permission budget has not been exhausted".into()))?;
        policy.budget_exhaustion = Some(failure.resource.code().into());
        expected[key] = serde_json::to_value(policy)?;
        if supplied != expected {
            return Err(StoreError::Invariant("permission budget mutation changes unrelated metadata".into()));
        }
        let tx = self.write_transaction()?;
        let changed = tx.execute(
            "UPDATE durable_tasks SET metadata_json=?9,updated_at_ms=?10
             WHERE id=?1 AND status='waiting_permission' AND cancel_requested=0
               AND updated_at_ms=?6 AND started_at_ms IS ?7 AND metadata_json=?8
               AND worker_id IS NULL AND ownership_token IS NULL AND lease_until_ms IS NULL
               AND EXISTS(SELECT 1 FROM runtime_task_permission_suspensions s
                 JOIN runtime_task_attempts a ON a.id=s.attempt_id AND a.task_id=s.task_id
                 WHERE s.task_id=?1 AND s.id=?2 AND s.revision=?3 AND s.state='pending'
                   AND a.id=?4 AND a.revision=?5 AND a.state='permission_suspended')
               AND NOT EXISTS(SELECT 1 FROM runtime_task_run_bindings b
                 LEFT JOIN runs r ON r.id=b.run_id AND r.conversation_id=b.child_session_id
                 LEFT JOIN core_checkpoints cp ON cp.conversation_id=r.conversation_id
                 LEFT JOIN core_delivery_witnesses w ON w.conversation_id=cp.conversation_id
                   AND w.checkpoint_revision=cp.revision AND w.event_cursor=cp.event_cursor
                 WHERE b.task_id=?1 AND (r.id IS NULL OR r.status='sealed'
                   OR w.witness_json IS NULL
                   OR json_extract(w.witness_json,'$.state.execution.active_turn.turn_id') IS NULL
                   OR json_extract(w.witness_json,'$.state.execution.active_turn.status') IS NULL
                   OR json_extract(w.witness_json,'$.state.execution.active_turn.status')
                     NOT IN ('awaiting_model','awaiting_tools','awaiting_permission','awaiting_input','compacting')))",
            params![mutation.task_id, mutation.suspension_id, mutation.expected_suspension_revision,
                mutation.attempt_id, mutation.expected_attempt_revision, mutation.expected_updated_at_ms,
                mutation.expected_started_at_ms, mutation.expected_metadata_json, mutation.metadata_json, mutation.now_ms],
        )? == 1;
        if !changed { tx.rollback()?; return Ok(false); }
        tx.execute("INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
            VALUES(?1,'budget_exhausted','waiting_permission',NULL,?2,?3)",
            params![mutation.task_id, serde_json::to_string(&failure)?, mutation.now_ms])?;
        tx.commit()?;
        Ok(true)
    }

    pub fn report_runtime_task_progress(
        &mut self,
        mutation: &nola_store_port::runtime_task::RuntimeTaskProgressMutation,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>> {
        require_id("runtime_task_progress.task_id", &mutation.task_id)?;
        require_runtime_task_claim_identity("attempt_id", &mutation.attempt_id)?;
        require_runtime_task_claim_identity("worker_generation", &mutation.worker_generation)?;
        require_runtime_task_claim_identity("lease_token", &mutation.lease_token)?;
        validate_json(
            "runtime_task_progress.expected_metadata_json",
            &mutation.expected_metadata_json,
        )?;
        validate_json(
            "runtime_task_progress.metadata_json",
            &mutation.metadata_json,
        )?;
        if let Some(activity)=&mutation.activity {
            if activity.summary.trim().is_empty() || activity.summary.chars().count()>4096 {
                return Err(StoreError::Invariant("invalid runtime activity summary".into()));
            }
            validate_json("runtime_task_progress.activity.detail_json",&activity.detail_json)?;
        }
        if mutation.expected_attempt_revision < 1
            || mutation.current_expiry_ms <= mutation.now_ms
            || mutation.now_ms < mutation.expected_updated_at_ms
        {
            return Err(StoreError::Invariant(
                "runtime task progress timestamp moved backwards".into(),
            ));
        }
        let transaction = self.write_transaction()?;
        validate_shell_dispatch_progress(&transaction, mutation)?;
        crate::terminal_input_store::validate_progress(&transaction, mutation)?;
        let attempt_changed = transaction.execute(
            "UPDATE runtime_task_attempts SET revision=revision+1
             WHERE id=?1 AND task_id=?2 AND revision=?3 AND worker_generation=?4
               AND lease_token=?5 AND lease_expires_at_ms=?6
               AND state='leased' AND lease_expires_at_ms>?7",
            params![
                mutation.attempt_id,
                mutation.task_id,
                mutation.expected_attempt_revision,
                mutation.worker_generation,
                mutation.lease_token,
                mutation.current_expiry_ms,
                mutation.now_ms
            ],
        )? == 1;
        if !attempt_changed {
            transaction.rollback()?;
            return Ok(None);
        }
        let task_changed = transaction.execute(
            "UPDATE durable_tasks
             SET progress_summary=?6, last_activity=?7, metadata_json=?8, updated_at_ms=?9
             WHERE id=?1 AND status='running' AND ownership_token=?2
               AND updated_at_ms=?3 AND lease_until_ms=?4 AND lease_until_ms>?9
               AND metadata_json=?5 AND cancel_requested=0",
            params![
                mutation.task_id,
                mutation.lease_token,
                mutation.expected_updated_at_ms,
                mutation.current_expiry_ms,
                mutation.expected_metadata_json,
                mutation.progress_summary,
                mutation.last_activity,
                mutation.metadata_json,
                mutation.now_ms,
            ],
        )? == 1;
        if !task_changed {
            transaction.rollback()?;
            return Ok(None);
        }
        if let Some(activity)=&mutation.activity {
            transaction.execute("INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
                VALUES(?1,'activity','running',?2,?3,?4)",params![mutation.task_id,activity.summary,activity.detail_json,mutation.now_ms])?;
        }
        let attempt = transaction.query_row(
            "SELECT id,task_id,attempt_number,worker_id,worker_generation,lease_token,
                    lease_expires_at_ms,state,revision,claimed_at_ms
             FROM runtime_task_attempts WHERE id=?1",
            params![mutation.attempt_id],
            |row| {
                Ok(nola_store_port::runtime_task::RuntimeTaskAttempt {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    attempt_number: row.get(2)?,
                    worker_id: row.get(3)?,
                    worker_generation: row.get(4)?,
                    lease_token: row.get(5)?,
                    lease_expires_at_ms: row.get(6)?,
                    state: row.get(7)?,
                    revision: row.get(8)?,
                    claimed_at_ms: row.get(9)?,
                })
            },
        )?;
        transaction.commit()?;
        Ok(Some(attempt))
    }

    pub fn runtime_task_permission_suspension(
        &self,
        id: &str,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskPermissionSuspension>> {
        require_id("runtime_task_permission_suspension.id", id)?;
        self.connection.query_row(
            "SELECT id,task_id,attempt_id,permission_request_id,continuation_id,continuation_json,state,revision,suspended_at_ms,resumed_at_ms FROM runtime_task_permission_suspensions WHERE id=?1",
            params![id],
            |row| Ok(nola_store_port::runtime_task::RuntimeTaskPermissionSuspension { id:row.get(0)?,task_id:row.get(1)?,attempt_id:row.get(2)?,permission_request_id:row.get(3)?,continuation_id:row.get(4)?,continuation_json:row.get(5)?,state:row.get(6)?,revision:row.get(7)?,suspended_at_ms:row.get(8)?,resumed_at_ms:row.get(9)? }),
        ).optional().map_err(StoreError::from)
    }
    pub fn pending_runtime_task_permission_suspension(
        &self,
        task_id: &str,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskPermissionSuspension>> {
        require_id("task_id", task_id)?;
        self.connection.query_row("SELECT id,task_id,attempt_id,permission_request_id,continuation_id,continuation_json,state,revision,suspended_at_ms,resumed_at_ms FROM runtime_task_permission_suspensions WHERE task_id=?1 AND state='pending' ORDER BY suspended_at_ms DESC,id DESC LIMIT 1",params![task_id],|row|Ok(nola_store_port::runtime_task::RuntimeTaskPermissionSuspension{id:row.get(0)?,task_id:row.get(1)?,attempt_id:row.get(2)?,permission_request_id:row.get(3)?,continuation_id:row.get(4)?,continuation_json:row.get(5)?,state:row.get(6)?,revision:row.get(7)?,suspended_at_ms:row.get(8)?,resumed_at_ms:row.get(9)?})).optional().map_err(StoreError::from)
    }
    pub fn latest_runtime_task_permission_suspension(
        &self,
        task_id: &str,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskPermissionSuspension>> {
        require_id("task_id", task_id)?;
        // Attempt order is causal. Wall-clock timestamps can be equal (or
        // regress after clock correction); random IDs cannot break that tie.
        self.connection
            .query_row(
                "SELECT s.id,s.task_id,s.attempt_id,s.permission_request_id,s.continuation_id,
                    s.continuation_json,s.state,s.revision,s.suspended_at_ms,s.resumed_at_ms
             FROM runtime_task_permission_suspensions s
             JOIN runtime_task_attempts a ON a.id=s.attempt_id AND a.task_id=s.task_id
             WHERE s.task_id=?1 ORDER BY a.attempt_number DESC LIMIT 1",
                params![task_id],
                |row| {
                    Ok(
                        nola_store_port::runtime_task::RuntimeTaskPermissionSuspension {
                            id: row.get(0)?,
                            task_id: row.get(1)?,
                            attempt_id: row.get(2)?,
                            permission_request_id: row.get(3)?,
                            continuation_id: row.get(4)?,
                            continuation_json: row.get(5)?,
                            state: row.get(6)?,
                            revision: row.get(7)?,
                            suspended_at_ms: row.get(8)?,
                            resumed_at_ms: row.get(9)?,
                        },
                    )
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn suspend_runtime_task_for_permission(
        &mut self,
        input: &nola_store_port::runtime_task::SuspendRuntimeTaskForPermission,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskPermissionSuspension>> {
        validate_permission_request(&input.permission_request)?;
        require_id("continuation_id", &input.continuation_id)?;
        validate_json("continuation_json", &input.continuation_json)?;
        let mut continuation: serde_json::Value = serde_json::from_str(&input.continuation_json)
            .map_err(|_| StoreError::Invariant("invalid continuation json".into()))?;
        if !continuation.is_object()
            || continuation.get("core_permission_source").is_some()
            || continuation
                .get("child_session_id")
                .and_then(serde_json::Value::as_str)
                != Some(input.permission_request.child_session_id.as_str())
            || continuation
                .get("core_permission_request_id")
                .and_then(serde_json::Value::as_str)
                != input
                    .permission_request
                    .core_permission_request_id
                    .as_deref()
            || continuation
                .get("run_id")
                .and_then(serde_json::Value::as_str)
                .is_none_or(str::is_empty)
        {
            return Err(StoreError::Invariant(
                "permission continuation identity mismatch".into(),
            ));
        }
        if input.permission_request.task_id != input.task_id
            || input.permission_request.status != "pending"
            || input.current_expiry_ms <= input.now_ms
            || input.expected_attempt_revision < 1
        {
            return Err(StoreError::Invariant(
                "invalid runtime task permission suspension".into(),
            ));
        }
        let tx = self.write_transaction()?;
        if !crate::agent_task_admission_store::permits_execution(&tx, &input.task_id)? {
            tx.rollback()?;
            return Ok(None);
        }
        // A Host continuation is not authority to select a different child
        // execution. Check the canonical binding inside the suspension write
        // transaction, before inserting the request or retiring its lease.
        let binding: Option<(String, String, Option<i64>)> = tx.query_row(
            "SELECT run_id,child_session_id,input_watermark FROM runtime_task_run_bindings WHERE task_id=?1",
            [&input.task_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        if let Some((run_id, child_session_id, input_watermark)) = binding {
            if continuation.get("run_id").and_then(serde_json::Value::as_str) != Some(run_id.as_str())
                || input.permission_request.child_session_id != child_session_id
                || continuation.get("input_watermark") != Some(&serde_json::json!(input_watermark))
            {
                return Err(StoreError::Conflict("permission continuation differs from canonical Task Run binding".into()));
            }
            // Validate before touching the permission row; retained authority is
            // recorded below only after the exact request identity is checked.
            crate::permission_source::capture(&tx, &run_id, &input.permission_request)?;
        }
        tx.execute("INSERT INTO permission_requests(request_id,task_id,child_session_id,capability,input_json,scope_rule,status,decision,custom_response,created_at_ms,resolved_at_ms,core_permission_request_id,permission_instructions_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(request_id) DO NOTHING",params![input.permission_request.request_id,input.permission_request.task_id,input.permission_request.child_session_id,input.permission_request.capability,input.permission_request.input_json,input.permission_request.scope_rule,input.permission_request.status,input.permission_request.decision,input.permission_request.custom_response,input.permission_request.created_at_ms,input.permission_request.resolved_at_ms,input.permission_request.core_permission_request_id,input.permission_request.permission_instructions_json])?;
        let exact:i64=tx.query_row("SELECT COUNT(*) FROM permission_requests WHERE request_id=?1 AND task_id=?2 AND child_session_id=?3 AND status='pending' AND core_permission_request_id IS ?4 AND capability=?5 AND input_json=?6 AND scope_rule=?7 AND created_at_ms=?8 AND decision IS NULL AND custom_response IS NULL AND resolved_at_ms IS NULL AND permission_instructions_json=?9",params![input.permission_request.request_id,input.task_id,input.permission_request.child_session_id,input.permission_request.core_permission_request_id,input.permission_request.capability,input.permission_request.input_json,input.permission_request.scope_rule,input.permission_request.created_at_ms,input.permission_request.permission_instructions_json],|r|r.get(0))?;
        if exact != 1 {
            tx.rollback()?;
            return Ok(None);
        }
        if let Some(proof) = crate::permission_source::retain_bound_source(&tx, &input.permission_request)? {
            continuation["core_permission_source"] = serde_json::to_value(proof)?;
        }
        let changed=tx.execute("UPDATE runtime_task_attempts SET state='permission_suspended',revision=revision+1 WHERE id=?1 AND task_id=?2 AND revision=?3 AND worker_generation=?4 AND lease_token=?5 AND lease_expires_at_ms=?6 AND state='leased' AND lease_expires_at_ms>?7",params![input.attempt_id,input.task_id,input.expected_attempt_revision,input.worker_generation,input.lease_token,input.current_expiry_ms,input.now_ms])?;
        if changed != 1 {
            tx.rollback()?;
            return Ok(None);
        }
        let task=tx.execute("UPDATE durable_tasks SET status='waiting_permission',worker_id=NULL,ownership_token=NULL,lease_until_ms=NULL,heartbeat_at_ms=NULL,progress_summary=?2,last_activity=COALESCE(?3,last_activity),updated_at_ms=?4,unread=1 WHERE id=?1 AND status='running' AND ownership_token=?5 AND lease_until_ms=?6 AND cancel_requested=0",params![input.task_id,input.progress_summary,input.last_activity,input.now_ms,input.lease_token,input.current_expiry_ms])?;
        if task != 1 {
            tx.rollback()?;
            return Ok(None);
        }
        let suspension = nola_store_port::runtime_task::RuntimeTaskPermissionSuspension {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: input.task_id.clone(),
            attempt_id: input.attempt_id.clone(),
            permission_request_id: input.permission_request.request_id.clone(),
            continuation_id: input.continuation_id.clone(),
            continuation_json: serde_json::to_string(&continuation)?,
            state: "pending".into(),
            revision: 1,
            suspended_at_ms: input.now_ms,
            resumed_at_ms: None,
        };
        tx.execute("INSERT INTO runtime_task_permission_suspensions(id,task_id,attempt_id,permission_request_id,continuation_id,continuation_json,state,revision,suspended_at_ms) VALUES(?1,?2,?3,?4,?5,?6,'pending',1,?7)",params![suspension.id,suspension.task_id,suspension.attempt_id,suspension.permission_request_id,suspension.continuation_id,suspension.continuation_json,suspension.suspended_at_ms])?;
        tx.execute("INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms) VALUES(?1,'permission_suspended','waiting_permission',?2,?3,?4)",params![input.task_id,input.progress_summary,serde_json::json!({"permission_request_id":input.permission_request.request_id,"continuation_id":input.continuation_id}).to_string(),input.now_ms])?;
        tx.commit()?;
        Ok(Some(suspension))
    }

    pub fn resume_runtime_task_after_permission_fair(
        &mut self,
        input: &nola_store_port::runtime_task::ResumeRuntimeTaskAfterPermissionFair,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>> {
        require_runtime_task_claim_identity("worker_id", &input.worker_id)?;
        require_runtime_task_claim_identity("worker_generation", &input.worker_generation)?;
        require_runtime_task_claim_identity("lease_token", &input.lease_token)?;
        require_id("task_id", &input.task_id)?;
        require_id("suspension_id", &input.suspension_id)?;
        require_id("permission_request_id", &input.permission_request_id)?;
        require_id("continuation_id", &input.continuation_id)?;
        if input.expected_suspension_revision < 1
            || input.lease_until_ms <= input.now_ms
            || input.lease_until_ms.saturating_sub(input.now_ms) > 10 * 60 * 1_000
            || input.max_running_global == 0
            || input.max_running_project == 0
        {
            return Err(StoreError::Invariant("invalid permission resume".into()));
        }
        let tx = self.write_transaction()?;
        if !crate::agent_task_admission_store::permits_execution(&tx, &input.task_id)? {
            tx.rollback()?;
            return Ok(None);
        }
        let eligible: i64 = tx.query_row(
            "SELECT COUNT(*) FROM runtime_task_permission_suspensions s
             JOIN permission_requests p ON p.request_id=s.permission_request_id AND p.task_id=s.task_id
             JOIN durable_tasks t ON t.id=s.task_id
             JOIN runtime_task_attempts a ON a.id=s.attempt_id AND a.task_id=s.task_id
             WHERE s.id=?1 AND s.task_id=?2 AND s.revision=?3 AND s.state='pending'
               AND s.permission_request_id=?4 AND s.continuation_id=?5
               AND p.status='resolved' AND p.decision IS NOT NULL
               AND t.status='waiting_permission' AND t.cancel_requested=0
               AND a.state='permission_suspended' AND a.lease_token<>?6
               AND a.worker_generation<>?8
               AND s.suspended_at_ms<=?7 AND p.resolved_at_ms<=?7 AND t.updated_at_ms<=?7",
            params![input.suspension_id,input.task_id,input.expected_suspension_revision,
                input.permission_request_id,input.continuation_id,input.lease_token,input.now_ms,input.worker_generation],
            |row| row.get(0),
        )?;
        let global: i64 = tx.query_row(
            "SELECT COUNT(*) FROM durable_tasks WHERE status='running' AND lease_until_ms>?1",
            params![input.now_ms],
            |r| r.get(0),
        )?;
        let project_ok:i64=tx.query_row("SELECT COUNT(*) FROM durable_tasks WHERE status='running' AND lease_until_ms>?1 AND project_root=(SELECT project_root FROM durable_tasks WHERE id=?2)",params![input.now_ms,input.task_id],|r|r.get(0))?;
        if eligible != 1
            || global >= i64::from(input.max_running_global)
            || project_ok >= i64::from(input.max_running_project)
            || crate::runtime_task_claim_store::fair_candidate(
                &tx,
                input.now_ms,
                input.max_running_project,
            )?
            .as_deref()
                != Some(input.task_id.as_str())
        {
            tx.rollback()?;
            return Ok(None);
        }
        crate::runtime_task_claim_store::require_fresh_lease_token(
            &tx,
            &input.task_id,
            &input.lease_token,
        )?;
        if tx.execute("UPDATE runtime_task_permission_suspensions SET state='resumed',revision=revision+1,resumed_at_ms=?2 WHERE id=?1 AND state='pending' AND revision=?3",params![input.suspension_id,input.now_ms,input.expected_suspension_revision])?!=1 { tx.rollback()?;return Ok(None); }
        let number: i64 = tx.query_row(
            "SELECT COALESCE(MAX(attempt_number),0)+1 FROM runtime_task_attempts WHERE task_id=?1",
            params![input.task_id],
            |r| r.get(0),
        )?;
        let attempt = nola_store_port::runtime_task::RuntimeTaskAttempt {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: input.task_id.clone(),
            attempt_number: number,
            worker_id: input.worker_id.clone(),
            worker_generation: input.worker_generation.clone(),
            lease_token: input.lease_token.clone(),
            lease_expires_at_ms: input.lease_until_ms,
            state: "leased".into(),
            revision: 1,
            claimed_at_ms: input.now_ms,
        };
        tx.execute("INSERT INTO runtime_task_attempts(id,task_id,attempt_number,worker_id,worker_generation,lease_token,lease_expires_at_ms,state,revision,claimed_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,'leased',1,?8)",params![attempt.id,attempt.task_id,attempt.attempt_number,attempt.worker_id,attempt.worker_generation,attempt.lease_token,attempt.lease_expires_at_ms,attempt.claimed_at_ms])?;
        if tx.execute("UPDATE durable_tasks SET status='running',worker_id=?2,ownership_token=?3,lease_until_ms=?4,heartbeat_at_ms=?5,updated_at_ms=?5 WHERE id=?1 AND status='waiting_permission' AND cancel_requested=0",params![input.task_id,input.worker_id,input.lease_token,input.lease_until_ms,input.now_ms])? != 1 { tx.rollback()?; return Ok(None); }
        tx.execute("INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms) VALUES(?1,'permission_resumed','running','Permission resolved',?2,?3)",params![input.task_id,serde_json::json!({"permission_request_id":input.permission_request_id,"suspension_id":input.suspension_id,"continuation_id":input.continuation_id,"attempt_id":attempt.id,"granted_attempt":attempt}).to_string(),input.now_ms])?;
        tx.commit()?;
        Ok(Some(attempt))
    }

    /// Recover the immutable grant returned by this particular resume commit.
    /// This is historical evidence, not authority to adopt a task's latest lease.
    pub fn runtime_task_permission_resume(
        &self,
        task_id: &str,
        suspension_id: &str,
        permission_request_id: &str,
        continuation_id: &str,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskPermissionResume>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let Some(suspension) = self.runtime_task_permission_suspension(suspension_id)? else {
            return Ok(None);
        };
        if suspension.task_id != task_id
            || suspension.permission_request_id != permission_request_id
            || suspension.continuation_id != continuation_id
        {
            return Err(StoreError::Conflict(
                "permission resume occurrence differs".into(),
            ));
        }
        if suspension.state != "resumed" {
            return Ok(None);
        }
        let mut statement = self.connection.prepare(
            "SELECT detail_json FROM durable_task_events WHERE task_id=?1 AND kind='permission_resumed'
             AND json_extract(detail_json,'$.suspension_id')=?2 LIMIT 2")?;
        let records = statement
            .query_map(params![task_id, suspension_id], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if records.len() != 1 {
            return Err(StoreError::Invariant(
                "permission resume grant evidence missing or duplicated".into(),
            ));
        }
        let record: serde_json::Value = serde_json::from_str(&records[0])?;
        let granted_attempt: nola_store_port::runtime_task::RuntimeTaskAttempt =
            serde_json::from_value(record["granted_attempt"].clone())?;
        let exact: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runtime_task_attempts a JOIN runtime_task_attempts previous ON previous.id=?1
             WHERE previous.task_id=?2 AND a.id=?3 AND a.task_id=?2
               AND a.attempt_number=previous.attempt_number+1 AND a.attempt_number=?4
               AND a.worker_id=?5 AND a.worker_generation=?6 AND a.lease_token=?7 AND a.claimed_at_ms=?8)",
            params![suspension.attempt_id,task_id,granted_attempt.id,granted_attempt.attempt_number,
                granted_attempt.worker_id,granted_attempt.worker_generation,granted_attempt.lease_token,granted_attempt.claimed_at_ms],
            |row| row.get(0),
        )?;
        if !exact
            || granted_attempt.task_id != task_id
            || granted_attempt.state != "leased"
            || granted_attempt.revision != 1
            || granted_attempt.lease_expires_at_ms <= granted_attempt.claimed_at_ms
            || suspension.resumed_at_ms != Some(granted_attempt.claimed_at_ms)
            || record["permission_request_id"].as_str() != Some(permission_request_id)
            || record["continuation_id"].as_str() != Some(continuation_id)
            || record["attempt_id"].as_str() != Some(granted_attempt.id.as_str())
        {
            return Err(StoreError::Invariant(
                "permission resume grant identity differs".into(),
            ));
        }
        Ok(Some(
            nola_store_port::runtime_task::RuntimeTaskPermissionResume {
                suspension,
                granted_attempt,
            },
        ))
    }

    pub fn cancel_runtime_task_permission_suspension(
        &mut self,
        input: &nola_store_port::runtime_task::CancelRuntimeTaskPermissionSuspension,
    ) -> StoreResult<Option<nola_store_port::runtime_task::RuntimeTaskPermissionSuspension>> {
        let tx = self.write_transaction()?;
        let exact: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM runtime_task_permission_suspensions s
             JOIN durable_tasks t ON t.id=s.task_id
             WHERE s.id=?1 AND s.task_id=?2 AND s.revision=?3 AND s.state='pending'
               AND s.permission_request_id=?4 AND s.continuation_id=?5
               AND t.status='waiting_permission' AND t.cancel_requested=0
               AND s.suspended_at_ms<=?6 AND t.updated_at_ms<=?6)",
            params![
                input.suspension_id,
                input.task_id,
                input.expected_suspension_revision,
                input.permission_request_id,
                input.continuation_id,
                input.requested_at_ms
            ],
            |row| row.get(0),
        )?;
        if !exact
            || !Self::request_runtime_task_cancel_in_transaction(
                &tx,
                &input.task_id,
                input.requested_at_ms,
            )?
        {
            tx.rollback()?;
            return Ok(None);
        }
        let out=tx.query_row("SELECT id,task_id,attempt_id,permission_request_id,continuation_id,continuation_json,state,revision,suspended_at_ms,resumed_at_ms FROM runtime_task_permission_suspensions WHERE id=?1",params![input.suspension_id],|row|Ok(nola_store_port::runtime_task::RuntimeTaskPermissionSuspension{id:row.get(0)?,task_id:row.get(1)?,attempt_id:row.get(2)?,permission_request_id:row.get(3)?,continuation_id:row.get(4)?,continuation_json:row.get(5)?,state:row.get(6)?,revision:row.get(7)?,suspended_at_ms:row.get(8)?,resumed_at_ms:row.get(9)?}))?;
        tx.commit()?;
        Ok(Some(out))
    }

    pub fn retry_runtime_task(
        &mut self,
        lease: &RuntimeTaskLeaseWitness,
        retry_at_ms: i64,
        error: &str,
        now_ms: i64,
    ) -> StoreResult<bool> {
        let transaction = self.write_transaction()?;
        if !revoke_runtime_task_lease(&transaction, lease, false, now_ms)? {
            transaction.rollback()?;
            return Ok(false);
        }
        let changed = transaction.execute(
            "UPDATE durable_tasks SET status='retry_waiting', worker_id=NULL,
             ownership_token=NULL, lease_until_ms=NULL, heartbeat_at_ms=NULL,
             retry_count=retry_count+1, retry_at_ms=?3, last_error=?4, updated_at_ms=?5
             WHERE id=?1 AND status='running' AND ownership_token=?2
               AND lease_until_ms=?6 AND lease_until_ms>?5 AND cancel_requested=0",
            params![
                lease.task_id,
                lease.lease_token,
                retry_at_ms,
                error,
                now_ms,
                lease.current_expiry_ms
            ],
        )? == 1;
        if !changed {
            transaction.rollback()?;
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
             VALUES (?1,'retry_scheduled','retry_waiting',?2,?3,?4)",
            params![
                lease.task_id,
                error,
                serde_json::json!({"attempt_id":lease.attempt_id,"worker_generation":lease.worker_generation,"retry_at_ms":retry_at_ms}).to_string(),
                now_ms,
            ],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_runtime_task_detailed_with_event(
        &mut self,
        lease: &RuntimeTaskLeaseWitness,
        status: &str,
        completion_watermark: Option<i64>,
        progress_summary: Option<&str>,
        result_summary: Option<&str>,
        error: Option<&str>,
        output_path: Option<&str>,
        unread: bool,
        event_kind: &str,
        event_status: &str,
        event_summary: Option<&str>,
        event_detail_json: &str,
        final_progress: Option<&nola_store_port::runtime_task::RuntimeTaskFinalProgress>,
        now_ms: i64,
    ) -> StoreResult<bool> {
        require_public_task_event_kind(event_kind)?;
        require_id("task_event.status", event_status)?;
        validate_json("task_event.detail_json", event_detail_json)?;
        if let Some(progress) = final_progress {
            if progress.expected_updated_at_ms < 0 || now_ms < progress.expected_updated_at_ms {
                return Err(StoreError::Invariant("final task observation timestamp moved backwards".into()));
            }
            for raw in [&progress.expected_metadata_json, &progress.metadata_json] {
                if !serde_json::from_str::<serde_json::Value>(raw)?.is_object() {
                    return Err(StoreError::Invariant("final task metadata must be a JSON object".into()));
                }
            }
            crate::runtime_task_shell_delivery_store::require_unchanged(
                &progress.expected_metadata_json, &progress.metadata_json)?;
        }
        if !matches!(status, "completed" | "failed" | "cancelled") {
            return Err(StoreError::Invariant(format!(
                "invalid terminal runtime task status: {status}"
            )));
        }
        let transaction = self.write_transaction()?;
        let bound: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM runtime_task_run_bindings WHERE task_id=?1)",
            [&lease.task_id],
            |row| row.get(0),
        )?;
        if bound {
            return Err(StoreError::Conflict(
                "bound agent task must settle from its canonical Run seal".into(),
            ));
        }
        let agent: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_tasks WHERE id=?1 AND kind='agent')", [&lease.task_id], |row| row.get(0),
        )?;
        if agent && status == "completed" {
            return Err(StoreError::Conflict(
                "agent task cannot complete without its canonical Run result".into(),
            ));
        }
        if !revoke_runtime_task_lease(&transaction, lease, status == "cancelled", now_ms)? {
            transaction.rollback()?;
            return Ok(false);
        }
        if let Some(progress) = final_progress {
            let observed = transaction.execute(
                "UPDATE durable_tasks SET metadata_json=?4 WHERE id=?1
                   AND updated_at_ms=?2 AND metadata_json=?3",
                params![lease.task_id, progress.expected_updated_at_ms,
                    progress.expected_metadata_json, progress.metadata_json],
            )? == 1;
            if !observed { transaction.rollback()?; return Ok(false); }
        }
        let changed = transaction.execute(
            "UPDATE durable_tasks
             SET status=?3, progress_summary=?4, result_summary=?5, error=?6,
                 output_path=?7, unread=?8,
                 metadata_json=json_set(metadata_json,
                    '$.model_notified', json('false')),
                 worker_id=NULL, ownership_token=NULL, lease_until_ms=NULL,
                 heartbeat_at_ms=NULL, retry_at_ms=NULL,
                 completion_watermark=?9, ended_at_ms=?10, updated_at_ms=?10
             WHERE id=?1 AND status='running' AND ownership_token=?2
               AND lease_until_ms=?11 AND lease_until_ms>?10
               AND (?3='cancelled' OR cancel_requested=0)",
            params![
                lease.task_id,
                lease.lease_token,
                status,
                progress_summary,
                result_summary,
                error,
                output_path,
                unread,
                completion_watermark,
                now_ms,
                lease.current_expiry_ms,
            ],
        )? == 1;
        if changed {
            transaction.execute(
                "INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    lease.task_id,
                    event_kind,
                    event_status,
                    event_summary,
                    event_detail_json,
                    now_ms,
                ],
            )?;
        } else {
            transaction.rollback()?;
            return Ok(false);
        }
        crate::runtime_task_terminal::record_terminal_receipt(
            &transaction,
            &lease.task_id,
            RuntimeTaskTerminalSource::WorkerFinish {
                lease: lease.clone(),
            },
            now_ms,
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    pub fn acknowledge_runtime_task_model_delivery(&mut self, task_id: &str) -> StoreResult<bool> {
        require_id("task_id", task_id)?;
        Ok(self.connection.execute(
            "UPDATE durable_tasks
             SET metadata_json=json_set(metadata_json, '$.model_notified', json('true'))
             WHERE id=?1
               AND COALESCE(json_extract(metadata_json, '$.model_notified'), 0) != 1",
            params![task_id],
        )? == 1)
    }

    pub fn acknowledge_runtime_task_result(
        &mut self,
        input: &nola_store_port::runtime_task::AcknowledgeRuntimeTaskResult,
    ) -> StoreResult<bool> {
        require_id(
            "task_result.owner_conversation_id",
            &input.owner_conversation_id,
        )?;
        require_id("task_result.task_id", &input.task_id)?;
        require_id(
            "task_result.terminal_receipt_id",
            &input.terminal_receipt_id,
        )?;
        let transaction = self.write_transaction()?;
        // The immutable receipt and live result projection must agree in this
        // transaction. No unrelated unread Task participates in this witness.
        let admitted: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_tasks t
             JOIN runtime_task_terminal_receipts r ON r.task_id=t.id
             WHERE t.id=?1 AND t.owner_session_id=?2 AND t.updated_at_ms=?3
               AND t.status IN ('sealed','completed','failed','cancelled') AND r.receipt_id=?4
               AND json_extract(r.receipt_json,'$.receipt_id')=r.receipt_id
               AND json_extract(r.receipt_json,'$.task_id')=t.id
               AND json_extract(r.receipt_json,'$.owner_session_id')=t.owner_session_id
               AND json_extract(r.receipt_json,'$.status')=t.status
               AND json_extract(r.receipt_json,'$.child_session_id') IS t.child_session_id
               AND json_extract(r.receipt_json,'$.input_watermark') IS t.input_watermark
               AND json_extract(r.receipt_json,'$.completion_watermark') IS t.completion_watermark
               AND json_extract(r.receipt_json,'$.ended_at_ms')=t.ended_at_ms)",
            params![
                input.task_id,
                input.owner_conversation_id,
                input.expected_updated_at_ms,
                input.terminal_receipt_id
            ],
            |row| row.get(0),
        )?;
        if !admitted {
            return Err(StoreError::Conflict(
                "exact terminal Task result delivery changed or belongs to another owner".into(),
            ));
        }
        let consumed = transaction.execute(
            "UPDATE durable_tasks SET unread=0,
                metadata_json=json_set(metadata_json,'$.model_notified',json('true'))
             WHERE id=?1 AND updated_at_ms=?2
               AND (unread=1 OR COALESCE(json_extract(metadata_json,'$.model_notified'),0) != 1)",
            params![input.task_id, input.expected_updated_at_ms],
        )? == 1;
        transaction.commit()?;
        Ok(consumed)
    }

    pub fn acknowledge_runtime_task_reads(
        &mut self,
        input: &AcknowledgeRuntimeTaskReadsInput,
    ) -> StoreResult<()> {
        require_id("runtime_task_read.conversation_id", &input.conversation_id)?;
        let transaction = self.write_transaction()?;
        let checkpoint_revision = transaction
            .query_row(
                "SELECT revision FROM core_checkpoints WHERE conversation_id=?1",
                params![input.conversation_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(checkpoint_revision) = checkpoint_revision else {
            return Err(StoreError::Conflict(format!(
                "resume checkpoint lineage is missing for conversation {}",
                input.conversation_id
            )));
        };
        if checkpoint_revision < input.minimum_checkpoint_revision {
            return Err(StoreError::Conflict(format!(
                "resume checkpoint lineage regressed: minimum={}, actual={checkpoint_revision}",
                input.minimum_checkpoint_revision
            )));
        }
        let unique_task_ids = input
            .tasks
            .iter()
            .map(|cursor| cursor.task_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if unique_task_ids.len() != input.tasks.len() {
            return Err(StoreError::Conflict(
                "resume mailbox cursor contains duplicate task identities".to_string(),
            ));
        }
        let unread_terminal_count = transaction.query_row(
            "SELECT COUNT(*) FROM durable_tasks
             WHERE owner_session_id=?1 AND unread=1
             AND status IN ('sealed','completed','failed','cancelled')",
            params![input.conversation_id],
            |row| row.get::<_, i64>(0),
        )?;
        if usize::try_from(unread_terminal_count).ok() != Some(input.tasks.len()) {
            return Err(StoreError::Conflict(format!(
                "resume mailbox changed before acknowledgement: expected={}, actual={unread_terminal_count}",
                input.tasks.len()
            )));
        }
        for cursor in &input.tasks {
            require_id("runtime_task_read.task_id", &cursor.task_id)?;
            let current = transaction
                .query_row(
                    "SELECT owner_session_id,status,updated_at_ms FROM durable_tasks WHERE id=?1",
                    params![cursor.task_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()?;
            let Some((owner, status, updated_at_ms)) = current else {
                return Err(StoreError::Conflict(format!(
                    "resume task disappeared: {}",
                    cursor.task_id
                )));
            };
            if owner.as_deref() != Some(input.conversation_id.as_str())
                || updated_at_ms != cursor.updated_at_ms
                || !matches!(status.as_str(), "sealed" | "completed" | "failed" | "cancelled")
            {
                return Err(StoreError::Conflict(format!(
                    "resume task changed before acknowledgement: {}",
                    cursor.task_id
                )));
            }
        }
        for cursor in &input.tasks {
            transaction.execute(
                "UPDATE durable_tasks SET unread=0 WHERE id=?1 AND updated_at_ms=?2",
                params![cursor.task_id, cursor.updated_at_ms],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn request_runtime_task_cancel(&mut self, task_id: &str, now_ms: i64) -> StoreResult<bool> {
        require_id("task_id", task_id)?;
        let transaction = self.write_transaction()?;
        let changed =
            Self::request_runtime_task_cancel_in_transaction(&transaction, task_id, now_ms)?;
        transaction.commit()?;
        Ok(changed)
    }

    fn request_runtime_task_cancel_in_transaction(
        transaction: &crate::connection::WriteTransaction<'_>,
        task_id: &str,
        now_ms: i64,
    ) -> StoreResult<bool> {
        let bound: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM runtime_task_run_bindings WHERE task_id=?1)",
            [task_id], |row| row.get(0),
        )?;
        if bound {
            // The Task owns stop intent, not its admitted Run's outcome or
            // permission decision. Preserve execution evidence until Core
            // drains/cancels that exact Run and commits its terminal seal.
            let changed = transaction.execute(
                "UPDATE durable_tasks SET cancel_requested=1,
                 cancel_requested_at_ms=COALESCE(cancel_requested_at_ms,?2),
                 progress_summary='cancellation_requested',unread=1,
                 metadata_json=json_set(metadata_json,'$.model_notified',json('false')),
                 updated_at_ms=?2
                 WHERE id=?1 AND status NOT IN ('sealed','completed','failed','cancelled')
                   AND updated_at_ms<=?2 AND cancel_requested=0",
                params![task_id, now_ms],
            )? == 1;
            if changed {
                crate::permission_transport::revoke_task_request_grants(transaction, task_id, now_ms)?;
            }
            return Ok(changed);
        }
        let changed = transaction.execute(
            "UPDATE durable_tasks SET
             cancel_requested=1,
             cancel_requested_at_ms=COALESCE(cancel_requested_at_ms, ?2),
             progress_summary=CASE
               WHEN status IN ('queued','retry_waiting','waiting_permission','suspended')
               THEN 'cancelled' ELSE 'cancellation_requested'
             END,
             unread=1,
             metadata_json=json_set(metadata_json,
                '$.model_notified', json('false')),
             status=CASE
               WHEN status IN ('queued','retry_waiting','waiting_permission','suspended')
               THEN 'cancelled'
               ELSE status
             END,
             worker_id=CASE
               WHEN status IN ('queued','retry_waiting','waiting_permission','suspended')
               THEN NULL ELSE worker_id
             END,
             ownership_token=CASE
               WHEN status IN ('queued','retry_waiting','waiting_permission','suspended')
               THEN NULL ELSE ownership_token
             END,
             lease_until_ms=CASE
               WHEN status IN ('queued','retry_waiting','waiting_permission','suspended')
               THEN NULL ELSE lease_until_ms
             END,
             heartbeat_at_ms=CASE
               WHEN status IN ('queued','retry_waiting','waiting_permission','suspended')
               THEN NULL ELSE heartbeat_at_ms
             END,
             ended_at_ms=CASE
               WHEN status IN ('queued','retry_waiting','waiting_permission','suspended')
               THEN ?2 ELSE ended_at_ms
             END,
             updated_at_ms=?2
             WHERE id=?1 AND status NOT IN ('sealed','completed','failed','cancelled') AND updated_at_ms<=?2",
            params![task_id, now_ms],
        )? == 1;
        if changed {
            // Task cancellation owns all still-pending permission continuations.
            // Preserve already recorded decisions; cancellation revokes execution,
            // not the historical decision that may have raced this transaction.
            crate::permission_transport::revoke_task_request_grants(transaction, task_id, now_ms)?;
            transaction.execute(
                "UPDATE runtime_task_attempts SET state='superseded',revision=revision+1
                 WHERE task_id=?1 AND state='permission_suspended' AND id IN
                 (SELECT attempt_id FROM runtime_task_permission_suspensions
                  WHERE task_id=?1 AND state='pending')",
                [task_id],
            )?;
            transaction.execute(
                "UPDATE runtime_task_permission_suspensions
                 SET state='cancelled',revision=revision+1,resumed_at_ms=?2
                 WHERE task_id=?1 AND state='pending'",
                params![task_id, now_ms],
            )?;
            crate::runtime_task_terminal::record_terminal_receipt(
                transaction,
                task_id,
                RuntimeTaskTerminalSource::Cancellation,
                now_ms,
            )?;
        }
        Ok(changed)
    }

    pub fn fail_expired_runtime_task(
        &mut self,
        task_id: &str,
        error: &str,
        now_ms: i64,
    ) -> StoreResult<bool> {
        require_id("task_id", task_id)?;
        let transaction = self.write_transaction()?;
        revoke_expired_runtime_task_attempts(&transaction, task_id, now_ms, false)?;
        let changed = transaction.execute(
            "UPDATE durable_tasks SET status='failed', worker_id=NULL, ownership_token=NULL,
             lease_until_ms=NULL, heartbeat_at_ms=NULL, retry_at_ms=NULL, last_error=?2,
             error=?2, ended_at_ms=?3, updated_at_ms=?3 WHERE id=?1 AND status='running'
             AND (lease_until_ms IS NULL OR lease_until_ms<=?3)",
            params![task_id, error, now_ms],
        )? == 1;
        if !changed {
            transaction.rollback()?;
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
             VALUES (?1,'lease_expired','failed',?2,'{}',?3)",
            params![task_id, error, now_ms],
        )?;
        crate::runtime_task_terminal::record_terminal_receipt(
            &transaction,
            task_id,
            RuntimeTaskTerminalSource::LeaseExpired,
            now_ms,
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn fail_expired_runtime_task_with_event(
        &mut self,
        task_id: &str,
        error: &str,
        event_kind: &str,
        event_status: &str,
        event_summary: Option<&str>,
        event_detail_json: &str,
        now_ms: i64,
    ) -> StoreResult<bool> {
        require_id("task_id", task_id)?;
        require_public_task_event_kind(event_kind)?;
        require_id("task_event.status", event_status)?;
        validate_json("task_event.detail_json", event_detail_json)?;
        let transaction = self.write_transaction()?;
        if !crate::runtime_task_claim_store::runtime_task_time_is_current(
            &transaction, task_id, now_ms,
        )? {
            transaction.rollback()?;
            return Ok(false);
        }
        revoke_expired_runtime_task_attempts(&transaction, task_id, now_ms, false)?;
        let changed = transaction.execute(
            "UPDATE durable_tasks SET status='failed', worker_id=NULL, ownership_token=NULL,
             lease_until_ms=NULL, heartbeat_at_ms=NULL, retry_at_ms=NULL, last_error=?2,
             error=?2, progress_summary=COALESCE(?4,'failed'), unread=1,
             metadata_json=json_set(metadata_json,
                '$.model_notified', json('false')),
             ended_at_ms=?3, updated_at_ms=?3 WHERE id=?1 AND status='running'
             AND (lease_until_ms IS NULL OR lease_until_ms<=?3)",
            params![task_id, error, now_ms, event_summary],
        )? == 1;
        if changed {
            transaction.execute(
                "INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    task_id,
                    event_kind,
                    event_status,
                    event_summary,
                    event_detail_json,
                    now_ms
                ],
            )?;
        } else {
            transaction.rollback()?;
            return Ok(false);
        }
        crate::runtime_task_terminal::record_terminal_receipt(
            &transaction,
            task_id,
            RuntimeTaskTerminalSource::LeaseExpired,
            now_ms,
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    pub fn recover_expired_runtime_task(
        &mut self,
        task_id: &str,
        error: &str,
        retry_at_ms: i64,
        now_ms: i64,
    ) -> StoreResult<bool> {
        require_id("task_id", task_id)?;
        let transaction = self.write_transaction()?;
        if !crate::runtime_task_claim_store::runtime_task_time_is_current(
            &transaction, task_id, now_ms,
        )? {
            transaction.rollback()?;
            return Ok(false);
        }
        if !shell_dispatch_allows_expired_recovery(&transaction, task_id)? {
            transaction.rollback()?;
            return Ok(false);
        }
        revoke_expired_runtime_task_attempts(&transaction, task_id, now_ms, true)?;
        let changed = transaction.execute(
            "UPDATE durable_tasks SET
             status=CASE WHEN cancel_requested=1 THEN 'cancelled' ELSE 'retry_waiting' END,
             worker_id=NULL,
             ownership_token=NULL, lease_until_ms=NULL, heartbeat_at_ms=NULL,
             retry_count=CASE WHEN cancel_requested=1 THEN retry_count ELSE retry_count+1 END,
             retry_at_ms=CASE WHEN cancel_requested=1 THEN NULL ELSE ?3 END,
             last_error=CASE WHEN cancel_requested=1 THEN last_error ELSE ?2 END,
             error=NULL,
             ended_at_ms=CASE WHEN cancel_requested=1 THEN ?4 ELSE NULL END,
             updated_at_ms=?4 WHERE id=?1
             AND status IN ('running','waiting_permission','suspended')
             AND NOT EXISTS (SELECT 1 FROM runtime_task_permission_suspensions s
                             WHERE s.task_id=durable_tasks.id AND s.state='pending')
             AND (lease_until_ms IS NULL OR lease_until_ms<=?4)",
            params![task_id, error, retry_at_ms, now_ms],
        )? == 1;
        if !changed {
            transaction.rollback()?;
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
             SELECT id,'lease_recovered',status,?2,json_object('retry_at_ms',retry_at_ms),?3
             FROM durable_tasks WHERE id=?1",
            params![task_id, error, now_ms],
        )?;
        crate::runtime_task_terminal::record_terminal_receipt(
            &transaction,
            task_id,
            RuntimeTaskTerminalSource::Cancellation,
            now_ms,
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn create_runtime_task_with_event(&mut self, task: &StoredRuntimeTask) -> StoreResult<()> {
        // Project aliases must not acquire independent writer slots.
        let mut normalized = task.clone();
        normalized.project_root =
            crate::runtime_task_claim_store::runtime_task_project_root(&task.project_root);
        let task = &normalized;
        if task.kind == "agent"
            && matches!(task.status.as_str(), "sealed" | "completed" | "failed" | "cancelled")
        {
            return Err(StoreError::Invariant(
                "agent tasks must reach terminal through a durable lifecycle transition".into(),
            ));
        }
        let mut transaction=self.write_transaction()?;
        transaction.with_store(|store| {
            if store.runtime_task(&task.id)?.is_some() {
                return Err(StoreError::Invariant(format!(
                    "runtime task {} already exists",
                    task.id
                )));
            }
            store.insert_runtime_task(task)?;
            store.append_runtime_task_event(
                &task.id,
                "created",
                &task.status,
                Some(&task.title),
                &serde_json::json!({"kind": task.kind}).to_string(),
                task.created_at_ms,
            )?;
            Ok(())
        })?;
        transaction.commit()
    }

    pub fn append_runtime_task_event(
        &mut self,
        task_id: &str,
        kind: &str,
        status: &str,
        summary: Option<&str>,
        detail_json: &str,
        created_at_ms: i64,
    ) -> StoreResult<i64> {
        require_id("task_id", task_id)?;
        require_public_task_event_kind(kind)?;
        require_id("task_event.status", status)?;
        validate_json("task_event.detail_json", detail_json)?;
        self.connection.execute(
            "INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![task_id, kind, status, summary, detail_json, created_at_ms],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    /// Returns the durable high-water mark used by active Hosts to notice
    /// scheduler progress made by another process. The cursor is advisory:
    /// task admission remains atomic in `claim_runtime_task_fair`, and callers
    /// must reconcile from durable task state rather than interpreting event
    /// payloads as a second scheduler fact source.
    pub fn latest_runtime_task_event_id(&self) -> StoreResult<i64> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(id), 0) FROM durable_task_events",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
    }

    pub fn runtime_task_events(
        &self,
        task_id: &str,
        limit: usize,
    ) -> StoreResult<Vec<StoredRuntimeTaskEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT id,task_id,kind,status,summary,detail_json,created_at_ms
             FROM durable_task_events WHERE task_id=?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![task_id, limit.max(1) as i64],
            runtime_task_event_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn runtime_task_events_after_session_cursor(
        &self,
        owner_session_id: &str,
        after_cursor: i64,
        limit: usize,
    ) -> StoreResult<Vec<StoredRuntimeTaskEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT e.id,e.task_id,e.kind,e.status,e.summary,e.detail_json,e.created_at_ms
             FROM durable_task_events e JOIN durable_tasks t ON t.id=e.task_id
             WHERE t.owner_session_id=?1 AND e.id>?2 ORDER BY e.id ASC LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![
                owner_session_id,
                after_cursor.max(0),
                limit.clamp(1, 256) as i64
            ],
            runtime_task_event_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}
