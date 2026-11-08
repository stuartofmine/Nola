// SPDX-License-Identifier: Apache-2.0
//! Narrow automation persistence on the Application-owned transaction.
use crate::{NolaStore, StoreError};
use nola_store_port::automation::*;

/// Borrowed automation capability; the full Store is not an automation repository.
///
/// ```compile_fail
/// use nola_store::{AutomationRepository, NolaStore};
/// fn requires_automation(_: &mut impl AutomationRepository) {}
/// requires_automation(&mut NolaStore::open(":memory:").unwrap());
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::replica_sync::ReplicaSyncRepository;
/// fn requires_sync(_: &mut impl ReplicaSyncRepository) {}
/// requires_sync(&mut NolaStore::open(":memory:").unwrap().automation_repository());
/// ```
pub struct SqliteAutomationRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn automation_repository(&mut self) -> SqliteAutomationRepository<'_> {
        SqliteAutomationRepository { store: self }
    }
}

impl AutomationRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteAutomationRepository<'a>;

    fn automation_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::automation_repository(self)
    }
}

impl AutomationRepository for SqliteAutomationRepository<'_> {
    type Error = StoreError;
    fn pending_automation_executions(&self, after_occurrence_id: Option<&str>, limit: usize) -> Result<Vec<String>, Self::Error> {
        self.store.pending_automation_executions(after_occurrence_id, limit)
    }
    fn validate_automation_execution(&self, lease: &AutomationExecutionLease, now_ms: i64) -> Result<(), Self::Error> {
        self.store.validate_automation_execution(lease, now_ms)
    }
    fn bind_automation_execution(&mut self, lease: &AutomationExecutionLease, binding: &AutomationExecutionBinding, request_digest: &str, accepted_json: &str) -> Result<(), Self::Error> {
        self.store.bind_automation_execution(lease, binding, request_digest, accepted_json)
    }
    fn replay_automation_admission(&self, occurrence_id: &str, request_digest: &str) -> Result<Option<String>, Self::Error> {
        self.store.replay_automation_admission(occurrence_id, request_digest)
    }
    fn observe_automation_execution(&self, occurrence_id: &str) -> Result<AutomationExecutionObservation, Self::Error> {
        self.store.observe_automation_execution(occurrence_id)
    }
    fn reconcile_automation_execution(&mut self, occurrence_id: &str, now_ms: i64) -> Result<Option<AutomationExecutionReceipt>, Self::Error> {
        self.store.reconcile_automation_execution(occurrence_id, now_ms)
    }
    fn observe_automation_worker(&mut self, lease: &AutomationExecutionLease, observation: &AutomationWorkerObservation, now_ms: i64) -> Result<bool, Self::Error> {
        self.store.observe_automation_worker(lease, observation, now_ms)
    }
    fn acknowledge_automation_notification(&mut self, occurrence_id: &str, notification_id: &str, now_ms: i64) -> Result<bool, Self::Error> {
        self.store.acknowledge_automation_notification(occurrence_id, notification_id, now_ms)
    }
    fn update_automation_definition_if_revision(&mut self, definition: &StoredAutomationDefinition, expected_revision: i64) -> Result<Option<StoredAutomationDefinition>, Self::Error> {
        self.store.update_automation_definition_if_revision(definition, expected_revision)
    }
    fn upsert_automation_definition(
        &mut self,
        definition: &StoredAutomationDefinition,
    ) -> Result<StoredAutomationDefinition, Self::Error> {
        self.store.upsert_automation_definition(definition)
    }
    fn reconcile_automation_definitions(
        &mut self,
        definitions: &[StoredAutomationDefinition],
        schedule_formats: &[String],
        reconciled_at_ms: i64,
    ) -> Result<Vec<StoredAutomationDefinition>, Self::Error> {
        self.store
            .reconcile_automation_definitions(definitions, schedule_formats, reconciled_at_ms)
    }
    fn automation_definition(
        &self,
        id: &str,
    ) -> Result<Option<StoredAutomationDefinition>, Self::Error> {
        self.store.automation_definition(id)
    }
    fn list_automation_definitions(&self) -> Result<Vec<StoredAutomationDefinition>, Self::Error> {
        self.store.list_automation_definitions()
    }
    fn delete_automation_definition(
        &mut self,
        id: &str,
        deleted_at_ms: i64,
    ) -> Result<bool, Self::Error> {
        self.store.delete_automation_definition(id, deleted_at_ms)
    }
    fn enqueue_automation_run(
        &mut self,
        run: &StoredAutomationRun,
    ) -> Result<StoredAutomationRun, Self::Error> {
        self.store.enqueue_automation_run(run)
    }
    fn claim_automation_run(
        &mut self,
        id: &str,
        worker_id: &str,
        lease_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<bool, Self::Error> {
        self.store
            .claim_automation_run(id, worker_id, lease_token, now_ms, lease_until_ms)
    }
    fn heartbeat_automation_run(
        &mut self,
        id: &str,
        lease_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<bool, Self::Error> {
        self.store
            .heartbeat_automation_run(id, lease_token, now_ms, lease_until_ms)
    }
    fn retry_automation_run(
        &mut self,
        id: &str,
        lease_token: &str,
        next_retry_at_ms: i64,
        error_json: &str,
        updated_at_ms: i64,
    ) -> Result<bool, Self::Error> {
        self.store.retry_automation_run(
            id,
            lease_token,
            next_retry_at_ms,
            error_json,
            updated_at_ms,
        )
    }
    fn automation_run(&self, id: &str) -> Result<Option<StoredAutomationRun>, Self::Error> {
        self.store.automation_run(id)
    }
    fn list_automation_runs(
        &self,
        automation_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredAutomationRun>, Self::Error> {
        self.store.list_automation_runs(automation_id, limit)
    }
}
