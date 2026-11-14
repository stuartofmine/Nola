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
    /// User-facing Room transcript, excluding expert-owned peer requests.
    fn list_public_room_submissions(&self, prefix: &str, limit: usize, offset: usize)
        -> Result<Vec<StoredProductSetting>, Self::Error> {
        let mut result=Vec::new();
        let mut scanned=0usize;
        let mut public=0usize;
        loop {
            let page=self.list_room_submissions(prefix,200,scanned)?;
            let more=page.len()==200;
            scanned+=page.len();
            for row in page {
                let id=serde_json::from_str::<serde_json::Value>(&row.value_json).ok()
                    .and_then(|value|value.pointer("/input/submission_id").and_then(|v|v.as_str()).map(str::to_owned));
                if let Some(id)=id {
                    if self.product_setting(&format!("expert_peer_private/v1/exchange/{id}"))?.is_some() {continue;}
                }
                if public>=offset && result.len()<limit {result.push(row);}
                public+=1;
                if result.len()==limit {return Ok(result);}
            }
            if !more {return Ok(result);}
        }
    }
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
