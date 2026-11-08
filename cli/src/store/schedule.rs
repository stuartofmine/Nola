use chrono::{DateTime, Utc};
use nola_application::{
    ApplicationCommand, AutomationCommand, AutomationDefinition, AutomationDefinitionCommand,
    AutomationRun, AutomationRunCommand, AutomationRunStatus,
};
use serde_json::Value;
use std::collections::HashSet;

use super::NolaStore;

const WEB_SCHEDULE_FORMATS: &[&str] = &["agent", "html", "search", "rss", "json", "icalendar", "file", "folder", "script", "reminder"];

impl NolaStore {
    pub fn find_web_automation_run(&self, automation_id: &str, occurrence_key: &str) -> Result<Option<AutomationRun>, String> {
        let mut application = self.canonical_automation_application()?;
        application.execute(ApplicationCommand::Automation { command: AutomationCommand::Run {
            command: AutomationRunCommand::Load { id: web_automation_occurrence_id(automation_id, occurrence_key) },
        } })
    }

    fn canonical_automation_application(
        &self,
    ) -> Result<crate::application_port::CliApplicationPort, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)
    }

    pub fn load_web_scheduled_actions(&self) -> Result<Vec<Value>, String> {
        let mut application = self.canonical_automation_application()?;
        let definitions: Vec<AutomationDefinition> =
            application.execute(ApplicationCommand::Automation {
                command: AutomationCommand::Definition {
                    command: AutomationDefinitionCommand::List,
                },
            })?;
        definitions
            .iter()
            .filter(|definition| WEB_SCHEDULE_FORMATS.contains(&definition.schedule_format.as_str()))
            .map(project_automation_definition)
            .collect()
    }

    pub fn replace_web_scheduled_actions(&mut self, actions: &[Value]) -> Result<(), String> {
        validate_unique_ids(actions, "id", "scheduled action")?;
        let now_ms = Utc::now().timestamp_millis();
        let definitions = actions
            .iter()
            .map(|action| automation_definition(action, now_ms))
            .collect::<Result<Vec<_>, _>>()?;
        let mut application = self.canonical_automation_application()?;
        let _: Value = application.execute(ApplicationCommand::Automation {
            command: AutomationCommand::Definition {
                command: AutomationDefinitionCommand::Reconcile {
                    inputs: definitions,
                    schedule_formats: WEB_SCHEDULE_FORMATS.iter().map(|value| value.to_string()).collect(),
                    reconciled_at_ms: now_ms,
                },
            },
        })?;
        Ok(())
    }

    /// Creates or updates one definition through the shared Application write
    /// boundary. The returned value is the committed product projection.
    pub fn upsert_web_scheduled_action(&mut self, action: &Value) -> Result<Value, String> {
        let persisted = self.commit_web_scheduled_action(action)?;
        project_automation_definition(&persisted)
    }

    fn commit_web_scheduled_action(&mut self, action: &Value) -> Result<AutomationDefinition, String> {
        validate_unique_ids(std::slice::from_ref(action), "id", "scheduled action")?;
        let definition = automation_definition(action, Utc::now().timestamp_millis())?;
        let mut application = self.canonical_automation_application()?;
        let persisted: AutomationDefinition =
            application.execute(ApplicationCommand::Automation {
                command: AutomationCommand::Definition {
                    command: AutomationDefinitionCommand::Upsert { input: definition },
                },
            })?;
        Ok(persisted)
    }

    /// Preserve the committed version even when subsequent verification fails.
    /// Read through a fresh Application connection, not the write's return value.
    pub(crate) fn upsert_web_scheduled_action_with_readback(
        &mut self,
        action: &Value,
    ) -> Result<(AutomationDefinition, Result<Value, String>), String> {
        let submitted = automation_definition(action, Utc::now().timestamp_millis())?;
        let committed = self.commit_web_scheduled_action(action)?;
        let verification = (|| {
            let mut application = self.canonical_automation_application()?;
            let observed: Option<AutomationDefinition> = application.execute(ApplicationCommand::Automation {
                command: AutomationCommand::Definition {
                    command: AutomationDefinitionCommand::Get { id: committed.id.clone() },
                },
            })?;
            if observed.as_ref() != Some(&committed) || committed.execution_spec != submitted.execution_spec {
                return Err("Committed automation does not match durable readback; it may have changed concurrently".into());
            }
            project_automation_definition(&committed)
        })();
        Ok((committed, verification))
    }

    pub fn update_web_scheduled_action_if_revision(&mut self, action: &Value, expected_revision: i64) -> Result<Option<Value>, String> {
        let definition = automation_definition(action, Utc::now().timestamp_millis())?;
        let mut application = self.canonical_automation_application()?;
        let persisted: Option<AutomationDefinition> = application.execute(ApplicationCommand::Automation {
            command: AutomationCommand::Definition { command: AutomationDefinitionCommand::UpdateIfRevision { input: definition, expected_revision } },
        })?;
        persisted.as_ref().map(project_automation_definition).transpose()
    }

    pub fn delete_web_scheduled_action(&mut self, id: &str) -> Result<(), String> {
        let mut application = self.canonical_automation_application()?;
        let _: Value = application.execute(ApplicationCommand::Automation {
            command: AutomationCommand::Definition {
                command: AutomationDefinitionCommand::Delete { id: id.to_string(), deleted_at_ms: chrono::Utc::now().timestamp_millis() },
            },
        })?;
        Ok(())
    }

    pub fn load_web_execution_results(&self) -> Result<Vec<Value>, String> {
        let mut application = self.canonical_automation_application()?;
        let runs: Vec<AutomationRun> = application.execute(ApplicationCommand::Automation {
            command: AutomationCommand::Run {
                command: AutomationRunCommand::List {
                    automation_id: None,
                    limit: 500,
                },
            },
        })?;
        let mut results = Vec::new();
        for run in runs {
            let observation = self.observe_web_automation_execution(&run.id)?;
            let definition: Option<AutomationDefinition> = application.execute(ApplicationCommand::Automation {
                command: AutomationCommand::Definition { command: AutomationDefinitionCommand::Get { id: run.automation_id.clone() } },
            })?;
            let timestamp = |ms| DateTime::<Utc>::from_timestamp_millis(ms)
                .map(|time| time.to_rfc3339()).ok_or_else(|| "invalid automation result timestamp".to_string());
            if observation.phase == nola_domain::automation_execution::AutomationExecutionPhase::LaunchFailed {
                let detail = match observation.latest_worker_observation {
                    Some(nola_domain::automation_execution::AutomationWorkerObservation::LaunchFailed { detail }) => detail,
                    _ => return Err("launch_failed occurrence has no durable failure fact".into()),
                };
                results.push(serde_json::json!({
                    "resultId":format!("automation-launch-failure:{}",run.id),"jobId":run.id,
                    "sourceId":run.automation_id,"sourceName":definition.as_ref().map(|value|value.name.as_str()).unwrap_or(""),
                    "sourceUrl":"","sourceFormat":definition.as_ref().map(|value|value.schedule_format.as_str()).unwrap_or(""),
                    "status":"failed","scheduledAt":timestamp(run.scheduled_for_ms)?,
                    "createdAt":timestamp(run.finished_at_ms.unwrap_or(run.updated_at_ms))?,
                    "fetchedAt":timestamp(run.finished_at_ms.unwrap_or(run.updated_at_ms))?,
                    "llmSummaryText":"","previewText":detail,"failureReason":detail,"events":[],"htmlSnapshot":null,
                    "ackCount":0,"expired":false,"conversationId":null,"runId":null
                }));
                continue;
            }
            let Some(receipt) = observation.receipt else { continue };
            let status = match &receipt.terminal.result.value {
                nola_domain::run_result::RunResultValue::Answered { .. } => "answered",
                nola_domain::run_result::RunResultValue::ActionCompleted { .. } => "action_completed",
                nola_domain::run_result::RunResultValue::DelegationAccepted { .. } => "delegation_accepted",
                nola_domain::run_result::RunResultValue::Incomplete { .. } => "incomplete",
                nola_domain::run_result::RunResultValue::Blocked { .. } => "blocked",
                nola_domain::run_result::RunResultValue::Cancelled { .. } => "cancelled",
                nola_domain::run_result::RunResultValue::Failed { .. } => "failed",
            };
            results.push(serde_json::json!({
                "resultId":receipt.terminal.result.result_id, "jobId":run.id,
                "sourceId":run.automation_id,
                "sourceName":definition.as_ref().map(|value|value.name.as_str()).unwrap_or(""),
                "sourceUrl":"", "sourceFormat":definition.as_ref().map(|value|value.schedule_format.as_str()).unwrap_or(""),
                "status":status,"scheduledAt":timestamp(run.scheduled_for_ms)?,
                "createdAt":timestamp(receipt.terminal.completed_at_ms)?,"fetchedAt":timestamp(receipt.terminal.completed_at_ms)?,
                "llmSummaryText":"","previewText":"","failureReason":null,"events":[],"htmlSnapshot":null,
                "ackCount":0,"expired":false,
                "conversationId":receipt.binding.conversation_id,"runId":receipt.binding.run_id,
                "executionReceipt":receipt
            }));
        }
        Ok(results)
    }

    pub fn enqueue_web_automation_run(
        &self,
        automation_id: &str,
        occurrence_key: &str,
        trigger: &str,
        scheduled_for_ms: i64,
        definition_revision: i64,
    ) -> Result<AutomationRun, String> {
        let mut application = self.canonical_automation_application()?;
        let run = AutomationRun {
            definition_revision,
            id: web_automation_occurrence_id(automation_id, occurrence_key),
            automation_id: automation_id.to_string(),
            occurrence_key: occurrence_key.to_string(),
            trigger: trigger.to_string(),
            status: AutomationRunStatus::Queued,
            attempt: 0,
            worker_id: None,
            lease_token: None,
            lease_until_ms: None,
            scheduled_for_ms,
            started_at_ms: None,
            finished_at_ms: None,
            next_retry_at_ms: None,
            result_json: None,
            error_json: None,
            created_at_ms: scheduled_for_ms,
            updated_at_ms: scheduled_for_ms,
        };
        application.execute(ApplicationCommand::Automation {
            command: AutomationCommand::Run {
                command: AutomationRunCommand::Enqueue { input: run },
            },
        })
    }

    pub fn claim_web_automation_run(
        &self,
        id: &str,
        worker_id: &str,
        lease_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<bool, String> {
        let mut application = self.canonical_automation_application()?;
        application.execute(ApplicationCommand::Automation {
            command: AutomationCommand::Run {
                command: AutomationRunCommand::Claim {
                    id: id.to_string(),
                    worker_id: worker_id.to_string(),
                    lease_token: lease_token.to_string(),
                    now_ms,
                    lease_until_ms,
                },
            },
        })
    }

    pub fn observe_web_automation_execution(&self, id: &str) -> Result<nola_domain::automation_execution::AutomationExecutionObservation, String> {
        let mut application = self.canonical_automation_application()?;
        application.execute(ApplicationCommand::ObserveAutomationExecution { occurrence_id: id.into() })
    }

    pub(crate) fn observe_web_automation_worker(&self,
        lease: nola_domain::automation_execution::AutomationExecutionLease,
        observation: nola_domain::automation_execution::AutomationWorkerObservation,
        observed_at_ms: i64,
    ) -> Result<bool, String> {
        let mut application = self.canonical_automation_application()?;
        application.execute(ApplicationCommand::ObserveAutomationWorker { lease, observation, observed_at_ms })
    }

    pub(crate) fn fence_expired_unadmitted_web_automation(&self, run: &AutomationRun, now_ms: i64) -> Result<(), String> {
        use nola_domain::automation_execution::*;
        if run.status != AutomationRunStatus::Running || !run.lease_until_ms.is_some_and(|until| until <= now_ms) {
            return Ok(());
        }
        if self.observe_web_automation_execution(&run.id)?.binding.is_some() { return Ok(()); }
        let lease = AutomationExecutionLease { occurrence_id: run.id.clone(), definition_revision: run.definition_revision,
            worker_id: run.worker_id.clone().ok_or("running automation has no worker")?,
            lease_token: run.lease_token.clone().ok_or("running automation has no lease")? };
        if !self.observe_web_automation_worker(lease, AutomationWorkerObservation::LaunchFailed {
            detail: "Expired unadmitted occurrence fenced during recovery. Original startup outcome was not recorded; no replacement execution was launched.".into()
        }, now_ms)? { return Err("automation recovery fence was not committed; observe the exact occurrence".into()); }
        Ok(())
    }

    pub fn reconcile_web_automation_execution(&self, id: &str, now_ms: i64) -> Result<Option<nola_domain::automation_execution::AutomationExecutionReceipt>, String> {
        let mut application = self.canonical_automation_application()?;
        application.execute(ApplicationCommand::ReconcileAutomationExecution {
            occurrence_id: id.into(), reconciled_at_ms: now_ms,
        })
    }
}

