// SPDX-License-Identifier: Apache-2.0
//! Aggregate inspection excludes secrets. Explicit recovery-target inspection
//! additionally returns original fencing only through the trusted local port.
use super::*;
use nola_store_port::run_recovery_inspection::{
    RunRecoveryInspectionRepository, StoredRunRecoveryInspection,
};

impl RunRecoveryInspectionRepository for NolaStore {
    type Error = StoreError;
    fn list_recoverable_runs(&self, after_run_id: Option<&str>, limit: usize)
        -> StoreResult<Vec<nola_store_port::run_recovery_inspection::RecoverableRunRef>> {
        if !(1..=256).contains(&limit) || after_run_id.is_some_and(|id| id.trim().is_empty()) {
            return Err(StoreError::Invariant("invalid recoverable Run discovery page".into()));
        }
        let mut statement = self.connection.prepare(
            "SELECT r.conversation_id,r.id,r.assistant_message_id FROM runs r
             JOIN messages m ON m.id=r.assistant_message_id
               AND m.conversation_id=r.conversation_id AND m.run_id=r.id AND m.role='assistant'
             WHERE r.kind='conversation' AND r.status IN ('queued','running','waiting_input')
               AND r.completed_at_ms IS NULL AND (?1 IS NULL OR r.id > ?1)
               AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id)
             ORDER BY r.id LIMIT ?2")?;
        let rows = statement.query_map(params![after_run_id, limit as i64], |row| {
            Ok(nola_store_port::run_recovery_inspection::RecoverableRunRef {
                conversation_id: row.get(0)?, run_id: row.get(1)?, assistant_message_id: row.get(2)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    fn inspect_unknown_effect_recovery(
        &self, conversation_id: &str, run_id: &str, assistant_message_id: &str,
    ) -> StoreResult<Option<nola_store_port::run_recovery_inspection::StoredUnknownEffectRecoveryInspection>> {
        let _snapshot = read_snapshot::pin(&self.connection)?;
        let Some(run) = self.inspect_exact_run(run_id)? else { return Ok(None); };
        if run.conversation_id != conversation_id
            || run.assistant_message_id.as_deref() != Some(assistant_message_id)
            || !run.assistant_owner_valid || run.source_host_id.is_some()
            || run.kind != "conversation"
        {
            return Err(StoreError::Invariant("unknown-effect recovery owner mismatch".into()));
        }
        if run.status == "sealed" { return Ok(None); }
        if !matches!(run.status.as_str(), "queued" | "running" | "waiting_input")
            || run.completed_at_ms.is_some() || run.terminal.is_some()
        {
            return Err(StoreError::Invariant("unknown-effect recovery requires a live local Run".into()));
        }
        let invocation = exact_run_invocation::load(self, conversation_id, run_id, assistant_message_id)?
            .ok_or_else(|| StoreError::Invariant("recovery Run has no exact local invocation".into()))?;
        let mut statement = self.connection.prepare(
            "SELECT a.record_json, COALESCE(
                a.id=json_extract(a.record_json,'$.id')
                AND a.run_id=json_extract(a.record_json,'$.run_id')
                AND a.effect_id=json_extract(a.record_json,'$.effect_id')
                AND a.request_id=json_extract(a.record_json,'$.request_id')
                AND a.state=json_extract(a.record_json,'$.state')
                AND a.revision=json_extract(a.record_json,'$.revision')
                AND a.attempt_number=json_extract(a.record_json,'$.attempt_number')
                AND a.replay_safety=json_extract(a.record_json,'$.replay_safety')
                AND a.predecessor_id IS json_extract(a.record_json,'$.predecessor_id')
                AND a.recovery_plan IS json_extract(a.record_json,'$.recovery_plan')
                AND a.worker_generation IS json_extract(a.record_json,'$.worker_generation')
                AND a.lease_owner IS json_extract(a.record_json,'$.lease_owner')
                AND a.lease_token IS json_extract(a.record_json,'$.lease_token')
                AND a.lease_expires_at_ms IS json_extract(a.record_json,'$.lease_expires_at_ms'), 0)
             FROM effect_delivery_attempts a
             JOIN effect_outbox e ON e.effect_id=a.effect_id AND e.run_id=a.run_id AND e.request_id=a.request_id
             WHERE a.run_id=?1 AND ((a.state='outcome_unknown' AND e.status='pending') OR
               EXISTS(SELECT 1 FROM effect_outbox c JOIN effect_outbox_payloads cp ON cp.effect_id=c.effect_id
                 WHERE c.run_id=a.run_id AND c.status='pending'
                 AND json_extract(cp.payload_json,'$.payload.type')='cancel'
                 AND json_extract(cp.payload_json,'$.payload.target_effect_id')=a.effect_id))
             AND NOT EXISTS(SELECT 1 FROM effect_delivery_attempts s WHERE s.predecessor_id=a.id)
             ORDER BY a.effect_id,a.id LIMIT 257")?;
        let records = statement.query_map(params![run_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        if records.len() > 256 {
            return Err(StoreError::Invariant("unknown-effect recovery exceeds bounded target window".into()));
        }
        let mut retry_allowed = true;
        let mut targets = Vec::with_capacity(records.len());
        let mut effects = std::collections::HashSet::new();
        for (record, columns_match) in records {
            let attempt: nola_domain::EffectDeliveryAttempt = serde_json::from_str(&record)?;
            nola_kernel::validate_effect_delivery_attempt(&attempt).map_err(StoreError::Invariant)?;
            let unknown = attempt.state == nola_domain::EffectDeliveryAttemptState::OutcomeUnknown;
            retry_allowed &= unknown;
            if !columns_match || attempt.run_id != run_id
                || (unknown && (attempt.execution_started_at_ms.is_none()
                    || attempt.finished_at_ms.is_none() || attempt.settlement.is_some()))
                || !effects.insert(attempt.effect_id.clone())
            {
                return Err(StoreError::Invariant("unknown-effect recovery attempt facts are inconsistent".into()));
            }
            let required = |value: Option<String>| value.filter(|value| !value.trim().is_empty())
                .ok_or_else(|| StoreError::Invariant("unknown-effect recovery has incomplete original fencing".into()));
            targets.push(nola_domain::unknown_effect_recovery::UnknownEffectRecoveryTarget {
                effect_id: attempt.effect_id, request_id: attempt.request_id, attempt_id: attempt.id,
                worker_generation: required(attempt.worker_generation)?,
                lease_owner: required(attempt.lease_owner)?, lease_token: required(attempt.lease_token)?,
                expected_attempt_revision: attempt.revision,
            });
        }
        // An ordinary pending/leased effect is not an unknown outcome. Only
        // actual OutcomeUnknown targets require the explicit risk-decision UI.
        // Independent pending work matters only alongside those targets.
        if targets.is_empty() { return Ok(None); }
        // A Cancel control for these exact targets belongs to the same
        // unresolved operation, not unrelated work. This is UI metadata only;
        // Application and Core validate the control relation again on commit.
        let mut independent_pending_effects = 0;
        for effect in self.pending_effects(run_id)? {
            if effects.contains(&effect.effect_id) { continue; }
            let payload: serde_json::Value = serde_json::from_str(&effect.payload_json)?;
            if payload["payload"]["type"] == "cancel"
                && payload["payload"]["target_effect_id"].as_str().is_some_and(|id|effects.contains(id)) { retry_allowed=false; continue; }
            independent_pending_effects += 1;
        }
        Ok(Some(nola_store_port::run_recovery_inspection::StoredUnknownEffectRecoveryInspection {
            invocation_config_digest: invocation.invocation_config_digest,
            targets, independent_pending_effects, retry_allowed,
        }))
    }

    fn load_exact_run_invocation(
        &self,
        conversation_id: &str,
        run_id: &str,
        assistant_message_id: &str,
    ) -> StoreResult<Option<nola_store_port::run_recovery_inspection::StoredExactRunInvocation>>
    {
        exact_run_invocation::load(self, conversation_id, run_id, assistant_message_id)
    }
    fn inspect_exact_run(&self, run_id: &str) -> StoreResult<Option<StoredRunRecoveryInspection>> {
        require_id("run_id", run_id)?;
        let _snapshot = read_snapshot::pin(&self.connection)?;
        let header:Option<(String,Option<String>,String,String,Option<i64>,bool)>=self.connection.query_row(
            "SELECT r.conversation_id,r.assistant_message_id,r.kind,r.status,r.completed_at_ms,
             EXISTS(SELECT 1 FROM messages m WHERE m.id=r.assistant_message_id AND m.run_id=r.id AND m.conversation_id=r.conversation_id AND m.role='assistant') FROM runs r WHERE r.id=?1",
            params![run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
        let Some((
            conversation_id,
            assistant_message_id,
            kind,
            status,
            completed_at_ms,
            assistant_owner_valid,
        )) = header
        else {
            return Ok(None);
        };
        let (checkpoint_present, checkpoint_body_present): (bool, bool) =
            self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM core_checkpoints WHERE conversation_id=?1),
                    EXISTS(SELECT 1 FROM core_checkpoint_bodies WHERE conversation_id=?1)",
                params![conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
        if checkpoint_present != checkpoint_body_present {
            return Err(StoreError::Invariant(
                "canonical checkpoint header/body is incomplete".into(),
            ));
        }
        let pending_effect_count = self.connection.query_row(
            "SELECT COUNT(*) FROM effect_outbox WHERE run_id=?1 AND status='pending'",
            params![run_id],
            |r| r.get(0),
        )?;
        let source_host_id = self
            .connection
            .query_row(
                "SELECT source_host_id FROM remote_run_sources WHERE run_id=?1",
                params![run_id],
                |r| r.get(0),
            )
            .optional()?;
        let has_session_lease = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_run_leases WHERE session_id=?1)",
            params![conversation_id],
            |r| r.get(0),
        )?;
        // Only the latest attempt of each still-pending effect matters. Groups
        // are bounded by the six schema states, not by unbounded attempt history.
        let mut statement=self.connection.prepare("SELECT a.state,COUNT(*) FROM effect_delivery_attempts a JOIN effect_outbox e ON e.effect_id=a.effect_id AND e.run_id=a.run_id WHERE a.run_id=?1 AND e.status='pending' AND NOT EXISTS(SELECT 1 FROM effect_delivery_attempts successor WHERE successor.predecessor_id=a.id) GROUP BY a.state ORDER BY a.state")?;
        let rows = statement.query_map(params![run_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })?;
        let mut deliveries = Vec::new();
        for row in rows {
            let (state, count) = row?;
            let state = serde_json::from_value(serde_json::Value::String(state))?;
            deliveries.push(nola_domain::RunDeliveryStateCount { state, count });
        }
        let terminal = if status == "sealed" {
            Some(self.canonical_run_terminal(&conversation_id,run_id)?.ok_or_else(||
                StoreError::Invariant("sealed Run has no exact canonical result seal".into()))?)
        } else { None };
        Ok(Some(StoredRunRecoveryInspection {
            terminal,
            conversation_id,
            assistant_message_id,
            kind,
            status,
            completed_at_ms,
            assistant_owner_valid,
            checkpoint_present,
            pending_effect_count,
            source_host_id,
            has_session_lease,
            deliveries,
        }))
    }
}
