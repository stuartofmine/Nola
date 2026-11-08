// SPDX-License-Identifier: Apache-2.0
//! Durable MCP server registry, revisioning, and sync outbox persistence.

use super::*;

impl NolaStore {
    pub fn upsert_mcp_server(
        &mut self,
        server: &StoredMcpServerRecord,
    ) -> StoreResult<StoredMcpServerRecord> {
        validate_mcp_server(server)?;
        let transaction = self.write_transaction()?;
        let previous_revision = transaction
            .query_row(
                "SELECT revision FROM mcp_servers WHERE id=?1",
                params![server.id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        transaction.execute(
            "INSERT INTO mcp_servers(id,display_name,enabled,configuration_json,revision,
                created_at_ms,updated_at_ms,deleted_at_ms)
             VALUES(?1,?2,?3,?4,1,?5,?6,NULL)
             ON CONFLICT(id) DO UPDATE SET display_name=excluded.display_name,
                enabled=excluded.enabled,configuration_json=excluded.configuration_json,
                revision=CASE WHEN mcp_servers.display_name<>excluded.display_name
                    OR mcp_servers.enabled<>excluded.enabled
                    OR mcp_servers.configuration_json<>excluded.configuration_json
                    OR mcp_servers.deleted_at_ms IS NOT NULL
                    THEN mcp_servers.revision+1 ELSE mcp_servers.revision END,
                updated_at_ms=CASE WHEN mcp_servers.display_name<>excluded.display_name
                    OR mcp_servers.enabled<>excluded.enabled
                    OR mcp_servers.configuration_json<>excluded.configuration_json
                    OR mcp_servers.deleted_at_ms IS NOT NULL
                    THEN excluded.updated_at_ms ELSE mcp_servers.updated_at_ms END,
                deleted_at_ms=NULL",
            params![
                server.id,
                server.display_name,
                server.enabled,
                server.configuration_json,
                server.created_at_ms,
                server.updated_at_ms
            ],
        )?;
        let stored = transaction.query_row(
            &format!("SELECT {MCP_SERVER_COLUMNS} FROM mcp_servers WHERE id=?1 AND deleted_at_ms IS NULL"),
            params![server.id],
            mcp_server_row,
        )?;
        if previous_revision != Some(stored.revision) {
            let payload = serde_json::to_string(&stored)?;
            enqueue_sync_change(
                &transaction,
                "mcp_server",
                &stored.id,
                stored.revision,
                false,
                &payload,
                stored.updated_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(stored)
    }

    pub fn mcp_server_identity_exists(&self, id: &str) -> StoreResult<bool> {
        require_id("mcp_server.id", id)?;
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM mcp_servers WHERE id=?1)",
                params![id],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
    }

    pub fn mcp_server(&self, id: &str) -> StoreResult<Option<StoredMcpServerRecord>> {
        require_id("mcp_server.id", id)?;
        self.connection
            .query_row(
                &format!("SELECT {MCP_SERVER_COLUMNS} FROM mcp_servers WHERE id=?1 AND deleted_at_ms IS NULL"),
                params![id],
                mcp_server_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn list_mcp_servers(&self) -> StoreResult<Vec<StoredMcpServerRecord>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MCP_SERVER_COLUMNS} FROM mcp_servers WHERE deleted_at_ms IS NULL
             ORDER BY lower(display_name),id"
        ))?;
        let rows = statement.query_map([], mcp_server_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn delete_mcp_server(&mut self, id: &str, deleted_at_ms: i64) -> StoreResult<bool> {
        require_id("mcp_server.id", id)?;
        let transaction = self.write_transaction()?;
        let updated = transaction.execute(
            "UPDATE mcp_servers SET enabled=0,deleted_at_ms=?2,updated_at_ms=?2,
                revision=revision+1 WHERE id=?1 AND deleted_at_ms IS NULL",
            params![id, deleted_at_ms],
        )?;
        if updated == 0 {
            transaction.rollback()?;
            return Ok(false);
        }
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM mcp_servers WHERE id=?1",
            params![id],
            |row| row.get(0),
        )?;
        enqueue_sync_change(
            &transaction,
            "mcp_server",
            id,
            revision,
            true,
            "null",
            deleted_at_ms,
        )?;
        transaction.commit()?;
        Ok(true)
    }
}
