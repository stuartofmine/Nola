// SPDX-License-Identifier: Apache-2.0
use crate::{NolaStore, StoreError};
use nola_domain::plugin::PluginInstallation;
use nola_store_port::plugin::{PluginRepository, PluginRepositoryProvider};
use rusqlite::{params, OptionalExtension};

pub struct SqlitePluginRepository<'a> { store: &'a mut NolaStore }
impl PluginRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqlitePluginRepository<'a>;
    fn plugin_repository(&mut self) -> Self::Repository<'_> { SqlitePluginRepository { store: self } }
}
impl PluginRepository for SqlitePluginRepository<'_> {
    type Error = StoreError;
    fn has_write_unit_of_work(&self) -> bool { self.store.has_write_unit_of_work() }
    fn get(&self, id: &str) -> Result<Option<PluginInstallation>, StoreError> {
        let json: Option<String> = self.store.connection.query_row(
            "SELECT record_json FROM plugin_installations WHERE id=?1", [id], |r| r.get(0),
        ).optional()?;
        json.map(|v| serde_json::from_str(&v).map_err(StoreError::from)).transpose()
    }
    fn list(&self, after: Option<&str>, limit: usize) -> Result<Vec<PluginInstallation>, StoreError> {
        if !(1..=128).contains(&limit) { return Err(StoreError::Invariant("plugin page limit must be 1..128".into())); }
        let mut statement = self.store.connection.prepare(
            "SELECT record_json FROM plugin_installations WHERE removed_at_ms IS NULL AND id>?1 ORDER BY id LIMIT ?2")?;
        let rows = statement.query_map(params![after.unwrap_or(""), limit as i64], |r| r.get::<_, String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
    fn save(&mut self, record: &PluginInstallation, expected_revision: Option<i64>) -> Result<(), StoreError> {
        if !self.has_write_unit_of_work() { return Err(StoreError::Invariant("plugin mutation requires Application transaction".into())); }
        record.manifest.validate().map_err(StoreError::Invariant)?;
        if record.revision != expected_revision.unwrap_or(0).checked_add(1)
            .ok_or_else(|| StoreError::Invariant("plugin revision overflow".into()))? {
            return Err(StoreError::Invariant("invalid plugin revision".into()));
        }
        let json = serde_json::to_string(record)?;
        let affected = if let Some(expected) = expected_revision {
            self.store.connection.execute(
                "UPDATE plugin_installations SET revision=?2,record_json=?3,removed_at_ms=?4 WHERE id=?1 AND revision=?5",
                params![record.manifest.id, record.revision, json, record.removed_at_ms, expected])?
        } else {
            self.store.connection.execute(
                "INSERT INTO plugin_installations(id,revision,record_json,removed_at_ms) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO NOTHING",
                params![record.manifest.id, record.revision, json, record.removed_at_ms])?
        };
        if affected != 1 { return Err(StoreError::Invariant("plugin_revision_conflict".into())); }
        Ok(())
    }
}
