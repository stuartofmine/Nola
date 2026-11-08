// SPDX-License-Identifier: Apache-2.0
//! Immutable Core task admission; worker authority remains in runtime_task_attempts.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn draft(id: &str, kind: &str) -> StoredRuntimeTask {
        serde_json::from_value(json!({
            "id":id,"kind":kind,"status":"queued","title":"fixture","project_root":"/fixture",
            "owner_session_id":"parent","child_session_id":"child","parent_tool_call_id":"call",
            "unread":false,"cancel_requested":false,"retry_count":0,"input_watermark":7,
            "created_at_ms":1,"updated_at_ms":1,"metadata_json":"{\"core_promoted\":true}"
        }))
        .unwrap()
    }

    #[test]
    fn prepared_agent_never_claims_or_blocks_an_admissible_project_head() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store
            .create_runtime_task_with_event(&draft("agent", "agent"))
            .unwrap();
        let authority = store
            .runtime_agent_task_authority("agent")
            .unwrap()
            .unwrap();
        assert_eq!(authority.phase, RuntimeAgentTaskAuthorityPhase::Prepared);
        assert_eq!(authority.authority_revision, 1);
        assert!(authority.admission.is_none());
        assert!(store
            .claim_runtime_task("agent", "worker", "generation", "token", 10, 100)
            .unwrap()
            .is_none());
        assert!(store
            .claim_runtime_task_fair("agent", "worker", "generation", "token", 10, 100, 1, 1)
            .unwrap()
            .is_none());
        store
            .create_runtime_task_with_event(&draft("shell", "shell"))
            .unwrap();
        assert!(store
            .claim_runtime_task_fair(
                "shell",
                "worker",
                "generation",
                "shell-token",
                10,
                100,
                1,
                1
            )
            .unwrap()
            .is_some());
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT count(*) FROM runtime_task_attempts WHERE task_id='agent'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert_eq!(
            store.runtime_agent_task_authority("agent").unwrap(),
            Some(authority)
        );
        assert!(store
            .runtime_agent_task_authority("shell")
            .unwrap()
            .is_none());
    }

    #[test]
    fn receiver_draft_and_prepared_authority_are_one_atomic_creation() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.connection.execute_batch("CREATE TRIGGER reject_prepared BEFORE INSERT ON runtime_agent_task_authorities BEGIN SELECT RAISE(ABORT,'prepared leaf fault'); END;").unwrap();
        assert!(store
            .create_runtime_task_with_event(&draft("agent", "agent"))
            .is_err());
        assert!(store.runtime_task("agent").unwrap().is_none());
        assert!(store
            .runtime_agent_task_authority("agent")
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM durable_task_events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

use nola_store_port::runtime_task::{
    AgentTaskAdmission, RuntimeAgentTaskAuthority, RuntimeAgentTaskAuthorityPhase,
    StoredAgentTaskAdmissionEvidence,
};

pub(super) fn authority(
    connection: &Connection,
    task_id: &str,
) -> StoreResult<Option<RuntimeAgentTaskAuthority>> {
    let row:Option<(String,i64,Option<String>)>=connection.query_row(
        "SELECT phase,authority_revision,admission_json FROM runtime_agent_task_authorities WHERE task_id=?1",
        [task_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((phase, revision, json)) = row else {
        return Ok(None);
    };
    let (phase, admission) = match (phase.as_str(), revision, json) {
        ("prepared", 1, None) => (RuntimeAgentTaskAuthorityPhase::Prepared, None),
        ("admitted", revision, Some(json)) if revision >= 2 => {
            if json.len() > nola_domain::agent_task_admission::AGENT_TASK_ADMISSION_MAX_BYTES {
                return Err(StoreError::Invariant(
                    "agent Task admission exceeds bounded metadata limit".into(),
                ));
            }
            let admission: AgentTaskAdmission = serde_json::from_str(&json)?;
            admission.validate().map_err(StoreError::Invariant)?;
            if admission.task_id != task_id {
                return Err(StoreError::Invariant(
                    "agent Task authority identity differs".into(),
                ));
            }
            (RuntimeAgentTaskAuthorityPhase::Admitted, Some(admission))
        }
        _ => {
            return Err(StoreError::Invariant(
                "agent Task authority phase is inconsistent".into(),
            ))
        }
    };
    let authority = RuntimeAgentTaskAuthority {
        task_id: task_id.into(),
        phase,
        authority_revision: revision as u64,
        admission,
    };
    authority.validate().map_err(StoreError::Invariant)?;
    Ok(Some(authority))
}

pub(super) fn permits_execution(connection: &Connection, task_id: &str) -> StoreResult<bool> {
    let draft:Option<(String,Option<String>,Option<String>,Option<String>,Option<i64>)>=connection.query_row(
        "SELECT kind,owner_session_id,child_session_id,parent_tool_call_id,input_watermark FROM durable_tasks WHERE id=?1",
        [task_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).optional()?;
    let Some((kind, parent, child, call, watermark)) = draft else {
        return Ok(false);
    };
    if kind != "agent" {
        // Changing a prepared/admitted agent's kind cannot bypass its gate.
        let has_agent_authority: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runtime_agent_task_authorities WHERE task_id=?1)",
            [task_id],
            |row| row.get(0),
        )?;
        return Ok(!has_agent_authority);
    }
    let Some(authority) = authority(connection, task_id)? else {
        return Ok(false);
    };
    let Some(admission) = authority.admission else {
        return Ok(false);
    };
    Ok(authority.phase == RuntimeAgentTaskAuthorityPhase::Admitted
        && parent.as_deref() == Some(admission.parent_conversation_id.as_str())
        && child.as_deref() == Some(admission.child_conversation_id.as_str())
        && call.as_deref() == Some(admission.parent_tool_call_id.as_str())
        && watermark.is_none_or(|value| value >= 0)
        && admission
            .input_watermark
            .is_none_or(|initial| watermark.is_some_and(|current| current >= initial)))
}

impl NolaStore {
    pub fn runtime_agent_task_authority(
        &self,
        task_id: &str,
    ) -> StoreResult<Option<RuntimeAgentTaskAuthority>> {
        require_id("task_id", task_id)?;
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        authority(&self.connection, task_id)
    }

    pub fn prepared_agent_task_ids(&self, owner: &str, after_id: &str, limit: usize) -> StoreResult<Vec<String>> {
        require_id("owner_conversation_id", owner)?;
        if !(1..=64).contains(&limit) {
            return Err(StoreError::Invariant("Task admission recovery page must contain 1..64 identities".into()));
        }
        let mut statement = self.connection.prepare(
            "SELECT t.id FROM durable_tasks t
             JOIN runtime_agent_task_authorities a ON a.task_id=t.id
             WHERE t.owner_session_id=?1 AND t.id>?2 AND t.kind='agent'
               AND t.status='queued' AND t.cancel_requested=0 AND a.phase='prepared'
             ORDER BY t.id LIMIT ?3")?;
        let rows = statement.query_map(params![owner, after_id, limit as i64], |row| row.get(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn source_agent_task_admission_location(&self, owner: &str, task_id: &str) -> StoreResult<Option<(String, u64)>> {
        require_id("owner_conversation_id", owner)?;
        require_id("task_id", task_id)?;
        // This is only a locator. The original bounded export verifier still
        // requires the matching Host inbox and committed transition evidence.
        let mut statement = self.connection.prepare(
            "SELECT r.id,json_extract(e.payload_json,'$.core_envelope.cursor.sequence')
             FROM runs r JOIN run_events e ON e.run_id=r.id
             WHERE r.conversation_id=?1 AND e.event_kind='task_updated'
               AND json_valid(e.payload_json)
               AND json_extract(e.payload_json,'$.task.id')=?2
               AND json_extract(e.payload_json,'$.task.kind')='agent'
               AND json_extract(e.payload_json,'$.task.state')='queued'
               AND EXISTS (
                 SELECT 1 FROM host_event_inbox h
                 JOIN committed_transitions t ON t.run_id=h.run_id AND t.transition_id='transition:'||h.event_id
                 WHERE h.run_id=r.id AND json_valid(h.payload_json)
                   AND json_extract(h.payload_json,'$.payload.type')='task_promoted'
                   AND json_extract(h.payload_json,'$.payload.task_id')=?2
                   AND t.next_event_sequence>e.sequence
                   AND NOT EXISTS (SELECT 1 FROM committed_transitions earlier
                     WHERE earlier.run_id=r.id AND earlier.next_event_sequence>e.sequence
                       AND earlier.next_event_sequence<t.next_event_sequence))
             ORDER BY r.id,e.sequence LIMIT 2")?;
        let mut rows = statement.query(params![owner, task_id])?;
        let Some(row) = rows.next()? else { return Ok(None) };
        let location = (row.get(0)?, row.get(1)?);
        if rows.next()?.is_some() {
            return Err(StoreError::Invariant("Task admission source location is ambiguous".into()));
        }
        Ok(Some(location))
    }

    pub fn source_agent_task_admission_evidence(
        &self,
        conversation_id: &str,
        run_id: &str,
        task_id: &str,
        event_cursor: u64,
    ) -> StoreResult<Option<StoredAgentTaskAdmissionEvidence>> {
        for (name, value) in [
            ("conversation_id", conversation_id),
            ("run_id", run_id),
            ("task_id", task_id),
        ] {
            require_id(name, value)?;
        }
        if event_cursor == 0 {
            return Err(StoreError::Invariant(
                "task admission requires a Core event cursor".into(),
            ));
        }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let event_id = format!("core:{run_id}:{event_cursor}");
        let mut statement=self.connection.prepare(
            "SELECT t.transition_id,h.event_id,t.checkpoint_revision,
                    length(CAST(e.payload_json AS BLOB)),length(CAST(h.payload_json AS BLOB)),
                    CASE WHEN length(CAST(e.payload_json AS BLOB))<=65536 THEN e.payload_json END,
                    CASE WHEN length(CAST(h.payload_json AS BLOB))<=65536 THEN h.payload_json END
             FROM runs r JOIN run_events e ON e.run_id=r.id
             JOIN host_event_inbox h ON h.run_id=r.id
             JOIN committed_transitions t ON t.run_id=r.id AND t.transition_id='transition:'||h.event_id
             WHERE r.id=?1 AND r.conversation_id=?2 AND e.event_id=?3 AND e.event_kind='task_updated'
               AND CASE WHEN json_valid(e.payload_json) THEN json_extract(e.payload_json,'$.task.id') END=?4
               AND CASE WHEN json_valid(h.payload_json) THEN json_extract(h.payload_json,'$.payload.type') END='task_promoted'
               AND CASE WHEN json_valid(h.payload_json) THEN json_extract(h.payload_json,'$.payload.task_id') END=?4
               AND t.next_event_sequence>e.sequence")?;
        let mut rows = statement.query(params![run_id, conversation_id, event_id, task_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let event_bytes: i64 = row.get(3)?;
        let response_bytes: i64 = row.get(4)?;
        if event_bytes > 65536 || response_bytes > 65536 {
            return Err(StoreError::Invariant(
                "task admission source evidence exceeds 64KiB bound".into(),
            ));
        }
        let result = StoredAgentTaskAdmissionEvidence {
            source_store_epoch: self.connection.query_row(
                "SELECT value FROM store_metadata WHERE key='assistant_source_epoch'",
                [],
                |row| row.get(0),
            )?,
            source_transition_id: row.get(0)?,
            source_host_event_id: row.get(1)?,
            source_checkpoint_revision: row.get(2)?,
            core_event_json: row.get(5)?,
            host_response_json: row.get(6)?,
        };
        if rows.next()?.is_some() {
            return Err(StoreError::Invariant(
                "task admission has ambiguous source evidence".into(),
            ));
        }
        Ok(Some(result))
    }

    pub fn admit_runtime_agent_task(
        &mut self,
        admission: &AgentTaskAdmission,
    ) -> StoreResult<RuntimeAgentTaskAuthority> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Invariant(
                "agent Task admission requires Application write unit of work".into(),
            ));
        }
        admission.validate().map_err(StoreError::Invariant)?;
        // Internal trusted Application/Host import only. Source authorization
        // is checked by the exporting Application against its original Store;
        // that Store may be ephemeral and must not be impersonated by a local
        // checkpoint, boolean, or a caller-computed digest. Transport ingress
        // must authenticate the Host before invoking this repository method.
        let current = authority(&self.connection, &admission.task_id)?
            .ok_or_else(|| StoreError::Conflict("agent Task receiver draft is missing".into()))?;
        if current.phase == RuntimeAgentTaskAuthorityPhase::Admitted {
            return if current.admission.as_ref() == Some(admission)
                && permits_execution(&self.connection, &admission.task_id)?
            {
                Ok(current)
            } else {
                Err(StoreError::Conflict(
                    "agent Task admission was already sealed with different evidence".into(),
                ))
            };
        }
        let tx = self.write_transaction()?;
        let matches:bool=tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM durable_tasks WHERE id=?1 AND kind='agent' AND status='queued'
                AND owner_session_id=?2 AND child_session_id=?3 AND parent_tool_call_id=?4 AND input_watermark IS ?5
                AND worker_id IS NULL AND ownership_token IS NULL AND lease_until_ms IS NULL)",
            params![admission.task_id,admission.parent_conversation_id,admission.child_conversation_id,admission.parent_tool_call_id,admission.input_watermark],|row|row.get(0))?;
        if !matches {
            return Err(StoreError::Conflict(
                "agent Task admission differs from its prepared receiver draft".into(),
            ));
        }
        if tx.execute("UPDATE runtime_agent_task_authorities SET phase='admitted',authority_revision=2,admission_json=?2 WHERE task_id=?1 AND phase='prepared' AND authority_revision=1",
            params![admission.task_id,serde_json::to_string(admission)?])?!=1 {return Err(StoreError::Conflict("agent Task admission revision changed".into()));}
        let result = authority(&tx, &admission.task_id)?
            .ok_or_else(|| StoreError::Invariant("agent Task admission disappeared".into()))?;
        tx.commit()?;
        Ok(result)
    }
}
