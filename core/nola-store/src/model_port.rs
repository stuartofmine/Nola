// SPDX-License-Identifier: Apache-2.0
//! Narrow model repository sharing the Application-owned SQLite unit of work.
use crate::{NolaStore, StoreError};
use nola_store_port::model::*;

/// The only Store object exposed to model use cases.
///
/// The repository borrows the owning Store, so it cannot outlive the command
/// transaction or become an independent canonical writer.
///
/// The full Store is intentionally not a model repository:
///
/// ```compile_fail
/// use nola_store::{ModelRepository, NolaStore};
///
/// fn requires_model_repository(_: &mut impl ModelRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// requires_model_repository(&mut store);
/// ```
///
/// The borrowed model repository cannot be widened into another capability:
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::mcp::McpRepository;
///
/// fn requires_mcp_repository(_: &mut impl McpRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// let mut models = store.model_repository();
/// requires_mcp_repository(&mut models);
/// ```
pub struct SqliteModelRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn model_repository(&mut self) -> SqliteModelRepository<'_> {
        SqliteModelRepository { store: self }
    }
}

impl ModelRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteModelRepository<'a>;

    fn model_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::model_repository(self)
    }
}

impl ModelRepository for SqliteModelRepository<'_> {
    type Error = StoreError;
    fn reserve_model_probe(&mut self, record: &nola_domain::model_connection::ModelProbeAttempt) -> Result<(),Self::Error> { self.store.reserve_model_probe(record) }
    fn finish_model_probe(&mut self, id:&str, confirmed:bool, at:i64) -> Result<(),Self::Error> { self.store.finish_model_probe(id,confirmed,at) }
    fn model_probe_attempt(&self,id:&str) -> Result<Option<nola_domain::model_connection::ModelProbeAttempt>,Self::Error> { self.store.model_probe_attempt(id) }
    fn save_gateway_attempt(&mut self, record:&nola_domain::model_connection::GatewayAttemptRecord, expected_revision:Option<i64>) -> Result<nola_domain::model_connection::GatewayAttemptSnapshot,Self::Error> { self.store.save_gateway_attempt(record,expected_revision) }
    fn list_gateway_attempts(&self, limit:usize, offset:usize) -> Result<Vec<nola_domain::model_connection::GatewayAttemptSnapshot>,Self::Error> { self.store.list_gateway_attempts(limit,offset) }
    fn model_connection(&self, id: &str) -> Result<Option<nola_domain::model_connection::ModelConnectionSnapshot>, Self::Error> { self.store.model_connection(id) }
    fn list_model_connections(&self) -> Result<Vec<nola_domain::model_connection::ModelConnectionSnapshot>, Self::Error> { self.store.list_model_connections() }
    fn commit_model_connection(&mut self, document: &nola_domain::model_connection::ModelConnectionDocument, expected_revision: Option<i64>, profiles: &[StoredModelProfile], expected_profile_revisions: &std::collections::BTreeMap<String, i64>, updated_at_ms: i64) -> Result<nola_domain::model_connection::ModelConnectionSnapshot, Self::Error> {
        self.store.commit_model_connection(document, expected_revision, profiles, expected_profile_revisions, updated_at_ms)
    }
    fn model_account(&self, id: &str) -> Result<Option<nola_domain::model_connection::ModelAccountSnapshot>, Self::Error> { self.store.model_account(id) }
    fn list_model_accounts(&self) -> Result<Vec<nola_domain::model_connection::ModelAccountSnapshot>, Self::Error> { self.store.list_model_accounts() }
    fn save_model_account(&mut self, account: &nola_domain::model_connection::ModelAccount, expected_revision: Option<i64>, updated_at_ms: i64) -> Result<nola_domain::model_connection::ModelAccountSnapshot, Self::Error> { self.store.save_model_account(account, expected_revision, updated_at_ms) }
    fn configure_model_endpoint(&mut self, endpoint_id: &str, expected_revisions: &std::collections::BTreeMap<String, i64>, profiles: &[StoredModelProfile]) -> Result<(), Self::Error> {
        self.store.configure_model_endpoint(endpoint_id, expected_revisions, profiles)
    }
    fn configure_model_parameters(
        &mut self,
        input: &StoredModelProfile,
    ) -> Result<(), Self::Error> {
        self.store.configure_model_parameters(input)
    }
    fn reset_model_parameters(
        &mut self,
        profile_ids: &[String],
        parameters_json: &str,
        updated_at_ms: i64,
    ) -> Result<(), Self::Error> {
        self.store
            .reset_model_parameters(profile_ids, parameters_json, updated_at_ms)
    }
    fn replace_model_profiles(
        &mut self,
        provider: &str,
        runtime: &str,
        profiles: &[StoredModelProfile],
        updated_at_ms: i64,
        expected_revisions: &std::collections::BTreeMap<String, i64>,
    ) -> Result<(), Self::Error> {
        self.store
            .replace_model_profiles(provider, runtime, profiles, updated_at_ms, expected_revisions)
    }
    fn upsert_model_profile(&mut self, input: &StoredModelProfile) -> Result<(), Self::Error> {
        self.store.upsert_model_profile(input)
    }
    fn model_profile(&self, id: &str) -> Result<Option<StoredModelProfile>, Self::Error> {
        self.store.model_profile(id)
    }
    fn list_model_profiles(&self) -> Result<Vec<StoredModelProfile>, Self::Error> {
        self.store.list_model_profiles()
    }
    fn delete_model_profile(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error> {
        self.store.delete_model_profile(id, deleted_at_ms)
    }
    fn report_model_runtime(&mut self, input: &StoredModelRuntime) -> Result<(), Self::Error> {
        self.store.report_model_runtime(input)
    }
    fn model_runtime(&self, host_id: &str) -> Result<Option<StoredModelRuntime>, Self::Error> {
        self.store.model_runtime(host_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, updated_at_ms: i64) -> StoredModelProfile {
        StoredModelProfile {
            id: id.to_string(),
            display_name: "Test model".to_string(),
            provider: "test-provider".to_string(),
            model_name: "test-model".to_string(),
            runtime: "remote".to_string(),
            source_json: r#"{"schemaVersion":1}"#.to_string(),
            parameters_json: r#"{"temperature":0.2}"#.to_string(),
            selected: true,
            enabled: true,
            revision: 1,
            created_at_ms: 10,
            updated_at_ms,
            deleted_at_ms: None,
        }
    }

    #[test]
    fn narrow_repository_reads_and_writes_inside_the_owner_unit_of_work() {
        let mut store = NolaStore::open(":memory:").expect("open Store");
        store
            .begin_write_unit_of_work()
            .expect("begin Application unit of work");
        {
            let mut models = store.model_repository();
            models
                .upsert_model_profile(&profile("model-a", 10))
                .expect("write model through narrow repository");
            assert_eq!(
                models
                    .model_profile("model-a")
                    .expect("read model through narrow repository")
                    .expect("stored model")
                    .model_name,
                "test-model"
            );
        }
        store
            .rollback_write_unit_of_work()
            .expect("rollback Application unit of work");
        assert!(store
            .model_repository()
            .model_profile("model-a")
            .expect("read after rollback")
            .is_none());

        store
            .begin_write_unit_of_work()
            .expect("begin second Application unit of work");
        store
            .model_repository()
            .upsert_model_profile(&profile("model-a", 20))
            .expect("write committed model");
        store
            .commit_write_unit_of_work()
            .expect("commit Application unit of work");
        assert_eq!(
            store
                .model_repository()
                .list_model_profiles()
                .expect("list committed models")
                .len(),
            1
        );
    }
}

impl ModelResolutionFactsRepository for NolaStore {
    type ModelResolutionError = StoreError;
    fn model_resolution_facts(&self, default_model_setting_key: &str)
        -> Result<ModelResolutionFacts, StoreError> {
        Ok(ModelResolutionFacts {
            profiles: self.list_model_profiles()?,
            default_chat_preference: self.product_setting(default_model_setting_key)?,
        })
    }
}
