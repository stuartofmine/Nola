// SPDX-License-Identifier: Apache-2.0
//! Immutable task admission and terminal reconciliation from its exact Run.
use super::*;

pub(crate) fn binding(connection: &Connection, task_id: &str) -> StoreResult<Option<RuntimeTaskRunBinding>> {
    connection.query_row(
        "SELECT task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,input_watermark,bound_at_ms
         FROM runtime_task_run_bindings WHERE task_id=?1", params![task_id], |row| Ok(RuntimeTaskRunBinding {
            task_id: row.get(0)?, owner_session_id: row.get(1)?, child_session_id: row.get(2)?, run_id: row.get(3)?,
            admission_attempt_id: row.get(4)?, input_watermark: row.get(5)?, bound_at_ms: row.get(6)?,
        }),
    ).optional().map_err(StoreError::from)
}

// A continuation extends an immutable chain of sealed task Runs. Neither a
// worker-supplied session ID nor mutable task metadata establishes this chain.
fn validate_child_history(connection: &Connection, task_id: &str, child: &str,
    admitted_run: Option<&str>, now: i64) -> StoreResult<()> {
    if crate::expert_task_target::validate_persistent_expert_history(connection,task_id,child,admitted_run)? {return Ok(());}
    let admission = crate::agent_task_admission_store::authority(connection, task_id)?
        .and_then(|authority| authority.admission)
        .ok_or_else(|| StoreError::Conflict("task continuation has no admitted source".into()))?;
    let (project, profile): (String, Option<String>) = connection.query_row(
        "SELECT project_root,agent_type FROM durable_tasks WHERE id=?1", [task_id],
        |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut predecessor = admission.predecessor_task_id.clone();
    let mut tasks = std::collections::HashSet::new();
    let mut runs = std::collections::HashSet::new();
    while let Some(id) = predecessor {
        if id == task_id || !tasks.insert(id.clone()) {
            return Err(StoreError::Conflict("task continuation lineage is cyclic".into()));
        }
        let prior = crate::agent_task_admission_store::authority(connection, &id)?
            .and_then(|authority| authority.admission)
            .ok_or_else(|| StoreError::Conflict("task predecessor admission is missing".into()))?;
        let bound = binding(connection, &id)?
            .ok_or_else(|| StoreError::Conflict("task predecessor Run binding is missing".into()))?;
        let raw: Option<String> = connection.query_row(
            "SELECT receipt_json FROM runtime_task_terminal_receipts WHERE task_id=?1", [&id],
            |row| row.get(0)).optional()?;
        let receipt: RuntimeTaskTerminalReceipt = serde_json::from_str(&raw
            .ok_or_else(|| StoreError::Conflict("task predecessor is not terminal".into()))?)?;
        let result = receipt.journal_result.as_ref()
            .ok_or_else(|| StoreError::Conflict("task predecessor has no canonical result".into()))?;
        result.seal.validate().map_err(StoreError::Invariant)?;
        let exact: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_tasks t JOIN runs r ON r.id=?2
             WHERE t.id=?1 AND t.kind='agent' AND t.status='sealed'
               AND t.owner_session_id=?3 AND t.child_session_id=?4
               AND t.project_root=?5 AND t.agent_type IS ?6
               AND r.conversation_id=?4 AND r.status='sealed' AND r.completed_at_ms=?7
               AND NOT EXISTS(SELECT 1 FROM permission_requests p WHERE p.task_id=t.id AND p.status='pending'))",
            params![id,bound.run_id,admission.parent_conversation_id,child,project,profile,result.seal.completed_at_ms],
            |row| row.get(0))?;
        if !exact || prior.parent_conversation_id != admission.parent_conversation_id
            || prior.child_conversation_id != child || bound.owner_session_id != admission.parent_conversation_id
            || bound.child_session_id != child || receipt.task_id != id || receipt.status != "sealed"
            || receipt.owner_session_id != admission.parent_conversation_id
            || receipt.child_session_id.as_deref() != Some(child)
            || result.seal.identity.run_id != bound.run_id || result.seal.result.conversation_id != child
            || receipt.recorded_at_ms > now
            || !matches!(&receipt.source, RuntimeTaskTerminalSource::BoundRunTerminal { binding, .. } if binding == &bound) {
            return Err(StoreError::Conflict("task predecessor ownership or terminal evidence differs".into()));
        }
        runs.insert(bound.run_id);
        predecessor = prior.predecessor_task_id;
    }
    let mut statement = connection.prepare("SELECT task_id,run_id FROM runtime_task_run_bindings WHERE child_session_id=?1")?;
    for row in statement.query_map([child], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))? {
        let (task, run) = row?;
        if !tasks.contains(&task) || !runs.contains(&run) {
            return Err(StoreError::Conflict("task continuation is not the current sealed lineage head".into()));
        }
    }
    let mut statement = connection.prepare("SELECT id FROM runs WHERE conversation_id=?1")?;
    for row in statement.query_map([child], |row| row.get::<_,String>(0))? {
        let run = row?;
        if Some(run.as_str()) != admitted_run && !runs.contains(&run) {
            return Err(StoreError::Conflict("task child contains an unrelated Run".into()));
        }
    }
    if tasks.is_empty() {
        let prior_task: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_tasks WHERE kind='agent' AND child_session_id=?1 AND id<>?2)",
            params![child,task_id], |row|row.get(0))?;
        if prior_task { return Err(StoreError::Conflict("existing task child requires explicit continuation lineage".into())); }
    }
    let competing: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM durable_tasks WHERE kind='agent' AND child_session_id=?1 AND id<>?2
         AND status NOT IN ('sealed','completed','failed','cancelled'))", params![child,task_id], |row|row.get(0))?;
    if competing { return Err(StoreError::Conflict("task child already has another pending continuation".into())); }
    Ok(())
}

