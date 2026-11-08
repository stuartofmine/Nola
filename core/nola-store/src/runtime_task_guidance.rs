// SPDX-License-Identifier: Apache-2.0
//! Guidance payloads and results share the RuntimeTask transaction owner.
use super::*;
impl NolaStore {
    pub fn outstanding_runtime_task_guidance_count(&self, task: &str) -> StoreResult<u64> {
        self.connection.query_row("SELECT count(*) FROM runtime_task_guidance WHERE task_id=?1 AND json_extract(record_json,'$.status') IN ('submitted','accepted')", [task], |row| row.get(0)).map_err(StoreError::from)
    }
    pub fn runtime_task_guidance_records(
        &self,
        task: &str,
    ) -> StoreResult<Vec<RuntimeTaskGuidance>> {
        let mut stmt = self.connection.prepare(
            "SELECT record_json FROM runtime_task_guidance WHERE task_id=?1 ORDER BY rowid",
        )?;
        let rows = stmt
            .query_map([task], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|json| serde_json::from_str(&json).map_err(StoreError::from))
            .collect()
    }
    pub fn runtime_task_guidance(
        &self,
        task: &str,
        message: &str,
    ) -> StoreResult<Option<RuntimeTaskGuidance>> {
        self.connection
            .query_row(
                "SELECT record_json FROM runtime_task_guidance WHERE task_id=?1 AND message_id=?2",
                params![task, message],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(|value| serde_json::from_str(&value).map_err(StoreError::from))
            .transpose()
    }
    pub fn pending_runtime_task_guidance(
        &self,
        task: &str,
        run: &str,
    ) -> StoreResult<Vec<RuntimeTaskGuidance>> {
        let mut statement = self.connection.prepare(
            "SELECT record_json FROM runtime_task_guidance
            WHERE task_id=?1 AND json_extract(record_json,'$.run_id')=?2
            AND json_extract(record_json,'$.status')='accepted'
            ORDER BY json_extract(record_json,'$.submitted_at_ms'),message_id",
        )?;
        let rows = statement
            .query_map(params![task, run], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|json| serde_json::from_str(&json).map_err(StoreError::from))
            .collect()
    }
    pub fn insert_runtime_task_guidance(
        &mut self,
        record: &RuntimeTaskGuidance,
    ) -> StoreResult<RuntimeTaskGuidance> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Invariant(
                "guidance requires the Application UoW".into(),
            ));
        }
        if record.status != "submitted"
            || record.accepted_at_ms.is_some()
            || record.resolved_at_ms.is_some()
            || record.reason.is_some()
            || record.delivery_transition_id.is_some()
        {
            return Err(StoreError::Invariant(
                "guidance starts as submitted without a caller-authored outcome".into(),
            ));
        }
        if let Some(existing) = self.runtime_task_guidance(&record.task_id, &record.message_id)? {
            let mut identity = existing.clone();
            identity.status = "submitted".into();
            identity.accepted_at_ms = None;
            identity.resolved_at_ms = None;
            identity.reason = None;
            identity.delivery_transition_id = None;
            return if identity == *record {
                Ok(existing)
            } else {
                Err(StoreError::Conflict(
                    "guidance message identity conflicts".into(),
                ))
            };
        }
        self.connection.execute(
            "INSERT INTO runtime_task_guidance(task_id,message_id,record_json) VALUES(?1,?2,?3)",
            params![
                record.task_id,
                record.message_id,
                serde_json::to_string(record)?
            ],
        )?;
        Ok(record.clone())
    }
    pub fn resolve_runtime_task_guidance(
        &mut self,
        task: &str,
        message: &str,
        status: &str,
        reason: Option<&str>,
        transition: Option<&str>,
        at: i64,
    ) -> StoreResult<RuntimeTaskGuidance> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Invariant(
                "guidance settlement requires the Application UoW".into(),
            ));
        }
        let mut record = self
            .runtime_task_guidance(task, message)?
            .ok_or_else(|| StoreError::Conflict("guidance message is absent".into()))?;
        if record.status == status
            && record.reason.as_deref() == reason
            && record.delivery_transition_id.as_deref() == transition
        {
            return Ok(record);
        }
        let allowed = matches!(
            (record.status.as_str(), status),
            ("submitted", "accepted")
                | ("submitted", "rejected")
                | ("accepted", "delivered")
                | ("accepted", "rejected")
        );
        if !allowed
            || at < record.submitted_at_ms
            || record.accepted_at_ms.is_some_and(|time| at < time)
            || (status == "delivered" && transition.is_none())
            || (status == "rejected" && reason.is_none())
        {
            return Err(StoreError::Conflict(
                "invalid guidance outcome transition".into(),
            ));
        }
        record.status = status.into();
        record.reason = reason.map(str::to_string);
        record.delivery_transition_id = transition.map(str::to_string);
        if status == "accepted" {
            record.accepted_at_ms = Some(at);
        } else {
            record.resolved_at_ms = Some(at);
        }
        self.connection.execute(
            "UPDATE runtime_task_guidance SET record_json=?3 WHERE task_id=?1 AND message_id=?2",
            params![task, message, serde_json::to_string(&record)?],
        )?;
        Ok(record)
    }
}
