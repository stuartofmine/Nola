// SPDX-License-Identifier: Apache-2.0
//! Conversation plan projection use-case boundary.

use nola_store_port::conversation_plan::{
    ConversationPlanRepository, StoredConversationPlan, StoredConversationPlanRevision,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPlanVerificationStatus {
    Pending,
    Passed,
    Failed,
    NotRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPlanTerminationReason {
    Success,
    VerificationNotApplicable,
    Cancelled,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPlanApprovalStatus {
    NotRequired,
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationPlanStep {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub active_form: String,
    pub status: ConversationPlanStepStatus,
    pub priority: String,
    pub dependency_ids: Vec<String>,
    pub approval_status: ConversationPlanApprovalStatus,
    pub capability_refs: Vec<String>,
    pub expected_artifact_refs: Vec<String>,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    #[serde(default)]
    pub work_step_transaction_refs: Vec<String>,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_key: Option<String>,
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub verification_status: ConversationPlanVerificationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination_reason: Option<ConversationPlanTerminationReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationPlanSopIdentity {
    pub run_id: String,
    pub sop_id: String,
    pub sop_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPlan {
    pub conversation_id: String,
    pub sop: ConversationPlanSopIdentity,
    pub todos: Vec<ConversationPlanStep>,
    pub revision: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_revision: Option<i64>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationPlanStepEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_refs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_artifact_refs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_criteria: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationPlanRevision {
    pub conversation_id: String,
    pub sop: ConversationPlanSopIdentity,
    pub todos: Vec<ConversationPlanStep>,
    pub revision: i64,
    pub change_kind: String,
    pub change_summary: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationPlanStepChange {
    pub id: String,
    pub before: ConversationPlanStep,
    pub after: ConversationPlanStep,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationPlanDiff {
    pub from_revision: i64,
    pub to_revision: i64,
    pub from_sop: ConversationPlanSopIdentity,
    pub to_sop: ConversationPlanSopIdentity,
    pub added: Vec<ConversationPlanStep>,
    pub removed: Vec<ConversationPlanStep>,
    pub changed: Vec<ConversationPlanStepChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationPlanCommand {
    Save {
        conversation_id: String,
        /// None creates an absent plan; Some compares the caller's read version.
        expected_revision: Option<i64>,
        sop: ConversationPlanSopIdentity,
        todos: Value,
        updated_at_ms: i64,
    },
    Get {
        conversation_id: String,
    },
    SetApproval {
        conversation_id: String,
        expected_revision: i64,
        step_ids: Vec<String>,
        decision: ConversationPlanApprovalStatus,
        updated_at_ms: i64,
    },
    EditStep {
        conversation_id: String,
        expected_revision: i64,
        step_id: String,
        edit: ConversationPlanStepEdit,
        updated_at_ms: i64,
    },
    Freeze {
        conversation_id: String,
        expected_revision: i64,
        updated_at_ms: i64,
    },
    Replan {
        conversation_id: String,
        expected_revision: i64,
        sop: ConversationPlanSopIdentity,
        todos: Value,
        reason: String,
        updated_at_ms: i64,
    },
    History {
        conversation_id: String,
    },
    Diff {
        conversation_id: String,
        from_revision: i64,
        to_revision: i64,
    },
}

pub(crate) fn execute_conversation_plan_command<S: ConversationPlanRepository + ?Sized>(
    store: &mut S,
    command: ConversationPlanCommand,
) -> Result<Value, String> {
    match command {
        ConversationPlanCommand::Save {
            conversation_id,
            expected_revision,
            sop,
            todos,
            updated_at_ms,
        } => {
            validate_sop_identity(&sop)?;
            let todos = decode_plan_steps(todos)?;
            let todos_json = serde_json::to_string(&todos)
                .map_err(|error| format!("encode conversation todos failed: {error}"))?;
            let stored = match store
                .conversation_plan(&conversation_id)
                .map_err(|error| error.to_string())?
            {
                None if expected_revision.is_none() => store
                    .save_conversation_plan(
                        &conversation_id,
                        &sop.sop_id,
                        sop.sop_version,
                        &sop.run_id,
                        &todos_json,
                        updated_at_ms,
                    )
                    .map_err(|error| error.to_string())?,
                Some(current) => {
                    if expected_revision != Some(current.revision) {
                        return Err("conversation plan revision conflict; preserve edits and reload".into());
                    }
                    let current_plan = conversation_plan(current.clone())?;
                    if current_plan.sop != sop {
                        return Err(
                            "conversation plan SOP identity is immutable; create an explicit replan revision"
                                .to_string(),
                        );
                    }
                    validate_retained_execution_references(&current_plan.todos, &todos)?;
                    if current_plan.frozen_revision.is_some()
                        && !same_plan_contract(&current_plan.todos, &todos)
                    {
                        return Err(
                            "conversation plan is frozen; create an explicit replan revision"
                                .to_string(),
                        );
                    }
                    store
                        .save_conversation_plan_revision_if_revision(
                            &conversation_id,
                            &sop.sop_id,
                            sop.sop_version,
                            &sop.run_id,
                            &todos_json,
                            expected_revision.expect("validated caller revision"),
                            None,
                            "save",
                            "",
                            updated_at_ms,
                        )
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| {
                            "conversation plan changed before save was committed".to_string()
                        })?
                }
                None => return Err("conversation plan revision conflict; plan no longer exists".into()),
            };
            serde_json::to_value(conversation_plan(stored)?)
        }
        ConversationPlanCommand::Get { conversation_id } => {
            let plan = store
                .conversation_plan(&conversation_id)
                .map_err(|error| error.to_string())?
                .map(conversation_plan)
                .transpose()?;
            serde_json::to_value(plan)
        }
        ConversationPlanCommand::SetApproval {
            conversation_id,
            expected_revision,
            step_ids,
            decision,
            updated_at_ms,
        } => {
            if !matches!(
                decision,
                ConversationPlanApprovalStatus::Approved | ConversationPlanApprovalStatus::Rejected
            ) {
                return Err("plan approval decision must be approved or rejected".to_string());
            }
            let stored = store
                .conversation_plan(&conversation_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "conversation plan not found".to_string())?;
            if stored.revision != expected_revision {
                return Err(format!(
                    "conversation plan revision conflict: expected {expected_revision}, current {}",
                    stored.revision
                ));
            }
            let mut plan = conversation_plan(stored)?;
            if plan.frozen_revision.is_some() {
                return Err("frozen conversation plans cannot change approval".to_string());
            }
            let selected = if step_ids.is_empty() {
                plan.todos
                    .iter()
                    .filter(|step| {
                        step.status == ConversationPlanStepStatus::Pending
                            && step.approval_status == ConversationPlanApprovalStatus::Pending
                    })
                    .map(|step| step.id.clone())
                    .collect::<std::collections::BTreeSet<_>>()
            } else {
                step_ids.into_iter().collect()
            };
            if selected.is_empty() {
                return Err("no pending plan steps selected".to_string());
            }
            let known = plan
                .todos
                .iter()
                .map(|step| step.id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            if let Some(unknown) = selected.iter().find(|id| !known.contains(id.as_str())) {
                return Err(format!("unknown plan step: {unknown}"));
            }
            for step in &mut plan.todos {
                if !selected.contains(&step.id) {
                    continue;
                }
                if step.status != ConversationPlanStepStatus::Pending
                    || step.approval_status != ConversationPlanApprovalStatus::Pending
                {
                    return Err(format!("plan step {} is not awaiting approval", step.id));
                }
                step.approval_status = decision.clone();
                if decision == ConversationPlanApprovalStatus::Rejected {
                    step.status = ConversationPlanStepStatus::Cancelled;
                    step.termination_reason = Some(ConversationPlanTerminationReason::Cancelled);
                }
            }
            validate_plan_steps(&plan.todos)?;
            let todos_json = serde_json::to_string(&plan.todos)
                .map_err(|error| format!("encode approved conversation plan failed: {error}"))?;
            let saved = store
                .save_conversation_plan_if_revision(
                    &conversation_id,
                    &plan.sop.sop_id,
                    plan.sop.sop_version,
                    &plan.sop.run_id,
                    &todos_json,
                    expected_revision,
                    updated_at_ms,
                )
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    "conversation plan changed before approval was committed".to_string()
                })?;
            serde_json::to_value(conversation_plan(saved)?)
        }
        ConversationPlanCommand::EditStep {
            conversation_id,
            expected_revision,
            step_id,
            edit,
            updated_at_ms,
        } => {
            let stored = current_plan_at_revision(store, &conversation_id, expected_revision)?;
            let mut plan = conversation_plan(stored)?;
            if plan.frozen_revision.is_some() {
                return Err("frozen conversation plans cannot be edited".to_string());
            }
            let step = plan
                .todos
                .iter_mut()
                .find(|step| step.id == step_id)
                .ok_or_else(|| format!("unknown plan step: {step_id}"))?;
            if let Some(value) = edit.content {
                step.content = value;
            }
            if let Some(value) = edit.active_form {
                step.active_form = value;
            }
            if let Some(value) = edit.priority {
                step.priority = value;
            }
            if let Some(value) = edit.dependency_ids {
                step.dependency_ids = value;
            }
            if let Some(value) = edit.capability_refs {
                step.capability_refs = value;
            }
            if let Some(value) = edit.expected_artifact_refs {
                step.expected_artifact_refs = value;
            }
            if let Some(value) = edit.success_criteria {
                step.success_criteria = value;
            }
            validate_plan_steps(&plan.todos)?;
            let todos_json = serde_json::to_string(&plan.todos)
                .map_err(|error| format!("encode edited conversation plan failed: {error}"))?;
            let saved = store
                .save_conversation_plan_revision_if_revision(
                    &conversation_id,
                    &plan.sop.sop_id,
                    plan.sop.sop_version,
                    &plan.sop.run_id,
                    &todos_json,
                    expected_revision,
                    None,
                    "edit",
                    &format!("edited step {step_id}"),
                    updated_at_ms,
                )
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "conversation plan changed before edit was committed".to_string())?;
            serde_json::to_value(conversation_plan(saved)?)
        }
        ConversationPlanCommand::Freeze {
            conversation_id,
            expected_revision,
            updated_at_ms,
        } => {
            let stored = current_plan_at_revision(store, &conversation_id, expected_revision)?;
            let plan = conversation_plan(stored)?;
            if plan.frozen_revision.is_some() {
                return Err("conversation plan is already frozen".to_string());
            }
            if plan.todos.iter().any(|step| {
                step.status != ConversationPlanStepStatus::Cancelled
                    && step.approval_status == ConversationPlanApprovalStatus::Pending
            }) {
                return Err("all executable plan steps must be approved before freeze".to_string());
            }
            let todos_json = serde_json::to_string(&plan.todos)
                .map_err(|error| format!("encode frozen conversation plan failed: {error}"))?;
            let saved = store
                .save_conversation_plan_revision_if_revision(
                    &conversation_id,
                    &plan.sop.sop_id,
                    plan.sop.sop_version,
                    &plan.sop.run_id,
                    &todos_json,
                    expected_revision,
                    Some(Some(expected_revision)),
                    "freeze",
                    "execution contract frozen",
                    updated_at_ms,
                )
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    "conversation plan changed before freeze was committed".to_string()
                })?;
            serde_json::to_value(conversation_plan(saved)?)
        }
        ConversationPlanCommand::Replan {
            conversation_id,
            expected_revision,
            sop,
            todos,
            reason,
            updated_at_ms,
        } => {
            validate_sop_identity(&sop)?;
            if reason.trim().is_empty() {
                return Err("replan reason must not be empty".to_string());
            }
            let current = current_plan_at_revision(store, &conversation_id, expected_revision)?;
            if current.frozen_revision.is_none() {
                return Err("only a frozen plan may create an explicit replan revision".to_string());
            }
            let current = conversation_plan(current)?;
            if current.sop.run_id == sop.run_id {
                return Err("explicit replan requires a new SOP run id".to_string());
            }
            let todos = decode_plan_steps(todos)?;
            let todos_json = serde_json::to_string(&todos)
                .map_err(|error| format!("encode replanned conversation plan failed: {error}"))?;
            let saved = store
                .save_conversation_plan_revision_if_revision(
                    &conversation_id,
                    &sop.sop_id,
                    sop.sop_version,
                    &sop.run_id,
                    &todos_json,
                    expected_revision,
                    Some(None),
                    "replan",
                    reason.trim(),
                    updated_at_ms,
                )
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    "conversation plan changed before replan was committed".to_string()
                })?;
            serde_json::to_value(conversation_plan(saved)?)
        }
        ConversationPlanCommand::History { conversation_id } => {
            let revisions = store
                .conversation_plan_revisions(&conversation_id)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(conversation_plan_revision)
                .collect::<Result<Vec<_>, _>>()?;
            serde_json::to_value(revisions)
        }
        ConversationPlanCommand::Diff {
            conversation_id,
            from_revision,
            to_revision,
        } => {
            let revisions = store
                .conversation_plan_revisions(&conversation_id)
                .map_err(|error| error.to_string())?;
            let from = revisions
                .iter()
                .find(|revision| revision.revision == from_revision)
                .cloned()
                .ok_or_else(|| format!("unknown plan revision: {from_revision}"))?;
            let to = revisions
                .iter()
                .find(|revision| revision.revision == to_revision)
                .cloned()
                .ok_or_else(|| format!("unknown plan revision: {to_revision}"))?;
            serde_json::to_value(diff_plan_revisions(
                conversation_plan_revision(from)?,
                conversation_plan_revision(to)?,
            ))
        }
    }
    .map_err(|error| format!("encode conversation plan result failed: {error}"))
}

pub(crate) fn conversation_plan(
    stored: StoredConversationPlan,
) -> Result<ConversationPlan, String> {
    let todos: Vec<ConversationPlanStep> = serde_json::from_str(&stored.todos_json)
        .map_err(|error| format!("decode stored conversation todos failed: {error}"))?;
    validate_plan_steps(&todos)?;
    Ok(ConversationPlan {
        conversation_id: stored.conversation_id,
        sop: ConversationPlanSopIdentity {
            run_id: stored.run_id,
            sop_id: stored.sop_id,
            sop_version: stored.sop_version,
        },
        todos,
        revision: stored.revision,
        frozen_revision: stored.frozen_revision,
        updated_at_ms: stored.updated_at_ms,
    })
}

fn conversation_plan_revision(
    stored: StoredConversationPlanRevision,
) -> Result<ConversationPlanRevision, String> {
    let todos: Vec<ConversationPlanStep> = serde_json::from_str(&stored.todos_json)
        .map_err(|error| format!("decode stored conversation plan revision failed: {error}"))?;
    validate_plan_steps(&todos)?;
    Ok(ConversationPlanRevision {
        conversation_id: stored.conversation_id,
        sop: ConversationPlanSopIdentity {
            run_id: stored.run_id,
            sop_id: stored.sop_id,
            sop_version: stored.sop_version,
        },
        todos,
        revision: stored.revision,
        change_kind: stored.change_kind,
        change_summary: stored.change_summary,
        updated_at_ms: stored.updated_at_ms,
    })
}

fn current_plan_at_revision<S: ConversationPlanRepository + ?Sized>(
    store: &S,
    conversation_id: &str,
    expected_revision: i64,
) -> Result<StoredConversationPlan, String> {
    let stored = store
        .conversation_plan(conversation_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "conversation plan not found".to_string())?;
    if stored.revision != expected_revision {
        return Err(format!(
            "conversation plan revision conflict: expected {expected_revision}, current {}",
            stored.revision
        ));
    }
    Ok(stored)
}

fn validate_retained_execution_references(
    current: &[ConversationPlanStep],
    next: &[ConversationPlanStep],
) -> Result<(), String> {
    for previous in current {
        let updated = next.iter().find(|step| step.id == previous.id);
        let references_retained = updated.is_some_and(|step| {
            previous.artifact_refs.iter().all(|id| step.artifact_refs.contains(id))
                && previous
                    .work_step_transaction_refs
                    .iter()
                    .all(|id| step.work_step_transaction_refs.contains(id))
        });
        if (!previous.artifact_refs.is_empty() || !previous.work_step_transaction_refs.is_empty())
            && !references_retained
        {
            return Err(format!(
                "plan step {} cannot discard persisted artifacts or work transactions; retain the step and its references",
                previous.id
            ));
        }
    }
    Ok(())
}

fn same_plan_contract(left: &[ConversationPlanStep], right: &[ConversationPlanStep]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.id == right.id
                && left.content == right.content
                && left.active_form == right.active_form
                && left.priority == right.priority
                && left.dependency_ids == right.dependency_ids
                && left.approval_status == right.approval_status
                && left.capability_refs == right.capability_refs
                && left.expected_artifact_refs == right.expected_artifact_refs
                && left.success_criteria == right.success_criteria
        })
}

fn diff_plan_revisions(
    from: ConversationPlanRevision,
    to: ConversationPlanRevision,
) -> ConversationPlanDiff {
    let from_sop = from.sop.clone();
    let to_sop = to.sop.clone();
    let before = from
        .todos
        .into_iter()
        .map(|step| (step.id.clone(), step))
        .collect::<std::collections::BTreeMap<_, _>>();
    let after = to
        .todos
        .into_iter()
        .map(|step| (step.id.clone(), step))
        .collect::<std::collections::BTreeMap<_, _>>();
    let added = after
        .iter()
        .filter(|(id, _)| !before.contains_key(*id))
        .map(|(_, step)| step.clone())
        .collect();
    let removed = before
        .iter()
        .filter(|(id, _)| !after.contains_key(*id))
        .map(|(_, step)| step.clone())
        .collect();
    let changed = before
        .iter()
        .filter_map(|(id, before)| {
            let after = after.get(id)?;
            (before != after).then(|| ConversationPlanStepChange {
                id: id.clone(),
                before: before.clone(),
                after: after.clone(),
            })
        })
        .collect();
    ConversationPlanDiff {
        from_revision: from.revision,
        to_revision: to.revision,
        from_sop,
        to_sop,
        added,
        removed,
        changed,
    }
}

fn validate_sop_identity(sop: &ConversationPlanSopIdentity) -> Result<(), String> {
    if sop.run_id.trim().is_empty() || sop.sop_id.trim().is_empty() || sop.sop_version == 0 {
        return Err(
            "conversation plan SOP identity requires runId, sopId, and a positive sopVersion"
                .to_string(),
        );
    }
    Ok(())
}

fn decode_plan_steps(todos: Value) -> Result<Vec<ConversationPlanStep>, String> {
    let steps: Vec<ConversationPlanStep> = serde_json::from_value(todos)
        .map_err(|error| format!("decode conversation plan steps failed: {error}"))?;
    validate_plan_steps(&steps)?;
    Ok(steps)
}

pub(crate) fn validate_plan_steps(steps: &[ConversationPlanStep]) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    let statuses = steps
        .iter()
        .map(|step| (step.id.as_str(), step.status.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut active = 0usize;
    for step in steps {
        if step.id.trim().is_empty()
            || step.content.trim().is_empty()
            || step.success_criteria.is_empty()
            || step
                .success_criteria
                .iter()
                .any(|item| item.trim().is_empty())
        {
            return Err(
                "plan steps require id, content, and non-empty success criteria".to_string(),
            );
        }
        if !ids.insert(step.id.as_str()) {
            return Err("conversation plan step ids must be unique".to_string());
        }
        if step.dependency_ids.iter().any(|id| id.trim().is_empty())
            || step.capability_refs.iter().any(|id| id.trim().is_empty())
            || step
                .expected_artifact_refs
                .iter()
                .any(|id| id.trim().is_empty())
            || step.artifact_refs.iter().any(|id| id.trim().is_empty())
            || step
                .work_step_transaction_refs
                .iter()
                .any(|id| id.trim().is_empty())
        {
            return Err(
                "plan dependency, capability, artifact, and transaction references must be non-empty"
                    .to_string(),
            );
        }
        if step
            .executor_ref
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
            || step
                .resume_key
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err("plan executor and resume references must be non-empty".to_string());
        }
        let mut dependency_ids = std::collections::BTreeSet::new();
        for dependency_id in &step.dependency_ids {
            if dependency_id == &step.id || !dependency_ids.insert(dependency_id.as_str()) {
                return Err(
                    "plan dependencies must be unique and cannot reference the same step"
                        .to_string(),
                );
            }
            let Some(dependency_status) = statuses.get(dependency_id.as_str()) else {
                return Err(format!("unknown plan dependency: {dependency_id}"));
            };
            if matches!(
                step.status,
                ConversationPlanStepStatus::InProgress | ConversationPlanStepStatus::Completed
            ) && *dependency_status != ConversationPlanStepStatus::Completed
            {
                return Err(
                    "active or completed plan steps require completed dependencies".to_string(),
                );
            }
        }
        if matches!(
            step.status,
            ConversationPlanStepStatus::InProgress | ConversationPlanStepStatus::Completed
        ) && matches!(
            step.approval_status,
            ConversationPlanApprovalStatus::Pending | ConversationPlanApprovalStatus::Rejected
        ) {
            return Err(
                "active or completed plan steps require approval or not_required approval"
                    .to_string(),
            );
        }
        if step.approval_status == ConversationPlanApprovalStatus::Rejected
            && step.status != ConversationPlanStepStatus::Cancelled
        {
            return Err("rejected plan steps must be cancelled".to_string());
        }
        if !matches!(step.priority.as_str(), "high" | "medium" | "low") {
            return Err("plan step priority must be high, medium, or low".to_string());
        }
        if step.status == ConversationPlanStepStatus::InProgress {
            active += 1;
        }
        match step.status {
            ConversationPlanStepStatus::Completed => {
                if !matches!(
                    step.verification_status,
                    ConversationPlanVerificationStatus::Passed
                        | ConversationPlanVerificationStatus::NotRequired
                ) || step.termination_reason.is_none()
                {
                    return Err(
                        "completed plan steps require verification and a termination reason"
                            .to_string(),
                    );
                }
                if step.verification_status == ConversationPlanVerificationStatus::Passed
                    && step.evidence_refs.is_empty()
                {
                    return Err("verified plan completion requires evidence references".to_string());
                }
            }
            ConversationPlanStepStatus::Cancelled => {
                if step.termination_reason.is_none() {
                    return Err("cancelled plan steps require a termination reason".to_string());
                }
            }
            ConversationPlanStepStatus::Pending | ConversationPlanStepStatus::InProgress => {
                if step.termination_reason.is_some() {
                    return Err(
                        "non-terminal plan steps cannot have a termination reason".to_string()
                    );
                }
            }
        }
    }
    if active > 1 {
        return Err("only one conversation plan step may be in progress".to_string());
    }
    fn visit<'a>(
        id: &'a str,
        steps: &std::collections::BTreeMap<&'a str, &'a ConversationPlanStep>,
        visiting: &mut std::collections::BTreeSet<&'a str>,
        visited: &mut std::collections::BTreeSet<&'a str>,
    ) -> Result<(), String> {
        if visited.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err("conversation plan dependencies must be acyclic".to_string());
        }
        for dependency_id in &steps[id].dependency_ids {
            visit(dependency_id, steps, visiting, visited)?;
        }
        visiting.remove(id);
        visited.insert(id);
        Ok(())
    }
    let by_id = steps
        .iter()
        .map(|step| (step.id.as_str(), step))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut visiting = std::collections::BTreeSet::new();
    let mut visited = std::collections::BTreeSet::new();
    for id in by_id.keys().copied() {
        visit(id, &by_id, &mut visiting, &mut visited)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn verified_completion_requires_evidence() {
        let error = decode_plan_steps(json!([{
            "id": "verify",
            "content": "Verify the result",
            "status": "completed",
            "priority": "high",
            "dependencyIds": [],
            "approvalStatus": "approved",
            "capabilityRefs": ["builtin:read_file"],
            "expectedArtifactRefs": ["artifact://result"],
            "successCriteria": ["The behavior is demonstrated"],
            "verificationStatus": "passed",
            "terminationReason": "success"
        }]))
        .unwrap_err();

        assert!(error.contains("requires evidence references"));
    }

    #[test]
    fn structured_plan_round_trips_without_losing_lifecycle_data() {
        let steps = decode_plan_steps(json!([
            {
                "id": "implement",
                "content": "Implement the change",
                "activeForm": "Implementing the change",
                "status": "in_progress",
                "priority": "high",
                "dependencyIds": [],
                "approvalStatus": "approved",
                "capabilityRefs": ["builtin:write_file"],
                "expectedArtifactRefs": ["artifact://implementation"],
                "successCriteria": ["The contract is implemented"],
                "evidenceRefs": [],
                "verificationStatus": "pending"
            },
            {
                "id": "obsolete",
                "content": "Remove the obsolete path",
                "status": "cancelled",
                "priority": "low",
                "dependencyIds": [],
                "approvalStatus": "rejected",
                "capabilityRefs": [],
                "expectedArtifactRefs": [],
                "successCriteria": ["The path is removed if applicable"],
                "evidenceRefs": [],
                "verificationStatus": "pending",
                "terminationReason": "cancelled"
            }
        ]))
        .unwrap();

        let encoded = serde_json::to_value(&steps).unwrap();
        let decoded = decode_plan_steps(encoded).unwrap();
        assert_eq!(decoded, steps);
    }

    #[test]
    fn dependency_graph_rejects_cycles_and_unfinished_predecessors() {
        let cycle = decode_plan_steps(json!([
            {"id":"a","content":"A","status":"pending","priority":"medium","dependencyIds":["b"],"approvalStatus":"not_required","capabilityRefs":[],"expectedArtifactRefs":[],"successCriteria":["A done"],"verificationStatus":"pending"},
            {"id":"b","content":"B","status":"pending","priority":"medium","dependencyIds":["a"],"approvalStatus":"not_required","capabilityRefs":[],"expectedArtifactRefs":[],"successCriteria":["B done"],"verificationStatus":"pending"}
        ]))
        .unwrap_err();
        assert!(cycle.contains("acyclic"));

        let unfinished = decode_plan_steps(json!([
            {"id":"a","content":"A","status":"pending","priority":"medium","dependencyIds":[],"approvalStatus":"not_required","capabilityRefs":[],"expectedArtifactRefs":[],"successCriteria":["A done"],"verificationStatus":"pending"},
            {"id":"b","content":"B","status":"in_progress","priority":"medium","dependencyIds":["a"],"approvalStatus":"approved","capabilityRefs":[],"expectedArtifactRefs":[],"successCriteria":["B done"],"verificationStatus":"pending"}
        ]))
        .unwrap_err();
        assert!(unfinished.contains("completed dependencies"));
    }
}

#[cfg(test)]
#[path = "plan_work_step_port_tests.rs"]
mod port_tests;
