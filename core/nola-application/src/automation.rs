// SPDX-License-Identifier: Apache-2.0
//! Automation use-case boundary.
//!
//! Application owns the product objects and lifecycle commands exposed to
//! clients. Store records are private persistence details mapped at this edge.

use nola_store_port::automation::{
    AutomationRepository, StoredAutomationDefinition, StoredAutomationRun,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationDefinition {
    pub execution_spec: nola_domain::automation_execution::AutomationDefinitionSpec,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunStatus {
    Queued,
    Running,
    RetryWait,
    LaunchFailed,
    Reconciled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationRun {
    pub definition_revision: i64,
    pub id: String,
    pub automation_id: String,
    pub occurrence_key: String,
    pub trigger: String,
    pub status: AutomationRunStatus,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationDefinitionCommand {
    Get {
        id: String,
    },
    UpdateIfRevision {
        input: AutomationDefinition,
        expected_revision: i64,
    },
    Upsert {
        input: AutomationDefinition,
    },
    Reconcile {
        inputs: Vec<AutomationDefinition>,
        schedule_formats: Vec<String>,
        reconciled_at_ms: i64,
    },
    Delete {
        id: String,
        deleted_at_ms: i64,
    },
    List,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationRunCommand {
    Enqueue {
        input: AutomationRun,
    },
    Claim {
        id: String,
        worker_id: String,
        lease_token: String,
        now_ms: i64,
        lease_until_ms: i64,
    },
    Heartbeat {
        id: String,
        lease_token: String,
        now_ms: i64,
        lease_until_ms: i64,
    },
    Retry {
        id: String,
        lease_token: String,
        next_retry_at_ms: i64,
        error_json: String,
        updated_at_ms: i64,
    },
    Load {
        id: String,
    },
    List {
        automation_id: Option<String>,
        limit: usize,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AutomationCommand {
    Definition {
        command: AutomationDefinitionCommand,
    },
    Run {
        command: AutomationRunCommand,
    },
}

pub(crate) fn execute_automation<R: AutomationRepository + ?Sized>(
    store: &mut R,
    command: AutomationCommand,
) -> Result<Value, String> {
    match command {
        AutomationCommand::Definition { command } => execute_definition(store, command),
        AutomationCommand::Run { command } => execute_run(store, command),
    }
}

fn execute_definition<R: AutomationRepository + ?Sized>(
    store: &mut R,
    command: AutomationDefinitionCommand,
) -> Result<Value, String> {
    match command {
        AutomationDefinitionCommand::Get { id } => {
            let definition = store.automation_definition(&id)
                .map_err(|error| format!("read automation failed: {error}"))?;
            serde_json::to_value(definition.map(AutomationDefinition::from))
                .map_err(|error| error.to_string())
        }
        AutomationDefinitionCommand::UpdateIfRevision {
            input,
            expected_revision,
        } => {
            if expected_revision < 1 {
                return Err("expected automation revision must be positive".into());
            }
            let committed = store
                .update_automation_definition_if_revision(
                    &StoredAutomationDefinition::from(input),
                    expected_revision,
                )
                .map_err(|error| format!("conditional automation update failed: {error}"))?;
            serde_json::to_value(committed.map(AutomationDefinition::from))
                .map_err(|error| error.to_string())
        }
        AutomationDefinitionCommand::Upsert { input } => {
            let stored = StoredAutomationDefinition::from(input);
            let committed = store
                .upsert_automation_definition(&stored)
                .map_err(|error| format!("save automation failed: {error}"))?;
            serde_json::to_value(AutomationDefinition::from(committed))
                .map_err(|error| error.to_string())
        }
        AutomationDefinitionCommand::Reconcile {
            inputs,
            schedule_formats,
            reconciled_at_ms,
        } => {
            let stored = inputs
                .into_iter()
                .map(StoredAutomationDefinition::from)
                .collect::<Vec<_>>();
            let committed = store
                .reconcile_automation_definitions(&stored, &schedule_formats, reconciled_at_ms)
                .map_err(|error| format!("reconcile automations failed: {error}"))?;
            serde_json::to_value(
                committed
                    .into_iter()
                    .map(AutomationDefinition::from)
                    .collect::<Vec<_>>(),
            )
            .map_err(|error| error.to_string())
        }
        AutomationDefinitionCommand::Delete { id, deleted_at_ms } => Ok(json!(store
            .delete_automation_definition(&id, deleted_at_ms)
            .map_err(|error| format!("delete automation failed: {error}"))?)),
        AutomationDefinitionCommand::List => {
            let definitions = store
                .list_automation_definitions()
                .map_err(|error| format!("list automations failed: {error}"))?;
            serde_json::to_value(
                definitions
                    .into_iter()
                    .map(AutomationDefinition::from)
                    .collect::<Vec<_>>(),
            )
            .map_err(|error| error.to_string())
        }
    }
}

fn execute_run<R: AutomationRepository + ?Sized>(
    store: &mut R,
    command: AutomationRunCommand,
) -> Result<Value, String> {
    match command {
        AutomationRunCommand::Enqueue { input } => {
            let stored = StoredAutomationRun::from(input);
            let committed = store
                .enqueue_automation_run(&stored)
                .map_err(|error| format!("enqueue automation run failed: {error}"))?;
            serde_json::to_value(AutomationRun::try_from(committed)?)
                .map_err(|error| error.to_string())
        }
        AutomationRunCommand::Claim {
            id,
            worker_id,
            lease_token,
            now_ms,
            lease_until_ms,
        } => Ok(json!(store
            .claim_automation_run(&id, &worker_id, &lease_token, now_ms, lease_until_ms)
            .map_err(|error| format!(
                "claim automation run failed: {error}"
            ))?)),
        AutomationRunCommand::Heartbeat {
            id,
            lease_token,
            now_ms,
            lease_until_ms,
        } => Ok(json!(store
            .heartbeat_automation_run(&id, &lease_token, now_ms, lease_until_ms)
            .map_err(|error| format!(
                "heartbeat automation run failed: {error}"
            ))?)),
        AutomationRunCommand::Retry {
            id,
            lease_token,
            next_retry_at_ms,
            error_json,
            updated_at_ms,
        } => Ok(json!(store
            .retry_automation_run(
                &id,
                &lease_token,
                next_retry_at_ms,
                &error_json,
                updated_at_ms,
            )
            .map_err(|error| format!(
                "retry automation run failed: {error}"
            ))?)),
        AutomationRunCommand::Load { id } => {
            let run = store
                .automation_run(&id)
                .map_err(|error| format!("load automation run failed: {error}"))?
                .map(AutomationRun::try_from)
                .transpose()?;
            serde_json::to_value(run).map_err(|error| error.to_string())
        }
        AutomationRunCommand::List {
            automation_id,
            limit,
        } => {
            let runs = store
                .list_automation_runs(automation_id.as_deref(), limit)
                .map_err(|error| format!("list automation runs failed: {error}"))?
                .into_iter()
                .map(AutomationRun::try_from)
                .collect::<Result<Vec<_>, _>>()?;
            serde_json::to_value(runs).map_err(|error| error.to_string())
        }
    }
}

impl AutomationRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::RetryWait => "retry_wait",
            Self::LaunchFailed => "launch_failed",
            Self::Reconciled => "reconciled",
        }
    }
}

impl TryFrom<&str> for AutomationRunStatus {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "retry_wait" => Ok(Self::RetryWait),
            "launch_failed" => Ok(Self::LaunchFailed),
            "reconciled" => Ok(Self::Reconciled),
            other => Err(format!("unknown automation run status: {other}")),
        }
    }
}

impl From<AutomationDefinition> for StoredAutomationDefinition {
    fn from(value: AutomationDefinition) -> Self {
        Self {
            execution_spec: value.execution_spec,
            id: value.id,
            name: value.name,
            schedule_format: value.schedule_format,
            enabled: value.enabled,
            revision: value.revision,
            metadata_json: value.metadata_json,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
            deleted_at_ms: value.deleted_at_ms,
        }
    }
}

impl From<StoredAutomationDefinition> for AutomationDefinition {
    fn from(value: StoredAutomationDefinition) -> Self {
        Self {
            execution_spec: value.execution_spec,
            id: value.id,
            name: value.name,
            schedule_format: value.schedule_format,
            enabled: value.enabled,
            revision: value.revision,
            metadata_json: value.metadata_json,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
            deleted_at_ms: value.deleted_at_ms,
        }
    }
}

impl From<AutomationRun> for StoredAutomationRun {
    fn from(value: AutomationRun) -> Self {
        Self {
            definition_revision: value.definition_revision,
            id: value.id,
            automation_id: value.automation_id,
            occurrence_key: value.occurrence_key,
            trigger: value.trigger,
            status: value.status.as_str().to_string(),
            attempt: value.attempt,
            worker_id: value.worker_id,
            lease_token: value.lease_token,
            lease_until_ms: value.lease_until_ms,
            scheduled_for_ms: value.scheduled_for_ms,
            started_at_ms: value.started_at_ms,
            finished_at_ms: value.finished_at_ms,
            next_retry_at_ms: value.next_retry_at_ms,
            result_json: value.result_json,
            error_json: value.error_json,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
        }
    }
}

impl TryFrom<StoredAutomationRun> for AutomationRun {
    type Error = String;

    fn try_from(value: StoredAutomationRun) -> Result<Self, Self::Error> {
        Ok(Self {
            definition_revision: value.definition_revision,
            id: value.id,
            automation_id: value.automation_id,
            occurrence_key: value.occurrence_key,
            trigger: value.trigger,
            status: AutomationRunStatus::try_from(value.status.as_str())?,
            attempt: value.attempt,
            worker_id: value.worker_id,
            lease_token: value.lease_token,
            lease_until_ms: value.lease_until_ms,
            scheduled_for_ms: value.scheduled_for_ms,
            started_at_ms: value.started_at_ms,
            finished_at_ms: value.finished_at_ms,
            next_retry_at_ms: value.next_retry_at_ms,
            result_json: value.result_json,
            error_json: value.error_json,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
        })
    }
}

#[cfg(test)]
mod port_tests {
    use super::*;
    #[test]
    fn reconcile_requires_an_explicit_format_scope_on_the_wire() {
        assert!(
            serde_json::from_value::<AutomationDefinitionCommand>(json!({
                "type":"reconcile", "inputs":[], "reconciled_at_ms":100
            }))
            .is_err()
        );
        assert!(serde_json::from_value::<AutomationDefinitionCommand>(json!({
            "type":"reconcile", "inputs":[], "schedule_formats":["script"], "reconciled_at_ms":100
        })).is_ok());
    }
    struct Facts {
        claim: Option<(String, String, String, i64, i64)>,
        fail: bool,
        run: StoredAutomationRun,
    }
    #[allow(unused_variables)]
    impl AutomationRepository for Facts {
        type Error = &'static str;
        fn pending_automation_executions(&self, _: Option<&str>, _: usize) -> Result<Vec<String>, Self::Error> { unreachable!() }
        fn validate_automation_execution(&self, _: &nola_domain::automation_execution::AutomationExecutionLease, _: i64) -> Result<(), Self::Error> { unreachable!() }
        fn bind_automation_execution(&mut self, _: &nola_domain::automation_execution::AutomationExecutionLease, _: &nola_domain::automation_execution::AutomationExecutionBinding, _: &str, _: &str) -> Result<(), Self::Error> { unreachable!() }
        fn replay_automation_admission(&self, _: &str, _: &str) -> Result<Option<String>, Self::Error> { unreachable!() }
        fn observe_automation_execution(&self, _: &str) -> Result<nola_domain::automation_execution::AutomationExecutionObservation, Self::Error> { unreachable!() }
        fn reconcile_automation_execution(&mut self, _: &str, _: i64) -> Result<Option<nola_domain::automation_execution::AutomationExecutionReceipt>, Self::Error> { unreachable!() }
        fn observe_automation_worker(&mut self, _: &nola_domain::automation_execution::AutomationExecutionLease, _: &nola_domain::automation_execution::AutomationWorkerObservation, _: i64) -> Result<bool, Self::Error> { unreachable!() }
        fn acknowledge_automation_notification(&mut self, _: &str, _: &str, _: i64) -> Result<bool, Self::Error> { unreachable!() }
        fn update_automation_definition_if_revision(
            &mut self,
            definition: &StoredAutomationDefinition,
            expected_revision: i64,
        ) -> Result<Option<StoredAutomationDefinition>, Self::Error> {
            unimplemented!("unexpected repository call")
        }
        fn upsert_automation_definition(
            &mut self,
            definition: &StoredAutomationDefinition,
        ) -> Result<StoredAutomationDefinition, Self::Error> {
            unimplemented!("unexpected repository call")
        }
        fn reconcile_automation_definitions(
            &mut self,
            definitions: &[StoredAutomationDefinition],
            schedule_formats: &[String],
            reconciled_at_ms: i64,
        ) -> Result<Vec<StoredAutomationDefinition>, Self::Error> {
            unimplemented!("unexpected repository call")
        }
        fn automation_definition(
            &self,
            id: &str,
        ) -> Result<Option<StoredAutomationDefinition>, Self::Error> {
            unimplemented!("unexpected repository call")
        }
        fn list_automation_definitions(
            &self,
        ) -> Result<Vec<StoredAutomationDefinition>, Self::Error> {
            unimplemented!("unexpected repository call")
        }
        fn delete_automation_definition(
            &mut self,
            id: &str,
            deleted_at_ms: i64,
        ) -> Result<bool, Self::Error> {
            unimplemented!("unexpected repository call")
        }
        fn enqueue_automation_run(
            &mut self,
            run: &StoredAutomationRun,
        ) -> Result<StoredAutomationRun, Self::Error> {
            unimplemented!("unexpected repository call")
        }
        fn claim_automation_run(
            &mut self,
            id: &str,
            worker_id: &str,
            lease_token: &str,
            now_ms: i64,
            lease_until_ms: i64,
        ) -> Result<bool, Self::Error> {
            self.claim = Some((
                id.into(),
                worker_id.into(),
                lease_token.into(),
                now_ms,
                lease_until_ms,
            ));
            if self.fail {
                Err("lease backend unavailable")
            } else {
                Ok(false)
            }
        }
        fn heartbeat_automation_run(
            &mut self,
            id: &str,
            lease_token: &str,
            now_ms: i64,
            lease_until_ms: i64,
        ) -> Result<bool, Self::Error> {
            unimplemented!("unexpected repository call")
        }
        fn retry_automation_run(
            &mut self,
            id: &str,
            lease_token: &str,
            next_retry_at_ms: i64,
            error_json: &str,
            updated_at_ms: i64,
        ) -> Result<bool, Self::Error> {
            unimplemented!("unexpected repository call")
        }
        fn automation_run(&self, id: &str) -> Result<Option<StoredAutomationRun>, Self::Error> {
            unimplemented!("unexpected repository call")
        }
        fn list_automation_runs(
            &self,
            automation_id: Option<&str>,
            limit: usize,
        ) -> Result<Vec<StoredAutomationRun>, Self::Error> {
            assert_eq!(automation_id, Some("a"));
            assert_eq!(limit, 7);
            Ok(vec![self.run.clone()])
        }
    }
    fn facts() -> Facts {
        Facts {
            claim: None,
            fail: false,
            run: StoredAutomationRun {
                definition_revision: 1,
                id: "r".into(),
                automation_id: "a".into(),
                occurrence_key: "occurrence".into(),
                trigger: "schedule".into(),
                status: "retry_wait".into(),
                attempt: 2,
                worker_id: None,
                lease_token: None,
                lease_until_ms: None,
                scheduled_for_ms: 10,
                started_at_ms: Some(10),
                finished_at_ms: None,
                next_retry_at_ms: Some(30),
                result_json: None,
                error_json: Some("{}".into()),
                created_at_ms: 1,
                updated_at_ms: 20,
            },
        }
    }
    #[test]
    fn non_sqlite_claim_keeps_lease_identity_and_repository_rejection() {
        let mut facts = facts();
        let command = || AutomationCommand::Run {
            command: AutomationRunCommand::Claim {
                id: "r".into(),
                worker_id: "worker".into(),
                lease_token: "unique-token".into(),
                now_ms: 30,
                lease_until_ms: 60,
            },
        };
        assert_eq!(
            execute_automation(&mut facts, command()).unwrap(),
            json!(false)
        );
        assert_eq!(
            facts.claim,
            Some(("r".into(), "worker".into(), "unique-token".into(), 30, 60))
        );
        facts.fail = true;
        assert!(execute_automation(&mut facts, command())
            .unwrap_err()
            .contains("lease backend unavailable"));
    }
    #[test]
    fn non_sqlite_run_projection_retains_retry_and_rejects_unknown_state() {
        let mut facts = facts();
        let command = || AutomationCommand::Run {
            command: AutomationRunCommand::List {
                automation_id: Some("a".into()),
                limit: 7,
            },
        };
        let projected = execute_automation(&mut facts, command()).unwrap();
        assert_eq!(projected[0]["status"], "retry_wait");
        assert_eq!(projected[0]["next_retry_at_ms"], 30);
        facts.run.status = "unknown".into();
        assert!(execute_automation(&mut facts, command()).is_err());
    }
}
