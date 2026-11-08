// SPDX-License-Identifier: Apache-2.0
//! Real SQLite model authority composed with the fault-injectable Run adapter.
//! Normal admission only reads this catalog; Goal mutation stays in the Run UoW.
use nola_store::{NolaStore, StoreError};
use nola_store_port::model::*;
#[derive(Clone)]
pub(super) struct Models(std::rc::Rc<std::cell::RefCell<NolaStore>>);
impl Default for Models {
    fn default() -> Self { Self(std::rc::Rc::new(std::cell::RefCell::new(NolaStore::open(":memory:").unwrap()))) }
}
impl ModelRepository for Models {
    type Error = StoreError;
    fn reserve_model_probe(&mut self, record: &nola_domain::model_connection::ModelProbeAttempt) -> Result<(),Self::Error> { self.0.borrow_mut().model_repository().reserve_model_probe(record) }
    fn finish_model_probe(&mut self, id:&str, confirmed:bool, at:i64) -> Result<(),Self::Error> { self.0.borrow_mut().model_repository().finish_model_probe(id,confirmed,at) }
    fn model_probe_attempt(&self,id:&str) -> Result<Option<nola_domain::model_connection::ModelProbeAttempt>,Self::Error> { self.0.borrow_mut().model_repository().model_probe_attempt(id) }
    fn save_gateway_attempt(&mut self, record:&nola_domain::model_connection::GatewayAttemptRecord, expected_revision:Option<i64>) -> Result<nola_domain::model_connection::GatewayAttemptSnapshot,Self::Error> { self.0.borrow_mut().model_repository().save_gateway_attempt(record,expected_revision) }
    fn list_gateway_attempts(&self, limit:usize, offset:usize) -> Result<Vec<nola_domain::model_connection::GatewayAttemptSnapshot>,Self::Error> { self.0.borrow_mut().model_repository().list_gateway_attempts(limit,offset) }
    fn model_connection(&self, id: &str) -> Result<Option<nola_domain::model_connection::ModelConnectionSnapshot>, Self::Error> { self.0.borrow_mut().model_repository().model_connection(id) }
    fn list_model_connections(&self) -> Result<Vec<nola_domain::model_connection::ModelConnectionSnapshot>, Self::Error> { self.0.borrow_mut().model_repository().list_model_connections() }
    fn commit_model_connection(&mut self, document: &nola_domain::model_connection::ModelConnectionDocument, expected_revision: Option<i64>, profiles: &[StoredModelProfile], expected_profile_revisions: &std::collections::BTreeMap<String, i64>, updated_at_ms: i64) -> Result<nola_domain::model_connection::ModelConnectionSnapshot, Self::Error> {
        self.0.borrow_mut().model_repository().commit_model_connection(document, expected_revision, profiles, expected_profile_revisions, updated_at_ms)
    }
    fn model_account(&self, id: &str) -> Result<Option<nola_domain::model_connection::ModelAccountSnapshot>, Self::Error> { self.0.borrow_mut().model_repository().model_account(id) }
    fn list_model_accounts(&self) -> Result<Vec<nola_domain::model_connection::ModelAccountSnapshot>, Self::Error> { self.0.borrow_mut().model_repository().list_model_accounts() }
    fn save_model_account(&mut self, account: &nola_domain::model_connection::ModelAccount, expected_revision: Option<i64>, updated_at_ms: i64) -> Result<nola_domain::model_connection::ModelAccountSnapshot, Self::Error> { self.0.borrow_mut().model_repository().save_model_account(account, expected_revision, updated_at_ms) }
    fn configure_model_endpoint(&mut self, endpoint_id: &str, expected_revisions: &std::collections::BTreeMap<String, i64>, profiles: &[StoredModelProfile]) -> Result<(), Self::Error> {
        self.0.borrow_mut().model_repository().configure_model_endpoint(endpoint_id, expected_revisions, profiles)
    }
    fn configure_model_parameters(
        &mut self,
        input: &StoredModelProfile,
    ) -> Result<(), Self::Error> {
        self.0.borrow_mut().model_repository().configure_model_parameters(input)
    }
    fn reset_model_parameters(
        &mut self,
        profile_ids: &[String],
        parameters_json: &str,
        updated_at_ms: i64,
    ) -> Result<(), Self::Error> {
        self.0.borrow_mut().model_repository()
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
        self.0.borrow_mut().model_repository()
            .replace_model_profiles(provider, runtime, profiles, updated_at_ms, expected_revisions)
    }
    fn upsert_model_profile(&mut self, input: &StoredModelProfile) -> Result<(), Self::Error> {
        self.0.borrow_mut().model_repository().upsert_model_profile(input)
    }
    fn model_profile(&self, id: &str) -> Result<Option<StoredModelProfile>, Self::Error> {
        self.0.borrow_mut().model_repository().model_profile(id)
    }
    fn list_model_profiles(&self) -> Result<Vec<StoredModelProfile>, Self::Error> {
        self.0.borrow_mut().model_repository().list_model_profiles()
    }
    fn delete_model_profile(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error> {
        self.0.borrow_mut().model_repository().delete_model_profile(id, deleted_at_ms)
    }
    fn report_model_runtime(&mut self, input: &StoredModelRuntime) -> Result<(), Self::Error> {
        self.0.borrow_mut().model_repository().report_model_runtime(input)
    }
    fn model_runtime(&self, host_id: &str) -> Result<Option<StoredModelRuntime>, Self::Error> {
        self.0.borrow_mut().model_repository().model_runtime(host_id)
    }
}
