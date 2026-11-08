// SPDX-License-Identifier: Apache-2.0
//! Host observations, not task acceptance or permission authority.
use crate::assistant_journal_operations::JournalNumber;
use crate::host_effect::HostToolOutcome;
use serde::{Deserialize, Serialize};

/// Correlation only. Full delivery fencing stays in the committed Host inbox;
/// copying a source identity neither authenticates nor authorizes an effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionEvidenceSource {
    pub effect_id: String,
    pub request_id: String,
    pub host_id: String,
    pub attempt_id: Option<String>,
    pub response_sequence: JournalNumber,
    pub catalog_revision: JournalNumber,
    pub availability_epoch: JournalNumber,
}

/// Original Host facts before model-context output cropping or business-level
/// interpretation. Missing typed receipt remains missing, never 'verified'.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionEvidenceObservation {
    pub evidence_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub source: ExecutionEvidenceSource,
    pub receipt: Option<ExecutionReceipt>,
    pub outcome: HostToolOutcome,
}

impl ExecutionEvidenceObservation {
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("evidence_id", self.evidence_id.as_str()),
            ("call_id", self.call_id.as_str()),
            ("tool_name", self.tool_name.as_str()),
            ("effect_id", self.source.effect_id.as_str()),
            ("request_id", self.source.request_id.as_str()),
            ("host_id", self.source.host_id.as_str()),
        ] {
            identity(field, value)?;
        }
        if let Some(attempt) = &self.source.attempt_id {
            identity("attempt_id", attempt)?;
        }
        for value in [
            self.source.response_sequence,
            self.source.catalog_revision,
            self.source.availability_epoch,
        ] {
            if value.0 > i64::MAX as u64 {
                return Err("execution evidence source exceeds durable integer range".into());
            }
        }
        if let Some(receipt) = &self.receipt {
            receipt.validate()?;
        }
        Ok(())
    }
}

fn identity(field: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(format!("invalid execution evidence {field}"));
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReceipt {
    pub state: ExecutionState,
    /// Explicit null means that no owned local lifecycle stop was observed.
    /// This is independent of effects and never changes Unknown into success.
    #[serde(deserialize_with = "required_local_stop")]
    pub local_stop: Option<LocalExecutionStop>,
    #[serde(default)]
    pub effects: Vec<ResourceEffect>,
    #[serde(default)]
    pub checks: Vec<PostconditionCheck>,
}

fn required_local_stop<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<LocalExecutionStop>, D::Error> {
    Option::<LocalExecutionStop>::deserialize(deserializer)
}

/// Minted by the Host for one actual invocation, not model-controlled input.
/// Consumers must compare every field, including the fresh nonce, with their
/// current dispatch. Validation alone does not authenticate this identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalExecutionIdentity {
    pub execution_id: String,
    pub host_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub effect_id: String,
    pub request_id: String,
    pub call_id: String,
    #[serde(deserialize_with = "required_delivery_attempt")]
    pub delivery_attempt_id: Option<String>,
}

fn required_delivery_attempt<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalExecutionStop {
    pub identity: LocalExecutionIdentity,
    pub scope: LocalExecutionStopScope,
}

/// The scope of the actual owned OS object, not a claim about remote effects
/// or descendants that escaped a Unix execution group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalExecutionStopScope {
    OwnedProcessGroup { leader_pid: u32 },
    OwnedJob { leader_pid: u32 },
}

