// SPDX-License-Identifier: Apache-2.0
//! Agent command enqueue, lease, release, and acknowledgement persistence.

use super::*;

impl NolaStore {
    pub fn peek_agent_command(&mut self, session_id: &str) -> StoreResult<Option<StoredAgentCommand>> {
        require_id("agent_command.session_id", session_id)?;
        self.connection.query_row(
            &format!("SELECT {AGENT_COMMAND_COLUMNS} FROM agent_commands WHERE session_id=?1 ORDER BY sequence LIMIT 1"),
            params![session_id], agent_command_row,
        ).optional().map_err(StoreError::from)
    }

    pub fn enqueue_agent_command(&mut self, input: &EnqueueAgentCommandInput) -> StoreResult<bool> {
        require_id("agent_command.session_id", &input.session_id)?;
        require_id("agent_command.command_id", &input.command_id)?;
        let _: serde_json::Value = serde_json::from_str(&input.payload_json)?;
        let transaction=self.write_transaction()?;
        let existing:Option<(String,i64)>=transaction.query_row(
            "SELECT payload_json,created_at_ms FROM agent_commands WHERE session_id=?1 AND command_id=?2",
            params![input.session_id,input.command_id],|row|Ok((row.get(0)?,row.get(1)?)),
        ).optional()?;
        if let Some((payload,_created))=existing {
            if payload!=input.payload_json {
                return Err(StoreError::Conflict("Agent command identity already has a different payload".into()));
            }
            transaction.commit()?;
            return Ok(false);
        }
        let inserted=transaction.execute(
            "INSERT OR IGNORE INTO agent_commands(
                session_id,command_id,payload_json,created_at_ms
             ) VALUES(?1,?2,?3,?4)",
            params![
                input.session_id,
                input.command_id,
                input.payload_json,
                input.created_at_ms
            ],
        )? == 1;
        transaction.commit()?;
        Ok(inserted)
    }

    pub fn claim_next_agent_command(
        &mut self,
        session_id: &str,
        lease_owner: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> StoreResult<Option<StoredAgentCommand>> {
        require_id("agent_command.session_id", session_id)?;
        require_id("agent_command.lease_owner", lease_owner)?;
        if lease_until_ms <= now_ms {
            return Err(StoreError::Invariant(
                "agent command lease must end after claim time".into(),
            ));
        }
        let transaction = self.write_transaction()?;
        let sequence = transaction
            .query_row(
                "SELECT sequence FROM agent_commands
                 WHERE sequence=(SELECT sequence FROM agent_commands
                     WHERE session_id=?1 ORDER BY sequence LIMIT 1)
                   AND (lease_until_ms IS NULL OR lease_until_ms<=?2)",
                params![session_id, now_ms],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(sequence) = sequence else {
            transaction.commit()?;
            return Ok(None);
        };
        let changed = transaction.execute(
            "UPDATE agent_commands
             SET lease_owner=?2,lease_until_ms=?3,attempt_count=attempt_count+1
             WHERE sequence=?1 AND (lease_until_ms IS NULL OR lease_until_ms<=?4)",
            params![sequence, lease_owner, lease_until_ms, now_ms],
        )?;
        if changed != 1 {
            return Err(StoreError::Invariant(
                "agent command lease changed during an immediate transaction".into(),
            ));
        }
        let command = transaction.query_row(
            &format!("SELECT {AGENT_COMMAND_COLUMNS} FROM agent_commands WHERE sequence=?1"),
            params![sequence],
            agent_command_row,
        )?;
        transaction.commit()?;
        Ok(Some(command))
    }

    pub fn leased_agent_commands(
        &mut self,
        session_id: &str,
    ) -> StoreResult<Vec<StoredAgentCommand>> {
        require_id("agent_command.session_id", session_id)?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {AGENT_COMMAND_COLUMNS} FROM agent_commands
             WHERE session_id=?1 AND lease_owner IS NOT NULL AND lease_until_ms IS NOT NULL
             ORDER BY sequence"
        ))?;
        let rows = statement.query_map(params![session_id], agent_command_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn release_agent_command_lease(
        &mut self,
        session_id: &str,
        command_id: &str,
        lease_owner: &str,
    ) -> StoreResult<bool> {
        require_id("agent_command.session_id", session_id)?;
        require_id("agent_command.command_id", command_id)?;
        require_id("agent_command.lease_owner", lease_owner)?;
        Ok(self.connection.execute(
            "UPDATE agent_commands SET lease_owner=NULL,lease_until_ms=NULL
             WHERE session_id=?1 AND command_id=?2 AND lease_owner=?3",
            params![session_id, command_id, lease_owner],
        )? == 1)
    }

    pub fn acknowledge_agent_command(
        &mut self,
        session_id: &str,
        command_id: &str,
        lease_owner: &str,
    ) -> StoreResult<bool> {
        require_id("agent_command.session_id", session_id)?;
        require_id("agent_command.command_id", command_id)?;
        require_id("agent_command.lease_owner", lease_owner)?;
        Ok(self.connection.execute(
            "DELETE FROM agent_commands
             WHERE session_id=?1 AND command_id=?2 AND lease_owner=?3",
            params![session_id, command_id, lease_owner],
        )? == 1)
    }
}
