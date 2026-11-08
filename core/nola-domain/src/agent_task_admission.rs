// SPDX-License-Identifier: Apache-2.0
//! Immutable evidence that Core admitted an agent task; never a worker lease.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const AGENT_TASK_ADMISSION_VERSION: u16 = 1;
pub const AGENT_TASK_ADMISSION_MAX_BYTES: usize = 32 * 1024;

/// One Store-authored guidance outcome, transported only by trusted Host IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskGuidanceProjection {
    pub admission: AgentTaskAdmission,
    pub authority_revision: u64,
    pub guidance: AgentTaskTerminalGuidance,
    pub settlement: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskGuidanceProjectionApplied {
    pub conversation_id: String,
    pub task_id: String,
    pub message_id: String,
    pub settlement: bool,
    pub core_revision: u64,
}

impl AgentTaskGuidanceProjection {
    pub fn validate(&self) -> Result<(), String> {
        self.admission.validate()?;
        let fact = &self.guidance;
        if self.authority_revision < 2 || fact.task_id != self.admission.task_id
            || fact.parent_session_id != self.admission.parent_core_session_id
            || fact.parent_turn_id != self.admission.parent_turn_id
            || fact.input_watermark < self.admission.input_watermark
            || fact.input_watermark.is_some_and(|watermark|watermark < 0)
            || fact.message_id.trim().is_empty() || fact.content.trim().is_empty()
            || fact.content.chars().count() > 8_000 || fact.submitted_at_ms < 0
            || !matches!(fact.status.as_str(), "accepted" | "delivered" | "rejected")
            || (self.settlement && fact.status == "accepted")
            || (!self.settlement && fact.status == "delivered")
            || (fact.status == "rejected" && fact.reason.is_none())
            || (fact.status != "rejected" && fact.reason.is_some())
        {
            return Err("guidance projection has inconsistent admission or message identity".into());
        }
        if serde_json::to_vec(self).map_err(|error| error.to_string())?.len() > 128 * 1024 {
            return Err("guidance projection exceeds bounded transfer limit".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskTerminalReconciliationApplied {
    pub conversation_id: String,
    pub task_id: String,
    pub receipt_id: String,
    pub core_revision: u64,
}

/// A bounded projection of one authoritative Task receipt, transferred only
/// over trusted Host IPC. This is not a bearer credential or worker claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskTerminalProjection {
    pub protocol_version: u16,
    pub admission: AgentTaskAdmission,
    pub authority_revision: u64,
    pub receipt_id: String,
    pub status: String,
    pub input_watermark: Option<i64>,
    pub completion_watermark: Option<i64>,
    pub result_ref: Option<String>,
    pub run_result: Option<crate::run_result::RunResult>,
    pub result_digest: Option<crate::assistant_journal::AssistantDigest>,
    pub ended_at_ms: i64,
    pub guidance_results: Vec<AgentTaskTerminalGuidance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskTerminalGuidance {
    pub task_id: String,
    pub message_id: String,
    pub parent_session_id: String,
    pub parent_turn_id: String,
    pub input_watermark: Option<i64>,
    pub content: String,
    pub submitted_at_ms: i64,
    pub status: String,
    pub reason: Option<String>,
}

impl AgentTaskTerminalProjection {
    pub fn validate(&self) -> Result<(), String> {
        self.admission.validate()?;
        if self.protocol_version != 1 || self.authority_revision < 2 || self.receipt_id.trim().is_empty()
            || !matches!(self.status.as_str(), "sealed" | "failed" | "cancelled")
            || self.input_watermark < self.admission.input_watermark
            || self.input_watermark.is_some_and(|value| value < 0)
            || self.completion_watermark != self.input_watermark || self.ended_at_ms < 0 {
            return Err("terminal Task projection has inconsistent admission or result identity".into());
        }
        match (&self.run_result, &self.result_digest) {
            (Some(result), Some(digest)) if self.status == "sealed" => {
                result.validate()?;
                if result.conversation_id != self.admission.child_conversation_id
                    || result.computed_digest()? != *digest {
                    return Err("terminal Task projection has a foreign or changed Run result".into());
                }
            }
            (None, None) if self.status != "sealed" => {}
            _ => return Err("sealed Task requires its exact Run result and digest".into()),
        }
        let mut ids = std::collections::BTreeSet::new();
        for guidance in &self.guidance_results {
            if guidance.task_id != self.admission.task_id || guidance.message_id.trim().is_empty()
                || guidance.parent_session_id != self.admission.parent_core_session_id
                || guidance.parent_turn_id != self.admission.parent_turn_id
                || guidance.input_watermark < self.admission.input_watermark
                || guidance.input_watermark > self.input_watermark
                || !matches!(guidance.status.as_str(), "delivered" | "rejected")
                || !ids.insert(&guidance.message_id) {
                return Err("terminal Task guidance is foreign, repeated, or unresolved".into());
            }
        }
        // 32 pending messages × 8,000 chars × worst-case JSON escaping, plus
        // immutable identity metadata, fits below the trusted IPC frame limit.
        if serde_json::to_vec(self).map_err(|error| error.to_string())?.len() > 2 * 1024 * 1024 {
            return Err("terminal Task projection exceeds bounded transfer limit".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAgentTaskAuthorityPhase {
    Prepared,
    Admitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAgentTaskAuthority {
    pub task_id: String,
    pub phase: RuntimeAgentTaskAuthorityPhase,
    pub authority_revision: u64,
    pub admission: Option<AgentTaskAdmission>,
}

impl RuntimeAgentTaskAuthority {
    pub fn validate(&self) -> Result<(), String> {
        if self.task_id.trim().is_empty() {
            return Err("agent Task authority requires a task identity".into());
        }
        match (&self.phase, self.authority_revision, &self.admission) {
            (RuntimeAgentTaskAuthorityPhase::Prepared, 1, None) => Ok(()),
            (RuntimeAgentTaskAuthorityPhase::Admitted, revision, Some(admission))
                if revision >= 2 && admission.task_id == self.task_id => admission.validate(),
            _ => Err("agent Task authority phase, revision, or identity is inconsistent".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskAdmission {
    pub protocol_version: u16,
    pub task_id: String,
    pub parent_conversation_id: String,
    pub parent_core_session_id: String,
    pub predecessor_task_id: Option<String>,
    pub parent_run_id: String,
    pub parent_turn_id: String,
    pub child_conversation_id: String,
    pub parent_tool_call_id: String,
    pub parent_effect_id: String,
    pub input_watermark: Option<i64>,
    pub source_store_epoch: String,
    pub source_transition_id: String,
    pub source_host_event_id: String,
    pub source_checkpoint_revision: i64,
    pub core_revision: u64,
    pub core_event_cursor: u64,
    pub admission_digest: String,
}

impl AgentTaskAdmission {
    fn validate_identity(&self) -> Result<(), String> {
        if self.protocol_version != AGENT_TASK_ADMISSION_VERSION {
            return Err("unsupported agent Task admission version".into());
        }
        if [
            &self.task_id,
            &self.parent_conversation_id,
            &self.parent_core_session_id,
            &self.parent_run_id,
            &self.parent_turn_id,
            &self.child_conversation_id,
            &self.parent_tool_call_id,
            &self.parent_effect_id,
            &self.source_store_epoch,
            &self.source_transition_id,
            &self.source_host_event_id,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
            || self.predecessor_task_id.as_ref().is_some_and(|id| id.trim().is_empty() || id == &self.task_id)
            || self.parent_conversation_id == self.child_conversation_id
            || self.source_checkpoint_revision < 1
            || self.core_revision == 0
            || self.core_event_cursor == 0
            || self.input_watermark.is_some_and(|value| value < 0)
        {
            return Err("agent Task admission identity is incomplete or inconsistent".into());
        }
        if serde_json::to_vec(self)
            .map_err(|error| error.to_string())?
            .len()
            > AGENT_TASK_ADMISSION_MAX_BYTES
        {
            return Err("agent Task admission exceeds bounded metadata limit".into());
        }
        Ok(())
    }

    pub fn canonical_digest(&self) -> Result<String, String> {
        self.validate_identity()?;
        let mut evidence = self.clone();
        evidence.admission_digest.clear();
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&evidence).map_err(|error| error.to_string())?)
        ))
    }

    pub fn seal(mut self) -> Result<Self, String> {
        self.admission_digest = self.canonical_digest()?;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.admission_digest != self.canonical_digest()? {
            return Err("agent Task admission digest differs from its immutable evidence".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_covers_every_admission_field_and_never_declares_worker_authority() {
        // A synthetic DTO tests integrity only, never grants Store execution.
        let fact = AgentTaskAdmission {
            protocol_version: AGENT_TASK_ADMISSION_VERSION,
            task_id: "task".into(),
            parent_conversation_id: "parent".into(),
            parent_core_session_id: "parent-core".into(),
            predecessor_task_id: None,
            parent_run_id: "run".into(),
            parent_turn_id: "turn".into(),
            child_conversation_id: "child".into(),
            parent_tool_call_id: "call".into(),
            parent_effect_id: "effect".into(),
            input_watermark: Some(7),
            source_store_epoch: "epoch".into(),
            source_transition_id: "transition".into(),
            source_host_event_id: "host-event".into(),
            source_checkpoint_revision: 1,
            core_revision: 1,
            core_event_cursor: 1,
            admission_digest: String::new(),
        }
        .seal()
        .unwrap();
        fact.validate().unwrap();
        // Pure DTO integrity coverage: execution authority still requires the
        // real Store exporter and original-Application trusted import.
        let mut terminal = AgentTaskTerminalProjection {
            protocol_version: 1, admission: fact.clone(), authority_revision: 3,
            receipt_id: "receipt".into(), status: "failed".into(),
            run_result: None, result_digest: None,
            input_watermark: Some(9), completion_watermark: Some(9),
            result_ref: None, ended_at_ms: 100, guidance_results: Vec::new(),
        };
        terminal.validate().unwrap();
        terminal.input_watermark = Some(6);
        terminal.completion_watermark = Some(6);
        assert!(terminal.validate().is_err(), "terminal cannot precede original input");
        terminal.input_watermark = Some(9);
        assert!(terminal.validate().is_err(), "completion must cover actual current input");
        terminal.completion_watermark = Some(9);
        terminal.guidance_results.push(AgentTaskTerminalGuidance {
            task_id: fact.task_id.clone(), message_id: "guidance".into(),
            parent_session_id: fact.parent_core_session_id.clone(),
            parent_turn_id: fact.parent_turn_id.clone(), input_watermark: Some(8),
            content: "actual earlier input".into(), submitted_at_ms: 50,
            status: "delivered".into(), reason: None,
        });
        terminal.validate().unwrap();
        terminal.guidance_results[0].input_watermark = Some(10);
        assert!(terminal.validate().is_err(), "guidance cannot exceed terminal input");
        let encoded = serde_json::to_value(&fact).unwrap();
        for (field, value) in encoded.as_object().unwrap() {
            if field == "admission_digest" {
                continue;
            }
            let mut changed = encoded.clone();
            changed[field] = if let Some(number) = value.as_u64() {
                serde_json::json!(number + 1)
            } else {
                serde_json::json!(format!("{}-foreign", value.as_str().unwrap_or("predecessor")))
            };
            assert!(
                serde_json::from_value::<AgentTaskAdmission>(changed)
                    .unwrap()
                    .validate()
                    .is_err(),
                "{field}"
            );
        }
        for field in ["lease_token", "worker_id", "lease_expires_at_ms"] {
            let mut changed = encoded.clone();
            changed[field] = serde_json::json!("unauthorized");
            assert!(serde_json::from_value::<AgentTaskAdmission>(changed).is_err());
        }
        for phase in [RuntimeAgentTaskAuthorityPhase::Prepared, RuntimeAgentTaskAuthorityPhase::Admitted] {
            for revision in [0, 1, 2, 3, u64::MAX] {
                for admission in [None, Some(fact.clone())] {
                    let expected = match phase {
                        RuntimeAgentTaskAuthorityPhase::Prepared => revision == 1 && admission.is_none(),
                        RuntimeAgentTaskAuthorityPhase::Admitted => revision >= 2 && admission.is_some(),
                    };
                    let authority = RuntimeAgentTaskAuthority {
                        task_id: fact.task_id.clone(), phase, authority_revision: revision, admission,
                    };
                    assert_eq!(authority.validate().is_ok(), expected, "{authority:?}");
                }
            }
        }
        let mut authority = RuntimeAgentTaskAuthority {
            task_id: "other-task".into(), phase: RuntimeAgentTaskAuthorityPhase::Admitted,
            authority_revision: 2, admission: Some(fact.clone()),
        };
        assert!(authority.validate().is_err());
        authority.task_id = fact.task_id.clone();
        authority.admission.as_mut().unwrap().parent_run_id = "changed".into();
        assert!(authority.validate().is_err());
        authority.phase = RuntimeAgentTaskAuthorityPhase::Prepared;
        authority.authority_revision = 1;
        authority.admission = None;
        authority.task_id.clear();
        assert!(authority.validate().is_err());

        let mut projection=AgentTaskGuidanceProjection {admission:fact.clone(),authority_revision:2,settlement:false,
            guidance:AgentTaskTerminalGuidance {task_id:fact.task_id.clone(),message_id:"guidance".into(),
                parent_session_id:fact.parent_core_session_id.clone(),parent_turn_id:fact.parent_turn_id.clone(),
                input_watermark:fact.input_watermark,content:"  保留空格  ".into(),submitted_at_ms:1,status:"accepted".into(),reason:None}};
        assert!(projection.validate().is_ok());
        projection.settlement=true;
        assert!(projection.validate().is_err());
        projection.guidance.status="delivered".into();
        assert!(projection.validate().is_ok());
        projection.guidance.parent_turn_id="foreign".into();
        assert!(projection.validate().is_err());
        projection.guidance.parent_turn_id=fact.parent_turn_id.clone();
        projection.guidance.content="水".repeat(8001);
        assert!(projection.validate().is_err());
    }
}

// A source-verified intent, never proof the child stopped. Authenticated Host
// transport is the issuer boundary; these fields alone are not bearer authority.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTaskCancellationAuthorization {
    pub admission: AgentTaskAdmission,
    pub source_host_id: String,
    pub control_attempt: crate::EffectDeliveryAttempt,
    pub deadline_ms: i64,
}
impl AgentTaskCancellationAuthorization {
    pub fn validate(&self) -> Result<(), String> {
        self.admission.validate()?;
        let attempt = &self.control_attempt;
        if self.source_host_id.trim().is_empty()
            || attempt.run_id != self.admission.parent_run_id
            || attempt.effect_id.trim().is_empty() || attempt.request_id.trim().is_empty()
            || attempt.id.trim().is_empty()
            || attempt.state != crate::EffectDeliveryAttemptState::ExecutionStarted
            || [&attempt.worker_generation, &attempt.lease_owner, &attempt.lease_token]
                .iter().any(|value| value.as_deref().is_none_or(|value|value.trim().is_empty()))
            || attempt.execution_started_at_ms.is_none_or(|at| at < 0 || at >= self.deadline_ms)
            || attempt.lease_expires_at_ms.is_none_or(|expiry| expiry < self.deadline_ms)
        { return Err("task cancellation authorization has inconsistent source fence".into()); }
        Ok(())
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum TaskCancellationExecution {
    Applied { cancel_requested: bool },
    Transfer { authorization: AgentTaskCancellationAuthorization },
}
