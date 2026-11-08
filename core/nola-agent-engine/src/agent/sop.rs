// SPDX-License-Identifier: Apache-2.0
//! Versioned, platform-neutral standard operating procedure contracts.
//!
//! A SOP describes execution structure and validation. It does not select a
//! SOP from user text, authorize effects, or execute Host capabilities.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const SOP_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopDefinition {
    pub schema_version: u32,
    pub id: String,
    pub version: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default = "empty_object")]
    pub inputs: Value,
    pub steps: Vec<SopStep>,
    #[serde(default)]
    pub verification: SopVerificationContract,
    #[serde(default)]
    pub compensation: SopCompensationContract,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopStep {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub mode: SopExecutionMode,
    #[serde(default)]
    pub concurrent: bool,
    #[serde(default = "empty_object")]
    pub inputs: Value,
    #[serde(default = "empty_object")]
    pub outputs: Value,
    #[serde(default)]
    pub preconditions: Vec<SopCondition>,
    pub completion: SopCompletionContract,
    #[serde(default)]
    pub required_capabilities: Vec<SopCapabilityRequirement>,
    #[serde(default)]
    pub required_permissions: Vec<String>,
    #[serde(default)]
    pub budget: SopStepBudget,
    #[serde(default)]
    pub retry: SopRetryPolicy,
    #[serde(default)]
    pub artifacts: Vec<SopArtifactContract>,
    #[serde(default)]
    pub verifier: Option<SopVerifierContract>,
    #[serde(default)]
    pub resume_key: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SopExecutionMode {
    #[default]
    Inline,
    Isolated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopCondition {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopCompletionContract {
    pub description: String,
    #[serde(default)]
    pub evidence_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopCapabilityRequirement {
    pub kind: SopCapabilityKind,
    pub id: String,
    #[serde(default)]
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SopCapabilityKind {
    Tool,
    Skill,
    Mcp,
    Platform,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopStepBudget {
    #[serde(default)]
    pub max_model_turns: Option<u32>,
    #[serde(default)]
    pub max_tool_calls: Option<u32>,
    #[serde(default)]
    pub max_elapsed_millis: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopRetryPolicy {
    pub max_attempts: u32,
    #[serde(default)]
    pub initial_backoff_millis: u64,
    #[serde(default)]
    pub max_backoff_millis: Option<u64>,
    #[serde(default)]
    pub retryable_errors: Vec<String>,
}

impl Default for SopRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            initial_backoff_millis: 0,
            max_backoff_millis: None,
            retryable_errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopArtifactContract {
    pub id: String,
    pub media_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopVerifierContract {
    pub id: String,
    pub kind: SopVerifierKind,
    #[serde(default)]
    pub capability: Option<SopCapabilityRequirement>,
    #[serde(default = "empty_object")]
    pub configuration: Value,
    pub success: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SopVerifierKind {
    Tool,
    Artifact,
    Evidence,
    UserAcceptance,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopVerificationContract {
    #[serde(default)]
    pub required_artifacts: Vec<String>,
    #[serde(default)]
    pub verifiers: Vec<SopVerifierContract>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopCompensationContract {
    #[serde(default)]
    pub actions: Vec<SopCompensationAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopCompensationAction {
    pub after_step: String,
    pub description: String,
    #[serde(default)]
    pub required_capabilities: Vec<SopCapabilityRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopValidationError {
    pub code: String,
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for SopValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.code, self.path, self.message
        )
    }
}

impl std::error::Error for SopValidationError {}

impl SopDefinition {
    pub fn validate(&self) -> Result<(), SopValidationError> {
        if self.schema_version != SOP_SCHEMA_VERSION {
            return invalid(
                "unsupported_schema_version",
                "schemaVersion",
                format!(
                    "expected schema version {SOP_SCHEMA_VERSION}, got {}",
                    self.schema_version
                ),
            );
        }
        require_id(&self.id, "id")?;
        if self.version == 0 {
            return invalid("invalid_version", "version", "version must be positive");
        }
        if self.steps.is_empty() {
            return invalid(
                "empty_steps",
                "steps",
                "a SOP must contain at least one step",
            );
        }

        let mut step_ids = BTreeSet::new();
        let mut artifact_ids = BTreeSet::new();
        for (index, step) in self.steps.iter().enumerate() {
            let path = format!("steps[{index}]");
            require_id(&step.id, &format!("{path}.id"))?;
            if !step_ids.insert(step.id.clone()) {
                return invalid(
                    "duplicate_step_id",
                    format!("{path}.id"),
                    format!("duplicate step id `{}`", step.id),
                );
            }
            if step.title.trim().is_empty() {
                return invalid(
                    "empty_title",
                    format!("{path}.title"),
                    "title must not be empty",
                );
            }
            if step.completion.description.trim().is_empty() {
                return invalid(
                    "empty_completion",
                    format!("{path}.completion.description"),
                    "completion description must not be empty",
                );
            }
            validate_step_limits(step, &path)?;
            let mut condition_ids = BTreeSet::new();
            for (condition_index, condition) in step.preconditions.iter().enumerate() {
                let condition_path = format!("{path}.preconditions[{condition_index}]");
                require_id(&condition.id, &format!("{condition_path}.id"))?;
                if condition.description.trim().is_empty() {
                    return invalid(
                        "empty_condition",
                        format!("{condition_path}.description"),
                        "condition description must not be empty",
                    );
                }
                if !condition_ids.insert(condition.id.clone()) {
                    return invalid(
                        "duplicate_condition_id",
                        format!("{condition_path}.id"),
                        "condition identifiers must be unique within a step",
                    );
                }
            }
            validate_capabilities(
                &step.required_capabilities,
                &format!("{path}.requiredCapabilities"),
            )?;
            for (artifact_index, artifact) in step.artifacts.iter().enumerate() {
                let artifact_path = format!("{path}.artifacts[{artifact_index}]");
                require_id(&artifact.id, &format!("{artifact_path}.id"))?;
                if artifact.media_type.trim().is_empty() {
                    return invalid(
                        "empty_media_type",
                        format!("{artifact_path}.mediaType"),
                        "artifact media type must not be empty",
                    );
                }
                if !artifact_ids.insert(artifact.id.clone()) {
                    return invalid(
                        "duplicate_artifact_id",
                        format!("{artifact_path}.id"),
                        format!("duplicate artifact id `{}`", artifact.id),
                    );
                }
            }
            if let Some(verifier) = step.verifier.as_ref() {
                validate_verifier(verifier, &format!("{path}.verifier"))?;
            }
        }

        for (index, step) in self.steps.iter().enumerate() {
            let mut dependencies = BTreeSet::new();
            for (dependency_index, dependency) in step.depends_on.iter().enumerate() {
                let path = format!("steps[{index}].dependsOn[{dependency_index}]");
                if dependency == &step.id {
                    return invalid("self_dependency", path, "a step cannot depend on itself");
                }
                if !step_ids.contains(dependency) {
                    return invalid(
                        "unknown_dependency",
                        path,
                        format!("unknown step `{dependency}`"),
                    );
                }
                if !dependencies.insert(dependency) {
                    return invalid(
                        "duplicate_dependency",
                        path,
                        format!("duplicate dependency `{dependency}`"),
                    );
                }
            }
        }
        validate_acyclic(&self.steps)?;

        for (index, artifact_id) in self.verification.required_artifacts.iter().enumerate() {
            if !artifact_ids.contains(artifact_id) {
                return invalid(
                    "unknown_artifact",
                    format!("verification.requiredArtifacts[{index}]"),
                    format!("unknown artifact `{artifact_id}`"),
                );
            }
        }
        let mut verifier_ids = BTreeSet::new();
        for (index, verifier) in self.verification.verifiers.iter().enumerate() {
            validate_verifier(verifier, &format!("verification.verifiers[{index}]"))?;
            if !verifier_ids.insert(verifier.id.clone()) {
                return invalid(
                    "duplicate_verifier_id",
                    format!("verification.verifiers[{index}].id"),
                    format!("duplicate verifier id `{}`", verifier.id),
                );
            }
        }
        let mut compensated_steps = BTreeSet::new();
        for (index, action) in self.compensation.actions.iter().enumerate() {
            let path = format!("compensation.actions[{index}]");
            if !step_ids.contains(&action.after_step) {
                return invalid(
                    "unknown_compensation_step",
                    format!("{path}.afterStep"),
                    format!("unknown step `{}`", action.after_step),
                );
            }
            if action.description.trim().is_empty() {
                return invalid(
                    "empty_compensation",
                    format!("{path}.description"),
                    "compensation description must not be empty",
                );
            }
            validate_capabilities(
                &action.required_capabilities,
                &format!("{path}.requiredCapabilities"),
            )?;
            if !compensated_steps.insert(action.after_step.clone()) {
                return invalid(
                    "duplicate_compensation_step",
                    format!("{path}.afterStep"),
                    "only one compensation action may be declared per step",
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SopRunStatus {
    Queued,
    Running,
    WaitingPermission,
    WaitingUser,
    WaitingExternal,
    Paused,
    Verifying,
    Compensating,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SopStepRunStatus {
    Queued,
    Running,
    Waiting,
    Verifying,
    Completed,
    Failed,
    Skipped,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SopConditionDecision {
    Pending,
    Passed,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SopWaitKind {
    Permission,
    UserInput,
    External,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SopWaitDisposition {
    Continue,
    Replan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopWaitRecord {
    pub kind: SopWaitKind,
    pub reference_id: String,
    pub reason: String,
    pub requested_at_epoch_ms: i64,
    #[serde(default)]
    pub disposition: Option<SopWaitDisposition>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub resolved_at_epoch_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopStepAttemptRecord {
    pub attempt: u32,
    pub started_at_epoch_ms: i64,
    #[serde(default)]
    pub ended_at_epoch_ms: Option<i64>,
    #[serde(default)]
    pub executor_ref: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopVerifierResult {
    pub verifier_id: String,
    pub passed: bool,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub completed_at_epoch_ms: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SopCompensationRunStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopCompensationRunRecord {
    pub after_step: String,
    pub status: SopCompensationRunStatus,
    pub attempt: u32,
    #[serde(default)]
    pub executor_ref: Option<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopRunRecord {
    pub run_id: String,
    pub sop_id: String,
    pub sop_version: u32,
    pub status: SopRunStatus,
    pub revision: u64,
    #[serde(default = "empty_object")]
    pub inputs: Value,
    pub steps: BTreeMap<String, SopStepRunRecord>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub verifier_results: BTreeMap<String, SopVerifierResult>,
    #[serde(default)]
    pub compensations: BTreeMap<String, SopCompensationRunRecord>,
    #[serde(default)]
    pub failure: Option<String>,
    pub created_at_epoch_ms: i64,
    pub updated_at_epoch_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopStepRunRecord {
    pub step_id: String,
    pub status: SopStepRunStatus,
    pub attempt: u32,
    #[serde(default)]
    pub attempts: Vec<SopStepAttemptRecord>,
    #[serde(default)]
    pub condition_decisions: BTreeMap<String, SopConditionDecision>,
    #[serde(default)]
    pub next_attempt_at_epoch_ms: Option<i64>,
    #[serde(default)]
    pub executor_ref: Option<String>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub resume_key: Option<String>,
    #[serde(default)]
    pub current_wait: Option<SopWaitRecord>,
    #[serde(default)]
    pub wait_history: Vec<SopWaitRecord>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub verifier_result: Option<SopVerifierResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SopRunEvent {
    ResolveCondition {
        step_id: String,
        condition_id: String,
        passed: bool,
    },
    StartStep {
        step_id: String,
        executor_ref: String,
    },
    SuspendStep {
        step_id: String,
        wait: SopWaitRecord,
    },
    ResumeStep {
        step_id: String,
        disposition: SopWaitDisposition,
        #[serde(default)]
        evidence_refs: Vec<String>,
    },
    CompleteStep {
        step_id: String,
        #[serde(default)]
        artifact_refs: Vec<String>,
        #[serde(default)]
        evidence_refs: Vec<String>,
    },
    FailStep {
        step_id: String,
        error_code: String,
        error: String,
    },
    CompleteStepVerification {
        step_id: String,
        passed: bool,
        #[serde(default)]
        evidence_refs: Vec<String>,
        #[serde(default)]
        error: Option<String>,
    },
    CompleteRunVerification {
        verifier_id: String,
        passed: bool,
        #[serde(default)]
        evidence_refs: Vec<String>,
        #[serde(default)]
        error: Option<String>,
    },
    StartCompensation {
        after_step: String,
        executor_ref: String,
    },
    CompleteCompensation {
        after_step: String,
        #[serde(default)]
        evidence_refs: Vec<String>,
    },
    FailCompensation {
        after_step: String,
        error: String,
    },
    Pause,
    Resume,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopRunTransitionError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for SopRunTransitionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SopRunTransitionError {}

impl SopRunRecord {
    pub fn new(
        definition: &SopDefinition,
        run_id: String,
        inputs: Value,
        now_epoch_ms: i64,
    ) -> Result<Self, SopValidationError> {
        definition.validate()?;
        require_id(&run_id, "runId")?;
        let steps = definition
            .steps
            .iter()
            .map(|step| {
                (
                    step.id.clone(),
                    SopStepRunRecord {
                        step_id: step.id.clone(),
                        status: SopStepRunStatus::Queued,
                        attempt: 0,
                        attempts: Vec::new(),
                        condition_decisions: step
                            .preconditions
                            .iter()
                            .map(|condition| (condition.id.clone(), SopConditionDecision::Pending))
                            .collect(),
                        next_attempt_at_epoch_ms: None,
                        executor_ref: None,
                        artifact_refs: Vec::new(),
                        evidence_refs: Vec::new(),
                        resume_key: step.resume_key.clone(),
                        current_wait: None,
                        wait_history: Vec::new(),
                        error: None,
                        verifier_result: None,
                    },
                )
            })
            .collect();
        Ok(Self {
            run_id,
            sop_id: definition.id.clone(),
            sop_version: definition.version,
            status: SopRunStatus::Queued,
            revision: 1,
            inputs,
            steps,
            artifact_refs: Vec::new(),
            evidence_refs: Vec::new(),
            verifier_results: BTreeMap::new(),
            compensations: BTreeMap::new(),
            failure: None,
            created_at_epoch_ms: now_epoch_ms,
            updated_at_epoch_ms: now_epoch_ms,
        })
    }

    pub fn runnable_step_ids(&self, definition: &SopDefinition) -> Vec<String> {
        self.runnable_step_ids_at(definition, self.updated_at_epoch_ms)
    }

    pub fn runnable_step_ids_at(
        &self,
        definition: &SopDefinition,
        now_epoch_ms: i64,
    ) -> Vec<String> {
        definition
            .steps
            .iter()
            .filter(|step| {
                self.steps
                    .get(&step.id)
                    .is_some_and(|run| run.status == SopStepRunStatus::Queued)
                    && self.steps.get(&step.id).is_some_and(|run| {
                        run.next_attempt_at_epoch_ms
                            .is_none_or(|retry_at| retry_at <= now_epoch_ms)
                            && run
                                .condition_decisions
                                .values()
                                .all(|decision| *decision == SopConditionDecision::Passed)
                    })
                    && step.depends_on.iter().all(|dependency| {
                        self.steps.get(dependency).is_some_and(|run| {
                            matches!(
                                run.status,
                                SopStepRunStatus::Completed | SopStepRunStatus::Skipped
                            )
                        })
                    })
            })
            .map(|step| step.id.clone())
            .collect()
    }

    pub fn apply(
        &mut self,
        definition: &SopDefinition,
        event: SopRunEvent,
        now_epoch_ms: i64,
    ) -> Result<(), SopRunTransitionError> {
        self.validate_identity(definition)?;
        if now_epoch_ms < self.updated_at_epoch_ms {
            return transition_error("stale_timestamp", "event timestamp precedes the run");
        }
        if matches!(
            self.status,
            SopRunStatus::Completed | SopRunStatus::Failed | SopRunStatus::Cancelled
        ) {
            return transition_error("terminal_run", "a terminal SOP run cannot transition");
        }

        match event {
            SopRunEvent::ResolveCondition {
                step_id,
                condition_id,
                passed,
            } => {
                let step = self.queued_step_mut(&step_id)?;
                let decision = step
                    .condition_decisions
                    .get_mut(&condition_id)
                    .ok_or_else(|| transition("unknown_condition", &condition_id))?;
                if *decision != SopConditionDecision::Pending {
                    return transition_error(
                        "condition_already_resolved",
                        "condition decisions are immutable",
                    );
                }
                *decision = if passed {
                    SopConditionDecision::Passed
                } else {
                    SopConditionDecision::Failed
                };
            }
            SopRunEvent::StartStep {
                step_id,
                executor_ref,
            } => {
                if !self
                    .runnable_step_ids_at(definition, now_epoch_ms)
                    .contains(&step_id)
                {
                    return transition_error(
                        "step_not_runnable",
                        "step dependencies, conditions, or retry delay are not satisfied",
                    );
                }
                if executor_ref.trim().is_empty() {
                    return transition_error(
                        "empty_executor_ref",
                        "executor reference must not be empty",
                    );
                }
                let step = self.step_mut(&step_id)?;
                step.attempt += 1;
                step.status = SopStepRunStatus::Running;
                step.executor_ref = Some(executor_ref.clone());
                step.next_attempt_at_epoch_ms = None;
                step.error = None;
                step.attempts.push(SopStepAttemptRecord {
                    attempt: step.attempt,
                    started_at_epoch_ms: now_epoch_ms,
                    ended_at_epoch_ms: None,
                    executor_ref: Some(executor_ref),
                    error_code: None,
                    error: None,
                });
                self.status = SopRunStatus::Running;
            }
            SopRunEvent::SuspendStep { step_id, wait } => {
                if wait.reference_id.trim().is_empty() {
                    return transition_error(
                        "empty_wait_reference",
                        "a suspended step must reference the authoritative pending request",
                    );
                }
                if wait.reason.trim().is_empty() {
                    return transition_error(
                        "empty_wait_reason",
                        "a suspended step must explain what it is waiting for",
                    );
                }
                if wait.requested_at_epoch_ms > now_epoch_ms
                    || wait.disposition.is_some()
                    || wait.resolved_at_epoch_ms.is_some()
                    || !wait.evidence_refs.is_empty()
                {
                    return transition_error("invalid_open_wait", "a new wait must be unresolved and cannot contain future or resolution evidence");
                }
                let run_status = match wait.kind {
                    SopWaitKind::Permission => SopRunStatus::WaitingPermission,
                    SopWaitKind::UserInput => SopRunStatus::WaitingUser,
                    SopWaitKind::External => SopRunStatus::WaitingExternal,
                };
                let step = self.step_with_status_mut(&step_id, &[SopStepRunStatus::Running])?;
                step.status = SopStepRunStatus::Waiting;
                step.current_wait = Some(wait);
                self.status = run_status;
            }
            SopRunEvent::ResumeStep {
                step_id,
                disposition,
                evidence_refs,
            } => {
                {
                    let step = self.step_with_status_mut(&step_id, &[SopStepRunStatus::Waiting])?;
                    let mut wait = step.current_wait.take().ok_or_else(|| {
                        transition(
                            "missing_wait_record",
                            "a waiting step cannot resume without its durable wait identity",
                        )
                    })?;
                    wait.disposition = Some(disposition);
                    wait.resolved_at_epoch_ms = Some(now_epoch_ms);
                    append_unique(&mut wait.evidence_refs, evidence_refs.clone());
                    step.wait_history.push(wait);
                    append_unique(&mut step.evidence_refs, evidence_refs.clone());
                    step.status = SopStepRunStatus::Running;
                }
                append_unique(&mut self.evidence_refs, evidence_refs);
                self.status = SopRunStatus::Running;
            }
            SopRunEvent::CompleteStep {
                step_id,
                artifact_refs,
                evidence_refs,
            } => {
                let contract = definition_step(definition, &step_id)?;
                if contract.completion.evidence_required && evidence_refs.is_empty() {
                    return transition_error(
                        "missing_step_evidence",
                        "step completion requires evidence",
                    );
                }
                let required_artifacts = contract
                    .artifacts
                    .iter()
                    .filter(|artifact| artifact.required)
                    .count();
                if artifact_refs.len() < required_artifacts {
                    return transition_error(
                        "missing_step_artifact",
                        "step completion is missing required artifacts",
                    );
                }
                let step = self.step_with_status_mut(&step_id, &[SopStepRunStatus::Running])?;
                finish_current_attempt(step, now_epoch_ms, None, None)?;
                append_unique(&mut step.artifact_refs, artifact_refs.clone());
                append_unique(&mut step.evidence_refs, evidence_refs.clone());
                step.status = if contract.verifier.is_some() {
                    SopStepRunStatus::Verifying
                } else {
                    SopStepRunStatus::Completed
                };
                append_unique(&mut self.artifact_refs, artifact_refs);
                append_unique(&mut self.evidence_refs, evidence_refs);
            }
            SopRunEvent::FailStep {
                step_id,
                error_code,
                error,
            } => self.fail_step(definition, &step_id, error_code, error, now_epoch_ms)?,
            SopRunEvent::CompleteStepVerification {
                step_id,
                passed,
                evidence_refs,
                error,
            } => {
                let contract = definition_step(definition, &step_id)?;
                let verifier = contract
                    .verifier
                    .as_ref()
                    .ok_or_else(|| transition("missing_step_verifier", &step_id))?;
                let result = SopVerifierResult {
                    verifier_id: verifier.id.clone(),
                    passed,
                    evidence_refs: evidence_refs.clone(),
                    error: error.clone(),
                    completed_at_epoch_ms: now_epoch_ms,
                };
                {
                    let step =
                        self.step_with_status_mut(&step_id, &[SopStepRunStatus::Verifying])?;
                    step.verifier_result = Some(result);
                    append_unique(&mut step.evidence_refs, evidence_refs.clone());
                    if passed {
                        step.status = SopStepRunStatus::Completed;
                    }
                }
                append_unique(&mut self.evidence_refs, evidence_refs);
                if !passed {
                    self.fail_step(
                        definition,
                        &step_id,
                        "verification_failed".into(),
                        error.unwrap_or_else(|| "step verification failed".into()),
                        now_epoch_ms,
                    )?;
                }
            }
            SopRunEvent::CompleteRunVerification {
                verifier_id,
                passed,
                evidence_refs,
                error,
            } => {
                if self.status != SopRunStatus::Verifying {
                    return transition_error(
                        "run_not_verifying",
                        "run verifier result is not expected",
                    );
                }
                if !definition
                    .verification
                    .verifiers
                    .iter()
                    .any(|item| item.id == verifier_id)
                {
                    return transition_error(
                        "unknown_run_verifier",
                        "run verifier is not declared",
                    );
                }
                if self.verifier_results.contains_key(&verifier_id) {
                    return transition_error(
                        "verifier_already_completed",
                        "verifier result is immutable",
                    );
                }
                self.verifier_results.insert(
                    verifier_id.clone(),
                    SopVerifierResult {
                        verifier_id,
                        passed,
                        evidence_refs: evidence_refs.clone(),
                        error: error.clone(),
                        completed_at_epoch_ms: now_epoch_ms,
                    },
                );
                append_unique(&mut self.evidence_refs, evidence_refs);
                if !passed {
                    self.enter_compensation(
                        definition,
                        error.unwrap_or_else(|| "run verification failed".into()),
                        now_epoch_ms,
                    );
                }
            }
            SopRunEvent::StartCompensation {
                after_step,
                executor_ref,
            } => {
                if self.status != SopRunStatus::Compensating {
                    return transition_error("run_not_compensating", "compensation is not active");
                }
                let item = self.compensation_mut(&after_step, SopCompensationRunStatus::Queued)?;
                item.status = SopCompensationRunStatus::Running;
                item.attempt += 1;
                item.executor_ref = Some(executor_ref);
            }
            SopRunEvent::CompleteCompensation {
                after_step,
                evidence_refs,
            } => {
                let item = self.compensation_mut(&after_step, SopCompensationRunStatus::Running)?;
                item.status = SopCompensationRunStatus::Completed;
                append_unique(&mut item.evidence_refs, evidence_refs.clone());
                append_unique(&mut self.evidence_refs, evidence_refs);
            }
            SopRunEvent::FailCompensation { after_step, error } => {
                let item = self.compensation_mut(&after_step, SopCompensationRunStatus::Running)?;
                item.status = SopCompensationRunStatus::Failed;
                item.error = Some(error);
            }
            SopRunEvent::Pause => self.status = SopRunStatus::Paused,
            SopRunEvent::Resume => {
                if self.status != SopRunStatus::Paused {
                    return transition_error("run_not_paused", "only a paused SOP run can resume");
                }
                self.status = SopRunStatus::Running;
            }
            SopRunEvent::Cancel => {
                self.status = SopRunStatus::Cancelled;
                for step in self.steps.values_mut().filter(|step| {
                    matches!(
                        step.status,
                        SopStepRunStatus::Queued
                            | SopStepRunStatus::Running
                            | SopStepRunStatus::Waiting
                            | SopStepRunStatus::Verifying
                    )
                }) {
                    step.status = SopStepRunStatus::Cancelled;
                }
            }
        }
        self.updated_at_epoch_ms = now_epoch_ms;
        self.revision = self.revision.saturating_add(1);
        self.refresh(definition, now_epoch_ms);
        Ok(())
    }

    fn validate_identity(&self, definition: &SopDefinition) -> Result<(), SopRunTransitionError> {
        definition
            .validate()
            .map_err(|error| transition("invalid_definition", error.to_string()))?;
        if self.sop_id != definition.id || self.sop_version != definition.version {
            return transition_error(
                "definition_mismatch",
                "run is pinned to another SOP identity or version",
            );
        }
        Ok(())
    }

    fn queued_step_mut(
        &mut self,
        step_id: &str,
    ) -> Result<&mut SopStepRunRecord, SopRunTransitionError> {
        self.step_with_status_mut(step_id, &[SopStepRunStatus::Queued])
    }

    fn step_mut(&mut self, step_id: &str) -> Result<&mut SopStepRunRecord, SopRunTransitionError> {
        self.steps
            .get_mut(step_id)
            .ok_or_else(|| transition("unknown_step", step_id))
    }

    fn step_with_status_mut(
        &mut self,
        step_id: &str,
        expected: &[SopStepRunStatus],
    ) -> Result<&mut SopStepRunRecord, SopRunTransitionError> {
        let step = self.step_mut(step_id)?;
        if !expected.contains(&step.status) {
            return transition_error("invalid_step_status", "step is not in the required state");
        }
        Ok(step)
    }

    fn fail_step(
        &mut self,
        definition: &SopDefinition,
        step_id: &str,
        error_code: String,
        error: String,
        now_epoch_ms: i64,
    ) -> Result<(), SopRunTransitionError> {
        let contract = definition_step(definition, step_id)?;
        let step = self.step_with_status_mut(
            step_id,
            &[SopStepRunStatus::Running, SopStepRunStatus::Verifying],
        )?;
        if step
            .attempts
            .last()
            .is_some_and(|attempt| attempt.ended_at_epoch_ms.is_none())
        {
            finish_current_attempt(
                step,
                now_epoch_ms,
                Some(error_code.clone()),
                Some(error.clone()),
            )?;
        } else if let Some(attempt) = step.attempts.last_mut() {
            attempt.error_code = Some(error_code.clone());
            attempt.error = Some(error.clone());
        }
        step.error = Some(error.clone());
        let retryable = contract.retry.retryable_errors.is_empty()
            || contract.retry.retryable_errors.contains(&error_code);
        if retryable && step.attempt < contract.retry.max_attempts {
            let exponent = step.attempt.saturating_sub(1).min(31);
            let delay = contract
                .retry
                .initial_backoff_millis
                .saturating_mul(1_u64 << exponent);
            let delay = contract
                .retry
                .max_backoff_millis
                .map_or(delay, |maximum| delay.min(maximum));
            step.status = SopStepRunStatus::Queued;
            step.next_attempt_at_epoch_ms = Some(now_epoch_ms.saturating_add(delay as i64));
            self.status = SopRunStatus::Running;
        } else {
            step.status = SopStepRunStatus::Failed;
            self.enter_compensation(definition, error, now_epoch_ms);
        }
        Ok(())
    }

    fn enter_compensation(
        &mut self,
        definition: &SopDefinition,
        failure: String,
        now_epoch_ms: i64,
    ) {
        self.failure = Some(failure);
        for step in self.steps.values_mut().filter(|step| {
            matches!(
                step.status,
                SopStepRunStatus::Queued
                    | SopStepRunStatus::Running
                    | SopStepRunStatus::Waiting
                    | SopStepRunStatus::Verifying
            )
        }) {
            if let Some(attempt) = step
                .attempts
                .last_mut()
                .filter(|attempt| attempt.ended_at_epoch_ms.is_none())
            {
                attempt.ended_at_epoch_ms = Some(now_epoch_ms);
                attempt.error_code = Some("cancelled_due_to_run_failure".into());
                attempt.error =
                    Some("another SOP step caused the run to enter compensation".into());
            }
            step.status = SopStepRunStatus::Cancelled;
            step.next_attempt_at_epoch_ms = None;
        }
        self.compensations = definition
            .compensation
            .actions
            .iter()
            .filter(|action| {
                self.steps
                    .get(&action.after_step)
                    .is_some_and(|step| step.status == SopStepRunStatus::Completed)
            })
            .map(|action| {
                (
                    action.after_step.clone(),
                    SopCompensationRunRecord {
                        after_step: action.after_step.clone(),
                        status: SopCompensationRunStatus::Queued,
                        attempt: 0,
                        executor_ref: None,
                        evidence_refs: Vec::new(),
                        error: None,
                    },
                )
            })
            .collect();
        self.status = if self.compensations.is_empty() {
            SopRunStatus::Failed
        } else {
            SopRunStatus::Compensating
        };
    }

    fn compensation_mut(
        &mut self,
        after_step: &str,
        expected: SopCompensationRunStatus,
    ) -> Result<&mut SopCompensationRunRecord, SopRunTransitionError> {
        let item = self
            .compensations
            .get_mut(after_step)
            .ok_or_else(|| transition("unknown_compensation", after_step))?;
        if item.status != expected {
            return transition_error(
                "invalid_compensation_status",
                "compensation is not in the required state",
            );
        }
        Ok(item)
    }

    fn refresh(&mut self, definition: &SopDefinition, now_epoch_ms: i64) {
        if matches!(
            self.status,
            SopRunStatus::Completed
                | SopRunStatus::Failed
                | SopRunStatus::Cancelled
                | SopRunStatus::Paused
        ) {
            return;
        }
        for step in self
            .steps
            .values_mut()
            .filter(|step| step.status == SopStepRunStatus::Queued)
        {
            if step
                .condition_decisions
                .values()
                .any(|decision| *decision == SopConditionDecision::Failed)
            {
                step.status = SopStepRunStatus::Skipped;
            }
        }
        if self.status == SopRunStatus::Compensating {
            if self.compensations.values().all(|item| {
                matches!(
                    item.status,
                    SopCompensationRunStatus::Completed | SopCompensationRunStatus::Failed
                )
            }) {
                self.status = SopRunStatus::Failed;
            }
            return;
        }
        if self.steps.values().all(|step| {
            matches!(
                step.status,
                SopStepRunStatus::Completed | SopStepRunStatus::Skipped
            )
        }) {
            let artifacts_ready =
                definition
                    .verification
                    .required_artifacts
                    .iter()
                    .all(|required| {
                        self.artifact_refs.iter().any(|reference| {
                            reference == required || reference.starts_with(&format!("{required}:"))
                        })
                    });
            if !artifacts_ready {
                self.enter_compensation(
                    definition,
                    "required run artifact is missing".into(),
                    now_epoch_ms,
                );
            } else if definition.verification.verifiers.is_empty() {
                self.status = SopRunStatus::Completed;
            } else if definition.verification.verifiers.iter().all(|verifier| {
                self.verifier_results
                    .get(&verifier.id)
                    .is_some_and(|result| result.passed)
            }) {
                self.status = SopRunStatus::Completed;
            } else {
                self.status = SopRunStatus::Verifying;
            }
        }
    }
}

fn definition_step<'a>(
    definition: &'a SopDefinition,
    step_id: &str,
) -> Result<&'a SopStep, SopRunTransitionError> {
    definition
        .steps
        .iter()
        .find(|step| step.id == step_id)
        .ok_or_else(|| transition("unknown_step", step_id))
}

fn finish_current_attempt(
    step: &mut SopStepRunRecord,
    now_epoch_ms: i64,
    error_code: Option<String>,
    error: Option<String>,
) -> Result<(), SopRunTransitionError> {
    let attempt = step
        .attempts
        .last_mut()
        .ok_or_else(|| transition("missing_attempt", &step.step_id))?;
    if attempt.ended_at_epoch_ms.is_some() {
        return transition_error(
            "attempt_already_finished",
            "attempt has already reached a terminal state",
        );
    }
    attempt.ended_at_epoch_ms = Some(now_epoch_ms);
    attempt.error_code = error_code;
    attempt.error = error;
    Ok(())
}

fn append_unique(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

fn transition(code: impl Into<String>, message: impl Into<String>) -> SopRunTransitionError {
    SopRunTransitionError {
        code: code.into(),
        message: message.into(),
    }
}

fn transition_error<T>(
    code: impl Into<String>,
    message: impl Into<String>,
) -> Result<T, SopRunTransitionError> {
    Err(transition(code, message))
}

fn validate_step_limits(step: &SopStep, path: &str) -> Result<(), SopValidationError> {
    if step.retry.max_attempts == 0 {
        return invalid(
            "invalid_retry_attempts",
            format!("{path}.retry.maxAttempts"),
            "max attempts must be positive",
        );
    }
    if step
        .retry
        .max_backoff_millis
        .is_some_and(|maximum| maximum < step.retry.initial_backoff_millis)
    {
        return invalid(
            "invalid_retry_backoff",
            format!("{path}.retry.maxBackoffMillis"),
            "maximum backoff must be greater than or equal to initial backoff",
        );
    }
    for (name, value) in [
        ("maxModelTurns", step.budget.max_model_turns.map(u64::from)),
        ("maxToolCalls", step.budget.max_tool_calls.map(u64::from)),
        ("maxElapsedMillis", step.budget.max_elapsed_millis),
    ] {
        if value == Some(0) {
            return invalid(
                "invalid_budget",
                format!("{path}.budget.{name}"),
                "an explicit budget must be positive",
            );
        }
    }
    if step
        .resume_key
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return invalid(
            "empty_resume_key",
            format!("{path}.resumeKey"),
            "resume key must not be empty",
        );
    }
    Ok(())
}

fn validate_verifier(verifier: &SopVerifierContract, path: &str) -> Result<(), SopValidationError> {
    require_id(&verifier.id, &format!("{path}.id"))?;
    if verifier.success.trim().is_empty() {
        return invalid(
            "empty_verifier_success",
            format!("{path}.success"),
            "verifier success condition must not be empty",
        );
    }
    if matches!(verifier.kind, SopVerifierKind::Tool) && verifier.capability.is_none() {
        return invalid(
            "missing_verifier_capability",
            format!("{path}.capability"),
            "tool verifier requires a capability",
        );
    }
    if let Some(capability) = verifier.capability.as_ref() {
        validate_capabilities(
            std::slice::from_ref(capability),
            &format!("{path}.capability"),
        )?;
    }
    Ok(())
}

fn validate_capabilities(
    capabilities: &[SopCapabilityRequirement],
    path: &str,
) -> Result<(), SopValidationError> {
    let mut identities = BTreeSet::new();
    for (index, capability) in capabilities.iter().enumerate() {
        let item_path = format!("{path}[{index}]");
        require_id(&capability.id, &format!("{item_path}.id"))?;
        if matches!(capability.kind, SopCapabilityKind::Mcp)
            && capability
                .server_id
                .as_ref()
                .is_none_or(|server| server.trim().is_empty())
        {
            return invalid(
                "missing_mcp_server",
                format!("{item_path}.serverId"),
                "MCP capability requires a server id",
            );
        }
        let identity = format!(
            "{:?}:{}:{:?}",
            capability.kind, capability.id, capability.server_id
        );
        if !identities.insert(identity) {
            return invalid(
                "duplicate_capability",
                item_path,
                "duplicate capability requirement",
            );
        }
    }
    Ok(())
}

fn validate_acyclic(steps: &[SopStep]) -> Result<(), SopValidationError> {
    fn visit(
        id: &str,
        dependencies: &BTreeMap<&str, Vec<&str>>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
    ) -> bool {
        if visited.contains(id) {
            return true;
        }
        if !visiting.insert(id.to_string()) {
            return false;
        }
        for dependency in dependencies.get(id).into_iter().flatten() {
            if !visit(dependency, dependencies, visiting, visited) {
                return false;
            }
        }
        visiting.remove(id);
        visited.insert(id.to_string());
        true
    }

    let dependencies = steps
        .iter()
        .map(|step| {
            (
                step.id.as_str(),
                step.depends_on
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for step in steps {
        if !visit(&step.id, &dependencies, &mut visiting, &mut visited) {
            return invalid(
                "dependency_cycle",
                "steps",
                format!("dependency cycle includes `{}`", step.id),
            );
        }
    }
    Ok(())
}

fn require_id(value: &str, path: &str) -> Result<(), SopValidationError> {
    if value.trim().is_empty() {
        return invalid("empty_id", path, "identifier must not be empty");
    }
    if value.chars().any(char::is_whitespace) {
        return invalid("invalid_id", path, "identifier must not contain whitespace");
    }
    Ok(())
}

fn invalid<T>(
    code: impl Into<String>,
    path: impl Into<String>,
    message: impl Into<String>,
) -> Result<T, SopValidationError> {
    Err(SopValidationError {
        code: code.into(),
        path: path.into(),
        message: message.into(),
    })
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(id: &str, dependencies: &[&str]) -> SopStep {
        SopStep {
            id: id.to_string(),
            title: id.to_string(),
            depends_on: dependencies.iter().map(|value| value.to_string()).collect(),
            mode: SopExecutionMode::Inline,
            concurrent: false,
            inputs: json!({}),
            outputs: json!({}),
            preconditions: Vec::new(),
            completion: SopCompletionContract {
                description: format!("{id} is complete"),
                evidence_required: true,
            },
            required_capabilities: Vec::new(),
            required_permissions: Vec::new(),
            budget: SopStepBudget::default(),
            retry: SopRetryPolicy::default(),
            artifacts: Vec::new(),
            verifier: None,
            resume_key: Some(format!("checkpoint:{id}")),
        }
    }

    fn definition() -> SopDefinition {
        SopDefinition {
            schema_version: SOP_SCHEMA_VERSION,
            id: "terminal.change".to_string(),
            version: 1,
            title: "Terminal change".to_string(),
            inputs: json!({"type": "object"}),
            steps: vec![
                step("inspect", &[]),
                step("change", &["inspect"]),
                step("verify", &["change"]),
            ],
            verification: SopVerificationContract::default(),
            compensation: SopCompensationContract::default(),
        }
    }

    #[test]
    fn valid_definition_builds_resumable_run_and_unlocks_dependencies() {
        let definition = definition();
        definition.validate().unwrap();
        let mut run = SopRunRecord::new(&definition, "run-1".to_string(), json!({}), 100).unwrap();
        assert_eq!(run.runnable_step_ids(&definition), vec!["inspect"]);
        run.steps.get_mut("inspect").unwrap().status = SopStepRunStatus::Completed;
        assert_eq!(run.runnable_step_ids(&definition), vec!["change"]);
        assert_eq!(
            run.steps["verify"].resume_key.as_deref(),
            Some("checkpoint:verify")
        );
    }

    #[test]
    fn validation_rejects_unknown_dependency_and_cycle() {
        let mut unknown = definition();
        unknown.steps[1].depends_on = vec!["missing".to_string()];
        assert_eq!(unknown.validate().unwrap_err().code, "unknown_dependency");

        let mut cyclic = definition();
        cyclic.steps[0].depends_on = vec!["verify".to_string()];
        assert_eq!(cyclic.validate().unwrap_err().code, "dependency_cycle");
    }

    #[test]
    fn validation_rejects_unverifiable_artifact_reference_and_zero_budget() {
        let mut definition = definition();
        definition.verification.required_artifacts = vec!["document".to_string()];
        assert_eq!(definition.validate().unwrap_err().code, "unknown_artifact");

        definition.verification.required_artifacts.clear();
        definition.steps[0].budget.max_tool_calls = Some(0);
        assert_eq!(definition.validate().unwrap_err().code, "invalid_budget");
    }

    #[test]
    fn schema_denies_unknown_fields_and_preserves_camel_case_contract() {
        let encoded = serde_json::to_value(definition()).unwrap();
        assert_eq!(encoded["schemaVersion"], SOP_SCHEMA_VERSION);
        assert!(encoded["steps"][1].get("dependsOn").is_some());

        let mut invalid = encoded;
        invalid["unexpected"] = json!(true);
        assert!(serde_json::from_value::<SopDefinition>(invalid).is_err());
    }

    #[test]
    fn executable_run_handles_conditions_parallel_fan_in_retry_and_verification() {
        let mut definition = definition();
        definition.steps = vec![
            step("left", &[]),
            step("right", &[]),
            step("optional", &[]),
            step("join", &["left", "right", "optional"]),
        ];
        definition.steps[0].concurrent = true;
        definition.steps[0].retry = SopRetryPolicy {
            max_attempts: 2,
            initial_backoff_millis: 100,
            max_backoff_millis: Some(100),
            retryable_errors: vec!["transient".into()],
        };
        definition.steps[1].concurrent = true;
        definition.steps[2].preconditions = vec![SopCondition {
            id: "wanted".into(),
            description: "the optional branch is requested".into(),
        }];
        definition.steps[3].verifier = Some(SopVerifierContract {
            id: "join-check".into(),
            kind: SopVerifierKind::Evidence,
            capability: None,
            configuration: json!({}),
            success: "joined output is valid".into(),
        });
        definition.verification.verifiers = vec![SopVerifierContract {
            id: "final-check".into(),
            kind: SopVerifierKind::Evidence,
            capability: None,
            configuration: json!({}),
            success: "run output is accepted".into(),
        }];

        let mut run = SopRunRecord::new(&definition, "run-flow".into(), json!({}), 10).unwrap();
        run.apply(
            &definition,
            SopRunEvent::ResolveCondition {
                step_id: "optional".into(),
                condition_id: "wanted".into(),
                passed: false,
            },
            11,
        )
        .unwrap();
        assert_eq!(run.steps["optional"].status, SopStepRunStatus::Skipped);
        assert_eq!(run.runnable_step_ids(&definition), vec!["left", "right"]);

        run.apply(
            &definition,
            SopRunEvent::StartStep {
                step_id: "left".into(),
                executor_ref: "worker-left-1".into(),
            },
            12,
        )
        .unwrap();
        run.apply(
            &definition,
            SopRunEvent::FailStep {
                step_id: "left".into(),
                error_code: "transient".into(),
                error: "temporary failure".into(),
            },
            13,
        )
        .unwrap();
        assert_eq!(run.runnable_step_ids_at(&definition, 112), vec!["right"]);
        assert_eq!(
            run.runnable_step_ids_at(&definition, 113),
            vec!["left", "right"]
        );

        for (step_id, executor, now) in [
            ("left", "worker-left-2", 113),
            ("right", "worker-right", 114),
        ] {
            run.apply(
                &definition,
                SopRunEvent::StartStep {
                    step_id: step_id.into(),
                    executor_ref: executor.into(),
                },
                now,
            )
            .unwrap();
            run.apply(
                &definition,
                SopRunEvent::CompleteStep {
                    step_id: step_id.into(),
                    artifact_refs: Vec::new(),
                    evidence_refs: vec![format!("evidence:{step_id}")],
                },
                now + 1,
            )
            .unwrap();
        }
        assert_eq!(run.runnable_step_ids(&definition), vec!["join"]);

        run.apply(
            &definition,
            SopRunEvent::StartStep {
                step_id: "join".into(),
                executor_ref: "worker-join".into(),
            },
            120,
        )
        .unwrap();
        run.apply(
            &definition,
            SopRunEvent::CompleteStep {
                step_id: "join".into(),
                artifact_refs: Vec::new(),
                evidence_refs: vec!["evidence:join".into()],
            },
            121,
        )
        .unwrap();
        assert_eq!(run.steps["join"].status, SopStepRunStatus::Verifying);
        run.apply(
            &definition,
            SopRunEvent::CompleteStepVerification {
                step_id: "join".into(),
                passed: true,
                evidence_refs: vec!["verification:join".into()],
                error: None,
            },
            122,
        )
        .unwrap();
        assert_eq!(run.status, SopRunStatus::Verifying);

        let encoded = serde_json::to_string(&run).unwrap();
        let mut restored: SopRunRecord = serde_json::from_str(&encoded).unwrap();
        restored
            .apply(
                &definition,
                SopRunEvent::CompleteRunVerification {
                    verifier_id: "final-check".into(),
                    passed: true,
                    evidence_refs: vec!["verification:final".into()],
                    error: None,
                },
                123,
            )
            .unwrap();
        assert_eq!(restored.status, SopRunStatus::Completed);
        assert_eq!(restored.steps["left"].attempts.len(), 2);
        assert_eq!(restored.revision, 12);
    }

    #[test]
    fn terminal_failure_requires_declared_compensation_to_finish() {
        let mut definition = definition();
        definition.steps.truncate(2);
        definition.steps[1].retry.retryable_errors = vec!["retryable".into()];
        definition.compensation.actions = vec![SopCompensationAction {
            after_step: "inspect".into(),
            description: "restore the inspected resource".into(),
            required_capabilities: Vec::new(),
        }];
        let mut run =
            SopRunRecord::new(&definition, "run-compensate".into(), json!({}), 1).unwrap();
        for event in [
            SopRunEvent::StartStep {
                step_id: "inspect".into(),
                executor_ref: "worker-a".into(),
            },
            SopRunEvent::CompleteStep {
                step_id: "inspect".into(),
                artifact_refs: Vec::new(),
                evidence_refs: vec!["evidence:inspect".into()],
            },
            SopRunEvent::StartStep {
                step_id: "change".into(),
                executor_ref: "worker-b".into(),
            },
            SopRunEvent::FailStep {
                step_id: "change".into(),
                error_code: "permanent".into(),
                error: "change failed".into(),
            },
        ] {
            let now = run.updated_at_epoch_ms + 1;
            run.apply(&definition, event, now).unwrap();
        }
        assert_eq!(run.status, SopRunStatus::Compensating);
        run.apply(
            &definition,
            SopRunEvent::StartCompensation {
                after_step: "inspect".into(),
                executor_ref: "rollback-worker".into(),
            },
            6,
        )
        .unwrap();
        run.apply(
            &definition,
            SopRunEvent::CompleteCompensation {
                after_step: "inspect".into(),
                evidence_refs: vec!["rollback:inspect".into()],
            },
            7,
        )
        .unwrap();
        assert_eq!(run.status, SopRunStatus::Failed);
        assert_eq!(
            run.compensations["inspect"].status,
            SopCompensationRunStatus::Completed
        );

        let mut failed = SopRunRecord::new(
            &definition,
            "run-compensation-failure".into(),
            json!({}),
            10,
        )
        .unwrap();
        for event in [
            SopRunEvent::StartStep {
                step_id: "inspect".into(),
                executor_ref: "worker-a".into(),
            },
            SopRunEvent::CompleteStep {
                step_id: "inspect".into(),
                artifact_refs: Vec::new(),
                evidence_refs: vec!["evidence:inspect".into()],
            },
            SopRunEvent::StartStep {
                step_id: "change".into(),
                executor_ref: "worker-b".into(),
            },
            SopRunEvent::FailStep {
                step_id: "change".into(),
                error_code: "permanent".into(),
                error: "change failed".into(),
            },
        ] {
            let now = failed.updated_at_epoch_ms + 1;
            failed.apply(&definition, event, now).unwrap();
        }
        failed
            .apply(
                &definition,
                SopRunEvent::StartCompensation {
                    after_step: "inspect".into(),
                    executor_ref: "rollback-worker".into(),
                },
                15,
            )
            .unwrap();
        failed
            .apply(
                &definition,
                SopRunEvent::FailCompensation {
                    after_step: "inspect".into(),
                    error: "rollback unavailable".into(),
                },
                16,
            )
            .unwrap();
        assert_eq!(failed.status, SopRunStatus::Failed);
        assert_eq!(
            failed.compensations["inspect"].status,
            SopCompensationRunStatus::Failed
        );
        assert_eq!(
            failed.compensations["inspect"].error.as_deref(),
            Some("rollback unavailable")
        );
    }

    #[test]
    fn typed_wait_survives_serialization_and_records_replan_evidence() {
        let definition = definition();
        let mut run = SopRunRecord::new(&definition, "run-wait".into(), json!({}), 10).unwrap();
        run.apply(
            &definition,
            SopRunEvent::StartStep {
                step_id: "inspect".into(),
                executor_ref: "host:inspect".into(),
            },
            11,
        )
        .unwrap();
        let invalid = run.apply(
            &definition,
            SopRunEvent::SuspendStep {
                step_id: "inspect".into(),
                wait: SopWaitRecord {
                    kind: SopWaitKind::Permission,
                    reference_id: String::new(),
                    reason: "Allow reading the protected directory".into(),
                    requested_at_epoch_ms: 12,
                    disposition: None,
                    evidence_refs: Vec::new(),
                    resolved_at_epoch_ms: None,
                },
            },
            12,
        );
        assert_eq!(invalid.unwrap_err().code, "empty_wait_reference");

        run.apply(
            &definition,
            SopRunEvent::SuspendStep {
                step_id: "inspect".into(),
                wait: SopWaitRecord {
                    kind: SopWaitKind::Permission,
                    reference_id: "permission:request-42".into(),
                    reason: "Allow reading the protected directory".into(),
                    requested_at_epoch_ms: 12,
                    disposition: None,
                    evidence_refs: Vec::new(),
                    resolved_at_epoch_ms: None,
                },
            },
            12,
        )
        .unwrap();
        assert_eq!(run.status, SopRunStatus::WaitingPermission);

        let encoded = serde_json::to_vec(&run).unwrap();
        let mut restored: SopRunRecord = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            restored.steps["inspect"]
                .current_wait
                .as_ref()
                .map(|wait| wait.reference_id.as_str()),
            Some("permission:request-42")
        );
        restored
            .apply(
                &definition,
                SopRunEvent::ResumeStep {
                    step_id: "inspect".into(),
                    disposition: SopWaitDisposition::Replan,
                    evidence_refs: vec!["user-input:message-7".into()],
                },
                13,
            )
            .unwrap();
        let step = &restored.steps["inspect"];
        assert_eq!(step.status, SopStepRunStatus::Running);
        assert!(step.current_wait.is_none());
        assert_eq!(step.wait_history.len(), 1);
        assert_eq!(
            step.wait_history[0].disposition,
            Some(SopWaitDisposition::Replan)
        );
        assert_eq!(step.wait_history[0].resolved_at_epoch_ms, Some(13));
        assert_eq!(restored.evidence_refs, vec!["user-input:message-7"]);
    }
}
