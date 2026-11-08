// SPDX-License-Identifier: Apache-2.0
//! Exact, fenced successor grants for an already resolved permission occurrence.
use super::*;
use nola_store_port::runtime_task::{RecoverRuntimeTaskPermissionContinuationFair, RuntimeTaskPermissionContinuationGrant, PermissionContinuationGrantSelector};

fn attempt(connection: &Connection, id: &str) -> StoreResult<Option<RuntimeTaskAttempt>> {
    connection.query_row(
        "SELECT id,task_id,attempt_number,worker_id,worker_generation,lease_token,lease_expires_at_ms,state,revision,claimed_at_ms
         FROM runtime_task_attempts WHERE id=?1", params![id], |row| Ok(RuntimeTaskAttempt {
            id: row.get(0)?, task_id: row.get(1)?, attempt_number: row.get(2)?, worker_id: row.get(3)?,
            worker_generation: row.get(4)?, lease_token: row.get(5)?, lease_expires_at_ms: row.get(6)?,
            state: row.get(7)?, revision: row.get(8)?, claimed_at_ms: row.get(9)?,
        }),
    ).optional().map_err(StoreError::from)
}

fn grant_chain(connection: &Connection, task_id: &str, suspension_id: &str, selector: &PermissionContinuationGrantSelector)
    -> StoreResult<Option<RuntimeTaskPermissionContinuationGrant>> {
    let suspension = connection.query_row(
        "SELECT id,task_id,attempt_id,permission_request_id,continuation_id,continuation_json,state,revision,suspended_at_ms,resumed_at_ms
         FROM runtime_task_permission_suspensions WHERE id=?1", params![suspension_id], |row| {
            Ok(nola_store_port::runtime_task::RuntimeTaskPermissionSuspension {
                id: row.get(0)?, task_id: row.get(1)?, attempt_id: row.get(2)?, permission_request_id: row.get(3)?,
                continuation_id: row.get(4)?, continuation_json: row.get(5)?, state: row.get(6)?, revision: row.get(7)?,
                suspended_at_ms: row.get(8)?, resumed_at_ms: row.get(9)?,
            })
        }).optional()?;
    let Some(suspension) = suspension else { return Ok(None); };
    if suspension.task_id != task_id { return Err(StoreError::Conflict("permission grant task differs".into())); }
    if !matches!(suspension.state.as_str(), "resumed" | "settled") { return Ok(None); }
    let original = attempt(connection, &suspension.attempt_id)?
        .ok_or_else(|| StoreError::Invariant("permission original attempt missing".into()))?;
    if original.task_id != task_id { return Err(StoreError::Invariant("permission original attempt belongs to another task".into())); }
    let mut statement = connection.prepare(
        "SELECT kind,detail_json FROM durable_task_events WHERE task_id=?1
         AND kind IN ('permission_resumed','permission_recovered') AND json_extract(detail_json,'$.suspension_id')=?2")?;
    let records = statement.query_map(params![task_id,suspension_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))?
        .collect::<Result<Vec<_>,_>>()?;
    let mut grants = Vec::new();
    for (kind, json) in records {
        let record: serde_json::Value = serde_json::from_str(&json)?;
        let grant: RuntimeTaskAttempt = serde_json::from_value(record["granted_attempt"].clone())?;
        grants.push((grant, kind, record));
    }
    grants.sort_by_key(|(grant,_,_)| grant.attempt_number);
    let mut ancestors = vec![original];
    let mut result = None;
    for (index, (grant, kind, record)) in grants.into_iter().enumerate() {
        let previous = ancestors.last().expect("original ancestor exists");
        let current = attempt(connection, &grant.id)?.ok_or_else(|| StoreError::Invariant("permission grant attempt missing".into()))?;
        let expected_kind = if index == 0 { "permission_resumed" } else { "permission_recovered" };
        if kind != expected_kind || grant.task_id != task_id || current.task_id != task_id
            || grant.id != current.id || grant.worker_id != current.worker_id || grant.worker_generation != current.worker_generation
            || grant.lease_token != current.lease_token || grant.claimed_at_ms != current.claimed_at_ms
            || grant.attempt_number != current.attempt_number || grant.attempt_number != previous.attempt_number + 1
            || grant.revision != 1 || grant.state != "leased" || grant.lease_expires_at_ms <= grant.claimed_at_ms
            || grant.claimed_at_ms < previous.claimed_at_ms
            || ancestors.iter().any(|a| a.id == grant.id || a.lease_token == grant.lease_token || a.worker_generation == grant.worker_generation)
            || record["attempt_id"].as_str() != Some(grant.id.as_str())
            || record["permission_request_id"].as_str() != Some(suspension.permission_request_id.as_str())
            || record["continuation_id"].as_str() != Some(suspension.continuation_id.as_str())
            || (index == 0 && suspension.resumed_at_ms != Some(grant.claimed_at_ms))
            || (index > 0 && record["predecessor_attempt_id"].as_str() != Some(previous.id.as_str())) {
            return Err(StoreError::Invariant("permission continuation grant lineage differs".into()));
        }
        if match selector {
            PermissionContinuationGrantSelector::ExactAttempt { attempt_id } => grant.id == *attempt_id,
            PermissionContinuationGrantSelector::RecoveryCandidate => true,
        } {
            result = Some(RuntimeTaskPermissionContinuationGrant {
                suspension: suspension.clone(), ancestors: ancestors.clone(), granted_attempt: grant.clone(),
            });
        }
        ancestors.push(grant);
    }
    Ok(result)
}