fn web_automation_occurrence_id(automation_id: &str, occurrence_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"nola.web.automation.occurrence.v1\0");
    for value in [automation_id, occurrence_key] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    format!("automation-run-{:x}", hash.finalize())
}

fn automation_definition(
    action: &Value,
    fallback_timestamp_ms: i64,
) -> Result<AutomationDefinition, String> {
    let id = required_string(action, "id", "scheduled action id")?;
    let name = required_string(action, "name", "scheduled action name")?;
    let schedule_format = required_string(action, "format", "scheduled action format")?;
    let created_at_ms = timestamp_ms(action, "createdAt", fallback_timestamp_ms)?;
    let updated_at_ms = timestamp_ms(action, "updatedAt", fallback_timestamp_ms)?;
    let execution_spec: nola_domain::automation_execution::AutomationDefinitionSpec = serde_json::from_value(
        action.get("executionSpec").cloned().ok_or("scheduled action requires executionSpec")?
    ).map_err(|error| format!("invalid automation executionSpec: {error}"))?;
    execution_spec.validate()?;
    Ok(AutomationDefinition {
        execution_spec,
        id,
        name,
        schedule_format,
        enabled: action
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        revision: 0,
        metadata_json: serde_json::to_string(action)
            .map_err(|error| format!("serialize scheduled action failed: {error}"))?,
        created_at_ms,
        updated_at_ms,
        deleted_at_ms: None,
    })
}