impl LocalExecutionStop {
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("execution_id", self.identity.execution_id.as_str()),
            ("host_id", self.identity.host_id.as_str()),
            ("session_id", self.identity.session_id.as_str()),
            ("turn_id", self.identity.turn_id.as_str()),
            ("effect_id", self.identity.effect_id.as_str()),
            ("request_id", self.identity.request_id.as_str()),
            ("call_id", self.identity.call_id.as_str()),
        ] {
            identity(field, value)?;
        }
        if let Some(attempt) = &self.identity.delivery_attempt_id {
            identity("delivery_attempt_id", attempt)?;
        }
        let leader_pid = match self.scope {
            LocalExecutionStopScope::OwnedProcessGroup { leader_pid }
            | LocalExecutionStopScope::OwnedJob { leader_pid } => leader_pid,
        };
        if leader_pid == 0 {
            return Err("local execution stop requires its original process identity".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    /// The adapter cannot establish whether effects occurred.
    #[default]
    Unknown,
    NotStarted,
    Completed,
    Deferred,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceEffect {
    pub resource_uri: String,
    pub operation: String,
    pub before_version: Option<String>,
    pub after_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostconditionCheck {
    pub kind: ExecutionCheckKind,
    pub resource_uri: String,
    pub version: String,
    pub predicate: String,
    pub passed: bool,
}

/// The Host's observation authority, never inferred from a predicate string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionCheckKind {
    /// A process exited successfully while the named resource stayed stable.
    /// This does not establish the resource's business correctness.
    ProcessExit,
    /// The Host evaluated the named predicate on this resource version.
    /// It is still not a claim that the overall user task is complete.
    Postcondition,
}

impl ExecutionReceipt {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(stop) = &self.local_stop {
            stop.validate()?;
            if matches!(self.state, ExecutionState::NotStarted | ExecutionState::Deferred) {
                return Err("an unexecuted/deferred receipt cannot attest local stop".into());
            }
        }
        if matches!(
            self.state,
            ExecutionState::NotStarted | ExecutionState::Deferred
        ) && (!self.effects.is_empty() || !self.checks.is_empty())
        {
            return Err(
                "An unexecuted/deferred operation cannot attest terminal effects or checks".into(),
            );
        }
        for effect in &self.effects {
            validate_uri(&effect.resource_uri)?;
            if effect.operation.trim().is_empty()
                || effect
                    .before_version
                    .as_ref()
                    .is_some_and(|v| v.trim().is_empty())
                || effect
                    .after_version
                    .as_ref()
                    .is_some_and(|v| v.trim().is_empty())
            {
                return Err("Effect operation and supplied versions must be nonempty".into());
            }
        }
        for check in &self.checks {
            validate_uri(&check.resource_uri)?;
            if check.version.trim().is_empty() || check.predicate.trim().is_empty() {
                return Err("Verification must name a concrete version and predicate".into());
            }
        }
        Ok(())
    }
}

fn validate_uri(uri: &str) -> Result<(), String> {
    crate::host_effect::HostArtifact {
        uri: uri.into(),
        name: "receipt".into(),
        mime_type: None,
        operation: None,
    }
    .validate()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn check_authority_is_required_and_never_inferred_from_predicate() {
        let mut wire = json!({"resource_uri":"file:///tmp/result", "version":"v1",
            "predicate":"all_business_requirements_correct", "passed":true});
        assert!(serde_json::from_value::<PostconditionCheck>(wire.clone()).is_err());
        wire["kind"] = json!("process_exit");
        let check: PostconditionCheck = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(check.kind, ExecutionCheckKind::ProcessExit);
        wire["kind"] = json!("postcondition");
        let check: PostconditionCheck = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(check.kind, ExecutionCheckKind::Postcondition);
        wire["kind"] = json!("task_complete");
        assert!(serde_json::from_value::<PostconditionCheck>(wire).is_err());
    }

    fn observed(receipt: Option<ExecutionReceipt>) -> ExecutionEvidenceObservation {
        ExecutionEvidenceObservation {
            evidence_id: "evidence".into(),
            call_id: "call".into(),
            tool_name: "tool".into(),
            source: ExecutionEvidenceSource {
                effect_id: "effect".into(),
                request_id: "request".into(),
                host_id: "host".into(),
                attempt_id: None,
                response_sequence: JournalNumber(1),
                catalog_revision: JournalNumber(1),
                availability_epoch: JournalNumber(0),
            },
            receipt,
            outcome: HostToolOutcome::Succeeded,
        }
    }

    #[test]
    fn missing_receipt_and_unknown_versions_are_not_rewritten_as_verified() {
        let missing = observed(None);
        missing.validate().unwrap();
        let wire = serde_json::to_value(&missing).unwrap();
        assert_eq!(wire["receipt"], json!(null));
        assert_eq!(
            serde_json::from_value::<ExecutionEvidenceObservation>(wire).unwrap(),
            missing
        );
        let partial = observed(Some(ExecutionReceipt {
            local_stop: None,
            state: ExecutionState::Partial,
            effects: vec![ResourceEffect {
                resource_uri: "file:///tmp/result".into(),
                operation: "write".into(),
                before_version: Some("old".into()),
                after_version: None,
            }],
            checks: vec![PostconditionCheck {
                kind: ExecutionCheckKind::Postcondition,
                resource_uri: "file:///tmp/result".into(),
                version: "old".into(),
                predicate: "valid".into(),
                passed: false,
            }],
        }));
        partial.validate().unwrap();
        assert_eq!(
            serde_json::from_value::<ExecutionEvidenceObservation>(
                serde_json::to_value(&partial).unwrap()
            )
            .unwrap(),
            partial
        );
    }

    #[test]
    fn source_requires_exact_identity_and_lossless_durable_counters() {
        let mut value = observed(None);
        value.source.response_sequence = JournalNumber(9_007_199_254_740_993);
        value.validate().unwrap();
        let mut wire = serde_json::to_value(&value).unwrap();
        assert_eq!(
            wire["source"]["response_sequence"],
            json!("9007199254740993")
        );
        wire["source"]["response_sequence"] = json!(9_007_199_254_740_993u64);
        assert!(serde_json::from_value::<ExecutionEvidenceObservation>(wire).is_err());
        value.source.response_sequence = JournalNumber(u64::MAX);
        assert!(value.validate().is_err());
        value.source.response_sequence = JournalNumber(1);
        value.source.attempt_id = Some(" ".into());
        assert!(value.validate().is_err());
        value.source.attempt_id = None;
        value.source.request_id = "foreign\nrequest".into();
        assert!(value.validate().is_err());
    }

    #[test]
    fn unexecuted_receipt_cannot_attest_effects_and_unknown_wire_fields_fail() {
        let mut value = observed(Some(ExecutionReceipt {
            local_stop: None,
            state: ExecutionState::NotStarted,
            effects: vec![ResourceEffect {
                resource_uri: "file:///tmp/result".into(),
                operation: "write".into(),
                before_version: None,
                after_version: Some("new".into()),
            }],
            checks: vec![],
        }));
        assert!(value.validate().is_err());
        value.receipt = None;
        let mut wire = serde_json::to_value(value).unwrap();
        wire["verified"] = json!(true);
        assert!(serde_json::from_value::<ExecutionEvidenceObservation>(wire).is_err());
    }

    #[test]
    fn local_stop_preserves_unknown_effects_and_requires_current_wire_fields() {
        let mut receipt = ExecutionReceipt {
            state: ExecutionState::Unknown,
            local_stop: Some(LocalExecutionStop {
                identity: LocalExecutionIdentity {
                    execution_id: "host-owned-invocation".into(), host_id: "host".into(),
                    session_id: "session".into(), turn_id: "turn".into(), effect_id: "effect".into(),
                    request_id: "request".into(), call_id: "call".into(), delivery_attempt_id: Some("attempt".into()),
                },
                scope: LocalExecutionStopScope::OwnedProcessGroup { leader_pid: 42 },
            }),
            ..Default::default()
        };
        receipt.validate().unwrap();
        assert_eq!(receipt.state, ExecutionState::Unknown);
        assert!(receipt.effects.is_empty() && receipt.checks.is_empty());
        let wire = serde_json::to_value(&receipt).unwrap();
        assert_eq!(serde_json::from_value::<ExecutionReceipt>(wire.clone()).unwrap(), receipt);
        let mut missing = wire.clone();
        missing.as_object_mut().unwrap().remove("local_stop");
        assert!(serde_json::from_value::<ExecutionReceipt>(missing).is_err());
        let mut missing_attempt = wire;
        missing_attempt["local_stop"]["identity"].as_object_mut().unwrap().remove("delivery_attempt_id");
        assert!(serde_json::from_value::<ExecutionReceipt>(missing_attempt).is_err());
        receipt.state = ExecutionState::NotStarted;
        assert!(receipt.validate().is_err());
        receipt.state = ExecutionState::Deferred;
        assert!(receipt.validate().is_err());
        receipt.state = ExecutionState::Unknown;
        receipt.local_stop.as_mut().unwrap().scope = LocalExecutionStopScope::OwnedProcessGroup { leader_pid: 0 };
        assert!(receipt.validate().is_err());
    }
}
