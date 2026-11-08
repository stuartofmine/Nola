// SPDX-License-Identifier: Apache-2.0
//! Narrow product-setting adapter sharing the Application-owned transaction.
use crate::{NolaStore, StoreError};
use nola_store_port::product_setting::*;

/// Borrowed product-setting capability.
///
/// ```compile_fail
/// use nola_store::{NolaStore, ProductSettingRepository};
/// fn requires_settings(_: &mut impl ProductSettingRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// requires_settings(&mut store);
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::document_source::DocumentSourceRepository;
/// fn requires_documents(_: &mut impl DocumentSourceRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// let mut settings = store.product_setting_repository();
/// requires_documents(&mut settings);
/// ```
pub struct SqliteProductSettingRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn product_setting_repository(&mut self) -> SqliteProductSettingRepository<'_> {
        SqliteProductSettingRepository { store: self }
    }
}

impl ProductSettingRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteProductSettingRepository<'a>;

    fn product_setting_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::product_setting_repository(self)
    }
}

impl ProductSettingRepository for SqliteProductSettingRepository<'_> {
    type Error = StoreError;
    fn list_room_submissions(&self, prefix: &str, limit: usize, offset: usize)
        -> Result<Vec<StoredProductSetting>, Self::Error> {
        self.store.list_room_submissions(prefix, limit, offset)
    }
    fn list_product_settings(&self, prefix: &str, limit: usize, offset: usize)
        -> Result<Vec<StoredProductSetting>, Self::Error> {
        self.store.list_product_settings(prefix, limit, offset)
    }
    fn put_product_setting(
        &mut self,
        key: &str,
        value_json: &str,
        updated_at_ms: i64,
    ) -> Result<StoredProductSetting, Self::Error> {
        self.store
            .put_product_setting(key, value_json, updated_at_ms)
    }
    fn product_setting(&self, key: &str) -> Result<Option<StoredProductSetting>, Self::Error> {
        self.store.product_setting(key)
    }
    fn delete_product_setting(&mut self, key: &str) -> Result<bool, Self::Error> {
        self.store.delete_product_setting(key)
    }
    fn has_write_unit_of_work(&self) -> bool {
        self.store.has_write_unit_of_work()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_repository_preserves_owner_rollback_and_commit() {
        let mut store = NolaStore::open(":memory:").expect("open Store");
        store.begin_write_unit_of_work().expect("begin rollback");
        store
            .product_setting_repository()
            .put_product_setting("setting-a", "true", 10)
            .expect("write setting");
        store.rollback_write_unit_of_work().expect("rollback");
        assert!(store
            .product_setting_repository()
            .product_setting("setting-a")
            .expect("read after rollback")
            .is_none());

        store.begin_write_unit_of_work().expect("begin commit");
        store
            .product_setting_repository()
            .put_product_setting("setting-a", "true", 20)
            .expect("write setting");
        store.commit_write_unit_of_work().expect("commit");
        assert!(store
            .product_setting_repository()
            .product_setting("setting-a")
            .expect("read after commit")
            .is_some());
    }
}
