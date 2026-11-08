// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Durable, evidence-gated state for long-horizon goals.
// - Boundary: Evidence provenance and requirement assessment are distinct facts.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub mod completion;
mod creation;
pub use creation::{create_goal_command, goal_requirement_records};
mod work;
pub use work::{GoalUserUpdate, GoalWorkEntry, GoalWorkKind, GoalWorkOrigin, GoalWorkProgress, GoalWorkState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalRuntimeCommandEnvelope {
    pub operation_id: String,
    pub command: GoalRuntimeCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GoalRuntimeCommand {
    AcceptCompletion {
        expected_revision: u64,
        contract_id: String,
        claims: Vec<GoalCompletionClaim>,
        now_epoch_ms: i64,
    },
    UpdateWork {
        goal_id: String,
        expected_revision: u64,
        entries: Vec<GoalWorkEntry>,
        now_epoch_ms: i64,
    },
    Start {
        id: String,
        objective: String,
        #[serde(default)]
        constraints: Vec<String>,
        requirements: Vec<GoalRequirement>,
        now_epoch_ms: i64,
    },
    PlanContract {
        contract: GoalSubtaskContract,
        now_epoch_ms: i64,
    },
    BlockExecution { now_epoch_ms: i64 },
    GuardedControl {
        expected_goal_id: String,
        expected_control_token: String,
        control: Box<GoalRuntimeCommand>,
    },
    BeginExecution {
        contract_id: String,
        run_id: String,
        now_epoch_ms: i64,
    },
    RecordExecution {
        report: GoalExecutionReport,
    },
    RegisterArtifact {
        artifact: GoalArtifact,
    },
    RegisterFact {
        fact: GoalFact,
    },
    ApplyAudit {
        report: GoalAuditReport,
    },
    Complete {
        now_epoch_ms: i64,
    },
    ConcludeIncomplete {
        now_epoch_ms: i64,
    },
    Pause {
        status: GoalStatus,
        #[serde(default)]
        wake_at_epoch_ms: Option<i64>,
        now_epoch_ms: i64,
    },
    Resume {
        now_epoch_ms: i64,
    },
    Cancel {
        now_epoch_ms: i64,
    },
    Claim {
        owner: String,
        lease_until_epoch_ms: i64,
        now_epoch_ms: i64,
    },
    Heartbeat {
        owner: String,
        lease_until_epoch_ms: i64,
        now_epoch_ms: i64,
    },
    Release {
        owner: String,
        #[serde(default)]
        wake_at_epoch_ms: Option<i64>,
        now_epoch_ms: i64,
    },
    ScheduleResume {
        wake_at_epoch_ms: i64,
        now_epoch_ms: i64,
    },
    ResumeDue {
        now_epoch_ms: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Running,
    Paused,
    NeedsUser,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStopReason {
    Incomplete,
    Completed,
    Suspended,
    Scheduled,
    AwaitingUser,
    Failed,
    Cancelled,
}

impl GoalStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalRecordStatus {
    Pending,
    Completed,
    Blocked,
    Untrusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalRecordKind {
    Requirement,
    Artifact,
    Fact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalContractStatus {
    Planned,
    Executing,
    AwaitingAudit,
    Audited,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalAuditVerdict {
    Passed,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalRequirement {
    pub acceptance_policy: GoalAcceptancePolicy,
    pub id: String,
    pub semantic_revision: u64,
    pub description: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub status: GoalRecordStatus,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalCompletionClaim {
    pub execution_evidence_refs: Vec<String>,
    pub task_replacements: Vec<GoalTaskReplacement>,
    pub requirement_id: String,
    pub requirement_revision: u64,
    pub summary: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GoalAcceptancePolicy {
    ModelJudgment,
    ExecutionChecks { checks: Vec<GoalRequiredCheck> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalRequiredCheck {
    pub kind: nola_domain::execution_evidence::ExecutionCheckKind,
    pub resource_uri: String,
    pub version: String,
    pub predicate: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalTaskReplacement {
    pub task_id: String,
    pub terminal_receipt_id: Option<String>,
    pub result_digest: Option<nola_domain::assistant_journal::AssistantDigest>,
    pub execution_evidence_refs: Vec<String>,
}

/// A projection of a real Core observation, never a model-submitted receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalExecutionObservation {
    /// Goal ledger order, not a Core event cursor or physical completion time.
    pub recorded_goal_revision: u64,
    pub turn_id: String,
    pub source: nola_domain::execution_evidence::ExecutionEvidenceSource,
    pub state: Option<nola_domain::execution_evidence::ExecutionState>,
    pub checks: Vec<nola_domain::execution_evidence::PostconditionCheck>,
    pub outcome: nola_domain::host_effect::HostToolOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalArtifact {
    pub id: String,
    pub uri: String,
    pub description: String,
    pub status: GoalRecordStatus,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalFact {
    pub id: String,
    pub statement: String,
    pub status: GoalRecordStatus,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalEvidence {
    pub id: String,
    pub source: String,
    pub summary: String,
    pub observed_at_epoch_ms: i64,
    /// The cited source was observed. This does not establish that a user
    /// requirement is satisfied or that an independent reviewer inspected it.
    pub source_verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalAssessmentMethod {
    /// Semantic judgment over retained sources, not independent verification.
    ModelJudgment,
    /// An explicit inspection supplied by the audit producer. The report's
    /// evidence must identify the observation; this tag grants no authority.
    EvidenceInspection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalAuditFinding {
    pub requirement_revision: Option<u64>,
    pub assessment_method: GoalAssessmentMethod,
    pub record_kind: GoalRecordKind,
    pub record_id: String,
    pub status: GoalRecordStatus,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalAuditReport {
    pub assessor: GoalAssessor,
    pub id: String,
    /// Exact projection observed by the assessor. No missing-version fallback.
    pub expected_goal_revision: u64,
    pub contract_id: String,
    pub verdict: GoalAuditVerdict,
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<GoalAuditFinding>,
    #[serde(default)]
    pub unmet_requirement_ids: Vec<String>,
    #[serde(default)]
    pub integrity_violations: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<GoalEvidence>,
    pub audited_at_epoch_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GoalAssessor {
    ModelTurn { session_id: String, turn_id: String },
    RuntimeOperation { operation_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementAcceptance {
    pub id: String,
    pub audit_id: String,
    pub contract_id: String,
    pub requirement_id: String,
    pub requirement_revision: u64,
    pub method: GoalAssessmentMethod,
    pub assessor: GoalAssessor,
    pub judgment: GoalRecordStatus,
    pub evidence_refs: Vec<String>,
    pub rationale: String,
    pub assessed_at_epoch_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalSubtaskContract {
    /// Exact canonical Run assigned by BeginExecution, not an observed UI guess.
    pub execution_run_id: Option<String>,
    pub id: String,
    pub round: u64,
    pub objective: String,
    #[serde(default)]
    pub proposed_checks: Vec<String>,
    /// Exact requirement versions this plan executes. Planning notes cannot
    /// replace the authoritative requirements or user-origin work records.
    pub requirement_versions: BTreeMap<String, u64>,
    #[serde(default)]
    pub additional_execution_constraints: Vec<String>,
    #[serde(default)]
    pub relevant_record_ids: Vec<String>,
    pub status: GoalContractStatus,
    pub created_at_epoch_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalExecutionReport {
    pub contract_id: String,
    pub summary: String,
    #[serde(default)]
    pub claimed_record_ids: Vec<String>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    pub reported_at_epoch_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalExecutionBlocker { pub code: String, pub message: String }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalState {
    pub execution_blocker: Option<GoalExecutionBlocker>,
    pub execution_observations: BTreeMap<String, GoalExecutionObservation>,
    pub completion_claims: BTreeMap<String, Vec<GoalCompletionClaim>>,
    pub id: String,
    pub objective: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    pub work: GoalWorkState,
    pub status: GoalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<GoalStopReason>,
    pub created_at_epoch_ms: i64,
    pub updated_at_epoch_ms: i64,
    #[serde(default)]
    pub next_wake_at_epoch_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<GoalLease>,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default)]
    pub last_progress_at_epoch_ms: Option<i64>,
    #[serde(default)]
    pub requirements: BTreeMap<String, GoalRequirement>,
    #[serde(default)]
    pub artifacts: BTreeMap<String, GoalArtifact>,
    #[serde(default)]
    pub facts: BTreeMap<String, GoalFact>,
    #[serde(default)]
    pub contracts: Vec<GoalSubtaskContract>,
    #[serde(default)]
    pub execution_reports: Vec<GoalExecutionReport>,
    #[serde(default)]
    pub audit_reports: Vec<GoalAuditReport>,
    pub requirement_acceptances: Vec<RequirementAcceptance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalLease {
    pub owner: String,
    pub acquired_at_epoch_ms: i64,
    pub heartbeat_at_epoch_ms: i64,
    pub lease_until_epoch_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalProjection {
    /// Semantic control epoch. Lease heartbeats and evidence registration do
    /// not invalidate a user control; lifecycle and execution ownership do.
    pub control_generation: u64,
    /// Session-lifetime semantic command identity, distinct from transport receipts.
    pub applied_operations: BTreeMap<String, GoalAppliedOperation>,
    #[serde(default)]
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<GoalState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalAppliedOperation {
    pub rejection: Option<String>,
    pub command_sha256: String,
    pub goal_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalExecutionOwner {
    pub goal_id: String,
    pub contract_id: String,
    pub run_id: String,
    pub control_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalControlObservation {
    pub goal_id: String,
    pub control_token: String,
    pub status: GoalStatus,
    pub can_submit: bool,
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_cancel: bool,
}

impl GoalProjection {
    pub fn control_token(&self) -> Option<String> {
        use sha2::{Digest, Sha256};
        let goal = self.active.as_ref()?;
        let bytes = serde_json::to_vec(&(goal.id.as_str(), self.control_generation))
            .expect("Goal control identity serializes");
        Some(format!("{:x}", Sha256::digest(bytes)))
    }
    pub fn control_observation(&self) -> Option<GoalControlObservation> {
        let goal = self.active.as_ref()?;
        Some(GoalControlObservation { goal_id: goal.id.clone(), control_token: self.control_token()?,
            status: goal.status, can_submit: goal.status.is_terminal(),
            // A Goal can request user input while its canonical Run is still
            // draining or producing output. Pausing is legal in that state.
            can_pause: !goal.status.is_terminal() && goal.status != GoalStatus::Paused,
            can_resume: matches!(goal.status, GoalStatus::Paused | GoalStatus::NeedsUser | GoalStatus::Failed),
            can_cancel: !goal.status.is_terminal() })
    }
    pub fn execution_owner(&self, run_id: &str) -> Option<GoalExecutionOwner> {
        let goal = self.active.as_ref()?;
        let mut contracts = goal.contracts.iter().filter(|contract| contract.execution_run_id.as_deref() == Some(run_id));
        let contract = contracts.next()?;
        if contracts.next().is_some() { return None; }
        Some(GoalExecutionOwner { goal_id: goal.id.clone(), contract_id: contract.id.clone(),
            run_id: run_id.into(), control_token: self.control_token()? })
    }
    pub fn is_empty(&self) -> bool {
        self.active.is_none()
    }

    pub(crate) fn validate_operation_ledger(&self) -> Result<(), String> {
        if self.control_generation > self.revision || (self.active.is_some() && self.control_generation == 0) {
            return Err("Goal control generation is invalid".into());
        }
        for (id, operation) in &self.applied_operations {
            if id.is_empty() || id.trim() != id || id.len() > 512 || id.chars().any(char::is_control)
                || operation.goal_revision > self.revision || operation.command_sha256.len() != 64
                || !operation.command_sha256.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
                return Err("Goal operation ledger identity or revision is invalid".into());
            }
        }
        Ok(())
    }

    pub fn apply(&mut self, envelope: GoalRuntimeCommandEnvelope) -> Result<(), String> {
        if self.operation_replayed(&envelope)? { return Ok(()); }
        let mut next = self.clone();
        if let GoalRuntimeCommand::GuardedControl { expected_goal_id, expected_control_token, control } = &envelope.command {
            if !matches!(control.as_ref(), GoalRuntimeCommand::Pause { .. } | GoalRuntimeCommand::Resume { .. }
                | GoalRuntimeCommand::Cancel { .. } | GoalRuntimeCommand::ScheduleResume { .. } | GoalRuntimeCommand::BlockExecution { .. }) {
                return Err("Guarded Goal control only permits pause, resume, cancel or schedule".into());
            }
            if self.active.as_ref().map(|goal| goal.id.as_str()) != Some(expected_goal_id.as_str()) || self.control_token().as_deref() != Some(expected_control_token.as_str()) {
                // A stale control is a committed rejection, not a poison FIFO
                // command that can block every later user instruction.
                next.revision = next.revision.saturating_add(1);
                next.applied_operations.insert(envelope.operation_id.clone(), GoalAppliedOperation {
                    command_sha256: goal_command_digest(&envelope.command)?, goal_revision: next.revision,
                    rejection: Some("Goal identity or control state changed; refresh before controlling it".into()),
                });
                *self = next;
                return Ok(());
            }
        }
        if let Err(error) = next.apply_unrecorded(envelope.clone()) {
            if !matches!(&envelope.command, GoalRuntimeCommand::GuardedControl { .. }) {
                return Err(error);
            }
            // A validly addressed user control can become inadmissible while
            // waiting in the FIFO. Persist its semantic rejection so it cannot
            // poison later commands. Discard every tentative state mutation.
            let mut rejected = self.clone();
            rejected.revision = rejected.revision.saturating_add(1);
            rejected.applied_operations.insert(envelope.operation_id.clone(), GoalAppliedOperation {
                command_sha256: goal_command_digest(&envelope.command)?,
                goal_revision: rejected.revision, rejection: Some(error),
            });
            *self = rejected;
            return Ok(());
        }
        next.record_operation(&envelope)?;
        *self = next;
        Ok(())
    }

    pub(crate) fn operation_replayed(&self, envelope: &GoalRuntimeCommandEnvelope) -> Result<bool, String> {
        let id = &envelope.operation_id;
        if id.is_empty() || id.trim() != id || id.len() > 512 || id.chars().any(char::is_control) {
            return Err("Invalid Goal operation identity".into());
        }
        let Some(previous) = self.applied_operations.get(id) else { return Ok(false); };
        if previous.command_sha256 != goal_command_digest(&envelope.command)? {
            return Err("Goal operation identity was already committed with a different command".into());
        }
        Ok(true)
    }

    pub(crate) fn record_operation(&mut self, envelope: &GoalRuntimeCommandEnvelope) -> Result<(), String> {
        if self.operation_replayed(envelope)? { return Ok(()); }
        self.applied_operations.insert(envelope.operation_id.clone(), GoalAppliedOperation {
            command_sha256: goal_command_digest(&envelope.command)?, goal_revision: self.revision, rejection: None,
        });
        Ok(())
    }

    fn apply_unrecorded(&mut self, envelope: GoalRuntimeCommandEnvelope) -> Result<(), String> {
        match envelope.command {
            GoalRuntimeCommand::BlockExecution { now_epoch_ms } => {
                self.pause(GoalStatus::NeedsUser, None, now_epoch_ms)?;
                self.active.as_mut().expect("paused Goal").execution_blocker = Some(GoalExecutionBlocker {
                    code: "configuration_unavailable".into(),
                    message: "Goal execution configuration is unavailable. Review this conversation's model and tools, then resume.".into(),
                });
                Ok(())
            }
            GoalRuntimeCommand::GuardedControl { control, .. } => self.apply_unrecorded(GoalRuntimeCommandEnvelope {
                operation_id: envelope.operation_id, command: *control,
            }),
            GoalRuntimeCommand::AcceptCompletion { .. } => Err("Goal acceptance requires canonical Core evidence".into()),
            GoalRuntimeCommand::ConcludeIncomplete { now_epoch_ms } => {
                self.pause(GoalStatus::Paused, None, now_epoch_ms)?;
                self.active.as_mut().expect("paused Goal").stop_reason = Some(GoalStopReason::Incomplete);
                Ok(())
            }
            GoalRuntimeCommand::UpdateWork { goal_id, expected_revision, entries, now_epoch_ms } =>
                self.update_work(&goal_id, expected_revision, entries, now_epoch_ms),
            GoalRuntimeCommand::Start {
                id,
                objective,
                constraints,
                requirements,
                now_epoch_ms,
            } => self.start(id, objective, constraints, requirements, now_epoch_ms),
            GoalRuntimeCommand::PlanContract {
                contract,
                now_epoch_ms,
            } => self.plan_contract(contract, now_epoch_ms),
            GoalRuntimeCommand::BeginExecution {
                contract_id, run_id, now_epoch_ms,
            } => self.begin_execution(&contract_id, &run_id, now_epoch_ms),
            GoalRuntimeCommand::RecordExecution { report } => self.record_execution_report(report),
            GoalRuntimeCommand::RegisterArtifact { artifact } => self.register_artifact(artifact),
            GoalRuntimeCommand::RegisterFact { fact } => self.register_fact(fact),
            GoalRuntimeCommand::ApplyAudit { report } => self.apply_audit(report),
            GoalRuntimeCommand::Complete { now_epoch_ms } => self.complete(now_epoch_ms),
            GoalRuntimeCommand::Pause {
                status,
                wake_at_epoch_ms,
                now_epoch_ms,
            } => self.pause(status, wake_at_epoch_ms, now_epoch_ms),
            GoalRuntimeCommand::Resume { now_epoch_ms } => self.resume(now_epoch_ms),
            GoalRuntimeCommand::Cancel { now_epoch_ms } => self.cancel(now_epoch_ms),
            GoalRuntimeCommand::Claim {
                owner,
                lease_until_epoch_ms,
                now_epoch_ms,
            } => self.claim(owner, lease_until_epoch_ms, now_epoch_ms),
            GoalRuntimeCommand::Heartbeat {
                owner,
                lease_until_epoch_ms,
                now_epoch_ms,
            } => self.heartbeat(&owner, lease_until_epoch_ms, now_epoch_ms),
            GoalRuntimeCommand::Release {
                owner,
                wake_at_epoch_ms,
                now_epoch_ms,
            } => self.release(&owner, wake_at_epoch_ms, now_epoch_ms),
            GoalRuntimeCommand::ScheduleResume {
                wake_at_epoch_ms,
                now_epoch_ms,
            } => self.schedule_resume(wake_at_epoch_ms, now_epoch_ms),
            GoalRuntimeCommand::ResumeDue { now_epoch_ms } => {
                self.resume_due(now_epoch_ms).map(|_| ())
            }
        }
    }

    pub fn start(
        &mut self,
        id: String,
        objective: String,
        constraints: Vec<String>,
        requirements: Vec<GoalRequirement>,
        now_epoch_ms: i64,
    ) -> Result<(), String> {
        if self
            .active
            .as_ref()
            .is_some_and(|goal| !goal.status.is_terminal())
        {
            return Err("An unfinished Goal is already active".to_string());
        }
        validate_nonempty("Goal id", &id)?;
        validate_nonempty("Goal objective", &objective)?;
        if requirements.is_empty() {
            return Err("Goal must define at least one verifiable requirement".to_string());
        }
        let mut indexed = BTreeMap::new();
        for mut requirement in requirements {
            validate_nonempty("Goal requirement id", &requirement.id)?;
            validate_nonempty("Goal requirement description", &requirement.description)?;
            if let GoalAcceptancePolicy::ExecutionChecks { checks } = &requirement.acceptance_policy {
                if checks.is_empty() || checks.len() > 32 { return Err("Goal requires 1..32 explicit checks".into()); }
                let mut identities = BTreeSet::new();
                for check in checks {
                    for value in [&check.resource_uri, &check.version, &check.predicate] {
                        if value.trim().is_empty() || value.len() > 4096 { return Err("Invalid Goal check identity".into()); }
                    }
                    if !identities.insert((&check.resource_uri, &check.version, &check.predicate)) {
                        return Err("Duplicate Goal check identity".into());
                    }
                }
            }
            requirement.status = GoalRecordStatus::Pending;
            requirement.semantic_revision = 1;
            requirement.evidence_refs.clear();
            if indexed
                .insert(requirement.id.clone(), requirement)
                .is_some()
            {
                return Err("Goal requirement ids must be unique".to_string());
            }
        }
        for requirement in indexed.values() {
            if requirement
                .dependencies
                .iter()
                .any(|dependency| !indexed.contains_key(dependency))
            {
                return Err("Goal requirement dependency is not registered".to_string());
            }
            if requirement
                .dependencies
                .iter()
                .any(|dependency| dependency == &requirement.id)
            {
                return Err("Goal requirement cannot depend on itself".to_string());
            }
        }
        self.revision = self.revision.saturating_add(1);
        self.control_generation = self.control_generation.saturating_add(1);
        self.active = Some(GoalState {
            execution_blocker: None,
            execution_observations: BTreeMap::new(),
            completion_claims: BTreeMap::new(),
            id,
            objective,
            constraints,
            work: GoalWorkState::default(),
            status: GoalStatus::Running,
            stop_reason: None,
            created_at_epoch_ms: now_epoch_ms,
            updated_at_epoch_ms: now_epoch_ms,
            next_wake_at_epoch_ms: None,
            lease: None,
            attempt_count: 0,
            last_progress_at_epoch_ms: Some(now_epoch_ms),
            requirements: indexed,
            artifacts: BTreeMap::new(),
            facts: BTreeMap::new(),
            contracts: Vec::new(),
            execution_reports: Vec::new(),
            audit_reports: Vec::new(),
            requirement_acceptances: Vec::new(),
        });
        Ok(())
    }

    pub fn plan_contract(
        &mut self,
        mut contract: GoalSubtaskContract,
        now_epoch_ms: i64,
    ) -> Result<(), String> {
        let goal = self.running_goal_mut()?;
        validate_nonempty("Goal contract id", &contract.id)?;
        if contract.execution_run_id.is_some() { return Err("A planned Goal contract cannot pre-bind an execution Run".into()); }
        validate_nonempty("Goal contract objective", &contract.objective)?;
        if contract.proposed_checks.is_empty() {
            return Err("Goal contract must define proposed checks".to_string());
        }
        if goal
            .contracts
            .last()
            .is_some_and(|item| !matches!(item.status, GoalContractStatus::Audited | GoalContractStatus::Interrupted))
        {
            return Err(
                "Previous Goal contract must be audited before planning another".to_string(),
            );
        }
        let expected_round = goal.contracts.len() as u64 + 1;
        if contract.round != expected_round {
            return Err(format!("Goal contract round must be {expected_round}"));
        }
        if contract
            .relevant_record_ids
            .iter()
            .any(|id| !record_exists(goal, id))
        {
            return Err("Goal contract references an unknown record".to_string());
        }
        if goal.contracts.iter().any(|previous| previous.id == contract.id) {
            return Err("Goal contract id is already registered".into());
        }
        validate_contract_requirements(goal, &contract)?;
        contract.status = GoalContractStatus::Planned;
        goal.contracts.push(contract);
        goal.updated_at_epoch_ms = now_epoch_ms;
        goal.last_progress_at_epoch_ms = Some(now_epoch_ms);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn begin_execution(&mut self, contract_id: &str, run_id: &str, now_epoch_ms: i64) -> Result<(), String> {
        validate_nonempty("Goal execution Run id", run_id)?;
        let goal = self.active.as_ref().ok_or("No active Goal")?;
        if goal.contracts.iter().any(|contract| contract.execution_run_id.as_deref() == Some(run_id)) {
            return Err("Goal execution Run is already bound to a contract".into());
        }
        let contract = goal.contracts.last().ok_or("Goal has no active contract")?;
        validate_contract_requirements(goal, contract)?;
        self.transition_contract(contract_id, GoalContractStatus::Planned, GoalContractStatus::Executing, now_epoch_ms)?;
        self.active.as_mut().expect("active Goal validated").contracts.last_mut()
            .expect("contract validated").execution_run_id = Some(run_id.into());
        Ok(())
    }

    pub fn record_execution_report(&mut self, report: GoalExecutionReport) -> Result<(), String> {
        let goal = self.running_goal_mut()?;
        let contract = goal
            .contracts
            .last_mut()
            .ok_or("Goal has no active contract")?;
        if contract.id != report.contract_id || contract.status != GoalContractStatus::Executing {
            return Err("Execution report does not match the executing Goal contract".to_string());
        }
        validate_nonempty("Goal execution report summary", &report.summary)?;
        contract.status = GoalContractStatus::AwaitingAudit;
        goal.updated_at_epoch_ms = report.reported_at_epoch_ms;
        goal.last_progress_at_epoch_ms = Some(report.reported_at_epoch_ms);
        goal.execution_reports.push(report);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn register_artifact(&mut self, mut artifact: GoalArtifact) -> Result<(), String> {
        let goal = self.running_goal_mut()?;
        validate_nonempty("Goal artifact id", &artifact.id)?;
        validate_nonempty("Goal artifact URI", &artifact.uri)?;
        artifact.status = GoalRecordStatus::Untrusted;
        artifact.evidence_refs.clear();
        if goal.artifacts.contains_key(&artifact.id) {
            return Err("Goal artifact id is already registered".to_string());
        }
        goal.artifacts.insert(artifact.id.clone(), artifact);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn register_fact(&mut self, mut fact: GoalFact) -> Result<(), String> {
        let goal = self.running_goal_mut()?;
        validate_nonempty("Goal fact id", &fact.id)?;
        validate_nonempty("Goal fact statement", &fact.statement)?;
        fact.status = GoalRecordStatus::Untrusted;
        fact.evidence_refs.clear();
        if goal.facts.contains_key(&fact.id) {
            return Err("Goal fact id is already registered".to_string());
        }
        goal.facts.insert(fact.id.clone(), fact);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn apply_audit(&mut self, report: GoalAuditReport) -> Result<(), String> {
        if report.expected_goal_revision != self.revision {
            return Err("Goal changed after the audit snapshot; reassess the current revision".into());
        }
        let goal = self.running_goal_mut()?;
        let contract_index = goal
            .contracts
            .len()
            .checked_sub(1)
            .ok_or("Goal has no contract to audit")?;
        let unexecuted_plan = goal.contracts[contract_index].status == GoalContractStatus::Planned
            && report.verdict == GoalAuditVerdict::Partial
            && report.findings.is_empty() && report.evidence.is_empty()
            && matches!(report.assessor, GoalAssessor::RuntimeOperation { .. });
        if goal.contracts[contract_index].id != report.contract_id
            || (goal.contracts[contract_index].status != GoalContractStatus::AwaitingAudit && !unexecuted_plan)
        {
            return Err("Audit report does not match the Goal contract awaiting audit".to_string());
        }
        validate_nonempty("Goal audit id", &report.id)?;
        validate_nonempty("Goal audit summary", &report.summary)?;
        match &report.assessor {
            GoalAssessor::ModelTurn { session_id, turn_id } => {
                validate_nonempty("Goal assessor session", session_id)?;
                validate_nonempty("Goal assessor Turn", turn_id)?;
            }
            GoalAssessor::RuntimeOperation { operation_id } => validate_nonempty("Goal assessor operation", operation_id)?,
        }
        if goal.audit_reports.iter().any(|audit| audit.id == report.id) {
            return Err("Goal audit id is already registered".to_string());
        }
        if report.verdict == GoalAuditVerdict::Passed
            && (!report.unmet_requirement_ids.is_empty() || !report.integrity_violations.is_empty())
        {
            return Err(
                "A passed Goal audit cannot report unmet requirements or integrity violations"
                    .to_string(),
            );
        }
        let evidence = report
            .evidence
            .iter()
            .map(|item| (item.id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        if evidence.len() != report.evidence.len() {
            return Err("Goal audit evidence ids must be unique".to_string());
        }
        for item in &report.evidence {
            validate_nonempty("Goal evidence id", &item.id)?;
            validate_nonempty("Goal evidence source", &item.source)?;
            validate_nonempty("Goal evidence summary", &item.summary)?;
            if item.observed_at_epoch_ms > report.audited_at_epoch_ms {
                return Err("Goal evidence cannot be observed after its audit".to_string());
            }
        }
        let finding_keys = report
            .findings
            .iter()
            .map(|finding| (finding.record_kind, finding.record_id.as_str()))
            .collect::<BTreeSet<_>>();
        if finding_keys.len() != report.findings.len() {
            return Err("Goal audit findings must identify unique records".to_string());
        }
        let unmet = report
            .unmet_requirement_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if unmet.len() != report.unmet_requirement_ids.len()
            || unmet.iter().any(|id| !goal.requirements.contains_key(*id))
        {
            return Err(
                "Goal audit unmet requirements must be unique registered requirements".to_string(),
            );
        }
        for finding in &report.findings {
            validate_nonempty("Goal audit finding summary", &finding.summary)?;
            validate_finding(goal, finding, &evidence)?;
            let contract = &goal.contracts[contract_index];
            if !contract.relevant_record_ids.contains(&finding.record_id) {
                return Err("Goal finding lies outside its execution contract".into());
            }
            if finding.record_kind == GoalRecordKind::Requirement
                && contract.requirement_versions.get(&finding.record_id).copied() != finding.requirement_revision {
                return Err("Goal execution contract no longer covers the assessed requirement version".into());
            }
        }
        if report.verdict == GoalAuditVerdict::Passed {
            validate_contract_requirements(goal, &goal.contracts[contract_index])?;
        }
        // Validate the complete proposed state, including findings that revoke
        // an old acceptance. Old Completed must not override a new Pending.
        let mut projected_status = goal.requirements.iter()
            .map(|(id, requirement)| (id.as_str(), requirement.status))
            .collect::<BTreeMap<_, _>>();
        for finding in &report.findings {
            if finding.record_kind == GoalRecordKind::Requirement {
                projected_status.insert(finding.record_id.as_str(), finding.status);
            }
        }
        for requirement in goal.requirements.values().filter(|requirement|
            projected_status[requirement.id.as_str()] == GoalRecordStatus::Completed)
        {
            if requirement.dependencies.iter().any(|dependency| {
                projected_status[dependency.as_str()] != GoalRecordStatus::Completed
            }) {
                return Err(
                    "Goal requirement cannot complete before all dependencies are verified"
                        .to_string(),
                );
            }
        }
        let projected_unmet = goal.contracts[contract_index]
            .relevant_record_ids
            .iter()
            .filter(|id| goal.requirements.contains_key(id.as_str()))
            .filter(|id| {
                projected_status[id.as_str()] != GoalRecordStatus::Completed
            })
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if projected_unmet != unmet {
            return Err(
                "Goal audit unmet requirements do not match the audited contract state".to_string(),
            );
        }
        for finding in &report.findings {
            apply_finding(goal, finding);
            if finding.record_kind == GoalRecordKind::Requirement {
                goal.requirement_acceptances.push(RequirementAcceptance {
                    id: serde_json::to_string(&(&report.id, &finding.record_id)).expect("string tuple"),
                    audit_id: report.id.clone(), requirement_id: finding.record_id.clone(),
                    contract_id: report.contract_id.clone(),
                    requirement_revision: finding.requirement_revision.expect("validated requirement revision"),
                    method: finding.assessment_method, assessor: report.assessor.clone(),
                    judgment: finding.status, evidence_refs: finding.evidence_refs.clone(),
                    rationale: finding.summary.clone(), assessed_at_epoch_ms: report.audited_at_epoch_ms,
                });
            }
        }
        goal.contracts[contract_index].status = GoalContractStatus::Audited;
        goal.updated_at_epoch_ms = report.audited_at_epoch_ms;
        goal.last_progress_at_epoch_ms = Some(report.audited_at_epoch_ms);
        goal.audit_reports.push(report);
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn complete(&mut self, now_epoch_ms: i64) -> Result<(), String> {
        let current_revision = self.revision;
        let goal = self.running_goal_mut()?;
        if goal.contracts.last().is_none_or(|contract| contract.status != GoalContractStatus::Audited) {
            return Err("Goal has an unsettled execution contract".into());
        }
        let latest = goal
            .audit_reports
            .last()
            .ok_or("Goal completion requires an audit report")?;
        if latest.expected_goal_revision.checked_add(1) != Some(current_revision) {
            return Err("Goal changed after acceptance; completion requires a current assessment".into());
        }
        if latest.verdict != GoalAuditVerdict::Passed
            || !latest.unmet_requirement_ids.is_empty()
            || !latest.integrity_violations.is_empty()
        {
            return Err("Latest Goal audit does not certify completion".to_string());
        }
        if goal
            .requirements
            .values()
            .any(|record| record.status != GoalRecordStatus::Completed ||
                goal.requirement_acceptances.iter().rev().find(|acceptance| acceptance.requirement_id == record.id)
                    .is_none_or(|acceptance| acceptance.requirement_revision != record.semantic_revision
                        || acceptance.judgment != GoalRecordStatus::Completed))
        {
            return Err("Goal still has requirements without verified completion".to_string());
        }
        goal.status = GoalStatus::Completed;
        goal.stop_reason = Some(GoalStopReason::Completed);
        goal.updated_at_epoch_ms = now_epoch_ms;
        goal.next_wake_at_epoch_ms = None;
        goal.lease = None;
        goal.last_progress_at_epoch_ms = Some(now_epoch_ms);
        self.revision = self.revision.saturating_add(1);
        self.control_generation = self.control_generation.saturating_add(1);
        Ok(())
    }

    pub fn schedule_resume(
        &mut self,
        wake_at_epoch_ms: i64,
        now_epoch_ms: i64,
    ) -> Result<(), String> {
        let goal = self.active.as_mut().ok_or("No Goal is active")?;
        if goal.status.is_terminal() {
            return Err("A terminal Goal cannot be scheduled".to_string());
        }
        if wake_at_epoch_ms <= now_epoch_ms {
            return Err("Goal wake time must be in the future".to_string());
        }
        goal.status = GoalStatus::Paused;
        goal.stop_reason = Some(GoalStopReason::Scheduled);
        goal.next_wake_at_epoch_ms = Some(wake_at_epoch_ms);
        goal.lease = None;
        goal.updated_at_epoch_ms = now_epoch_ms;
        goal.last_progress_at_epoch_ms = Some(now_epoch_ms);
        self.revision = self.revision.saturating_add(1);
        self.control_generation = self.control_generation.saturating_add(1);
        Ok(())
    }

    pub fn pause(
        &mut self,
        status: GoalStatus,
        wake_at_epoch_ms: Option<i64>,
        now_epoch_ms: i64,
    ) -> Result<(), String> {
        if !matches!(
            status,
            GoalStatus::Paused | GoalStatus::NeedsUser | GoalStatus::Failed
        ) {
            return Err("Goal pause status must be paused, needs_user, or failed".to_string());
        }
        if status != GoalStatus::Paused && wake_at_epoch_ms.is_some() {
            return Err("Only a scheduled pause may have a Goal wake time".to_string());
        }
        let goal = self.active.as_mut().ok_or("No Goal is active")?;
        if goal.status.is_terminal() {
            return Err("A terminal Goal cannot be paused".to_string());
        }
        if wake_at_epoch_ms.is_some_and(|wake| wake <= now_epoch_ms) {
            return Err("Goal wake time must be in the future".to_string());
        }
        goal.status = status;
        goal.stop_reason = Some(match status {
            GoalStatus::Paused if wake_at_epoch_ms.is_some() => GoalStopReason::Scheduled,
            GoalStatus::Paused => GoalStopReason::Suspended,
            GoalStatus::NeedsUser => GoalStopReason::AwaitingUser,
            GoalStatus::Failed => GoalStopReason::Failed,
            _ => unreachable!("pause status was validated above"),
        });
        goal.next_wake_at_epoch_ms = wake_at_epoch_ms;
        goal.lease = None;
        goal.updated_at_epoch_ms = now_epoch_ms;
        goal.last_progress_at_epoch_ms = Some(now_epoch_ms);
        self.revision = self.revision.saturating_add(1);
        self.control_generation = self.control_generation.saturating_add(1);
        Ok(())
    }

    pub fn resume(&mut self, now_epoch_ms: i64) -> Result<(), String> {
        let goal = self.active.as_mut().ok_or("No Goal is active")?;
        if !matches!(
            goal.status,
            GoalStatus::Paused | GoalStatus::NeedsUser | GoalStatus::Failed
        ) {
            return Err("Goal is not resumable".to_string());
        }
        goal.status = GoalStatus::Running;
        goal.execution_blocker = None;
        goal.stop_reason = None;
        goal.next_wake_at_epoch_ms = None;
        goal.updated_at_epoch_ms = now_epoch_ms;
        goal.last_progress_at_epoch_ms = Some(now_epoch_ms);
        self.revision = self.revision.saturating_add(1);
        self.control_generation = self.control_generation.saturating_add(1);
        Ok(())
    }

    /// A committed answer may continue only the executing contract that asked
    /// for it. Explicit pauses, configuration blockers and other Runs remain
    /// untouched. This deterministic transition has no Host wall-clock input;
    /// the next execution/lease transition records its own observed time.
    pub(crate) fn resume_answered_execution(&mut self, run_id: &str) -> bool {
        if self.execution_owner(run_id).is_none() { return false; }
        let Some(goal) = self.active.as_mut() else { return false; };
        if goal.status != GoalStatus::NeedsUser || goal.execution_blocker.is_some()
            || !goal.contracts.last().is_some_and(|contract|
                contract.status == GoalContractStatus::Executing
                    && contract.execution_run_id.as_deref() == Some(run_id)) {
            return false;
        }
        goal.status = GoalStatus::Running;
        goal.stop_reason = None;
        goal.next_wake_at_epoch_ms = None;
        self.revision = self.revision.saturating_add(1);
        self.control_generation = self.control_generation.saturating_add(1);
        true
    }

    pub fn cancel(&mut self, now_epoch_ms: i64) -> Result<(), String> {
        let goal = self.active.as_mut().ok_or("No Goal is active")?;
        if goal.status.is_terminal() {
            return Err("Goal is already terminal".to_string());
        }
        goal.status = GoalStatus::Cancelled;
        goal.stop_reason = Some(GoalStopReason::Cancelled);
        goal.next_wake_at_epoch_ms = None;
        goal.lease = None;
        goal.updated_at_epoch_ms = now_epoch_ms;
        goal.last_progress_at_epoch_ms = Some(now_epoch_ms);
        self.revision = self.revision.saturating_add(1);
        self.control_generation = self.control_generation.saturating_add(1);
        Ok(())
    }

    pub fn claim(
        &mut self,
        owner: String,
        lease_until_epoch_ms: i64,
        now_epoch_ms: i64,
    ) -> Result<(), String> {
        validate_nonempty("Goal lease owner", &owner)?;
        if lease_until_epoch_ms <= now_epoch_ms {
            return Err("Goal lease expiry must be in the future".to_string());
        }
        let goal = self.active.as_mut().ok_or("No Goal is active")?;
        let previous_control = (goal.status, goal.stop_reason, goal.next_wake_at_epoch_ms);
        if goal.status.is_terminal() {
            return Err("A terminal Goal cannot be claimed".to_string());
        }
        if goal.status != GoalStatus::Running {
            let scheduled = goal.status == GoalStatus::Paused
                && goal.stop_reason == Some(GoalStopReason::Scheduled);
            match (scheduled, goal.next_wake_at_epoch_ms) {
                (true, Some(wake)) if wake <= now_epoch_ms => {}
                (true, Some(_)) => return Err("Goal is not due".to_string()),
                _ => return Err("Goal requires explicit resume before claim".to_string()),
            }
        }
        if goal
            .lease
            .as_ref()
            .is_some_and(|lease| lease.lease_until_epoch_ms > now_epoch_ms && lease.owner != owner)
        {
            return Err("Goal is leased by another worker".to_string());
        }
        goal.status = GoalStatus::Running;
        goal.execution_blocker = None;
        goal.stop_reason = None;
        goal.next_wake_at_epoch_ms = None;
        goal.attempt_count = goal.attempt_count.saturating_add(1);
        goal.last_progress_at_epoch_ms = Some(now_epoch_ms);
        goal.updated_at_epoch_ms = now_epoch_ms;
        goal.lease = Some(GoalLease {
            owner,
            acquired_at_epoch_ms: now_epoch_ms,
            heartbeat_at_epoch_ms: now_epoch_ms,
            lease_until_epoch_ms,
        });
        let changed_control = previous_control != (goal.status, goal.stop_reason, goal.next_wake_at_epoch_ms);
        self.revision = self.revision.saturating_add(1);
        if changed_control { self.control_generation = self.control_generation.saturating_add(1); }
        Ok(())
    }

    pub fn heartbeat(
        &mut self,
        owner: &str,
        lease_until_epoch_ms: i64,
        now_epoch_ms: i64,
    ) -> Result<(), String> {
        let goal = self.active.as_mut().ok_or("No Goal is active")?;
        let lease = goal.lease.as_mut().ok_or("Goal has no active lease")?;
        if lease.owner != owner || lease.lease_until_epoch_ms <= now_epoch_ms {
            return Err("Goal lease is not owned by this worker".to_string());
        }
        if lease_until_epoch_ms <= now_epoch_ms {
            return Err("Goal lease expiry must be in the future".to_string());
        }
        lease.heartbeat_at_epoch_ms = now_epoch_ms;
        lease.lease_until_epoch_ms = lease_until_epoch_ms;
        goal.last_progress_at_epoch_ms = Some(now_epoch_ms);
        goal.updated_at_epoch_ms = now_epoch_ms;
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn release(
        &mut self,
        owner: &str,
        wake_at_epoch_ms: Option<i64>,
        now_epoch_ms: i64,
    ) -> Result<(), String> {
        let goal = self.active.as_mut().ok_or("No Goal is active")?;
        let previous_control = (goal.status, goal.stop_reason, goal.next_wake_at_epoch_ms);
        let lease = goal.lease.as_ref().ok_or("Goal has no active lease")?;
        if lease.owner != owner {
            return Err("Goal lease is not owned by this worker".to_string());
        }
        if wake_at_epoch_ms.is_some_and(|wake| wake <= now_epoch_ms) {
            return Err("Goal wake time must be in the future".to_string());
        }
        goal.lease = None;
        goal.next_wake_at_epoch_ms = wake_at_epoch_ms;
        if wake_at_epoch_ms.is_some() {
            goal.status = GoalStatus::Paused;
            goal.stop_reason = Some(GoalStopReason::Scheduled);
        } else {
            goal.stop_reason = None;
        }
        goal.updated_at_epoch_ms = now_epoch_ms;
        goal.last_progress_at_epoch_ms = Some(now_epoch_ms);
        let changed_control = previous_control != (goal.status, goal.stop_reason, goal.next_wake_at_epoch_ms);
        self.revision = self.revision.saturating_add(1);
        if changed_control { self.control_generation = self.control_generation.saturating_add(1); }
        Ok(())
    }

    pub fn resume_due(&mut self, now_epoch_ms: i64) -> Result<bool, String> {
        let Some(goal) = self.active.as_mut() else {
            return Ok(false);
        };
        if goal.status != GoalStatus::Paused
            || goal.stop_reason != Some(GoalStopReason::Scheduled)
            || !goal
                .next_wake_at_epoch_ms
                .is_some_and(|wake| wake <= now_epoch_ms)
        {
            return Ok(false);
        }
        goal.status = GoalStatus::Running;
        goal.execution_blocker = None;
        goal.stop_reason = None;
        goal.next_wake_at_epoch_ms = None;
        goal.updated_at_epoch_ms = now_epoch_ms;
        self.revision = self.revision.saturating_add(1);
        self.control_generation = self.control_generation.saturating_add(1);
        Ok(true)
    }

    fn running_goal_mut(&mut self) -> Result<&mut GoalState, String> {
        let goal = self.active.as_mut().ok_or("No Goal is active")?;
        if goal.status != GoalStatus::Running {
            return Err("Goal is not running".to_string());
        }
        Ok(goal)
    }

    fn transition_contract(
        &mut self,
        contract_id: &str,
        expected: GoalContractStatus,
        next: GoalContractStatus,
        now_epoch_ms: i64,
    ) -> Result<(), String> {
        let goal = self.running_goal_mut()?;
        let contract = goal
            .contracts
            .last_mut()
            .ok_or("Goal has no active contract")?;
        if contract.id != contract_id || contract.status != expected {
            return Err("Goal contract transition does not match current state".to_string());
        }
        contract.status = next;
        goal.updated_at_epoch_ms = now_epoch_ms;
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
}

pub fn goal_command_digest(command: &GoalRuntimeCommand) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    // Typed struct order and BTreeMap fields define stable bytes; arrays and all
    // caller timestamps/CAS values remain part of the semantic command.
    serde_json::to_vec(command).map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(|error| error.to_string())
}

fn validate_nonempty(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(())
    }
}

fn record_exists(goal: &GoalState, id: &str) -> bool {
    goal.requirements.contains_key(id)
        || goal.artifacts.contains_key(id)
        || goal.facts.contains_key(id)
}

fn validate_contract_requirements(goal: &GoalState, contract: &GoalSubtaskContract) -> Result<(), String> {
    let records = contract.relevant_record_ids.iter().collect::<BTreeSet<_>>();
    if records.len() != contract.relevant_record_ids.len() {
        return Err("Goal execution contract repeats a record".into());
    }
    let expected = contract.relevant_record_ids.iter()
        .filter_map(|id| goal.requirements.get(id).map(|requirement| (id.clone(), requirement.semantic_revision)))
        .collect::<BTreeMap<_, _>>();
    if expected != contract.requirement_versions {
        return Err("Goal execution contract must bind the exact current requirement versions".into());
    }
    Ok(())
}

fn validate_finding(
    goal: &GoalState,
    finding: &GoalAuditFinding,
    evidence: &BTreeMap<&str, &GoalEvidence>,
) -> Result<(), String> {
    match finding.record_kind {
        GoalRecordKind::Requirement => {
            let requirement = goal.requirements.get(&finding.record_id).ok_or("Unknown Goal requirement")?;
            if finding.requirement_revision != Some(requirement.semantic_revision) {
                return Err("Goal requirement changed after assessment".into());
            }
            if finding.status == GoalRecordStatus::Completed
                && matches!(requirement.acceptance_policy, GoalAcceptancePolicy::ExecutionChecks { .. })
                && finding.assessment_method != GoalAssessmentMethod::EvidenceInspection {
                return Err("A declared execution check cannot be accepted as ModelJudgment".into());
            }
        }
        _ if finding.requirement_revision.is_some() => return Err("Non-requirement finding cannot carry a requirement revision".into()),
        _ => {}
    }
    let exists = match finding.record_kind {
        GoalRecordKind::Requirement => goal.requirements.contains_key(&finding.record_id),
        GoalRecordKind::Artifact => goal.artifacts.contains_key(&finding.record_id),
        GoalRecordKind::Fact => goal.facts.contains_key(&finding.record_id),
    };
    if !exists {
        return Err("Goal audit finding references an unknown record".to_string());
    }
    let unique_refs = finding.evidence_refs.iter().collect::<BTreeSet<_>>();
    if unique_refs.len() != finding.evidence_refs.len() {
        return Err("Goal audit finding repeats an evidence reference".to_string());
    }
    if finding.status == GoalRecordStatus::Completed {
        if finding.evidence_refs.is_empty() {
            return Err("Completed Goal record requires audit evidence".to_string());
        }
        if finding.evidence_refs.iter().any(|id| {
            !evidence
                .get(id.as_str())
                .is_some_and(|item| item.source_verified)
        }) {
            return Err(
                "Completed Goal record requires source-verified evidence".to_string(),
            );
        }
    }
    Ok(())
}

fn apply_finding(goal: &mut GoalState, finding: &GoalAuditFinding) {
    match finding.record_kind {
        GoalRecordKind::Requirement => {
            if let Some(record) = goal.requirements.get_mut(&finding.record_id) {
                record.status = finding.status;
                record.evidence_refs = finding.evidence_refs.clone();
            }
        }
        GoalRecordKind::Artifact => {
            if let Some(record) = goal.artifacts.get_mut(&finding.record_id) {
                record.status = finding.status;
                record.evidence_refs = finding.evidence_refs.clone();
            }
        }
        GoalRecordKind::Fact => {
            if let Some(record) = goal.facts.get_mut(&finding.record_id) {
                record.status = finding.status;
                record.evidence_refs = finding.evidence_refs.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_observation_exposes_pause_for_needs_user_and_matches_transitions() {
        let mut projection = GoalProjection::default();
        projection.start("goal".into(), "Verify".into(), vec![], vec![requirement("r")], 1).unwrap();
        for (status, pause, resume, cancel) in [
            (GoalStatus::Running, true, false, true),
            (GoalStatus::Paused, false, true, true),
            (GoalStatus::NeedsUser, true, true, true),
            (GoalStatus::Failed, true, true, true),
            (GoalStatus::Completed, false, false, false),
            (GoalStatus::Cancelled, false, false, false),
        ] {
            projection.active.as_mut().unwrap().status = status;
            let observation = projection.control_observation().unwrap();
            assert_eq!((observation.can_pause, observation.can_resume, observation.can_cancel), (pause, resume, cancel));
            if pause { assert!(projection.clone().pause(GoalStatus::Paused, None, 2).is_ok()); }
            assert_eq!(projection.clone().resume(2).is_ok(), resume);
            assert_eq!(observation.can_submit, status.is_terminal());
        }
    }

    #[test]
    fn operation_identity_survives_reopen_and_new_goals_without_reapplying() {
        let mut projection = GoalProjection::default();
        let first = GoalRuntimeCommandEnvelope { operation_id:"first-start".into(), command:GoalRuntimeCommand::Start {
            id:"first".into(),objective:"First".into(),constraints:vec![],requirements:vec![requirement("r")],now_epoch_ms:1,
        }};
        projection.apply(first.clone()).unwrap();
        projection.apply(GoalRuntimeCommandEnvelope { operation_id:"cancel".into(),command:GoalRuntimeCommand::Cancel { now_epoch_ms:2 } }).unwrap();
        projection.apply(GoalRuntimeCommandEnvelope { operation_id:"second-start".into(),command:GoalRuntimeCommand::Start {
            id:"second".into(),objective:"Second".into(),constraints:vec![],requirements:vec![requirement("r")],now_epoch_ms:3,
        }}).unwrap();
        let mut restored: GoalProjection = serde_json::from_slice(&serde_json::to_vec(&projection).unwrap()).unwrap();
        restored.apply(first.clone()).unwrap();
        assert_eq!(restored,projection); // current Goal remains second, not historical first.
        let mut changed = first;
        if let GoalRuntimeCommand::Start { now_epoch_ms, .. } = &mut changed.command { *now_epoch_ms = 4; }
        assert!(restored.apply(changed).is_err());
        assert_eq!(restored,projection);
        let invalid = GoalRuntimeCommandEnvelope { operation_id:"retryable".into(),command:GoalRuntimeCommand::Start {
            id:"third".into(),objective:"Third".into(),constraints:vec![],requirements:vec![],now_epoch_ms:4,
        }};
        assert!(restored.apply(invalid).is_err());
        assert!(!restored.applied_operations.contains_key("retryable"));
        assert_eq!(restored,projection);
        assert!(serde_json::from_str::<GoalProjection>(r#"{"revision":0}"#).is_err());
    }

    fn requirement(id: &str) -> GoalRequirement {
        GoalRequirement {
            acceptance_policy: crate::agent::goal::GoalAcceptancePolicy::ModelJudgment,
            semantic_revision: 1, id: id.to_string(),
            description: format!("verify {id}"),
            dependencies: Vec::new(),
            status: GoalRecordStatus::Completed,
            evidence_refs: vec!["executor-claim".to_string()],
        }
    }

    fn contract() -> GoalSubtaskContract {
        GoalSubtaskContract { execution_run_id: None,
            id: "contract-1".to_string(),
            round: 1,
            objective: "produce and verify output".to_string(),
            proposed_checks: vec!["requirement-1 is observed in the environment".to_string()],
            requirement_versions: BTreeMap::from([("requirement-1".into(), 1)]),
            additional_execution_constraints: vec!["workspace only".to_string()],
            relevant_record_ids: vec!["requirement-1".to_string()],
            status: GoalContractStatus::Audited,
            created_at_epoch_ms: 2,
        }
    }

    #[test]
    fn answered_execution_resumes_only_its_waiting_contract() {
        let mut projection = GoalProjection::default();
        projection.start("goal-1".into(), "verify".into(), vec![], vec![requirement("requirement-1")], 1).unwrap();
        projection.plan_contract(contract(), 2).unwrap();
        projection.begin_execution("contract-1", "run-1", 3).unwrap();
        projection.pause(GoalStatus::NeedsUser, None, 4).unwrap();
        let waiting = projection.clone();
        assert!(!projection.resume_answered_execution("other-run"));
        assert_eq!(projection, waiting);
        assert!(projection.resume_answered_execution("run-1"));
        assert_eq!(projection.active.as_ref().unwrap().status, GoalStatus::Running);
        assert_eq!(projection.active.as_ref().unwrap().contracts, waiting.active.as_ref().unwrap().contracts);
        let resumed = projection.clone();
        assert!(!projection.resume_answered_execution("run-1"));
        assert_eq!(projection, resumed);
        for status in [GoalStatus::Paused, GoalStatus::Cancelled, GoalStatus::Failed] {
            let mut projection = waiting.clone();
            projection.active.as_mut().unwrap().status = status;
            let before = projection.clone();
            assert!(!projection.resume_answered_execution("run-1"));
            assert_eq!(projection, before);
        }
        let mut blocked = waiting;
        blocked.active.as_mut().unwrap().execution_blocker = Some(GoalExecutionBlocker {
            code: "configuration_unavailable".into(), message: "Configuration required".into(),
        });
        let before = blocked.clone();
        assert!(!blocked.resume_answered_execution("run-1"));
        assert_eq!(blocked, before);
    }

    #[test]
    fn executor_claim_cannot_complete_goal_without_independent_audit() {
        let mut projection = GoalProjection::default();
        projection
            .start(
                "goal-1".to_string(),
                "finish reliably".to_string(),
                Vec::new(),
                vec![requirement("requirement-1")],
                1,
            )
            .unwrap();
        assert_eq!(
            projection.active.as_ref().unwrap().requirements["requirement-1"].status,
            GoalRecordStatus::Pending
        );
        assert!(projection.complete(2).is_err());
    }

    #[test]
    fn audited_state_transition_can_complete_goal() {
        let mut projection = GoalProjection::default();
        projection
            .start(
                "goal-1".to_string(),
                "finish reliably".to_string(),
                Vec::new(),
                vec![requirement("requirement-1")],
                1,
            )
            .unwrap();
        projection.plan_contract(contract(), 2).unwrap();
        projection.begin_execution("contract-1", concat!("contract-1", "-run"), 3).unwrap();
        projection
            .record_execution_report(GoalExecutionReport {
                contract_id: "contract-1".to_string(),
                summary: "executor says it is done".to_string(),
                claimed_record_ids: vec!["requirement-1".to_string()],
                artifact_refs: Vec::new(),
                blockers: Vec::new(),
                reported_at_epoch_ms: 4,
            })
            .unwrap();
        projection
            .apply_audit(GoalAuditReport {
                assessor: GoalAssessor::ModelTurn { session_id: "fixture-session".into(), turn_id: "fixture-turn".into() }, expected_goal_revision: projection.revision,
                id: "audit-1".to_string(),
                contract_id: "contract-1".to_string(),
                verdict: GoalAuditVerdict::Passed,
                summary: "environment confirms the requirement".to_string(),
                findings: vec![GoalAuditFinding {
                    requirement_revision: Some(1), assessment_method: GoalAssessmentMethod::EvidenceInspection,
                    record_kind: GoalRecordKind::Requirement,
                    record_id: "requirement-1".to_string(),
                    status: GoalRecordStatus::Completed,
                    evidence_refs: vec!["evidence-1".to_string()],
                    summary: "verified".to_string(),
                }],
                unmet_requirement_ids: Vec::new(),
                integrity_violations: Vec::new(),
                evidence: vec![GoalEvidence {
                    id: "evidence-1".to_string(),
                    source: "read-only validator".to_string(),
                    summary: "observed expected state".to_string(),
                    observed_at_epoch_ms: 5,
                    source_verified: true,
                }],
                audited_at_epoch_ms: 5,
            })
            .unwrap();
        let mut changed = projection.clone();
        changed.register_fact(GoalFact {
            id: "new-fact".into(), statement: "additional observation after acceptance".into(),
            status: GoalRecordStatus::Pending, evidence_refs: vec![],
        }).unwrap();
        let before = changed.clone();
        assert_eq!(changed.complete(6).unwrap_err(),
            "Goal changed after acceptance; completion requires a current assessment");
        assert_eq!(changed, before);
        // A conflict has an explicit recovery route; it is not a permanent
        // validation debt and does not erase the earlier evidence.
        let mut reassessment = changed.active.as_ref().unwrap().audit_reports[0].clone();
        let mut next_contract = contract();
        next_contract.id = "contract-2".into();
        next_contract.round = 2;
        changed.plan_contract(next_contract, 7).unwrap();
        changed.begin_execution("contract-2", concat!("contract-2", "-run"), 8).unwrap();
        changed.record_execution_report(GoalExecutionReport {
            contract_id: "contract-2".into(), summary: "reassess with the new fact".into(),
            claimed_record_ids: vec![], artifact_refs: vec![], blockers: vec![], reported_at_epoch_ms: 9,
        }).unwrap();
        reassessment.id = "audit-2".into();
        reassessment.contract_id = "contract-2".into();
        reassessment.expected_goal_revision = changed.revision;
        reassessment.summary = "new fact does not invalidate retained evidence".into();
        reassessment.audited_at_epoch_ms = 10;
        changed.apply_audit(reassessment).unwrap();
        changed.complete(11).unwrap();
        projection.complete(6).unwrap();
        assert_eq!(
            projection.active.as_ref().unwrap().status,
            GoalStatus::Completed
        );
    }

    #[test]
    fn unverified_evidence_is_rejected() {
        let mut projection = GoalProjection::default();
        projection
            .start(
                "goal-1".to_string(),
                "finish reliably".to_string(),
                Vec::new(),
                vec![requirement("requirement-1")],
                1,
            )
            .unwrap();
        projection.plan_contract(contract(), 2).unwrap();
        projection.begin_execution("contract-1", concat!("contract-1", "-run"), 3).unwrap();
        projection
            .record_execution_report(GoalExecutionReport {
                contract_id: "contract-1".to_string(),
                summary: "claimed done".to_string(),
                claimed_record_ids: vec!["requirement-1".to_string()],
                artifact_refs: Vec::new(),
                blockers: Vec::new(),
                reported_at_epoch_ms: 4,
            })
            .unwrap();
        let result = projection.apply_audit(GoalAuditReport {
            assessor: GoalAssessor::ModelTurn { session_id: "fixture-session".into(), turn_id: "fixture-turn".into() }, expected_goal_revision: projection.revision,
            id: "audit-1".to_string(),
            contract_id: "contract-1".to_string(),
            verdict: GoalAuditVerdict::Passed,
            summary: "copied executor claim".to_string(),
            findings: vec![GoalAuditFinding {
                    requirement_revision: Some(1), assessment_method: GoalAssessmentMethod::EvidenceInspection,
                record_kind: GoalRecordKind::Requirement,
                record_id: "requirement-1".to_string(),
                status: GoalRecordStatus::Completed,
                evidence_refs: vec!["claim".to_string()],
                summary: "not inspected".to_string(),
            }],
            unmet_requirement_ids: Vec::new(),
            integrity_violations: Vec::new(),
            evidence: vec![GoalEvidence {
                id: "claim".to_string(),
                source: "executor".to_string(),
                summary: "I finished".to_string(),
                observed_at_epoch_ms: 5,
                source_verified: false,
            }],
            audited_at_epoch_ms: 5,
        });
        assert!(result.is_err());
    }

    #[test]
    fn audit_rejects_contradictory_findings_and_inaccurate_unmet_state() {
        let mut projection = GoalProjection::default();
        projection
            .start(
                "goal-1".to_string(),
                "finish reliably".to_string(),
                Vec::new(),
                vec![requirement("requirement-1")],
                1,
            )
            .unwrap();
        projection.plan_contract(contract(), 2).unwrap();
        projection.begin_execution("contract-1", concat!("contract-1", "-run"), 3).unwrap();
        projection
            .record_execution_report(GoalExecutionReport {
                contract_id: "contract-1".to_string(),
                summary: "ready for audit".to_string(),
                claimed_record_ids: Vec::new(),
                artifact_refs: Vec::new(),
                blockers: Vec::new(),
                reported_at_epoch_ms: 4,
            })
            .unwrap();
        let finding = GoalAuditFinding {
                    requirement_revision: Some(1), assessment_method: GoalAssessmentMethod::EvidenceInspection,
            record_kind: GoalRecordKind::Requirement,
            record_id: "requirement-1".to_string(),
            status: GoalRecordStatus::Pending,
            evidence_refs: Vec::new(),
            summary: "still pending".to_string(),
        };
        let base = GoalAuditReport {
            assessor: GoalAssessor::ModelTurn { session_id: "fixture-session".into(), turn_id: "fixture-turn".into() }, expected_goal_revision: projection.revision,
            id: "audit-1".to_string(),
            contract_id: "contract-1".to_string(),
            verdict: GoalAuditVerdict::Partial,
            summary: "not complete".to_string(),
            findings: vec![finding.clone(), finding],
            unmet_requirement_ids: vec!["requirement-1".to_string()],
            integrity_violations: Vec::new(),
            evidence: Vec::new(),
            audited_at_epoch_ms: 5,
        };
        assert_eq!(
            projection.apply_audit(base.clone()).unwrap_err(),
            "Goal audit findings must identify unique records"
        );
        let mut inaccurate = base;
        inaccurate.findings.truncate(1);
        inaccurate.unmet_requirement_ids.clear();
        assert_eq!(
            projection.apply_audit(inaccurate).unwrap_err(),
            "Goal audit unmet requirements do not match the audited contract state"
        );
    }

    #[test]
    fn audit_revocation_uses_proposed_status_and_rejects_stale_dependant_acceptance() {
        let mut projection = GoalProjection::default();
        projection.start("goal-1".into(), "verify".into(), vec![], vec![requirement("requirement-1")], 1).unwrap();
        projection.plan_contract(contract(), 2).unwrap();
        projection.begin_execution("contract-1", concat!("contract-1", "-run"), 3).unwrap();
        projection.record_execution_report(GoalExecutionReport {
            contract_id: "contract-1".into(), summary: "reassess prior acceptance".into(),
            claimed_record_ids: vec![], artifact_refs: vec![], blockers: vec![], reported_at_epoch_ms: 4,
        }).unwrap();
        let goal = projection.active.as_mut().unwrap();
        goal.requirements.get_mut("requirement-1").unwrap().status = GoalRecordStatus::Completed;
        goal.requirements.insert("dependent".into(), GoalRequirement {
            acceptance_policy: crate::agent::goal::GoalAcceptancePolicy::ModelJudgment,
            semantic_revision: 1, id: "dependent".into(), description: "depends on the first acceptance".into(),
            dependencies: vec!["requirement-1".into()], status: GoalRecordStatus::Completed,
            evidence_refs: vec!["old-evidence".into()],
        });
        let contract = goal.contracts.last_mut().unwrap();
        contract.relevant_record_ids.push("dependent".into());
        contract.requirement_versions.insert("dependent".into(), 1);
        let finding = GoalAuditFinding {
            requirement_revision: Some(1), assessment_method: GoalAssessmentMethod::ModelJudgment, record_kind: GoalRecordKind::Requirement,
            record_id: "requirement-1".into(), status: GoalRecordStatus::Pending,
            evidence_refs: vec![], summary: "prior judgment needs reassessment".into(),
        };
        let mut report = GoalAuditReport {
            assessor: GoalAssessor::ModelTurn { session_id: "fixture-session".into(), turn_id: "fixture-turn".into() }, id: "audit-1".into(), expected_goal_revision: projection.revision,
            contract_id: "contract-1".into(), verdict: GoalAuditVerdict::Partial,
            summary: "revoke old acceptance".into(), findings: vec![finding.clone()],
            unmet_requirement_ids: vec!["requirement-1".into()], integrity_violations: vec![],
            evidence: vec![], audited_at_epoch_ms: 5,
        };
        let before = projection.clone();
        assert_eq!(projection.apply_audit(report.clone()).unwrap_err(),
            "Goal requirement cannot complete before all dependencies are verified");
        assert_eq!(projection, before);
        report.findings.push(GoalAuditFinding {record_id: "dependent".into(), ..finding});
        report.unmet_requirement_ids.push("dependent".into());
        projection.apply_audit(report).unwrap();
        assert!(projection.active.as_ref().unwrap().requirements.values()
            .all(|requirement| requirement.status == GoalRecordStatus::Pending));
    }

    #[test]
    fn paused_goal_resumes_only_after_durable_wake_time() {
        let mut projection = GoalProjection::default();
        projection
            .start(
                "goal-1".to_string(),
                "run for days".to_string(),
                Vec::new(),
                vec![requirement("requirement-1")],
                1,
            )
            .unwrap();
        projection.schedule_resume(10_000, 100).unwrap();
        assert!(!projection.resume_due(9_999).unwrap());
        assert!(projection.resume_due(10_000).unwrap());
        assert_eq!(
            projection.active.as_ref().unwrap().status,
            GoalStatus::Running
        );
    }

    #[test]
    fn execution_plan_is_version_bound_and_cannot_replace_goal_requirements() {
        let mut projection = GoalProjection::default();
        projection.start("goal-1".into(), "deliver".into(), vec!["No deployment".into()], vec![requirement("requirement-1")], 1).unwrap();
        let original = projection.active.as_ref().unwrap().requirements.clone();
        let mut plan = contract();
        plan.proposed_checks = vec!["An executor-suggested check".into()];
        plan.additional_execution_constraints.clear();
        projection.plan_contract(plan, 2).unwrap();
        assert_eq!(projection.active.as_ref().unwrap().requirements, original);
        assert_eq!(projection.active.as_ref().unwrap().constraints, vec!["No deployment"]);
        projection.update_work("goal-1", projection.revision, vec![GoalWorkEntry {
            id:"new-condition".into(), kind:GoalWorkKind::RequirementDetail, origin:GoalWorkOrigin::Model,
            source_ref:"turn".into(), content:"Verify the durable output too".into(), requirement_ids:vec!["requirement-1".into()],
            task_ids:vec![], evidence_refs:vec![], progress:None, supersedes:None,
        }], 3).unwrap();
        let before = projection.clone();
        assert!(projection.begin_execution("contract-1", concat!("contract-1", "-run"), 4).unwrap_err().contains("exact current requirement versions"));
        assert_eq!(projection, before);
        projection.apply_audit(GoalAuditReport {
            assessor: GoalAssessor::RuntimeOperation { operation_id:"retire-stale-plan".into() },
            id:"not-executed".into(), expected_goal_revision:projection.revision,
            contract_id:"contract-1".into(), verdict:GoalAuditVerdict::Partial,
            summary:"The stale plan never executed".into(), findings:vec![],
            unmet_requirement_ids:vec!["requirement-1".into()], integrity_violations:vec![],
            evidence:vec![], audited_at_epoch_ms:4,
        }).unwrap();
        let mut next = contract();
        next.id = "contract-2".into(); next.round = 2;
        next.requirement_versions.insert("requirement-1".into(), 2);
        projection.plan_contract(next, 5).unwrap();
        projection.begin_execution("contract-2", concat!("contract-2", "-run"), 6).unwrap();
        assert!(projection.active.as_ref().unwrap().requirement_acceptances.is_empty());
    }

    #[test]
    fn stale_audit_cannot_apply_after_work_changes_or_checkpoint_restore() {
        let mut projection = GoalProjection::default();
        projection.start("goal-1".into(), "verify work".into(), vec![], vec![requirement("requirement-1")], 1).unwrap();
        projection.plan_contract(contract(), 2).unwrap();
        projection.begin_execution("contract-1", concat!("contract-1", "-run"), 3).unwrap();
        projection.record_execution_report(GoalExecutionReport {
            contract_id: "contract-1".into(), summary: "ready".into(),
            claimed_record_ids: vec![], artifact_refs: vec![], blockers: vec![], reported_at_epoch_ms: 4,
        }).unwrap();
        let report = GoalAuditReport {
            assessor: GoalAssessor::ModelTurn { session_id: "fixture-session".into(), turn_id: "fixture-turn".into() }, id: "audit-1".into(), expected_goal_revision: projection.revision,
            contract_id: "contract-1".into(), verdict: GoalAuditVerdict::Partial,
            summary: "assessed the original snapshot".into(), findings: vec![],
            unmet_requirement_ids: vec!["requirement-1".into()], integrity_violations: vec![],
            evidence: vec![], audited_at_epoch_ms: 6,
        };
        let mut old_wire = serde_json::to_value(&report).unwrap();
        old_wire.as_object_mut().unwrap().remove("expected_goal_revision");
        assert!(serde_json::from_value::<GoalAuditReport>(old_wire).is_err());
        projection.update_work("goal-1", projection.revision, vec![GoalWorkEntry {
            id: "detail-1".into(), kind: GoalWorkKind::RequirementDetail,
            origin: GoalWorkOrigin::Model, source_ref: "turn-1".into(),
            content: "Also verify restart recovery".into(), requirement_ids: vec!["requirement-1".into()],
            task_ids: vec![], evidence_refs: vec![], progress: None, supersedes: None,
        }], 5).unwrap();
        let mut restored: GoalProjection = serde_json::from_slice(&serde_json::to_vec(&projection).unwrap()).unwrap();
        assert_eq!(restored.apply_audit(report.clone()).unwrap_err(),
            "Goal changed after the audit snapshot; reassess the current revision");
        assert_eq!(restored, projection, "rejected stale audit must not mutate any Goal state");
        restored.apply_audit(GoalAuditReport {
            assessor: GoalAssessor::ModelTurn { session_id: "fixture-session".into(), turn_id: "fixture-turn".into() }, expected_goal_revision: restored.revision,
            summary: "reassessed the updated work; requirement remains pending".into(),
            ..report
        }).unwrap();
        assert_eq!(restored.active.as_ref().unwrap().requirements["requirement-1"].status, GoalRecordStatus::Pending);
    }

    #[test]
    fn duplicate_record_registration_is_rejected_without_replacing_retained_evidence() {
        let mut projection = GoalProjection::default();
        projection.start("goal-1".into(), "verify".into(), vec![], vec![requirement("requirement-1")], 1).unwrap();
        let artifact = GoalArtifact {
            id: "artifact".into(), uri: "artifact://original".into(), description: "original".into(),
            status: GoalRecordStatus::Pending, evidence_refs: vec![],
        };
        let fact = GoalFact {
            id: "fact".into(), statement: "original observation".into(),
            status: GoalRecordStatus::Pending, evidence_refs: vec![],
        };
        projection.register_artifact(artifact.clone()).unwrap();
        projection.register_fact(fact.clone()).unwrap();
        let before = projection.clone();
        assert!(projection.register_artifact(GoalArtifact {
            uri: "artifact://replacement".into(), ..artifact
        }).is_err());
        assert_eq!(projection, before);
        assert!(projection.register_fact(GoalFact {
            statement: "replacement claim".into(), ..fact
        }).is_err());
        assert_eq!(projection, before);
    }

    #[test]
    fn manual_resume_recovers_paused_needs_user_and_failed_goals() {
        for status in [
            GoalStatus::Paused,
            GoalStatus::NeedsUser,
            GoalStatus::Failed,
        ] {
            let mut projection = GoalProjection::default();
            projection
                .start(
                    "goal-1".to_string(),
                    "run for days".to_string(),
                    Vec::new(),
                    vec![requirement("requirement-1")],
                    1,
                )
                .unwrap();
            projection.pause(status, None, 2).unwrap();
            let expected_stop_reason = match status {
                GoalStatus::Paused => GoalStopReason::Suspended,
                GoalStatus::NeedsUser => GoalStopReason::AwaitingUser,
                GoalStatus::Failed => GoalStopReason::Failed,
                _ => unreachable!(),
            };
            assert_eq!(
                projection.active.as_ref().unwrap().stop_reason,
                Some(expected_stop_reason)
            );
            projection.resume(3).unwrap();
            assert_eq!(
                projection.active.as_ref().unwrap().status,
                GoalStatus::Running
            );
            assert_eq!(projection.active.as_ref().unwrap().stop_reason, None);
        }
    }

    #[test]
    fn claiming_a_restored_goal_cannot_bypass_an_explicit_resume() {
        for status in [GoalStatus::Paused, GoalStatus::NeedsUser, GoalStatus::Failed] {
            let mut projection = GoalProjection::default();
            projection.start("goal-1".into(), "deliver".into(), vec![],
                vec![requirement("requirement-1")], 1).unwrap();
            projection.pause(status, None, 2).unwrap();
            let bytes = serde_json::to_vec(&projection).unwrap();
            let mut restored: GoalProjection = serde_json::from_slice(&bytes).unwrap();
            let before = restored.clone();
            assert!(!restored.resume_due(100).unwrap());
            assert_eq!(restored.claim("worker".into(), 200, 100).unwrap_err(),
                "Goal requires explicit resume before claim");
            assert_eq!(restored, before);
            restored.resume(101).unwrap();
            restored.claim("worker".into(), 200, 102).unwrap();
            assert_eq!(restored.active.as_ref().unwrap().status, GoalStatus::Running);
        }
    }

    #[test]
    fn incomplete_goal_is_not_an_automatically_claimable_pause() {
        let mut projection = GoalProjection::default();
        projection.start("goal-1".into(), "deliver".into(), vec![],
            vec![requirement("requirement-1")], 1).unwrap();
        projection.apply(GoalRuntimeCommandEnvelope {
            operation_id: "conclude-incomplete".into(),
            command: GoalRuntimeCommand::ConcludeIncomplete { now_epoch_ms: 2 },
        }).unwrap();
        let bytes = serde_json::to_vec(&projection).unwrap();
        let mut restored: GoalProjection = serde_json::from_slice(&bytes).unwrap();
        let before = restored.clone();
        assert!(!restored.resume_due(100).unwrap());
        assert!(restored.claim("worker".into(), 200, 100).is_err());
        assert_eq!(restored, before);
    }

    #[test]
    fn only_scheduled_pause_allows_automatic_claim_at_its_wake_time() {
        let mut projection = GoalProjection::default();
        projection.start("goal-1".into(), "deliver".into(), vec![],
            vec![requirement("requirement-1")], 1).unwrap();
        for status in [GoalStatus::NeedsUser, GoalStatus::Failed] {
            let before = projection.clone();
            assert!(projection.pause(status, Some(100), 2).is_err());
            assert_eq!(projection, before);
        }
        projection.schedule_resume(100, 2).unwrap();
        let bytes = serde_json::to_vec(&projection).unwrap();
        let mut restored: GoalProjection = serde_json::from_slice(&bytes).unwrap();
        let before = restored.clone();
        assert_eq!(restored.claim("worker".into(), 200, 99).unwrap_err(), "Goal is not due");
        assert_eq!(restored, before);
        restored.claim("worker".into(), 200, 100).unwrap();
        let goal = restored.active.as_ref().unwrap();
        assert_eq!(goal.status, GoalStatus::Running);
        assert_eq!(goal.stop_reason, None);
        assert_eq!(goal.next_wake_at_epoch_ms, None);
    }

    #[test]
    fn durable_lease_prevents_duplicate_workers_and_expired_lease_can_be_reclaimed() {
        let mut projection = GoalProjection::default();
        projection
            .start(
                "goal-1".to_string(),
                "run for days".to_string(),
                Vec::new(),
                vec![requirement("requirement-1")],
                1,
            )
            .unwrap();
        projection
            .claim("worker-a".to_string(), 1_000, 100)
            .unwrap();
        assert!(projection
            .claim("worker-b".to_string(), 1_100, 200)
            .is_err());
        projection.heartbeat("worker-a", 1_500, 500).unwrap();
        assert!(projection
            .claim("worker-b".to_string(), 1_600, 1_000)
            .is_err());
        let before_expired_heartbeat = projection.clone();
        assert!(projection.heartbeat("worker-a", 2_500, 1_500).is_err());
        assert_eq!(projection, before_expired_heartbeat);
        projection
            .claim("worker-b".to_string(), 2_500, 1_500)
            .unwrap();
        assert_eq!(projection.active.as_ref().unwrap().attempt_count, 2);
        assert_eq!(
            projection
                .active
                .as_ref()
                .unwrap()
                .lease
                .as_ref()
                .unwrap()
                .owner,
            "worker-b"
        );
    }
    #[test]
    fn stale_guarded_control_is_durable_rejection_without_stopping_current_goal() {
        let mut projection = GoalProjection::default();
        projection.start("current-goal".into(), "deliver".into(), vec![], vec![requirement("r")], 1).unwrap();
        let revision = projection.revision;
        let stale = GoalRuntimeCommandEnvelope {
            operation_id: "stale-pause".into(),
            command: GoalRuntimeCommand::GuardedControl {
                expected_goal_id: "previous-goal".into(), expected_control_token: projection.control_token().unwrap(),
                control: Box::new(GoalRuntimeCommand::Pause { status: GoalStatus::Paused, wake_at_epoch_ms: None, now_epoch_ms: 2 }),
            },
        };
        projection.apply(stale.clone()).unwrap();
        assert_eq!(projection.active.as_ref().unwrap().status, GoalStatus::Running);
        assert!(projection.applied_operations["stale-pause"].rejection.is_some());
        assert_eq!(projection.revision, revision + 1);
        let committed = projection.clone();
        projection.apply(stale).unwrap();
        assert_eq!(projection, committed);
        projection.apply(GoalRuntimeCommandEnvelope {
            operation_id: "current-pause".into(),
            command: GoalRuntimeCommand::GuardedControl {
                expected_goal_id: "current-goal".into(), expected_control_token: projection.control_token().unwrap(),
                control: Box::new(GoalRuntimeCommand::Pause { status: GoalStatus::Paused, wake_at_epoch_ms: None, now_epoch_ms: 3 }),
            },
        }).unwrap();
        assert_eq!(projection.active.as_ref().unwrap().status, GoalStatus::Paused);
        assert_eq!(projection.applied_operations["current-pause"].rejection, None);
        assert!(!projection.control_observation().unwrap().can_submit);
    }

    #[test]
    fn guarded_control_survives_heartbeat_and_cold_roundtrip() {
        let mut projection = GoalProjection::default();
        projection.start("goal".into(), "deliver".into(), vec![], vec![requirement("r")], 1).unwrap();
        projection.claim("worker".into(), 1000, 2).unwrap();
        let token = projection.control_token().unwrap();
        let revision = projection.revision;
        projection.heartbeat("worker", 2000, 3).unwrap();
        assert!(projection.revision > revision);
        assert_eq!(projection.control_token().as_deref(), Some(token.as_str()));
        let mut cold: GoalProjection = serde_json::from_slice(&serde_json::to_vec(&projection).unwrap()).unwrap();
        cold.apply(GoalRuntimeCommandEnvelope { operation_id: "pause-after-heartbeat".into(),
            command: GoalRuntimeCommand::GuardedControl { expected_goal_id: "goal".into(),
                expected_control_token: token, control: Box::new(GoalRuntimeCommand::Pause {
                    status: GoalStatus::Paused, wake_at_epoch_ms: None, now_epoch_ms: 4 }) } }).unwrap();
        assert_eq!(cold.active.as_ref().unwrap().status, GoalStatus::Paused);
        assert!(cold.applied_operations["pause-after-heartbeat"].rejection.is_none());
    }

    #[test]
    fn guarded_control_rejects_aba_and_preserves_replayed_rejection() {
        let mut projection = GoalProjection::default();
        projection.start("goal".into(), "deliver".into(), vec![], vec![requirement("r")], 1).unwrap();
        let token = projection.control_token().unwrap();
        projection.pause(GoalStatus::Paused, None, 2).unwrap();
        projection.resume(3).unwrap();
        assert_ne!(projection.control_token().as_deref(), Some(token.as_str()));
        let command = GoalRuntimeCommandEnvelope { operation_id: "stale-cancel".into(),
            command: GoalRuntimeCommand::GuardedControl { expected_goal_id: "goal".into(),
                expected_control_token: token, control: Box::new(GoalRuntimeCommand::Cancel { now_epoch_ms: 4 }) } };
        projection.apply(command.clone()).unwrap();
        assert_eq!(projection.active.as_ref().unwrap().status, GoalStatus::Running);
        assert!(projection.applied_operations["stale-cancel"].rejection.is_some());
        let committed = projection.clone();
        projection.apply(command).unwrap();
        assert_eq!(projection, committed);
        let mut invalid = serde_json::to_value(&projection).unwrap();
        invalid.as_object_mut().unwrap().remove("control_generation");
        assert!(serde_json::from_value::<GoalProjection>(invalid).is_err());
    }

    #[test]
    fn control_token_survives_execution_binding_but_owner_remains_exact() {
        let mut projection = GoalProjection::default();
        projection.start("goal".into(), "deliver".into(), vec![], vec![requirement("r")], 1).unwrap();
        let mut planned = contract();
        planned.requirement_versions = [("r".into(), 1)].into();
        planned.relevant_record_ids = vec!["r".into()];
        let id = planned.id.clone();
        projection.plan_contract(planned, 2).unwrap();
        let before = projection.control_token().unwrap();
        projection.begin_execution(&id, "new-run", 3).unwrap();
        let owner = projection.execution_owner("new-run").unwrap();
        assert_eq!(owner.control_token, before);
        assert_eq!(owner.control_token, projection.control_token().unwrap());
        assert!(projection.execution_owner("other-run").is_none());
    }

    #[test]
    fn guarded_pause_survives_complete_internal_round_and_worker_handoff() {
        let mut projection = GoalProjection::default();
        projection.start("goal-1".into(), "deliver".into(), vec![], vec![requirement("requirement-1")], 1).unwrap();
        let token = projection.control_token().unwrap();
        projection.claim("worker".into(), 1000, 2).unwrap();
        projection.plan_contract(contract(), 3).unwrap();
        projection.begin_execution("contract-1", "run-1", 4).unwrap();
        projection.record_execution_report(GoalExecutionReport {
            contract_id: "contract-1".into(), summary: "needs another round".into(),
            claimed_record_ids: vec![], artifact_refs: vec![], blockers: vec![], reported_at_epoch_ms: 5,
        }).unwrap();
        projection.apply_audit(GoalAuditReport {
            assessor: GoalAssessor::ModelTurn { session_id: "fixture-session".into(), turn_id: "fixture-turn".into() },
            id: "audit-1".into(), expected_goal_revision: projection.revision,
            contract_id: "contract-1".into(), verdict: GoalAuditVerdict::Partial,
            summary: "still pending".into(), findings: vec![],
            unmet_requirement_ids: vec!["requirement-1".into()], integrity_violations: vec![],
            evidence: vec![], audited_at_epoch_ms: 6,
        }).unwrap();
        projection.release("worker", None, 7).unwrap();
        projection.claim("next-worker".into(), 2000, 8).unwrap();
        assert_eq!(projection.control_token().as_deref(), Some(token.as_str()));
        assert!(projection.execution_owner("unrelated-run").is_none());
        let mut cold: GoalProjection = serde_json::from_slice(&serde_json::to_vec(&projection).unwrap()).unwrap();
        cold.apply(GoalRuntimeCommandEnvelope { operation_id: "pause-after-round".into(),
            command: GoalRuntimeCommand::GuardedControl { expected_goal_id: "goal-1".into(),
                expected_control_token: token, control: Box::new(GoalRuntimeCommand::Pause {
                    status: GoalStatus::Paused, wake_at_epoch_ms: None, now_epoch_ms: 9 }) } }).unwrap();
        assert_eq!(cold.active.as_ref().unwrap().status, GoalStatus::Paused);
        assert!(cold.applied_operations["pause-after-round"].rejection.is_none());
    }

    #[test]
    fn inadmissible_guarded_control_commits_rejection_without_poisoning_followups() {
        let mut projection = GoalProjection::default();
        projection.start("goal".into(), "deliver".into(), vec![], vec![requirement("r")], 1).unwrap();
        let token = projection.control_token().unwrap();
        let active = projection.active.clone();
        let invalid = GoalRuntimeCommandEnvelope { operation_id: "already-running".into(),
            command: GoalRuntimeCommand::GuardedControl { expected_goal_id: "goal".into(),
                expected_control_token: token.clone(), control: Box::new(GoalRuntimeCommand::Resume { now_epoch_ms: 2 }) } };
        projection.apply(invalid.clone()).unwrap();
        assert_eq!(projection.active, active);
        assert_eq!(projection.control_token().as_deref(), Some(token.as_str()));
        assert_eq!(projection.applied_operations["already-running"].rejection.as_deref(), Some("Goal is not resumable"));
        let committed = projection.clone();
        projection.apply(invalid).unwrap();
        assert_eq!(projection, committed);
        projection.apply(GoalRuntimeCommandEnvelope { operation_id: "next-pause".into(),
            command: GoalRuntimeCommand::GuardedControl { expected_goal_id: "goal".into(),
                expected_control_token: token, control: Box::new(GoalRuntimeCommand::Pause {
                    status: GoalStatus::Paused, wake_at_epoch_ms: None, now_epoch_ms: 3 }) } }).unwrap();
        assert_eq!(projection.active.as_ref().unwrap().status, GoalStatus::Paused);
        assert!(projection.applied_operations["next-pause"].rejection.is_none());
    }

}
