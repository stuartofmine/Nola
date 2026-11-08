// SPDX-License-Identifier: Apache-2.0
//! Durable data-source contracts, conversation overrides, and disclosure records.

use super::*;

impl NolaStore {
    pub fn upsert_data_source(
        &mut self,
        source: &StoredDataSourceRecord,
    ) -> StoreResult<StoredDataSourceRecord> {
        validate_data_source(source)?;
        let transaction = self.write_transaction()?;
        let previous_revision = transaction
            .query_row(
                "SELECT revision FROM data_sources WHERE id=?1",
                params![source.id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        transaction.execute(
            "INSERT INTO data_sources(id,name,kind,location,enabled,access_policy,validation_contract_json,
                capabilities_json,privacy,metadata_json,revision,created_at_ms,updated_at_ms,deleted_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1,?11,?12,NULL)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name,kind=excluded.kind,
                location=excluded.location,enabled=excluded.enabled,access_policy=excluded.access_policy,
                validation_contract_json=excluded.validation_contract_json,
                capabilities_json=excluded.capabilities_json,privacy=excluded.privacy,
                metadata_json=excluded.metadata_json,
                revision=CASE WHEN data_sources.name<>excluded.name OR data_sources.kind<>excluded.kind
                    OR data_sources.location<>excluded.location OR data_sources.enabled<>excluded.enabled
                    OR data_sources.access_policy<>excluded.access_policy
                    OR data_sources.validation_contract_json<>excluded.validation_contract_json
                    OR data_sources.capabilities_json<>excluded.capabilities_json
                    OR data_sources.privacy<>excluded.privacy OR data_sources.metadata_json<>excluded.metadata_json
                    OR data_sources.deleted_at_ms IS NOT NULL
                    THEN data_sources.revision+1 ELSE data_sources.revision END,
                updated_at_ms=CASE WHEN data_sources.name<>excluded.name OR data_sources.kind<>excluded.kind
                    OR data_sources.location<>excluded.location OR data_sources.enabled<>excluded.enabled
                    OR data_sources.access_policy<>excluded.access_policy
                    OR data_sources.validation_contract_json<>excluded.validation_contract_json
                    OR data_sources.capabilities_json<>excluded.capabilities_json
                    OR data_sources.privacy<>excluded.privacy OR data_sources.metadata_json<>excluded.metadata_json
                    OR data_sources.deleted_at_ms IS NOT NULL
                    THEN excluded.updated_at_ms ELSE data_sources.updated_at_ms END,
                deleted_at_ms=NULL",
            params![source.id,source.name,source.kind,source.location,source.enabled,source.access_policy,
                source.validation_contract_json,source.capabilities_json,source.privacy,source.metadata_json,
                source.created_at_ms,source.updated_at_ms],
        )?;
        let stored = transaction.query_row(
            &format!("SELECT {DATA_SOURCE_COLUMNS} FROM data_sources WHERE id=?1 AND deleted_at_ms IS NULL"),
            params![source.id],
            data_source_row,
        )?;
        if previous_revision != Some(stored.revision) {
            let payload = serde_json::to_string(&stored)?;
            enqueue_sync_change(
                &transaction,
                "data_source",
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

    pub fn data_source(&self, id: &str) -> StoreResult<Option<StoredDataSourceRecord>> {
        require_id("data_source.id", id)?;
        self.connection
            .query_row(
                &format!("SELECT {DATA_SOURCE_COLUMNS} FROM data_sources WHERE id=?1 AND deleted_at_ms IS NULL"),
                params![id], data_source_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn list_data_sources(&self) -> StoreResult<Vec<StoredDataSourceRecord>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {DATA_SOURCE_COLUMNS} FROM data_sources WHERE deleted_at_ms IS NULL ORDER BY lower(name),id"
        ))?;
        let rows = statement.query_map([], data_source_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn delete_data_source(&mut self, id: &str, deleted_at_ms: i64) -> StoreResult<bool> {
        require_id("data_source.id", id)?;
        let transaction = self.write_transaction()?;
        let changed = transaction.execute(
            "UPDATE data_sources SET deleted_at_ms=?2,updated_at_ms=?2,revision=revision+1
             WHERE id=?1 AND deleted_at_ms IS NULL",
            params![id, deleted_at_ms],
        )? == 1;
        if changed {
            transaction.execute(
                "DELETE FROM data_source_overrides WHERE source_id=?1",
                params![id],
            )?;
            let revision: i64 = transaction.query_row(
                "SELECT revision FROM data_sources WHERE id=?1",
                params![id],
                |row| row.get(0),
            )?;
            enqueue_sync_change(
                &transaction,
                "data_source",
                id,
                revision,
                true,
                "null",
                deleted_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }

    pub fn set_data_source_override(
        &mut self,
        conversation_id: &str,
        source_id: &str,
        enabled: Option<bool>,
        updated_at_ms: i64,
    ) -> StoreResult<()> {
        require_id("data_source_override.conversation_id", conversation_id)?;
        require_id("data_source_override.source_id", source_id)?;
        let transaction = self.write_transaction()?;
        let conversation_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            params![conversation_id],
            |row| row.get(0),
        )?;
        if !conversation_exists {
            return Err(StoreError::Invariant(format!(
                "unknown conversation {conversation_id}"
            )));
        }
        let source_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM data_sources WHERE id=?1 AND deleted_at_ms IS NULL)",
            params![source_id],
            |row| row.get(0),
        )?;
        if !source_exists {
            return Err(StoreError::Invariant(format!(
                "unknown data source {source_id}"
            )));
        }
        if let Some(enabled) = enabled {
            transaction.execute(
                "INSERT INTO data_source_overrides(conversation_id,source_id,enabled,updated_at_ms)
                 VALUES(?1,?2,?3,?4) ON CONFLICT(conversation_id,source_id) DO UPDATE SET
                 enabled=excluded.enabled,updated_at_ms=excluded.updated_at_ms",
                params![conversation_id, source_id, enabled, updated_at_ms],
            )?;
        } else {
            transaction.execute(
                "DELETE FROM data_source_overrides WHERE conversation_id=?1 AND source_id=?2",
                params![conversation_id, source_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn data_source_override(
        &self,
        conversation_id: &str,
        source_id: &str,
    ) -> StoreResult<Option<bool>> {
        self.connection.query_row(
            "SELECT enabled FROM data_source_overrides WHERE conversation_id=?1 AND source_id=?2",
            params![conversation_id,source_id], |row| row.get(0),
        ).optional().map_err(StoreError::from)
    }

    pub fn append_data_source_usage(&mut self, usage: &StoredDataSourceUsage) -> StoreResult<()> {
        validate_data_source_usage(usage)?;
        self.connection.execute(
            "INSERT INTO data_source_usage(id,source_id,source_title,consumer,purpose,conversation_id,
                disclosed_to_cloud,metadata_json,used_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(id) DO NOTHING",
            params![usage.id,usage.source_id,usage.source_title,usage.consumer,usage.purpose,
                usage.conversation_id,usage.disclosed_to_cloud,usage.metadata_json,usage.used_at_ms],
        )?;
        Ok(())
    }

    pub fn list_data_source_usage(&self, limit: usize) -> StoreResult<Vec<StoredDataSourceUsage>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {DATA_SOURCE_USAGE_COLUMNS} FROM data_source_usage ORDER BY used_at_ms DESC,id DESC LIMIT ?1"
        ))?;
        let rows =
            statement.query_map(params![limit.clamp(1, 1000) as i64], data_source_usage_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}