impl NolaStore {
    pub fn runtime_task_permission_continuation_grant(&self, task_id: &str, suspension_id: &str, selector: &PermissionContinuationGrantSelector)
        -> StoreResult<Option<RuntimeTaskPermissionContinuationGrant>> {
        for (name, value) in [("task_id",task_id),("suspension_id",suspension_id)] { require_id(name,value)?; }
        if let PermissionContinuationGrantSelector::ExactAttempt { attempt_id } = selector { require_id("attempt_id",attempt_id)?; }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        grant_chain(&self.connection, task_id, suspension_id, selector)
    }

    pub fn recover_runtime_task_permission_continuation_fair(&mut self, input: &RecoverRuntimeTaskPermissionContinuationFair)
        -> StoreResult<Option<RuntimeTaskAttempt>> {
        let old = &input.predecessor;
        for (name,value) in [("task_id",old.task_id.as_str()),("attempt_id",old.attempt_id.as_str()),
            ("suspension_id",input.suspension_id.as_str()),("permission_request_id",input.permission_request_id.as_str()),
            ("continuation_id",input.continuation_id.as_str()),("grant_attempt_id",input.grant_attempt_id.as_str()),
            ("child_session_id",input.expected_child_session_id.as_str()),("run_id",input.expected_run_id.as_str()),
            ("worker_id",input.worker_id.as_str()),("worker_generation",input.worker_generation.as_str()),("lease_token",input.lease_token.as_str())] {
            require_runtime_task_claim_identity(name,value)?;
        }
        if old.expected_attempt_revision < 1 || input.expected_suspension_revision < 2
            || old.current_expiry_ms > input.now_ms || input.lease_until_ms <= input.now_ms
            || input.lease_until_ms.saturating_sub(input.now_ms) > 600_000
            || input.max_running_global == 0 || input.max_running_project == 0 || input.max_recoveries == 0 {
            return Err(StoreError::Invariant("invalid permission continuation recovery witness".into()));
        }
        let tx = self.write_transaction()?;
        if !crate::agent_task_admission_store::permits_execution(&tx,&old.task_id)? {
            tx.rollback()?;return Ok(None);
        }
        let proof = grant_chain(&tx, &old.task_id, &input.suspension_id,
            &PermissionContinuationGrantSelector::ExactAttempt { attempt_id: old.attempt_id.clone() })?;
        let Some(proof) = proof else { tx.rollback()?; return Ok(None); };
        let continuation: serde_json::Value = serde_json::from_str(&proof.suspension.continuation_json)?;
        if proof.suspension.revision != input.expected_suspension_revision
            || proof.suspension.permission_request_id != input.permission_request_id
            || proof.suspension.continuation_id != input.continuation_id
            || continuation["child_session_id"].as_str() != Some(input.expected_child_session_id.as_str())
            || continuation["run_id"].as_str() != Some(input.expected_run_id.as_str())
            || proof.ancestors.iter().chain(std::iter::once(&proof.granted_attempt))
                .any(|a| a.id == input.grant_attempt_id || a.worker_generation == input.worker_generation || a.lease_token == input.lease_token) {
            tx.rollback()?; return Ok(None);
        }
        let eligible: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_tasks t JOIN runtime_task_attempts a ON a.task_id=t.id
             JOIN permission_requests p ON p.request_id=?8 AND p.task_id=t.id
             WHERE t.id=?1 AND t.status='running' AND t.cancel_requested=0 AND t.ownership_token=?4
               AND t.lease_until_ms=?5 AND t.updated_at_ms<=?6 AND t.retry_count<?9 AND t.child_session_id=?10
               AND t.worker_id=a.worker_id
               AND a.id=?2 AND a.revision=?3 AND a.worker_generation=?7 AND a.lease_token=?4
               AND a.lease_expires_at_ms=?5 AND a.lease_expires_at_ms<=?6 AND a.state='leased'
               AND p.status='resolved' AND p.decision IS NOT NULL
               AND NOT EXISTS(SELECT 1 FROM runtime_task_attempts newer WHERE newer.task_id=t.id AND newer.attempt_number>a.attempt_number)
               AND EXISTS(SELECT 1 FROM runs r WHERE r.id=?11 AND r.conversation_id=?10 AND r.status <> 'sealed')
               AND ?11=(SELECT r.id FROM runs r WHERE r.conversation_id=?10 AND r.status <> 'sealed' ORDER BY r.created_at_ms DESC LIMIT 1)
               AND NOT EXISTS(SELECT 1 FROM runtime_task_permission_suspensions s JOIN runtime_task_attempts sa ON sa.id=s.attempt_id
                 WHERE s.task_id=t.id AND sa.attempt_number>(SELECT attempt_number FROM runtime_task_attempts WHERE id=?12)))",
            params![old.task_id,old.attempt_id,old.expected_attempt_revision,old.lease_token,old.current_expiry_ms,input.now_ms,
                old.worker_generation,input.permission_request_id,input.max_recoveries,input.expected_child_session_id,input.expected_run_id,proof.suspension.attempt_id],
            |row| row.get(0),
        )?;
        let global: i64 = tx.query_row("SELECT COUNT(*) FROM durable_tasks WHERE status='running' AND lease_until_ms>?1",params![input.now_ms],|row|row.get(0))?;
        if !eligible || global >= i64::from(input.max_running_global)
            || crate::runtime_task_claim_store::fair_candidate(&tx,input.now_ms,input.max_running_project)?.as_deref() != Some(old.task_id.as_str()) {
            tx.rollback()?; return Ok(None);
        }
        crate::runtime_task_claim_store::require_fresh_lease_token(&tx, &old.task_id, &input.lease_token)?;
        let grant = RuntimeTaskAttempt {
            id: input.grant_attempt_id.clone(), task_id: old.task_id.clone(), attempt_number: proof.granted_attempt.attempt_number+1,
            worker_id: input.worker_id.clone(), worker_generation: input.worker_generation.clone(), lease_token: input.lease_token.clone(),
            lease_expires_at_ms: input.lease_until_ms, state: "leased".into(), revision: 1, claimed_at_ms: input.now_ms,
        };
        tx.execute("UPDATE runtime_task_attempts SET state='superseded',revision=revision+1 WHERE id=?1",params![old.attempt_id])?;
        tx.execute("INSERT INTO runtime_task_attempts(id,task_id,attempt_number,worker_id,worker_generation,lease_token,lease_expires_at_ms,state,revision,claimed_at_ms)
            VALUES(?1,?2,?3,?4,?5,?6,?7,'leased',1,?8)",params![grant.id,grant.task_id,grant.attempt_number,grant.worker_id,grant.worker_generation,grant.lease_token,grant.lease_expires_at_ms,grant.claimed_at_ms])?;
        tx.execute("UPDATE durable_tasks SET worker_id=?2,ownership_token=?3,lease_until_ms=?4,heartbeat_at_ms=?5,updated_at_ms=?5,retry_count=retry_count+1
            WHERE id=?1",params![old.task_id,grant.worker_id,grant.lease_token,grant.lease_expires_at_ms,input.now_ms])?;
        tx.execute("UPDATE runtime_task_permission_suspensions SET revision=revision+1 WHERE id=?1",params![input.suspension_id])?;
        tx.execute("INSERT INTO durable_task_events(task_id,kind,status,summary,detail_json,created_at_ms)
            VALUES(?1,'permission_recovered','running','Permission continuation recovered',?2,?3)",params![old.task_id,
            serde_json::json!({"suspension_id":input.suspension_id,"permission_request_id":input.permission_request_id,
                "continuation_id":input.continuation_id,"predecessor_attempt_id":old.attempt_id,
                "attempt_id":grant.id,"granted_attempt":grant}).to_string(),input.now_ms])?;
        tx.commit()?;
        Ok(Some(grant))
    }
}