fn validate_admission(
    connection: &Connection,
    lease: &RuntimeTaskLeaseWitness,
    child: &str,
    now: i64,
    admitted_run: Option<&str>,
) -> StoreResult<()> {
    for (name, value) in [
        ("task_id", lease.task_id.as_str()),
        ("attempt_id", lease.attempt_id.as_str()),
        ("worker_generation", lease.worker_generation.as_str()),
        ("lease_token", lease.lease_token.as_str()),
        ("child_session_id", child),
    ] {
        require_runtime_task_claim_identity(name, value)?;
    }
    if lease.expected_attempt_revision < 1 || lease.current_expiry_ms <= now {
        return Err(StoreError::Conflict(
            "task turn admission requires a live exact lease".into(),
        ));
    }
    if !crate::agent_task_admission_store::permits_execution(connection,&lease.task_id)? {
        return Err(StoreError::Conflict("agent Task has no admitted Core authority".into()));
    }
    let owned: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM durable_tasks t JOIN runtime_task_attempts a ON a.task_id=t.id
         WHERE t.id=?1 AND t.kind='agent' AND t.status='running' AND t.cancel_requested=0
           AND t.child_session_id=?2 AND t.owner_session_id IS NOT NULL AND length(trim(t.owner_session_id))>0
           AND t.owner_session_id<>t.child_session_id AND t.updated_at_ms<=?7
           AND t.ownership_token=?3 AND t.lease_until_ms=?4 AND t.worker_id=a.worker_id
           AND a.id=?5 AND a.revision=?6 AND a.worker_generation=?8 AND a.lease_token=?3
           AND a.lease_expires_at_ms=?4 AND a.state='leased'
           AND NOT EXISTS(SELECT 1 FROM runtime_task_run_bindings b WHERE b.task_id=t.id)
           AND NOT EXISTS(SELECT 1 FROM permission_requests p WHERE p.task_id=t.id)
)",
        params![lease.task_id,child,lease.lease_token,lease.current_expiry_ms,lease.attempt_id,lease.expected_attempt_revision,now,lease.worker_generation],
        |row|row.get(0),
    )?;
    if !owned {
        return Err(StoreError::Conflict(
            "task turn admission identity, lease, prior Run, or prior permission differs".into(),
        ));
    }
    validate_child_history(connection, &lease.task_id, child, admitted_run, now)
}

