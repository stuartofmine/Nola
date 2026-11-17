// SPDX-License-Identifier: Apache-2.0
//! Real SQLite model authority composed with the fault-injectable Run adapter.
//! Normal admission only reads this catalog; Goal mutation stays in the Run UoW.
use nola_store::{NolaStore, StoreError};
use nola_store_port::model::*;
#[derive(Clone)]
pub(super) struct Models(pub(super) std::rc::Rc<std::cell::RefCell<NolaStore>>);
impl Default for Models {
    fn default() -> Self { Self(std::rc::Rc::new(std::cell::RefCell::new(NolaStore::open(":memory:").unwrap()))) }
}
impl nola_store_port::product_setting::ProductSettingRepository for Models {
    type Error = StoreError;
    fn put_product_setting(&mut self, key: &str, value_json: &str, updated_at_ms: i64) -> Result<nola_store_port::product_setting::StoredProductSetting, Self::Error> {
        self.0.borrow_mut().product_setting_repository().put_product_setting(key, value_json, updated_at_ms)
    }
    fn product_setting(&self, key: &str) -> Result<Option<nola_store_port::product_setting::StoredProductSetting>, Self::Error> {
        self.0.borrow_mut().product_setting_repository().product_setting(key)
    }
    fn list_product_settings(&self, prefix: &str, limit: usize, offset: usize) -> Result<Vec<nola_store_port::product_setting::StoredProductSetting>, Self::Error> {
        self.0.borrow_mut().product_setting_repository().list_product_settings(prefix, limit, offset)
    }
    fn list_room_submissions(&self, prefix: &str, limit: usize, offset: usize) -> Result<Vec<nola_store_port::product_setting::StoredProductSetting>, Self::Error> {
        self.0.borrow_mut().product_setting_repository().list_room_submissions(prefix, limit, offset)
    }
    fn delete_product_setting(&mut self, key: &str) -> Result<bool, Self::Error> {
        self.0.borrow_mut().product_setting_repository().delete_product_setting(key)
    }
    fn has_write_unit_of_work(&self) -> bool {
        self.0.borrow_mut().product_setting_repository().has_write_unit_of_work()
    }
}
impl nola_store_port::conversation_profile::ConversationProfileRepository for Models {
    type Error = StoreError;
    fn patch_conversation_metadata(&mut self, input: &nola_store_port::conversation_profile::PatchConversationMetadataInput) -> Result<Option<i64>, Self::Error> {
        self.0.borrow_mut().conversation_write_repository().patch_conversation_metadata(input)
    }
    fn patch_conversation_configuration(&mut self, input: &nola_store_port::conversation_profile::PatchConversationConfigurationInput) -> Result<nola_domain::conversation_configuration::ConfigurationCommitResult, Self::Error> {
        self.0.borrow_mut().conversation_write_repository().patch_conversation_configuration(input)
    }
    fn create_conversation(&mut self, input: &nola_store_port::conversation_profile::CreateConversationInput) -> Result<nola_store_port::conversation_profile::CreateConversationResult, Self::Error> {
        self.0.borrow_mut().conversation_write_repository().create_conversation(input)
    }
    fn rename_conversation(&mut self, id: &str, title: &str, title_source: Option<nola_domain::conversation_configuration::ConversationTitleSource>, updated_at_ms: i64) -> Result<bool, Self::Error> {
        self.0.borrow_mut().conversation_write_repository().rename_conversation(id, title, title_source, updated_at_ms)
    }
}
impl nola_store_port::canonical_branch::CanonicalBranchRepository for Models {
    type Error = StoreError;
    fn create_conversation_branch(&mut self, intent: &nola_domain::CanonicalBranchIntent) -> Result<nola_domain::CanonicalBranchResult, Self::Error> {
        self.0.borrow_mut().conversation_write_repository().create_conversation_branch(intent)
    }
}
impl nola_store_port::conversation_deletion::ConversationDeletionRepository for Models {
    type Error = StoreError;
    fn delete_conversation(&mut self, conversation_id: &str) -> Result<bool, Self::Error> {
        self.0.borrow_mut().conversation_write_repository().delete_conversation(conversation_id)
    }
}
impl nola_store_port::transcript_write::TranscriptWriteRepository for Models {
    type Error = StoreError;
    fn message_frontier(&self, conversation_id: &str) -> Result<(usize, i64), Self::Error> {
        self.0.borrow_mut().conversation_write_repository().message_frontier(conversation_id)
    }
    fn has_active_run(&self, conversation_id: &str) -> Result<bool, Self::Error> {
        self.0.borrow_mut().conversation_write_repository().has_active_run(conversation_id)
    }
    fn conversation_id_for_run(&self, run_id: &str) -> Result<Option<String>, Self::Error> {
        self.0.borrow_mut().conversation_write_repository().conversation_id_for_run(run_id)
    }
    fn append_messages(&mut self, input: &nola_store_port::transcript_write::AppendMessagesInput) -> Result<nola_store_port::transcript_write::AppendMessagesResult, Self::Error> {
        self.0.borrow_mut().conversation_write_repository().append_messages(input)
    }
}
impl nola_store_port::transcript_write::TranscriptEditRepository for Models {
    fn apply_conversation_projection(&mut self, input: &nola_store_port::conversation_revert::ApplyConversationProjectionInput) -> Result<nola_store_port::conversation_revert::ApplyConversationProjectionResult, Self::Error> {
        self.0.borrow_mut().conversation_write_repository().apply_conversation_projection(input)
    }
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
