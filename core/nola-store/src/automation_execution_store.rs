// SPDX-License-Identifier: Apache-2.0
//! Occurrence ownership and exact Run result reconciliation; no exit-code result inference.
use crate::{NolaStore, StoreError, StoreResult};
use nola_store_port::automation::*;
use nola_store_port::assistant_journal::AssistantJournalRepository;
use rusqlite::{params, OptionalExtension};

impl NolaStore {
    pub fn pending_automation_executions(&self, after: Option<&str>, limit: usize) -> StoreResult<Vec<String>> {
        if !(1..=256).contains(&limit) { return Err(StoreError::Invariant("automation recovery page size must be 1...256".into())); }
        let mut statement = self.connection.prepare("SELECT r.id FROM automation_runs r
            LEFT JOIN automation_execution_receipts receipt ON receipt.occurrence_id=r.id
            WHERE (?1 IS NULL OR r.id>?1) AND receipt.delivered_at_ms IS NULL
            AND (r.status='running' OR receipt.occurrence_id IS NOT NULL)
            ORDER BY r.id LIMIT ?2")?;
        let rows = statement.query_map(params![after,limit as i64], |row|row.get(0))?;
        rows.collect::<Result<Vec<_>,_>>().map_err(StoreError::from)
    }
    pub fn validate_automation_execution(&self, lease: &AutomationExecutionLease, now: i64) -> StoreResult<()> {
        let occurrence = self.automation_run(&lease.occurrence_id)?
            .ok_or_else(|| StoreError::Invariant("automation occurrence missing".into()))?;
        self.automation_definition(&occurrence.automation_id)?
            .ok_or_else(|| StoreError::Invariant("automation definition missing".into()))?
            .execution_spec.agent().map_err(StoreError::Invariant)?;
        if lease.occurrence_id.trim().is_empty() || lease.worker_id.trim().is_empty()
            || lease.lease_token.trim().is_empty() || lease.definition_revision < 1 {
            return Err(StoreError::Invariant("automation admission requires exact lease identity".into()));
        }
        let valid: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM automation_runs r JOIN automation_definitions d ON d.id=r.automation_id
             WHERE r.id=?1 AND r.worker_id=?2 AND r.lease_token=?3 AND r.status='running'
             AND r.lease_until_ms>?4 AND r.updated_at_ms<=?4 AND r.definition_revision=?5
             AND d.revision=r.definition_revision AND d.enabled=1 AND d.deleted_at_ms IS NULL)",
            params![lease.occurrence_id,lease.worker_id,lease.lease_token,now,lease.definition_revision], |r|r.get(0))?;
        if !valid { return Err(StoreError::Conflict("automation definition or execution lease changed".into())); }
        Ok(())
    }

    pub fn replay_automation_admission(&self, id: &str, digest: &str) -> StoreResult<Option<String>> {
        let existing: Option<(String,String)> = self.connection.query_row(
            "SELECT request_digest,accepted_json FROM automation_execution_bindings WHERE occurrence_id=?1", [id],
            |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        match existing {
            Some((expected, accepted)) if expected == digest => Ok(Some(accepted)),
            Some(_) => Err(StoreError::Conflict("automation occurrence is bound to a different admission; observe and recover its Run".into())),
            None => Ok(None),
        }
    }

    pub fn bind_automation_execution(&mut self, lease: &AutomationExecutionLease, binding: &AutomationExecutionBinding, request_digest: &str, accepted_json: &str) -> StoreResult<()> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Invariant("automation binding requires Run admission transaction".into()));
        }
        self.validate_automation_execution(lease, binding.bound_at_ms)?;
        let identity = AutomationExecutionIdentity::for_occurrence(&lease.occurrence_id);
        if binding.occurrence_id != lease.occurrence_id || binding.definition_revision != lease.definition_revision
            || binding.conversation_id != identity.conversation_id || binding.submission_id != identity.submission_id {
            return Err(StoreError::Conflict("automation binding differs from occurrence admission".into()));
        }
        let exact: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND conversation_id=?2 AND kind='conversation'
             AND created_at_ms=?3 AND invocation_config_digest=?4 AND status<>'sealed' AND user_message_id=?5 AND assistant_message_id=?6)",
            params![binding.run_id,binding.conversation_id,binding.bound_at_ms,binding.invocation_config_digest,identity.user_message_id,identity.assistant_message_id], |r|r.get(0))?;
        if !exact { return Err(StoreError::Conflict("automation binding has no exact newly admitted Run".into())); }
        let accepted: serde_json::Value = serde_json::from_str(accepted_json)?;
        if request_digest.len()!=64 || !request_digest.bytes().all(|b|b.is_ascii_hexdigit())
            || accepted["begin"]["run_id"].as_str()!=Some(binding.run_id.as_str())
            || accepted["begin"]["conversation_id"].as_str()!=Some(binding.conversation_id.as_str())
            || accepted["invocation_config_digest"].as_str()!=Some(binding.invocation_config_digest.as_str()) {
            return Err(StoreError::Conflict("automation admission replay receipt differs from exact Run".into()));
        }
        self.connection.execute("INSERT INTO automation_execution_bindings(occurrence_id,conversation_id,run_id,binding_json,request_digest,accepted_json) VALUES(?1,?2,?3,?4,?5,?6)",
            params![binding.occurrence_id,binding.conversation_id,binding.run_id,serde_json::to_string(binding)?,request_digest,accepted_json])?;
        Ok(())
    }

    pub fn observe_automation_execution(&self, id: &str) -> StoreResult<AutomationExecutionObservation> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let occurrence = self.automation_run(id)?.ok_or_else(|| StoreError::Conflict("automation occurrence does not exist".into()))?;
        let worker_raw: Option<String> = self.connection.query_row(
            "SELECT observation_json FROM automation_worker_observations WHERE occurrence_id=?1
             AND (?2 IS NULL OR (worker_id=?2 AND lease_token=?3)) ORDER BY id DESC LIMIT 1",
            params![id,occurrence.worker_id,occurrence.lease_token], |r| r.get(0)).optional()?;
        let latest_worker_observation: Option<AutomationWorkerObservation> = worker_raw.map(|raw| serde_json::from_str(&raw)).transpose()?;
        let raw: Option<String> = self.connection.query_row("SELECT binding_json FROM automation_execution_bindings WHERE occurrence_id=?1",
            [id], |r|r.get(0)).optional()?;
        let binding: Option<AutomationExecutionBinding> = raw.map(|s|serde_json::from_str(&s)).transpose()?;
        let (run_status, terminal) = if let Some(binding) = &binding {
            let status: String = self.connection.query_row("SELECT status FROM runs WHERE id=?1 AND conversation_id=?2 AND invocation_config_digest=?3",
                params![binding.run_id,binding.conversation_id,binding.invocation_config_digest], |r|r.get(0))?;
            let seal = self.local_assistant_run_terminal_seal(&binding.conversation_id, &binding.run_id)?;
            if let Some(seal) = &seal {
                seal.validate().map_err(StoreError::Invariant)?;
                if status != "sealed" || seal.result.run_id != binding.run_id
                    || seal.result.conversation_id != binding.conversation_id
                    || seal.result.originating_submission_id != binding.submission_id {
                    return Err(StoreError::Conflict("automation seal does not match bound occurrence".into()));
                }
            } else if status == "sealed" { return Err(StoreError::Invariant("sealed automation Run has no result seal".into())); }
            (Some(status), seal)
        } else { (None, None) };
        let receipt_row: Option<(String, Option<i64>)> = self.connection.query_row(
            "SELECT receipt_json,delivered_at_ms FROM automation_execution_receipts WHERE occurrence_id=?1", [id],
            |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let notification_delivered = receipt_row.as_ref().is_some_and(|row|row.1.is_some());
        let receipt: Option<AutomationExecutionReceipt> = receipt_row.map(|row|serde_json::from_str(&row.0)).transpose()?;
        if let Some(receipt) = &receipt {
            if binding.as_ref()!=Some(&receipt.binding) || terminal.as_ref()!=Some(&receipt.terminal) {
                return Err(StoreError::Invariant("automation reconciliation receipt differs from exact bound seal".into()));
            }
        }
        let phase = if terminal.is_some() { AutomationExecutionPhase::Terminal }
            else if binding.is_some() { AutomationExecutionPhase::Admitted }
            else if occurrence.status == "launch_failed" { AutomationExecutionPhase::LaunchFailed }
            else if occurrence.status != "running" { AutomationExecutionPhase::Queued }
            else {
                let admission_requested: bool = self.connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM automation_worker_observations WHERE occurrence_id=?1 AND json_extract(observation_json,'$.kind')='admission_requested')",
                    [id], |r| r.get(0))?;
                if admission_requested { AutomationExecutionPhase::AdmissionPending }
                else if matches!(latest_worker_observation.as_ref(), Some(AutomationWorkerObservation::Preparing)) { AutomationExecutionPhase::Preparing }
                else { AutomationExecutionPhase::Claimed }
            };
        Ok(AutomationExecutionObservation { phase, latest_worker_observation, identity: AutomationExecutionIdentity::for_occurrence(id), binding, run_status, terminal, receipt, notification_delivered })
    }

    pub fn reconcile_automation_execution(&mut self, id: &str, now: i64) -> StoreResult<Option<AutomationExecutionReceipt>> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Invariant("automation reconciliation requires Application unit of work".into()));
        }
        let observed = self.observe_automation_execution(id)?;
        let tx = self.write_transaction()?;
        let existing: Option<String> = tx.query_row("SELECT receipt_json FROM automation_execution_receipts WHERE occurrence_id=?1", [id], |r|r.get(0)).optional()?;
        if let Some(existing) = existing { return Ok(Some(serde_json::from_str(&existing)?)); }
        // The transaction stays open while all exact binding/seal reads use the same connection.
        let (Some(binding), Some(terminal)) = (observed.binding, observed.terminal) else { return Ok(None); };
        if now < terminal.completed_at_ms { return Err(StoreError::Conflict("automation reconciliation precedes sealed result".into())); }
        let receipt = AutomationExecutionReceipt { notification_id: format!("automation-result:{id}"), binding, terminal, reconciled_at_ms: now };
        tx.execute("INSERT INTO automation_execution_receipts(occurrence_id,notification_id,receipt_json) VALUES(?1,?2,?3)",
            params![id,receipt.notification_id,serde_json::to_string(&receipt)?])?;
        tx.execute("UPDATE automation_runs SET status='reconciled',finished_at_ms=?2,updated_at_ms=?2,worker_id=NULL,lease_token=NULL,lease_until_ms=NULL WHERE id=?1",params![id,now])?;
        tx.commit()?;
        Ok(Some(receipt))
    }

    pub fn observe_automation_worker(&mut self, lease: &AutomationExecutionLease, observation: &AutomationWorkerObservation, now: i64) -> StoreResult<bool> {
        let tx = self.write_transaction()?;
        // A late exit remains an observation only when this is still the current worker;
        // expired leases never gain dispatch authority through this receipt.
        let owned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM automation_runs WHERE id=?1 AND worker_id=?2 AND lease_token=?3 AND updated_at_ms<=?4 AND definition_revision=?5 AND status='running'
            AND NOT EXISTS(SELECT 1 FROM automation_worker_observations WHERE occurrence_id=?1 AND observed_at_ms>?4))",
            params![lease.occurrence_id,lease.worker_id,lease.lease_token,now,lease.definition_revision], |r|r.get(0))?;
        if !owned { return Ok(false); }
        let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM automation_execution_bindings WHERE occurrence_id=?1)",
            [&lease.occurrence_id], |r| r.get(0))?;
        if matches!(observation, AutomationWorkerObservation::Preparing | AutomationWorkerObservation::AdmissionRequested) {
            let live: bool = tx.query_row("SELECT lease_until_ms>?2 FROM automation_runs WHERE id=?1", params![lease.occurrence_id,now], |r|r.get(0))?;
            if !live || bound { return Ok(false); }
        }
        let bounded_detail = |detail: &str| {
            let mut chars = detail.chars();
            let head: String = chars.by_ref().take(2_048).collect();
            if chars.next().is_some() { format!("{head} [truncated]") } else { head }
        };
        let recorded = match observation {
            AutomationWorkerObservation::LaunchFailed { detail } if bound => AutomationWorkerObservation::Disconnected {
                detail: format!("Startup transport failed after Run admission: {}", bounded_detail(detail)),
            },
            AutomationWorkerObservation::LaunchFailed { detail } => AutomationWorkerObservation::LaunchFailed { detail: bounded_detail(detail) },
            AutomationWorkerObservation::Disconnected { detail } => AutomationWorkerObservation::Disconnected { detail: bounded_detail(detail) },
            other => other.clone(),
        };
        let payload = serde_json::to_string(&recorded)?;
        if payload.len()>16384 { return Err(StoreError::Invariant("automation observation exceeds diagnostic bound".into())); }
        tx.execute("INSERT INTO automation_worker_observations(occurrence_id,worker_id,lease_token,observation_json,observed_at_ms) VALUES(?1,?2,?3,?4,?5)",
            params![lease.occurrence_id,lease.worker_id,lease.lease_token,payload,now])?;
        // Run publication and binding are one transaction. Closing the unbound
        // occurrence under this same write fence revokes any delayed admission;
        // a previously committed admission wins and is reconciled by exact Run.
        if matches!(observation, AutomationWorkerObservation::LaunchFailed { .. }) && !bound {
            tx.execute("UPDATE automation_runs SET status='launch_failed',error_json=?2,finished_at_ms=?3,updated_at_ms=?3,
                worker_id=NULL,lease_token=NULL,lease_until_ms=NULL,next_retry_at_ms=NULL WHERE id=?1",
                params![lease.occurrence_id,payload,now])?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn acknowledge_automation_notification(&mut self, id: &str, notification_id: &str, now: i64) -> StoreResult<bool> {
        let Some(receipt) = self.observe_automation_execution(id)?.receipt else { return Ok(false); };
        if now < receipt.reconciled_at_ms { return Ok(false); }
        Ok(self.connection.execute("UPDATE automation_execution_receipts SET delivered_at_ms=COALESCE(delivered_at_ms,?3) WHERE occurrence_id=?1 AND notification_id=?2",
            params![id,notification_id,now])?>0)
    }
}