impl NolaStore {
    pub fn runtime_task_run_binding(
        &self,
        task_id: &str,
    ) -> StoreResult<Option<RuntimeTaskRunBinding>> {
        require_id("task_id", task_id)?;
        binding(&self.connection, task_id)
    }

    pub fn is_runtime_task_child_session(&self, child_session_id: &str) -> StoreResult<bool> {
        Ok(!matches!(crate::conversation_history_port::source_execution_ownership(&self.connection,child_session_id)?,
            nola_domain::ConversationExecutionOwnership::User))
    }

    pub fn validate_runtime_task_turn_admission(
        &self,
        lease: &RuntimeTaskLeaseWitness,
        child_session_id: &str,
        now_ms: i64,
    ) -> StoreResult<()> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        validate_admission(&self.connection, lease, child_session_id, now_ms, None)
    }

    pub fn bind_runtime_task_run(
        &mut self,
        lease: &RuntimeTaskLeaseWitness,
        child_session_id: &str,
        run_id: &str,
        now_ms: i64,
    ) -> StoreResult<RuntimeTaskRunBinding> {
        require_id("run_id", run_id)?;
        // Binding is never a post-commit repair API. The caller must own the
        // same Application UoW that has just created the canonical Run.
        if self.connection.is_autocommit() {
            return Err(StoreError::Invariant(
                "task Run binding requires the admission unit of work".into(),
            ));
        }
        let tx = self.write_transaction()?;
        validate_admission(&tx, lease, child_session_id, now_ms, Some(run_id))?;
        let exact: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND conversation_id=?2 AND kind='conversation'
                AND created_at_ms=?3 AND status <> 'sealed')",
            params![run_id,child_session_id,now_ms], |row|row.get(0),
        )?;
        if !exact {
            return Err(StoreError::Conflict(
                "task Run binding does not match its new admission".into(),
            ));
        }
        tx.execute(
            "INSERT INTO runtime_task_run_bindings(task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,input_watermark,bound_at_ms)
             SELECT id,owner_session_id,child_session_id,?2,?3,input_watermark,?4 FROM durable_tasks WHERE id=?1",
            params![lease.task_id,run_id,lease.attempt_id,now_ms],
        )?;
        let result = binding(&tx, &lease.task_id)?
            .ok_or_else(|| StoreError::Invariant("task Run binding was not inserted".into()))?;
        tx.execute(
            "INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
            VALUES(?1,'run_bound','running','Task Run admitted',?2,?3)",
            params![lease.task_id, serde_json::to_string(&result)?, now_ms],
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn reconcile_runtime_task_bound_run_terminal(
        &mut self,
        task_id: &str,
        expected_run_id: &str,
        now_ms: i64,
    ) -> StoreResult<bool> {
        require_id("task_id", task_id)?;
        require_id("expected_run_id", expected_run_id)?;
        let Some(observed_binding) = self.runtime_task_run_binding(task_id)? else {
            return Ok(false);
        };
        if observed_binding.run_id != expected_run_id {
            return Ok(false);
        }
        if self
            .runtime_task(task_id)?
            .is_some_and(|task| task.status == "awaiting_reconciliation")
        {
            return Ok(false);
        }
        // Replays reuse the immutable result, never rescan terminal bodies or
        // replace its captured publication with a newer presentation head.
        let existing = self.runtime_task_terminal_receipt(task_id)?;
        let recovered = if let Some(receipt) = existing.as_ref() {
            match &receipt.source {
                RuntimeTaskTerminalSource::BoundRunTerminal { binding, .. }
                    if binding == &observed_binding =>
                {
                    receipt.journal_result.clone()
                }
                _ => return Ok(false),
            }
        } else {
            crate::runtime_task_terminal::journal_result(self, &observed_binding)?
        };
        let Some(mut result) = recovered else {
            return Ok(false);
        };
        let tx = self.write_transaction()?;
        let Some(bound) = binding(&tx, task_id)? else {
            tx.rollback()?;
            return Ok(false);
        };
        if bound.run_id != expected_run_id {
            tx.rollback()?;
            return Ok(false);
        }
        let terminal: Option<(String,String,i64)> = tx.query_row(
            "SELECT status,assistant_message_id,completed_at_ms FROM runs WHERE id=?1 AND conversation_id=?2 AND kind='conversation'
             AND status='sealed' AND completed_at_ms IS NOT NULL AND assistant_message_id IS NOT NULL",
            params![bound.run_id,bound.child_session_id], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?;
        let Some((status, message_id, completed_at)) = terminal else {
            tx.rollback()?;
            return Ok(false);
        };
        result.seal.validate().map_err(StoreError::Invariant)?;
        if bound != observed_binding
            || result.seal.identity.assistant_message_id != message_id
            || result.seal.identity.run_id != bound.run_id
            || result.seal.result.conversation_id != bound.child_session_id
            || result.seal.completed_at_ms != completed_at
        {
            return Err(StoreError::Conflict(
                "bound task result seal differs from canonical terminal Run".into(),
            ));
        }
        let task: Option<(String,Option<String>,Option<String>,Option<i64>,bool,i64,String,Option<i64>)> = tx.query_row(
            "SELECT status,owner_session_id,child_session_id,input_watermark,cancel_requested,updated_at_ms,metadata_json,started_at_ms FROM durable_tasks WHERE id=?1 AND kind='agent'",
            params![task_id], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
        ).optional()?;
        let Some((task_status, owner, child, watermark, _cancel_requested, updated_at, metadata_json, started_at)) = task
        else {
            tx.rollback()?;
            return Ok(false);
        };
        if owner.as_deref() != Some(bound.owner_session_id.as_str())
            || child.as_deref() != Some(bound.child_session_id.as_str())
            || watermark != bound.input_watermark
            || task_status == "awaiting_reconciliation"
            || completed_at > now_ms
            || updated_at > now_ms
        {
            tx.rollback()?;
            return Ok(false);
        }
        // The child seal records the child's outcome. The supervising Task can
        // fail its resource contract even when that child completed or was
        // cancelled by the watchdog. Preserve both immutable facts.
        let mut metadata: serde_json::Value = serde_json::from_str(&metadata_json)?;
        let mut scheduling = metadata.get(nola_domain::runtime_task_scheduling::SCHEDULING_METADATA_KEY)
            .map(|value| serde_json::from_value::<nola_domain::runtime_task_scheduling::RuntimeTaskScheduling>(value.clone()))
            .transpose()?;
        // The immutable child journal proves what was produced, including a
        // final chunk that raced the watchdog. Only the Task's retained preview
        // is budgeted here; never truncate its evidence or rewrite its seal.
        // Use this same bounded result for both the Task and terminal receipt.
        let preview_limit = scheduling.as_ref()
            .and_then(|state| state.resource_budget.max_output_bytes)
            .unwrap_or(4096).min(4096) as usize;
        let mut preview_end = result.preview.len().min(preview_limit);
        while !result.preview.is_char_boundary(preview_end) {
            preview_end -= 1;
        }
        result.preview.truncate(preview_end);
        result.preview_truncated = (result.preview.len() as u64) < result.public_utf8_bytes.0;
        if let Some(scheduling) = scheduling.as_mut() {
            // A sealed public body is an authoritative lower bound even if a
            // worker has not yet reported its final output count. Never let a
            // raced completion receipt permanently bypass its output budget.
            scheduling.output_bytes = scheduling.output_bytes.max(result.public_utf8_bytes.0);
            if scheduling.budget_exhaustion.is_none()
                && scheduling.resource_budget.max_output_bytes
                    .is_some_and(|limit| scheduling.output_bytes > limit)
            {
                scheduling.budget_exhaustion = Some("output_bytes".into());
            }
        }
        let budget_failure = scheduling.as_ref()
            .map(|scheduling| scheduling.measured_budget_failure(started_at, now_ms))
            .transpose().map_err(StoreError::Invariant)?.flatten();
        // Sealing settles execution, not the Task's delivery obligations. The
        // child's exact result and the supervisor's resource outcome remain
        // independent facts in the receipt; neither implies parent acceptance.
        let task_terminal_status = "sealed";
        if matches!(task_status.as_str(), "sealed" | "failed" | "cancelled") {
            let receipt: Option<String> = tx
                .query_row(
                    "SELECT receipt_json FROM runtime_task_terminal_receipts WHERE task_id=?1",
                    [task_id],
                    |row| row.get(0),
                )
                .optional()?;
            let receipt = receipt
                .map(|raw| serde_json::from_str::<RuntimeTaskTerminalReceipt>(&raw))
                .transpose()?;
            let matched = task_status == task_terminal_status
                && receipt.is_some_and(|receipt| receipt.journal_result.as_ref() == Some(&result));
            tx.rollback()?;
            return Ok(matched);
        }
        if let Some(scheduling) = scheduling {
            metadata[nola_domain::runtime_task_scheduling::SCHEDULING_METADATA_KEY] = serde_json::to_value(scheduling)?;
            tx.execute("UPDATE durable_tasks SET metadata_json=?2 WHERE id=?1",
                params![task_id, serde_json::to_string(&metadata)?])?;
        }
        // Canonical Run terminal is authority to settle, not permission to
        // execute again. Retire every live worker in the same transaction.
        crate::permission_transport::revoke_task_request_grants(&tx, task_id, now_ms)?;
        tx.execute("UPDATE runtime_task_attempts SET state='superseded',revision=revision+1 WHERE task_id=?1 AND state IN ('leased','permission_suspended')",params![task_id])?;
        crate::permission_source::close_from_run(&tx, task_id, &result.seal, now_ms)?;
        tx.execute("UPDATE runtime_task_permission_suspensions SET state='cancelled',revision=revision+1 WHERE task_id=?1 AND state='pending'",params![task_id])?;
        tx.execute("UPDATE durable_tasks SET status=?2,worker_id=NULL,ownership_token=NULL,lease_until_ms=NULL,heartbeat_at_ms=NULL,
            retry_at_ms=NULL,completion_watermark=input_watermark,ended_at_ms=?3,updated_at_ms=?4,unread=1,
            progress_summary=?2,result_summary=?5,error=?6,
            metadata_json=json_set(metadata_json,'$.model_notified',json('false'))
            WHERE id=?1",params![task_id,task_terminal_status,completed_at,now_ms,&result.preview,
                budget_failure.as_ref().map(|failure| format!("Task resource budget exceeded: {:?} (observed {}, limit {})",failure.resource,failure.observed,failure.limit))])?;
        tx.execute("INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
            VALUES(?1,'bound_run_terminal',?2,'Recovered committed child Run result',?3,?4)",
            params![task_id,task_terminal_status,serde_json::json!({"binding":bound,"assistant_message_id":message_id,"completed_at_ms":completed_at,"child_status":status,"budget_failure":budget_failure}).to_string(),now_ms])?;
        crate::runtime_task_terminal::record_terminal_receipt_with_journal(
            &tx,
            task_id,
            RuntimeTaskTerminalSource::BoundRunTerminal {
                binding: bound,
                assistant_message_id: message_id,
                budget_failure,
            },
            now_ms,
            Some(result),
        )?;
        tx.commit()?;
        Ok(true)
    }
}
