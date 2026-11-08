// SPDX-License-Identifier: Apache-2.0
//! Revisioned product setting persistence and sync outbox recording.

use super::*;

impl NolaStore {
    pub fn list_room_submissions(&self, prefix: &str, limit: usize, offset: usize)
        -> StoreResult<Vec<StoredProductSetting>> {
        require_id("room_submission.prefix", prefix)?;
        if prefix.starts_with(nola_domain::edited_resend::EDITED_RESEND_PREFIX) {
            return Err(StoreError::Invariant("edited_resend_records_require_private_reader".into()));
        }
        if limit == 0 || limit > 200 || offset > i64::MAX as usize {
            return Err(StoreError::Invariant("invalid room submission page bounds".into()));
        }
        let mut query = self.connection.prepare(&format!(
            "SELECT {PRODUCT_SETTING_COLUMNS} FROM product_settings
             WHERE substr(key,1,length(?1))=?1 AND substr(key,1,14)!='edited_resend/'
             ORDER BY json_extract(value_json,'$.input.at_ms') DESC,key ASC
             LIMIT ?2 OFFSET ?3"))?;
        let rows = query.query_map(params![prefix,limit as i64,offset as i64], product_setting_row)?;
        rows.collect::<Result<Vec<_>,_>>().map_err(StoreError::from)
    }
    pub fn list_product_settings(&self, prefix: &str, limit: usize, offset: usize)
        -> StoreResult<Vec<StoredProductSetting>> {
        require_id("product_setting.prefix", prefix)?;
        if prefix.starts_with(nola_domain::edited_resend::EDITED_RESEND_PREFIX) {
            return Err(StoreError::Invariant("edited_resend_records_require_private_reader".into()));
        }
        if limit == 0 || limit > 256 || offset > i64::MAX as usize {
            return Err(StoreError::Invariant("invalid product setting page bounds".into()));
        }
        let mut query = self.connection.prepare(&format!(
            "SELECT {PRODUCT_SETTING_COLUMNS} FROM product_settings
             WHERE substr(key,1,length(?1))=?1 AND substr(key,1,14)!='edited_resend/'
             ORDER BY updated_at_ms DESC,key ASC LIMIT ?2 OFFSET ?3"))?;
        let rows = query.query_map(params![prefix,limit as i64,offset as i64], product_setting_row)?;
        rows.collect::<Result<Vec<_>,_>>().map_err(StoreError::from)
    }
    pub fn put_product_setting(
        &mut self,
        key: &str,
        value_json: &str,
        updated_at_ms: i64,
    ) -> StoreResult<StoredProductSetting> {
        require_id("product_setting.key", key)?;
        if key.starts_with(nola_domain::edited_resend::EDITED_RESEND_PREFIX) {
            return Err(StoreError::Invariant("edited_resend_records_require_owning_command".into()));
        }
        validate_json("product_setting.value_json", value_json)?;
        let transaction = self.write_transaction()?;
        transaction.execute(
            "INSERT INTO product_settings(key,value_json,revision,updated_at_ms)
             VALUES(
                ?1,
                ?2,
                COALESCE((SELECT revision + 1 FROM sync_entity_revisions
                          WHERE entity_type='product_setting' AND entity_id=?1), 1),
                ?3
             )
             ON CONFLICT(key) DO UPDATE SET
                value_json=excluded.value_json,
                revision=product_settings.revision+1,
                updated_at_ms=excluded.updated_at_ms",
            params![key, value_json, updated_at_ms],
        )?;
        let stored = transaction.query_row(
            &format!("SELECT {PRODUCT_SETTING_COLUMNS} FROM product_settings WHERE key=?1"),
            params![key],
            product_setting_row,
        )?;
        enqueue_sync_change(
            &transaction,
            "product_setting",
            key,
            stored.revision,
            false,
            &serde_json::to_string(&stored)?,
            updated_at_ms,
        )?;
        transaction.commit()?;
        Ok(stored)
    }

    pub fn product_setting(&self, key: &str) -> StoreResult<Option<StoredProductSetting>> {
        if key.starts_with(nola_domain::edited_resend::EDITED_RESEND_PREFIX) {
            return Err(StoreError::Invariant("edited_resend_records_require_private_reader".into()));
        }
        require_id("product_setting.key", key)?;
        self.connection
            .query_row(
                &format!("SELECT {PRODUCT_SETTING_COLUMNS} FROM product_settings WHERE key=?1"),
                params![key],
                product_setting_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn delete_product_setting(&mut self, key: &str) -> StoreResult<bool> {
        if key.starts_with(nola_domain::edited_resend::EDITED_RESEND_PREFIX) {
            return Err(StoreError::Invariant("edited_resend_records_require_owning_command".into()));
        }
        require_id("product_setting.key", key)?;
        let transaction = self.write_transaction()?;
        let existing = transaction
            .query_row(
                &format!("SELECT {PRODUCT_SETTING_COLUMNS} FROM product_settings WHERE key=?1"),
                params![key],
                product_setting_row,
            )
            .optional()?;
        let Some(existing) = existing else {
            transaction.commit()?;
            return Ok(false);
        };
        enqueue_sync_change(
            &transaction,
            "product_setting",
            key,
            existing.revision.saturating_add(1),
            true,
            "null",
            existing.updated_at_ms.saturating_add(1),
        )?;
        transaction.execute("DELETE FROM product_settings WHERE key=?1", params![key])?;
        transaction.commit()?;
        Ok(true)
    }
}
