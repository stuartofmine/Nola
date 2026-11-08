// SPDX-License-Identifier: Apache-2.0
//! Persistence facts independent of the concrete database.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredProductSetting {
    pub key: String,
    pub value_json: String,
    pub revision: i64,
    pub updated_at_ms: i64,
}

/// Mutations participate in the caller's existing receipt Unit of Work.
pub trait ProductSettingRepository {
    type Error: std::fmt::Display;
    fn put_product_setting(
        &mut self,
        key: &str,
        value_json: &str,
        updated_at_ms: i64,
    ) -> Result<StoredProductSetting, Self::Error>;
    fn product_setting(&self, key: &str) -> Result<Option<StoredProductSetting>, Self::Error>;
    fn list_product_settings(&self, prefix: &str, limit: usize, offset: usize)
        -> Result<Vec<StoredProductSetting>, Self::Error>;
    /// Room transcript pages use immutable submission time, not last mutation.
    fn list_room_submissions(&self, prefix: &str, limit: usize, offset: usize)
        -> Result<Vec<StoredProductSetting>, Self::Error>;
    fn delete_product_setting(&mut self, key: &str) -> Result<bool, Self::Error>;
    fn has_write_unit_of_work(&self) -> bool;
}

/// Composition port that lends only product-setting persistence for one use case.
pub trait ProductSettingRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: ProductSettingRepository<Error = Self::Error>
    where
        Self: 'a;

    fn product_setting_repository(&mut self) -> Self::Repository<'_>;
}
