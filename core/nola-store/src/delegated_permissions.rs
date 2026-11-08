// SPDX-License-Identifier: Apache-2.0
//! Immutable permission occurrence identity and application-planned delivery evidence.
use super::*;
use nola_domain::{DelegatedPermissionDeliveryMutation, DelegatedPermissionDeliveryRecord};

fn decode(raw: String) -> StoreResult<DelegatedPermissionDeliveryRecord> {
    serde_json::from_str(&raw)
        .map_err(|e| StoreError::Invariant(format!("invalid delegated permission: {e}")))
}
fn read(
    connection: &Connection,
    id: &str,
) -> StoreResult<Option<DelegatedPermissionDeliveryRecord>> {
    connection
        .query_row(
            "SELECT record_json FROM delegated_permission_deliveries WHERE id=?1",
            params![id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(decode)
        .transpose()
}
impl NolaStore {
    pub fn open_delegated_permissions_for_target(
        &self,
        effect_id: &str,
    ) -> StoreResult<Vec<DelegatedPermissionDeliveryRecord>> {
        require_id("target_effect_id", effect_id)?;
        let mut statement=self.connection.prepare("SELECT record_json FROM delegated_permission_deliveries WHERE target_effect_id=?1 AND state IN ('awaiting_decision','prepared','claimed') ORDER BY id")?;
        let raw = statement
            .query_map(params![effect_id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        raw.into_iter().map(decode).collect()
    }

    pub fn delegated_permission(
        &self,
        id: &str,
    ) -> StoreResult<Option<DelegatedPermissionDeliveryRecord>> {
        require_id("delegated_permission.id", id)?;
        read(&self.connection, id)
    }
    pub fn delegated_permission_for_occurrence(
        &self,
        invocation_id: &str,
        occurrence_id: &str,
    ) -> StoreResult<Option<DelegatedPermissionDeliveryRecord>> {
        require_id("invocation_id", invocation_id)?;
        require_id("occurrence_id", occurrence_id)?;
        self.connection.query_row("SELECT record_json FROM delegated_permission_deliveries WHERE invocation_id=?1 AND occurrence_id=?2",params![invocation_id,occurrence_id], |r|r.get::<_,String>(0)).optional()?.map(decode).transpose()
    }
    pub fn delegated_permission_for_control(
        &self,
        effect_id: &str,
    ) -> StoreResult<Option<DelegatedPermissionDeliveryRecord>> {
        require_id("control_effect_id", effect_id)?;
        self.connection.query_row("SELECT record_json FROM delegated_permission_deliveries WHERE control_effect_id=?1",params![effect_id], |r|r.get::<_,String>(0)).optional()?.map(decode).transpose()
    }
}
pub(super) fn recovery_records(
    connection: &Connection,
    run_id: &str,
) -> StoreResult<Vec<DelegatedPermissionDeliveryRecord>> {
    let mut statement = connection.prepare("SELECT record_json FROM delegated_permission_deliveries d WHERE run_id=?1 AND (state IN ('awaiting_decision','prepared','claimed','outcome_unknown') OR EXISTS(SELECT 1 FROM effect_outbox e WHERE e.run_id=d.run_id AND e.effect_id=d.control_effect_id AND e.status='pending')) ORDER BY id")?;
    let raw = statement
        .query_map(params![run_id], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    raw.into_iter().map(decode).collect()
}
pub(super) fn apply(
    connection: &Connection,
    run_id: &str,
    mutations: &[DelegatedPermissionDeliveryMutation],
) -> StoreResult<()> {
    for mutation in mutations {
        let r = &mutation.record;
        for (name, id) in [
            ("id", &r.id),
            ("run_id", &r.run_id),
            ("invocation_id", &r.invocation_id),
            ("target_effect_id", &r.target_effect_id),
            ("worker_generation", &r.worker_generation),
            ("occurrence_id", &r.occurrence_id),
            ("backend_id", &r.backend_id),
            ("backend_session_id", &r.backend_session_id),
            ("rpc_request_id", &r.rpc_request_id),
            ("call_id", &r.call_id),
            ("request_digest", &r.request_digest),
        ] {
            require_id(name, id)?;
        }
        for id in [&r.control_effect_id, &r.control_request_id]
            .into_iter()
            .flatten()
        {
            require_id("permission_control", id)?;
        }
        if r.run_id != run_id {
            return Err(StoreError::Conflict(
                "delegated permission belongs to another Run".into(),
            ));
        }
        let owner = crate::delegated_invocations::for_effect(connection, &r.target_effect_id)?
            .ok_or_else(|| StoreError::Conflict("permission target invocation missing".into()))?;
        if owner.id != r.invocation_id
            || owner.run_id != r.run_id
            || owner.backend_id != r.backend_id
            || owner.backend_session_id.as_deref() != Some(r.backend_session_id.as_str())
        {
            return Err(StoreError::Conflict(
                "permission target ownership mismatch".into(),
            ));
        }
        if r.control_effect_id.is_some() != r.control_request_id.is_some()
            || r.control_effect_id.is_some() != r.decision.is_some()
        {
            return Err(StoreError::Invariant(
                "permission control binding must be complete".into(),
            ));
        }
        let raw = serde_json::to_string(r).map_err(|e| StoreError::Invariant(e.to_string()))?;
        if raw.len() > 65536 {
            return Err(StoreError::Invariant(
                "delegated permission exceeds 64KiB".into(),
            ));
        }
        let state =
            serde_json::to_value(r.state).map_err(|e| StoreError::Invariant(e.to_string()))?;
        let state = state
            .as_str()
            .ok_or_else(|| StoreError::Invariant("permission state must be a string".into()))?;
        match mutation.expected_revision {
            None => {
                if r.revision != 1 {
                    return Err(StoreError::Invariant(
                        "new permission revision must be one".into(),
                    ));
                }
                if let Some(old) = read(connection, &r.id)? {
                    if old == *r {
                        continue;
                    }
                    return Err(StoreError::Conflict(
                        "permission identity already exists".into(),
                    ));
                }
                connection.execute("INSERT INTO delegated_permission_deliveries(id,run_id,invocation_id,target_effect_id,occurrence_id,control_effect_id,control_request_id,state,revision,record_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![r.id,r.run_id,r.invocation_id,r.target_effect_id,r.occurrence_id,r.control_effect_id,r.control_request_id,state,r.revision,raw])?;
            }
            Some(expected) => {
                let old = read(connection, &r.id)?
                    .ok_or_else(|| StoreError::Conflict("permission missing".into()))?;
                if old.revision != expected || expected.checked_add(1) != Some(r.revision) {
                    return Err(StoreError::Conflict("permission revision changed".into()));
                }
                if old.run_id != r.run_id
                    || old.invocation_id != r.invocation_id
                    || old.target_effect_id != r.target_effect_id
                    || old.worker_generation != r.worker_generation
                    || old.occurrence_id != r.occurrence_id
                    || old.backend_id != r.backend_id
                    || old.backend_session_id != r.backend_session_id
                    || old.rpc_request_id != r.rpc_request_id
                    || old.call_id != r.call_id
                    || old.request_json != r.request_json
                    || old.request_digest != r.request_digest
                    || old.created_at_ms != r.created_at_ms
                    || (old.control_effect_id.is_some()
                        && (old.control_effect_id != r.control_effect_id
                            || old.control_request_id != r.control_request_id
                            || old.decision != r.decision))
                    || (old.claimed_at_ms.is_some() && old.claimed_at_ms != r.claimed_at_ms)
                    || (old.written_at_ms.is_some() && old.written_at_ms != r.written_at_ms)
                {
                    return Err(StoreError::Conflict(
                        "permission immutable identity or evidence changed".into(),
                    ));
                }
                connection.execute("UPDATE delegated_permission_deliveries SET control_effect_id=?2,control_request_id=?3,state=?4,revision=?5,record_json=?6 WHERE id=?1 AND revision=?7",params![r.id,r.control_effect_id,r.control_request_id,state,r.revision,raw,expected])?;
            }
        }
    }
    Ok(())
}

pub(super) fn append_initial_events(
    connection: &Connection,
    run_id: &str,
    mutations: &[DelegatedPermissionDeliveryMutation],
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
        connection.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES(?1,?2,?3,'delegated_permission_changed',?4,?5)",params![format!("delegated-permission:{}:{}",record.id,record.revision),run_id,sequence,payload,at_ms])?;
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