fn decode_metadata(id: &str, metadata_json: &str) -> Result<Value, String> {
    serde_json::from_str(metadata_json)
        .map_err(|error| format!("parse automation metadata {id} failed: {error}"))
}

fn project_automation_definition(definition: &AutomationDefinition) -> Result<Value, String> {
    let mut value = decode_metadata(&definition.id, &definition.metadata_json)?;
    let object = value.as_object_mut().ok_or_else(|| "automation metadata must be an object".to_string())?;
    object.insert("id".into(), Value::from(definition.id.clone()));
    object.insert("name".into(), Value::from(definition.name.clone()));
    object.insert("format".into(), Value::from(definition.schedule_format.clone()));
    object.insert("enabled".into(), Value::from(definition.enabled));
    object.insert("revision".into(), Value::from(definition.revision));
    object.insert("executionSpec".into(), serde_json::to_value(&definition.execution_spec).map_err(|error| error.to_string())?);
    Ok(value)
}

fn required_string(value: &Value, key: &str, label: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{label} must not be empty"))
}

fn timestamp_ms(value: &Value, key: &str, fallback_timestamp_ms: i64) -> Result<i64, String> {
    let Some(raw) = value.get(key).filter(|value| !value.is_null()) else {
        return Ok(fallback_timestamp_ms);
    };
    if let Some(milliseconds) = raw.as_i64() {
        return Ok(milliseconds);
    }
    let text = raw
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "automation timestamp must be RFC 3339 text or milliseconds".to_string())?;
    DateTime::parse_from_rfc3339(text)
        .map(|timestamp| timestamp.timestamp_millis())
        .map_err(|error| format!("parse automation timestamp {text} failed: {error}"))
}

