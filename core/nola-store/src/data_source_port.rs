// SPDX-License-Identifier: Apache-2.0
//! Narrow data-source adapter sharing the Application-owned transaction.
use crate::{NolaStore, StoreError};
use nola_store_port::data_source::{
    DataSourceReadRepository, DataSourceRepository, DataSourceRepositoryProvider,
    StoredDataSourceRecord, StoredDataSourceUsage,
};

/// Borrowed data-source capability; unrelated Store capabilities are inaccessible.
///
/// ```compile_fail
/// use nola_store::{DataSourceRepository, NolaStore};
/// fn requires_data_sources(_: &mut impl DataSourceRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// requires_data_sources(&mut store);
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::context::ContextRepository;
/// fn requires_context(_: &mut impl ContextRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// let mut sources = store.data_source_repository();
/// requires_context(&mut sources);
/// ```
pub struct SqliteDataSourceRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn data_source_repository(&mut self) -> SqliteDataSourceRepository<'_> {
        SqliteDataSourceRepository { store: self }
    }
}

impl DataSourceRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteDataSourceRepository<'a>;

    fn data_source_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::data_source_repository(self)
    }
}

impl DataSourceReadRepository for SqliteDataSourceRepository<'_> {
    type Error = StoreError;
    fn data_source(&self, id: &str) -> Result<Option<StoredDataSourceRecord>, Self::Error> {
        self.store.data_source(id)
    }
    fn list_data_sources(&self) -> Result<Vec<StoredDataSourceRecord>, Self::Error> {
        self.store.list_data_sources()
    }
    fn list_data_sources_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<(StoredDataSourceRecord, Option<bool>)>, Self::Error> {
        crate::require_id("conversation_id", conversation_id)?;
        // Join the existing command transaction when present. Standalone reads
        // hold a SQLite read snapshot across the two bounded-query-count reads.
        let snapshot = if self.store.connection.is_autocommit() {
            Some(self.store.connection.unchecked_transaction()?)
        } else {
            None
        };
        let sources = self.store.list_data_sources()?;
        let mut query = self.store.connection.prepare(
            "SELECT source_id,enabled FROM data_source_overrides WHERE conversation_id=?1",
        )?;
        let overrides = query
            .query_map([conversation_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
            })?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()?;
        drop(query);
        let result = sources
            .into_iter()
            .map(|source| {
                let enabled = overrides.get(&source.id).copied();
                (source, enabled)
            })
            .collect();
        if let Some(snapshot) = snapshot {
            snapshot.commit()?;
        }
        Ok(result)
    }
    fn data_source_override(
        &self,
        conversation_id: &str,
        source_id: &str,
    ) -> Result<Option<bool>, Self::Error> {
        self.store.data_source_override(conversation_id, source_id)
    }
    fn list_data_source_usage(
        &self,
        limit: usize,
    ) -> Result<Vec<StoredDataSourceUsage>, Self::Error> {
        self.store.list_data_source_usage(limit)
    }
}

impl DataSourceRepository for SqliteDataSourceRepository<'_> {
    fn upsert_data_source(
        &mut self,
        source: &StoredDataSourceRecord,
    ) -> Result<StoredDataSourceRecord, Self::Error> {
        self.store.upsert_data_source(source)
    }
    fn delete_data_source(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error> {
        self.store.delete_data_source(id, deleted_at_ms)
    }
    fn set_data_source_override(
        &mut self,
        conversation_id: &str,
        source_id: &str,
        enabled: Option<bool>,
        updated_at_ms: i64,
    ) -> Result<(), Self::Error> {
        self.store
            .set_data_source_override(conversation_id, source_id, enabled, updated_at_ms)
            .map(|_| ())
    }
    fn append_data_source_usage(
        &mut self,
        usage: &StoredDataSourceUsage,
    ) -> Result<(), Self::Error> {
        self.store.append_data_source_usage(usage).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: &str) -> StoredDataSourceRecord {
        StoredDataSourceRecord {
            id: id.into(),
            name: "Test source".into(),
            kind: "test".into(),
            location: "memory://source".into(),
            enabled: true,
            access_policy: "explicit".into(),
            validation_contract_json: "{}".into(),
            capabilities_json: "[]".into(),
            privacy: "local".into(),
            metadata_json: "{}".into(),
            revision: 1,
            created_at_ms: 10,
            updated_at_ms: 10,
            deleted_at_ms: None,
        }
    }

    #[test]
    fn narrow_repository_preserves_owner_rollback_and_commit() {
        let mut store = NolaStore::open(":memory:").expect("open Store");
        store.begin_write_unit_of_work().expect("begin rollback");
        store
            .data_source_repository()
            .upsert_data_source(&source("source-a"))
            .expect("write source");
        store.rollback_write_unit_of_work().expect("rollback");
        assert!(store
            .data_source_repository()
            .list_data_sources()
            .expect("list after rollback")
            .is_empty());

        store.begin_write_unit_of_work().expect("begin commit");
        store
            .data_source_repository()
            .upsert_data_source(&source("source-a"))
            .expect("write source");
        store.commit_write_unit_of_work().expect("commit");
        assert_eq!(
            store
                .data_source_repository()
                .list_data_sources()
                .expect("list after commit")
                .len(),
            1
        );
    }
}
