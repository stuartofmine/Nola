// SPDX-License-Identifier: Apache-2.0
//! Data-source persistence contracts; lifecycle decisions belong to Application.
use serde::{Deserialize, Serialize};

pub trait DataSourceReadRepository {
    type Error: std::fmt::Display;
    fn data_source(&self, id: &str) -> Result<Option<StoredDataSourceRecord>, Self::Error>;
    fn list_data_sources(&self) -> Result<Vec<StoredDataSourceRecord>, Self::Error>;
    /// Read live sources and their conversation overrides from one snapshot,
    /// without issuing a separate persistence query for each source.
    fn list_data_sources_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<(StoredDataSourceRecord, Option<bool>)>, Self::Error>;
    fn data_source_override(
        &self,
        conversation_id: &str,
        source_id: &str,
    ) -> Result<Option<bool>, Self::Error>;
    fn list_data_source_usage(
        &self,
        limit: usize,
    ) -> Result<Vec<StoredDataSourceUsage>, Self::Error>;
}

/// Implementations preserve the caller's unit of work. Application's revision
/// and pending-token checks and the resulting write must share that unit of work
/// (or equivalent serialization); an adapter must not open a separate writer.
pub trait DataSourceRepository: DataSourceReadRepository {
    fn upsert_data_source(
        &mut self,
        source: &StoredDataSourceRecord,
    ) -> Result<StoredDataSourceRecord, Self::Error>;
    fn delete_data_source(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error>;
    fn set_data_source_override(
        &mut self,
        conversation_id: &str,
        source_id: &str,
        enabled: Option<bool>,
        updated_at_ms: i64,
    ) -> Result<(), Self::Error>;
    fn append_data_source_usage(
        &mut self,
        usage: &StoredDataSourceUsage,
    ) -> Result<(), Self::Error>;
}

/// Composition port that lends only data-source persistence for one use case.
pub trait DataSourceRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: DataSourceRepository<Error = Self::Error>
    where
        Self: 'a;

    fn data_source_repository(&mut self) -> Self::Repository<'_>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredDataSourceRecord {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub location: String,
    pub enabled: bool,
    pub access_policy: String,
    pub validation_contract_json: String,
    pub capabilities_json: String,
    pub privacy: String,
    pub metadata_json: String,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredDataSourceUsage {
    pub id: String,
    pub source_id: String,
    pub source_title: String,
    pub consumer: String,
    pub purpose: String,
    pub conversation_id: Option<String>,
    pub disclosed_to_cloud: Option<bool>,
    pub metadata_json: String,
    pub used_at_ms: i64,
}