fn validate_unique_ids(items: &[Value], key: &str, label: &str) -> Result<(), String> {
    let mut seen = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let id = item
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("{label} at index {index} requires a non-empty ID"))?;
        if !seen.insert(id.to_string()) {
            return Err(format!("duplicate {label} ID: {id}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    fn execution_spec() -> nola_domain::automation_execution::AutomationDefinitionSpec {
        serde_json::from_value(serde_json::json!({"kind":"agent","spec":{
            "prompt":"Fixture task", "execution_scope":{"kind":"local_workspace","root":"/tmp"},
            "permission_profile":"cautious","model_selection":"use_current_default_at_admission",
            "context_refs":[],"skill_refs":[]
        }})).unwrap()
    }

    fn action(mut value: serde_json::Value) -> serde_json::Value {
        value["executionSpec"] = serde_json::to_value(execution_spec()).unwrap();
        value
    }

    #[test]
    fn automation_projection_uses_canonical_identity_and_rejects_non_objects() {
        let mut definition = super::AutomationDefinition {
            execution_spec: execution_spec(),
            id:"canonical-a".into(),name:"Canonical name".into(),schedule_format:"script".into(),enabled:false,
            revision:7,metadata_json:r#"{"id":"wrong-b","name":"Wrong","format":"html","enabled":true,"revision":99}"#.into(),
            created_at_ms:1,updated_at_ms:2,deleted_at_ms:None,
        };
        let value = super::project_automation_definition(&definition).unwrap();
        assert_eq!(value, action(serde_json::json!({"id":"canonical-a","name":"Canonical name","format":"script","enabled":false,"revision":7})));
        for metadata in ["[]", "null", "42", "\"text\""] {
            definition.metadata_json = metadata.into();
            assert!(super::project_automation_definition(&definition).is_err());
        }
    }
    use super::{automation_definition, validate_unique_ids};
    use crate::store::NolaStore;
    use serde_json::json;
    use std::fs;

    #[test]
    fn web_replacement_preserves_native_tasks_in_shared_application_store() {
        use nola_application::{ApplicationCommand, AutomationCommand, AutomationDefinition, AutomationDefinitionCommand};
        let root = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(&root.path().join("test.sqlite")).unwrap();
        let mut application = store.canonical_automation_application().unwrap();
        let native = automation_definition(&action(json!({"id":"native","name":"Mac task","format":"nola.mac.automation.v1"})), 100).unwrap();
        let committed: AutomationDefinition = application.execute(ApplicationCommand::Automation {
            command: AutomationCommand::Definition { command: AutomationDefinitionCommand::Upsert { input: native } }
        }).unwrap();
        assert!(store.load_web_scheduled_actions().unwrap().is_empty());
        store.replace_web_scheduled_actions(&[action(json!({"id":"web","name":"Web task","format":"script"}))]).unwrap();
        assert_eq!(store.load_web_scheduled_actions().unwrap().len(), 1);
        store.replace_web_scheduled_actions(&[]).unwrap();
        let remaining: Vec<AutomationDefinition> = application.execute(ApplicationCommand::Automation {
            command: AutomationCommand::Definition { command: AutomationDefinitionCommand::List }
        }).unwrap();
        assert_eq!(remaining, vec![committed]);
    }

    #[test]
    fn replacement_payload_ids_must_be_present_and_unique() {
        assert!(validate_unique_ids(&[json!({})], "id", "scheduled action").is_err());
        assert!(validate_unique_ids(
            &[json!({"id":"same"}), json!({"id":"same"})],
            "id",
            "scheduled action"
        )
        .is_err());
        assert!(validate_unique_ids(
            &[json!({"resultId":"result-1"})],
            "resultId",
            "execution result"
        )
        .is_ok());
    }

    #[test]
    fn execution_authority_is_required_and_occurrence_lookup_is_exact() {
        let record = json!({"id":"a","name":"A","format":"agent"});
        assert!(automation_definition(&record, 1).unwrap_err().contains("executionSpec"));
        assert_ne!(super::web_automation_occurrence_id("a:b", "c"), super::web_automation_occurrence_id("a", "b:c"));
        let root = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(&root.path().join("isolated.sqlite")).unwrap();
        store.upsert_web_scheduled_action(&action(record)).unwrap();
        assert!(store.find_web_automation_run("a", "manual:one").unwrap().is_none());
        let first = store.enqueue_web_automation_run("a", "manual:one", "manual", 1, 1).unwrap();
        assert_eq!(store.find_web_automation_run("a", "manual:one").unwrap().unwrap().id, first.id);
        assert!(store.find_web_automation_run("a", "manual:two").unwrap().is_none());
        // Expiry does not invent a Run/receipt or a new occurrence identity.
        assert!(store.claim_web_automation_run(&first.id, "worker", "lease", 1, 2).unwrap());
        assert_eq!(store.enqueue_web_automation_run("a", "manual:one", "manual", 10, 1).unwrap().id, first.id);
        assert!(store.observe_web_automation_execution(&first.id).unwrap().binding.is_none());
        let claimed = store.find_web_automation_run("a", "manual:one").unwrap().unwrap();
        store.fence_expired_unadmitted_web_automation(&claimed, 11).unwrap();
        assert_eq!(store.observe_web_automation_execution(&first.id).unwrap().phase,
            nola_domain::automation_execution::AutomationExecutionPhase::LaunchFailed);
        assert!(!store.claim_web_automation_run(&first.id, "replacement", "replacement", 12, 20).unwrap());
        let result = store.load_web_execution_results().unwrap().pop().unwrap();
        assert_eq!(result["sourceId"], "a");
        assert_eq!(result["jobId"], first.id);
        assert_eq!(result["status"], "failed");
        assert!(result["runId"].is_null());
        assert!(result["conversationId"].is_null());
        assert!(result["failureReason"].as_str().unwrap().contains("Original startup outcome was not recorded"));
    }

    #[test]
    fn canonical_projection_rejects_incomplete_product_records() {
        assert!(automation_definition(&json!({"id":"a"}), 1).is_err());
    }

    #[test]
    fn web_automation_run_uses_canonical_occurrence_and_lease_across_connections() {
        let root = std::env::temp_dir().join(format!(
            "nola-cli-web-automation-run-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("nola.db");
        let mut store = NolaStore::open(&database).unwrap();
        store
            .upsert_web_scheduled_action(&action(json!({
                "id": "automation-web",
                "name": "Web fixture",
                "format": "file",
                "enabled": true,
                "createdAt": 1000,
                "updatedAt": 1000
            })))
            .unwrap();

        let first = store
            .enqueue_web_automation_run("automation-web", "manual:fixture", "manual", 1000, 1)
            .unwrap();
        let duplicate = store
            .enqueue_web_automation_run("automation-web", "manual:fixture", "manual", 1000, 1)
            .unwrap();
        assert_eq!(first.id, duplicate.id);
        assert!(store
            .claim_web_automation_run(&first.id, "worker-a", "lease-a", 1000, 61_000)
            .unwrap());
        assert!(!store
            .claim_web_automation_run(&first.id, "worker-b", "lease-b", 1001, 61_001)
            .unwrap());
        // A claimed worker alone cannot publish a successful execution. Only
        // an admitted canonical Run and terminal seal may reconcile it.
        assert!(store.reconcile_web_automation_execution(&first.id, 1002).unwrap().is_none());
        drop(store);

        let reopened = NolaStore::open(&database).unwrap();
        let results = reopened.load_web_execution_results().unwrap();
        assert!(results.is_empty());
        assert!(reopened.observe_web_automation_execution(&first.id).unwrap().receipt.is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
