// SPDX-License-Identifier: Apache-2.0
//! Exact scheduled occurrence execution ownership. Process exit is not a result.
use serde::{Deserialize, Serialize};
use crate::assistant_run_terminal::AssistantRunTerminalSeal;
use sha2::{Digest, Sha256};

/// Definition kind is authority, not a discriminator inferred from UI metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AutomationDefinitionSpec {
    Agent { spec: AutomationExecutionSpec },
    UserTask { task: AutomationUserTaskSpec },
}
impl AutomationDefinitionSpec {
    pub fn agent(&self) -> Result<&AutomationExecutionSpec, String> {
        match self {
            Self::Agent { spec } => Ok(spec),
            Self::UserTask { .. } => Err("wrong-definition-kind: user_task is not Agent execution".into()),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Agent { spec } => spec.validate(),
            Self::UserTask { task } => task.validate(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationUserTaskSpec {
    pub task_type: AutomationUserTaskType,
    pub description: Option<String>,
    pub target_at_ms: Option<i64>,
    pub recurrence: Option<AutomationUserTaskRecurrence>,
    pub related_person: Option<String>,
    /// Base-ten decimal text; never a floating-point money conversion.
    pub amount_decimal: Option<String>,
    pub currency: Option<String>,
    pub due_at_ms: Option<i64>,
    pub completed: bool,
    pub notification_ids: Vec<String>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationUserTaskType { Reminder, Birthday, Debt, Appointment, Custom }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationUserTaskRecurrence {
    pub frequency: AutomationRecurrenceFrequency,
    pub interval: u32,
    pub days_of_week: Option<Vec<u8>>,
    pub days_of_month: Option<Vec<u8>>,
    pub months_of_year: Option<Vec<u8>>,
    pub ends_at_ms: Option<i64>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRecurrenceFrequency { Daily, Weekly, Monthly, Yearly }
impl AutomationUserTaskSpec {
    fn validate(&self) -> Result<(), String> {
        if let Some(value) = &self.amount_decimal {
            let unsigned = value.strip_prefix('-').unwrap_or(value);
            let parts: Vec<_> = unsigned.split('.').collect();
            if parts.len() > 2 || parts.iter().any(|part|part.is_empty() || !part.bytes().all(|b|b.is_ascii_digit())) {
                return Err("user task amount requires base-ten decimal text".into());
            }
        }
        if let Some(rule) = &self.recurrence {
            let invalid = |values: &Option<Vec<u8>>, max| values.as_ref().is_some_and(|values| values.is_empty() || values.iter().any(|v| *v == 0 || *v > max));
            if rule.interval == 0 || invalid(&rule.days_of_week, 7) || invalid(&rule.days_of_month, 31) || invalid(&rule.months_of_year, 12) {
                return Err("user task recurrence requires positive interval and valid calendar ranges".into());
            }
        }
        if self.notification_ids.iter().any(|id|id.trim().is_empty()) { return Err("user task notification identity is empty".into()); }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationExecutionSpec {
    pub prompt: String,
    pub execution_scope: AutomationExecutionScope,
    pub permission_profile: AutomationPermissionProfile,
    pub model_selection: AutomationModelSelection,
    pub context_refs: Vec<AutomationContextReference>,
    pub skill_refs: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AutomationExecutionScope {
    LocalWorkspace { root: String },
    DeviceLocal { host_id: String },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationPermissionProfile { Cautious, AllowAll }
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationModelSelection { UseCurrentDefaultAtAdmission }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationContextReference { pub kind: AutomationContextKind, pub id: String }
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationContextKind { PersonalData, WebSource }
impl AutomationExecutionSpec {
    pub fn validate(&self) -> Result<(), String> {
        let invalid_scope = match &self.execution_scope {
            AutomationExecutionScope::LocalWorkspace { root } => root.trim().is_empty(),
            AutomationExecutionScope::DeviceLocal { host_id } => host_id.trim().is_empty(),
        };
        if self.prompt.trim().is_empty() || invalid_scope
            || self.context_refs.iter().any(|reference|reference.id.trim().is_empty())
            || self.skill_refs.iter().any(|reference|reference.trim().is_empty()) {
            return Err("automation execution spec requires explicit prompt, execution scope and source identities".into());
        }
        let mut refs = std::collections::BTreeSet::new();
        for reference in &self.context_refs {
            let kind = match reference.kind { AutomationContextKind::PersonalData => "personal_data", AutomationContextKind::WebSource => "web_source" };
            if !refs.insert((kind, reference.id.as_str())) { return Err("duplicate automation context reference".into()); }
        }
        let mut skills = std::collections::BTreeSet::new();
        if self.skill_refs.iter().any(|id| !skills.insert(id)) { return Err("duplicate automation Skill reference".into()); }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutomationExecutionLease {
    pub occurrence_id: String,
    pub definition_revision: i64,
    pub worker_id: String,
    pub lease_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationExecutionIdentity {
    pub conversation_id: String,
    pub submission_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
}
impl AutomationExecutionIdentity {
    pub fn for_occurrence(occurrence_id: &str) -> Self {
        let key = format!("{:x}", Sha256::digest(occurrence_id.as_bytes()));
        Self { conversation_id: format!("automation-{key}"),
            submission_id: format!("automation-submission-{key}"),
            user_message_id: format!("automation-user-{key}"),
            assistant_message_id: format!("automation-assistant-{key}") }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationExecutionBinding {
    pub occurrence_id: String,
    pub definition_revision: i64,
    pub conversation_id: String,
    pub submission_id: String,
    pub run_id: String,
    pub invocation_config_digest: String,
    pub bound_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationExecutionObservation {
    pub phase: AutomationExecutionPhase,
    pub latest_worker_observation: Option<AutomationWorkerObservation>,
    pub identity: AutomationExecutionIdentity,
    pub binding: Option<AutomationExecutionBinding>,
    pub run_status: Option<String>,
    pub terminal: Option<AssistantRunTerminalSeal>,
    pub receipt: Option<AutomationExecutionReceipt>,
    pub notification_delivered: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationExecutionPhase { Queued, Claimed, Preparing, AdmissionPending, Admitted, Terminal, LaunchFailed }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationExecutionReceipt {
    pub binding: AutomationExecutionBinding,
    pub terminal: AssistantRunTerminalSeal,
    pub reconciled_at_ms: i64,
    pub notification_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AutomationWorkerObservation {
    Preparing,
    AdmissionRequested,
    Exited { exit_code: i32 },
    Disconnected { detail: String },
    LaunchFailed { detail: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn occurrence_identity_is_stable_safe_and_distinguishes_exact_source() {
        let identity = AutomationExecutionIdentity::for_occurrence("a/b:中文");
        assert_eq!(identity, AutomationExecutionIdentity::for_occurrence("a/b:中文"));
        assert_ne!(identity, AutomationExecutionIdentity::for_occurrence("a-b:中文"));
        for id in [&identity.conversation_id, &identity.submission_id, &identity.user_message_id, &identity.assistant_message_id] {
            assert!(id.bytes().all(|byte|byte.is_ascii_alphanumeric() || byte==b'-'));
        }
    }
    #[test]
    fn execution_spec_requires_explicit_policy_not_private_metadata_defaults() {
        assert!(serde_json::from_str::<AutomationExecutionSpec>(r#"{"prompt":"hello","workspace_root":"/work"}"#).is_err());
    }
    #[test]
    fn execution_scope_is_explicit_and_never_inferred_from_an_empty_path() {
        let mut value = serde_json::json!({
            "prompt":"Run once", "execution_scope":{"kind":"device_local","host_id":"ios-device"},
            "permission_profile":"cautious", "model_selection":"use_current_default_at_admission",
            "context_refs":[], "skill_refs":[]
        });
        let spec: AutomationExecutionSpec = serde_json::from_value(value.clone()).unwrap();
        spec.validate().unwrap();
        assert!(matches!(spec.execution_scope, AutomationExecutionScope::DeviceLocal { .. }));
        value["execution_scope"] = serde_json::json!({"kind":"local_workspace","root":""});
        assert!(serde_json::from_value::<AutomationExecutionSpec>(value.clone()).unwrap().validate().is_err());
        value.as_object_mut().unwrap().remove("execution_scope");
        value["workspace_root"] = serde_json::json!("/old-format");
        assert!(serde_json::from_value::<AutomationExecutionSpec>(value).is_err());
    }
}
