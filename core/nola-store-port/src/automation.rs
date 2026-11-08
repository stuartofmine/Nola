// SPDX-License-Identifier: Apache-2.0
//! Durable automation schedules and leased occurrence facts.
use serde::{Deserialize, Serialize};
pub use nola_domain::automation_execution::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredAutomationDefinition {
    pub execution_spec: AutomationDefinitionSpec,
    pub id: String,
    pub name: String,
    pub schedule_format: String,
    pub enabled: bool,
    pub revision: i64,
    pub metadata_json: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredAutomationRun {
    pub definition_revision: i64,
    pub id: String,
    pub automation_id: String,
    pub occurrence_key: String,
    pub trigger: String,
    pub status: String,
    pub attempt: i64,
    pub worker_id: Option<String>,
    pub lease_token: Option<String>,
    pub lease_until_ms: Option<i64>,
    pub scheduled_for_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
    pub next_retry_at_ms: Option<i64>,
    pub result_json: Option<String>,
    pub error_json: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// All mutations join the owning command Unit of Work. Claims and updates
/// must enforce the durable lease token, status and occurrence uniqueness.
pub trait AutomationRepository {
    type Error: std::fmt::Display;
    fn validate_automation_execution(&self, lease: &AutomationExecutionLease, now_ms: i64) -> Result<(), Self::Error>;
    fn bind_automation_execution(&mut self, lease: &AutomationExecutionLease, binding: &AutomationExecutionBinding, request_digest: &str, accepted_json: &str) -> Result<(), Self::Error>;
    fn replay_automation_admission(&self, occurrence_id: &str, request_digest: &str) -> Result<Option<String>, Self::Error>;
    fn pending_automation_executions(&self, after_occurrence_id: Option<&str>, limit: usize) -> Result<Vec<String>, Self::Error>;
    fn observe_automation_execution(&self, occurrence_id: &str) -> Result<AutomationExecutionObservation, Self::Error>;
    fn reconcile_automation_execution(&mut self, occurrence_id: &str, now_ms: i64) -> Result<Option<AutomationExecutionReceipt>, Self::Error>;
    fn observe_automation_worker(&mut self, lease: &AutomationExecutionLease, observation: &AutomationWorkerObservation, now_ms: i64) -> Result<bool, Self::Error>;
    fn acknowledge_automation_notification(&mut self, occurrence_id: &str, notification_id: &str, now_ms: i64) -> Result<bool, Self::Error>;
    fn update_automation_definition_if_revision(
        &mut self,
        definition: &StoredAutomationDefinition,
        expected_revision: i64,
    ) -> Result<Option<StoredAutomationDefinition>, Self::Error>;
    fn upsert_automation_definition(
        &mut self,
        definition: &StoredAutomationDefinition,
    ) -> Result<StoredAutomationDefinition, Self::Error>;
    fn reconcile_automation_definitions(
        &mut self,
        definitions: &[StoredAutomationDefinition],
        schedule_formats: &[String],
        reconciled_at_ms: i64,
    ) -> Result<Vec<StoredAutomationDefinition>, Self::Error>;
    fn automation_definition(
        &self,
        id: &str,
    ) -> Result<Option<StoredAutomationDefinition>, Self::Error>;
    fn list_automation_definitions(&self) -> Result<Vec<StoredAutomationDefinition>, Self::Error>;
    fn delete_automation_definition(
        &mut self,
        id: &str,
        deleted_at_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn enqueue_automation_run(
        &mut self,
        run: &StoredAutomationRun,
    ) -> Result<StoredAutomationRun, Self::Error>;
    fn claim_automation_run(
        &mut self,
        id: &str,
        worker_id: &str,
        lease_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn heartbeat_automation_run(
        &mut self,
        id: &str,
        lease_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn retry_automation_run(
        &mut self,
        id: &str,
        lease_token: &str,
        next_retry_at_ms: i64,
        error_json: &str,
        updated_at_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn automation_run(&self, id: &str) -> Result<Option<StoredAutomationRun>, Self::Error>;
    fn list_automation_runs(
        &self,
        automation_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredAutomationRun>, Self::Error>;
}

/// Composition port that lends only automation scheduling persistence.
pub trait AutomationRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: AutomationRepository<Error = Self::Error>
    where
        Self: 'a;

    fn automation_repository(&mut self) -> Self::Repository<'_>;
}
