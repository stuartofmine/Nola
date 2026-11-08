// SPDX-License-Identifier: Apache-2.0
//! Mutations participate in the caller's unit of work, including sync publication.
use serde::{Deserialize, Serialize};

/// Durable model configuration shared by every product surface. Artifact
/// download and process lifecycle remain Host effects; this record stores the
/// validated identity and generation configuration acknowledged by the app.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredModelProfile {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub model_name: String,
    pub runtime: String,
    pub source_json: String,
    pub parameters_json: String,
    pub selected: bool,
    pub enabled: bool,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
}

/// Last committed lifecycle report from one Host. A `ready` record is only a
/// recovery hint until that Host probes the process after restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredModelRuntime {
    pub host_id: String,
    pub profile_id: String,
    pub phase: String,
    pub endpoint_json: Option<String>,
    pub process_id: Option<i64>,
    pub error: Option<String>,
    pub started_at_ms: Option<i64>,
    pub stopped_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

pub trait ModelRepository {
    type Error: std::fmt::Display;
    fn reserve_model_probe(&mut self, record: &nola_domain::model_connection::ModelProbeAttempt) -> Result<(), Self::Error>;
    fn finish_model_probe(&mut self, id: &str, confirmed: bool, at: i64) -> Result<(), Self::Error>;
    fn model_probe_attempt(&self, id: &str) -> Result<Option<nola_domain::model_connection::ModelProbeAttempt>, Self::Error>;
    fn save_gateway_attempt(&mut self, record:&nola_domain::model_connection::GatewayAttemptRecord, expected_revision:Option<i64>) -> Result<nola_domain::model_connection::GatewayAttemptSnapshot,Self::Error>;
    fn list_gateway_attempts(&self, limit:usize, offset:usize) -> Result<Vec<nola_domain::model_connection::GatewayAttemptSnapshot>,Self::Error>;
    fn model_connection(&self, id: &str) -> Result<Option<nola_domain::model_connection::ModelConnectionSnapshot>, Self::Error>;
    fn list_model_connections(&self) -> Result<Vec<nola_domain::model_connection::ModelConnectionSnapshot>, Self::Error>;
    fn commit_model_connection(
        &mut self,
        document: &nola_domain::model_connection::ModelConnectionDocument,
        expected_revision: Option<i64>,
        profiles: &[StoredModelProfile],
        expected_profile_revisions: &std::collections::BTreeMap<String, i64>,
        updated_at_ms: i64,
    ) -> Result<nola_domain::model_connection::ModelConnectionSnapshot, Self::Error>;
    fn model_account(&self, id: &str) -> Result<Option<nola_domain::model_connection::ModelAccountSnapshot>, Self::Error>;
    fn list_model_accounts(&self) -> Result<Vec<nola_domain::model_connection::ModelAccountSnapshot>, Self::Error>;
    fn save_model_account(&mut self, account: &nola_domain::model_connection::ModelAccount, expected_revision: Option<i64>, updated_at_ms: i64) -> Result<nola_domain::model_connection::ModelAccountSnapshot, Self::Error>;
    fn configure_model_endpoint(
        &mut self,
        endpoint_id: &str,
        expected_revisions: &std::collections::BTreeMap<String, i64>,
        profiles: &[StoredModelProfile],
    ) -> Result<(), Self::Error>;
    fn configure_model_parameters(&mut self, input: &StoredModelProfile)
        -> Result<(), Self::Error>;
    fn reset_model_parameters(
        &mut self,
        profile_ids: &[String],
        parameters_json: &str,
        updated_at_ms: i64,
    ) -> Result<(), Self::Error>;
    fn replace_model_profiles(
        &mut self,
        provider: &str,
        runtime: &str,
        profiles: &[StoredModelProfile],
        updated_at_ms: i64,
        expected_revisions: &std::collections::BTreeMap<String, i64>,
    ) -> Result<(), Self::Error>;
    fn upsert_model_profile(&mut self, input: &StoredModelProfile) -> Result<(), Self::Error>;
    fn model_profile(&self, id: &str) -> Result<Option<StoredModelProfile>, Self::Error>;
    fn list_model_profiles(&self) -> Result<Vec<StoredModelProfile>, Self::Error>;
    fn delete_model_profile(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error>;
    fn report_model_runtime(&mut self, input: &StoredModelRuntime) -> Result<(), Self::Error>;
    fn model_runtime(&self, host_id: &str) -> Result<Option<StoredModelRuntime>, Self::Error>;
}

/// Composition port for an owner that can lend only its model capability.
///
/// Returning a separate repository object prevents a model use case from
/// acquiring conversation, task, permission, or synchronization capabilities
/// merely because those aggregates share one physical database.
pub trait ModelRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: ModelRepository<Error = Self::Error>
    where
        Self: 'a;

    fn model_repository(&mut self) -> Self::Repository<'_>;
}

/// Immutable catalog/default facts read on the caller's current Store snapshot.
/// This port performs no selection, runtime start, or credential resolution.
pub struct ModelResolutionFacts {
    pub profiles: Vec<StoredModelProfile>,
    pub default_chat_preference: Option<crate::product_setting::StoredProductSetting>,
}
pub trait ModelResolutionFactsRepository {
    type ModelResolutionError: std::fmt::Display;
    fn model_resolution_facts(&self, default_model_setting_key: &str)
        -> Result<ModelResolutionFacts, Self::ModelResolutionError>;
}
