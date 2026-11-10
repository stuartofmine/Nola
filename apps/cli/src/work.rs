// SPDX-License-Identifier: Apache-2.0
//! Read-only projection that presents every durable CLI work lifecycle through
//! one control surface. The projection never owns or copies lifecycle state.

use std::collections::BTreeMap;


use crate::config::Config;
use crate::i18n::{ui_text, UiText};
use crate::store::NolaStore;
use crate::sop_context::{enum_name, latest_sop_definition, roots_refer_to_same_directory};
#[cfg(test)]
use crate::sop_context::{sop_run_is_visible_to_session, sop_tool_invocation_protocol};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkKind {
    Goal,
    Sop,
    Skill,
    Task,
    BackgroundProcess,
    ExternalExecutor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkState {
    Queued,
    Planning,
    Ready,
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

impl WorkState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkProjection {
    pub id: String,
    pub kind: WorkKind,
    pub state: WorkState,
    pub raw_state: String,
    pub title: String,
    pub parent_id: Option<String>,
    pub session_id: Option<String>,
    pub current_step: Option<String>,
    pub progress: Option<f64>,
    pub result_ref: Option<String>,
    pub artifact_refs: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub error: Option<String>,
    pub updated_at_ms: i64,
}

impl WorkProjection {
    pub fn is_goal(&self) -> bool {
        self.kind == WorkKind::Goal
    }

    pub fn is_sop(&self) -> bool {
        self.kind == WorkKind::Sop
    }
}

pub fn collect(config: &Config) -> Result<Vec<WorkProjection>, String> {
    let mut items = BTreeMap::<String, WorkProjection>::new();
    let session_id = config.session_id.as_deref();
    let session_projection = session_id
        .map(crate::session::load_session_projection)
        .transpose()?
        .flatten();
    let goal_id = collect_goal(session_id, session_projection.as_ref(), &mut items)?;
    collect_sop_runs(config, goal_id.as_deref(), &mut items)?;
    collect_skill_runs(
        session_id,
        session_projection.as_ref(),
        goal_id.as_deref(),
        &mut items,
    )?;
    collect_core_tasks(
        session_id,
        session_projection.as_ref(),
        goal_id.as_deref(),
        &mut items,
    )?;
    collect_runtime_tasks(config, goal_id.as_deref(), &mut items)?;
    collect_background_processes(config, goal_id.as_deref(), &mut items)?;
    collect_incomplete_work_step_transactions(session_id, &mut items)?;

    let mut items = items.into_values().collect::<Vec<_>>();
    items.sort_by(|left, right| {
        state_rank(left.state)
            .cmp(&state_rank(right.state))
            .then_with(|| right.updated_at_ms.cmp(&left.updated_at_ms))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(items)
}


fn collect_incomplete_work_step_transactions(
    session_id: Option<&str>,
    items: &mut BTreeMap<String, WorkProjection>,
) -> Result<(), String> {
    let transactions = crate::work_step_port::incomplete_work_step_transactions(session_id)?;
    for transaction in transactions {
        let (state, recovery) = match transaction.phase {
            nola_application::WorkStepTransactionPhase::Prepared => (
                WorkState::Ready,
                "The side effect was prepared but execution was not claimed; it can be resumed safely.",
            ),
            nola_application::WorkStepTransactionPhase::Checkpointed => (
                WorkState::WaitingUser,
                "Execution was claimed after the checkpoint. The side effect may have started and must not be replayed automatically.",
            ),
            nola_application::WorkStepTransactionPhase::EffectObserved => (
                WorkState::Verifying,
                "The side effect result was observed and must be committed without repeating execution.",
            ),
            nola_application::WorkStepTransactionPhase::Committed
            | nola_application::WorkStepTransactionPhase::Failed => continue,
        };
        let request_capability = transaction
            .request
            .get("capability_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("tool");
        items.insert(
            transaction.id.clone(),
            WorkProjection {
                id: transaction.id,
                kind: WorkKind::Task,
                state,
                raw_state: format!("work_step_{:?}", transaction.phase).to_ascii_lowercase(),
                title: format!("{request_capability} · {}", transaction.effect_kind),
                parent_id: Some(transaction.work_id),
                session_id: session_id.map(str::to_string),
                current_step: Some(format!("{} · {recovery}", transaction.step_id)),
                progress: None,
                result_ref: transaction.result_ref,
                artifact_refs: transaction
                    .workspace_checkpoint_id
                    .into_iter()
                    .map(|id| format!("workspace-checkpoint:{id}"))
                    .collect(),
                evidence_refs: vec![format!("tool-call:{}", transaction.tool_call_id)],
                error: transaction.error,
                updated_at_ms: transaction.updated_at_ms,
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod work_step_transaction_projection_tests {
    use super::*;
    use nola_application::WorkStepTransactionPhase;
    use serde_json::json;

    #[test]
    fn uncertain_side_effect_is_visible_in_the_unified_work_projection() {
        let _environment = crate::store::test_env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-work-projection-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("work projection directory");
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
        let prepared = crate::work_step_port::prepare_work_step_transaction(
            "work-step:conversation-projection:call-1",
            "conversation-projection",
            "plan-step-7",
            "call-1",
            root.to_str().unwrap(),
            "mutate",
            None,
            json!({"capability_id":"write_file","arguments":{"path":"report.md"}}),
        )
        .expect("prepare projected transaction")
        .transaction;
        crate::work_step_port::advance_work_step_transaction(
            &prepared.id,
            prepared.revision,
            WorkStepTransactionPhase::Checkpointed,
            None,
            None,
            None,
            None,
        )
        .expect("claim projected transaction");

        let mut items = BTreeMap::new();
        collect_incomplete_work_step_transactions(Some("conversation-projection"), &mut items)
            .expect("project incomplete transaction");
        let item = items
            .get("work-step:conversation-projection:call-1")
            .expect("transaction appears in Work");
        assert_eq!(item.state, WorkState::WaitingUser);
        assert_eq!(item.parent_id.as_deref(), Some("conversation-projection"));
        assert!(item
            .current_step
            .as_deref()
            .is_some_and(|value| value.contains("plan-step-7")));
        assert_eq!(item.evidence_refs, vec!["tool-call:call-1"]);

        std::env::remove_var("NOLA_DB_PATH");
        let _ = std::fs::remove_dir_all(root);
    }
}

pub fn render_list(config: &Config) -> Result<String, String> {
    let items = collect(config)?;
    if items.is_empty() {
        return Ok(ui_text(UiText::NoWork).to_string());
    }
    let active = items
        .iter()
        .filter(|item| !item.state.is_terminal())
        .count();
    let mut lines = vec![format!(
        "# {}\n\n{} · {} {}",
        ui_text(UiText::Work),
        items.len(),
        active,
        ui_text(UiText::WorkActive)
    )];
    for item in items {
        let marker = work_marker(item.state);
        let progress = item
            .progress
            .map(|value| format!(" · {:.0}%", value.clamp(0.0, 1.0) * 100.0))
            .unwrap_or_default();
        let step = item
            .current_step
            .as_deref()
            .filter(|step| !step.trim().is_empty())
            .map(|step| format!(" · {}", truncate(step, 72)))
            .unwrap_or_default();
        lines.push(format!(
            "- {marker} `{}` · {} · {} · {}{progress}{step}",
            item.id,
            kind_label(item.kind),
            state_label(item.state),
            truncate(&item.title, 80),
        ));
    }
    lines.push(format!("\n{}", ui_text(UiText::WorkFooter)));
    Ok(lines.join("\n"))
}

pub fn render_detail(config: &Config, work_id: &str) -> Result<String, String> {
    let item = collect(config)?
        .into_iter()
        .find(|item| item.id == work_id)
        .ok_or_else(|| format!("{}: {work_id}", ui_text(UiText::WorkNotFound)))?;
    let mut lines = vec![format!("# {}", item.title)];
    lines.push(format!("- {}: `{}`", ui_text(UiText::WorkId), item.id));
    lines.push(format!(
        "- {}: {}",
        ui_text(UiText::WorkKind),
        kind_label(item.kind)
    ));
    lines.push(format!(
        "- {}: {}",
        ui_text(UiText::Status),
        state_label(item.state)
    ));
    if let Some(parent_id) = item.parent_id.as_deref() {
        lines.push(format!("- {}: `{parent_id}`", ui_text(UiText::WorkParent)));
    }
    if let Some(step) = item.current_step.as_deref() {
        lines.push(format!("- {}: {step}", ui_text(UiText::WorkCurrentStep)));
    }
    if item.is_sop() {
        let store = NolaStore::open_default()?;
        if let Some(projection) = store.sop_run(work_id.to_string())? {
            lines.push(format!(
                "- {}: v{}",
                ui_text(UiText::WorkSopVersion),
                projection.record.sop_version
            ));
            let available_definitions = crate::host_prompts::current_host_sops();
            if let Some(latest) = latest_sop_definition(
                available_definitions.iter(),
                &projection.record.sop_id,
                projection.record.sop_version.saturating_add(1),
            ) {
                lines.push(format!(
                    "- {}: v{} → v{}",
                    ui_text(UiText::WorkSopUpdateAvailable),
                    projection.record.sop_version,
                    latest.version
                ));
                lines.push(format!("\n{}", ui_text(UiText::WorkSopPinnedHint)));
            }
        }
    }
    if item.raw_state == "work_step_checkpointed" {
        lines.push(format!("\n{}", ui_text(UiText::WorkResolveHint)));
    }
    if let Some(progress) = item.progress {
        lines.push(format!(
            "- {}: {:.0}%",
            ui_text(UiText::Progress),
            progress.clamp(0.0, 1.0) * 100.0
        ));
    }
    if let Some(result) = item.result_ref.as_deref() {
        lines.push(format!("\n## {}\n\n{result}", ui_text(UiText::Result)));
    }
    if !item.artifact_refs.is_empty() {
        lines.push(format!("\n## {}", ui_text(UiText::WorkArtifacts)));
        lines.extend(
            item.artifact_refs
                .iter()
                .map(|reference| format!("- `{reference}`")),
        );
    }
    if !item.evidence_refs.is_empty() {
        lines.push(format!("\n## {}", ui_text(UiText::WorkEvidence)));
        lines.extend(
            item.evidence_refs
                .iter()
                .map(|reference| format!("- `{reference}`")),
        );
    }
    if let Some(error) = item.error.as_deref() {
        lines.push(format!("\n## {}\n\n{error}", ui_text(UiText::Error)));
    }
    if matches!(item.kind, WorkKind::Task | WorkKind::BackgroundProcess) {
        if let Ok(store) = NolaStore::open_default() {
            let events = store.runtime_task_events(&item.id, 12).unwrap_or_default();
            if !events.is_empty() {
                lines.push(format!("\n## {}", ui_text(UiText::RecentActivity)));
                for event in events.into_iter().rev() {
                    lines.push(format!(
                        "- {} · {}",
                        state_label(map_state(&event.status)),
                        event.summary.unwrap_or(event.kind)
                    ));
                }
            }
        }
    }
    Ok(lines.join("\n"))
}

pub fn find(config: &Config, work_id: &str) -> Result<Option<WorkProjection>, String> {
    Ok(collect(config)?.into_iter().find(|item| item.id == work_id))
}


pub fn control_sop(
    config: &Config,
    run_id: &str,
    control: &str,
) -> Result<nola_application::SopRunProjection, String> {
    let store = NolaStore::open_default()?;
    let projection = store
        .sop_run(run_id.to_string())?
        .ok_or_else(|| format!("{}: {run_id}", ui_text(UiText::WorkNotFound)))?;
    if !roots_refer_to_same_directory(
        &projection.project_root,
        &config.project_root.to_string_lossy(),
    ) {
        return Err(format!("{}: {run_id}", ui_text(UiText::WorkNotFound)));
    }
    let event = match control {
        "pause" => nola_core::agent::sop::SopRunEvent::Pause,
        "resume" => nola_core::agent::sop::SopRunEvent::Resume,
        "stop" | "cancel" => nola_core::agent::sop::SopRunEvent::Cancel,
        _ => return Err(ui_text(UiText::WorkControlUnsupported).to_string()),
    };
    store.apply_sop_run_event(run_id.to_string(), projection.record.revision, event)
}

pub fn rerun_sop(
    config: &Config,
    run_id: &str,
) -> Result<nola_application::SopRunProjection, String> {
    rerun_sop_with_definitions(config, run_id, crate::host_prompts::current_host_sops())
}

fn rerun_sop_with_definitions(
    config: &Config,
    run_id: &str,
    definitions: Vec<nola_core::agent::sop::SopDefinition>,
) -> Result<nola_application::SopRunProjection, String> {
    let store = NolaStore::open_default()?;
    let previous = store
        .sop_run(run_id.to_string())?
        .ok_or_else(|| format!("{}: {run_id}", ui_text(UiText::WorkNotFound)))?;
    if !roots_refer_to_same_directory(
        &previous.project_root,
        &config.project_root.to_string_lossy(),
    ) {
        return Err(format!("{}: {run_id}", ui_text(UiText::WorkNotFound)));
    }
    if !matches!(
        previous.record.status,
        nola_core::agent::sop::SopRunStatus::Completed
            | nola_core::agent::sop::SopRunStatus::Failed
            | nola_core::agent::sop::SopRunStatus::Cancelled
    ) {
        return Err(ui_text(UiText::WorkRerunRequiresTerminal).to_string());
    }
    let definition = latest_sop_definition(
        definitions.iter(),
        &previous.record.sop_id,
        previous.record.sop_version,
    )
    .cloned()
    .ok_or_else(|| ui_text(UiText::WorkSopDefinitionUnavailable).to_string())?;
    store.create_sop_run(
        definition,
        uuid::Uuid::new_v4().to_string(),
        previous.record.inputs,
        config.session_id.clone().or(previous.owner_session_id),
        previous.project_root,
    )
}


fn collect_sop_runs(
    config: &Config,
    goal_id: Option<&str>,
    items: &mut BTreeMap<String, WorkProjection>,
) -> Result<(), String> {
    let root = config.project_root.to_string_lossy();
    let store = NolaStore::open_default()?;
    for projection in store.list_sop_runs(None, None, 200)? {
        if !roots_refer_to_same_directory(&projection.project_root, &root) {
            continue;
        }
        let record = &projection.record;
        let raw_state = enum_name(record.status);
        let active_step_titles = projection
            .definition
            .steps
            .iter()
            .filter(|step| {
                record.steps.get(&step.id).is_some_and(|run| {
                    matches!(
                        run.status,
                        nola_core::agent::sop::SopStepRunStatus::Running
                            | nola_core::agent::sop::SopStepRunStatus::Waiting
                            | nola_core::agent::sop::SopStepRunStatus::Verifying
                    )
                })
            })
            .map(|step| step.title.clone())
            .collect::<Vec<_>>();
        let runnable_step_titles = record
            .runnable_step_ids(&projection.definition)
            .into_iter()
            .filter_map(|id| {
                projection
                    .definition
                    .steps
                    .iter()
                    .find(|step| step.id == id)
                    .map(|step| step.title.clone())
            })
            .collect::<Vec<_>>();
        let current_step = if record.status == nola_core::agent::sop::SopRunStatus::Compensating {
            record
                .compensations
                .values()
                .find(|compensation| {
                    !matches!(
                        compensation.status,
                        nola_core::agent::sop::SopCompensationRunStatus::Completed
                    )
                })
                .map(|compensation| compensation.after_step.clone())
        } else if !active_step_titles.is_empty() {
            let waits = record
                .steps
                .values()
                .filter_map(|step| step.current_wait.as_ref())
                .map(|wait| wait.reason.as_str())
                .collect::<Vec<_>>();
            let title = active_step_titles.join(" + ");
            Some(if waits.is_empty() {
                title
            } else {
                format!("{title} · {}", waits.join(" + "))
            })
        } else if !runnable_step_titles.is_empty() {
            Some(runnable_step_titles.join(" + "))
        } else {
            None
        };
        let completed = record
            .steps
            .values()
            .filter(|step| {
                matches!(
                    step.status,
                    nola_core::agent::sop::SopStepRunStatus::Completed
                        | nola_core::agent::sop::SopStepRunStatus::Skipped
                )
            })
            .count();
        let progress =
            (!record.steps.is_empty()).then_some(completed as f64 / record.steps.len() as f64);
        let mut artifact_refs = record.artifact_refs.clone();
        artifact_refs.extend(
            record
                .steps
                .values()
                .flat_map(|step| step.artifact_refs.iter().cloned()),
        );
        artifact_refs.sort();
        artifact_refs.dedup();
        let mut evidence_refs = record.evidence_refs.clone();
        evidence_refs.extend(
            record
                .steps
                .values()
                .flat_map(|step| step.evidence_refs.iter().cloned()),
        );
        evidence_refs.sort();
        evidence_refs.dedup();
        items.insert(
            record.run_id.clone(),
            WorkProjection {
                id: record.run_id.clone(),
                kind: WorkKind::Sop,
                state: map_state(&raw_state),
                raw_state,
                title: projection.definition.title,
                parent_id: projection
                    .owner_session_id
                    .as_deref()
                    .filter(|owner| Some(*owner) == config.session_id.as_deref())
                    .and(goal_id)
                    .map(str::to_string),
                session_id: projection.owner_session_id,
                current_step,
                progress,
                result_ref: None,
                artifact_refs,
                evidence_refs,
                error: record.failure.clone(),
                updated_at_ms: record.updated_at_epoch_ms,
            },
        );
    }
    Ok(())
}

fn collect_goal(
    session_id: Option<&str>,
    session_projection: Option<&nola_application::SessionProjection>,
    items: &mut BTreeMap<String, WorkProjection>,
) -> Result<Option<String>, String> {
    let Some(session_id) = session_id else {
        return Ok(None);
    };
    let Some(goal) =
        session_projection.and_then(|projection| projection.goal_projection.active.clone())
    else {
        return Ok(None);
    };
    let raw_state = enum_name(goal.status);
    let id = goal.id.clone();
    let current_step = goal
        .contracts
        .last()
        .map(|contract| contract.objective.clone())
        .or_else(|| {
            goal.requirements
                .values()
                .find(|requirement| {
                    !matches!(
                        requirement.status,
                        nola_core::agent::GoalRecordStatus::Completed
                    )
                })
                .map(|requirement| requirement.description.clone())
        });
    let completed = goal
        .requirements
        .values()
        .filter(|requirement| {
            matches!(
                requirement.status,
                nola_core::agent::GoalRecordStatus::Completed
            )
        })
        .count();
    let progress = (!goal.requirements.is_empty())
        .then_some(completed as f64 / goal.requirements.len() as f64);
    items.insert(
        id.clone(),
        WorkProjection {
            id: id.clone(),
            kind: WorkKind::Goal,
            state: map_state(&raw_state),
            raw_state,
            title: goal.objective,
            parent_id: None,
            session_id: Some(session_id.to_string()),
            current_step,
            progress,
            result_ref: goal
                .artifacts
                .values()
                .last()
                .map(|artifact| artifact.uri.clone()),
            artifact_refs: goal
                .artifacts
                .values()
                .map(|artifact| artifact.uri.clone())
                .collect(),
            evidence_refs: goal
                .audit_reports
                .iter()
                .flat_map(|report| report.evidence.iter().map(|evidence| evidence.id.clone()))
                .collect(),
            error: goal
                .audit_reports
                .last()
                .filter(|report| {
                    matches!(report.verdict, nola_core::agent::GoalAuditVerdict::Failed)
                })
                .map(|report| report.summary.clone()),
            updated_at_ms: goal.updated_at_epoch_ms,
        },
    );
    Ok(Some(id))
}

fn collect_skill_runs(
    session_id: Option<&str>,
    session_projection: Option<&nola_application::SessionProjection>,
    goal_id: Option<&str>,
    items: &mut BTreeMap<String, WorkProjection>,
) -> Result<(), String> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    let Some(projection) =
        session_projection.map(|projection| &projection.skill_runtime_projection)
    else {
        return Ok(());
    };
    for run_id in &projection.order {
        let Some(run) = projection.runs.get(run_id) else {
            continue;
        };
        let raw_state = enum_name(run.status);
        let contract = &run.descriptor.execution_contract;
        let completed = contract
            .progress_steps
            .iter()
            .filter(|step| run.completed_progress_steps.contains(&step.id))
            .count();
        let progress = (!contract.progress_steps.is_empty())
            .then_some(completed as f64 / contract.progress_steps.len() as f64);
        let current_step = run.current_progress_step.as_deref().and_then(|id| {
            contract
                .progress_steps
                .iter()
                .find(|step| step.id == id)
                .map(|step| step.title.clone())
        });
        items.insert(
            run.run_id.clone(),
            WorkProjection {
                id: run.run_id.clone(),
                kind: WorkKind::Skill,
                state: map_state(&raw_state),
                raw_state,
                title: run.descriptor.display_name.clone(),
                parent_id: goal_id.map(str::to_string),
                session_id: Some(session_id.to_string()),
                current_step,
                progress,
                result_ref: run.result_ref.clone(),
                artifact_refs: run.artifacts.values().cloned().collect(),
                evidence_refs: run
                    .verifications
                    .values()
                    .flat_map(|result| result.evidence.sources().into_iter().map(|source| match source {
                        nola_core::agent::capability::SkillEvidenceReference::ExecutionEvidence { evidence_id } => evidence_id.clone(),
                        nola_core::agent::capability::SkillEvidenceReference::ToolCall { call_id } => call_id.clone(),
                    }))
                    .collect(),
                error: run.failure.clone(),
                updated_at_ms: 0,
            },
        );
    }
    Ok(())
}

fn collect_core_tasks(
    session_id: Option<&str>,
    session_projection: Option<&nola_application::SessionProjection>,
    goal_id: Option<&str>,
    items: &mut BTreeMap<String, WorkProjection>,
) -> Result<(), String> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    let Some(projection) = session_projection.map(|projection| &projection.task_projection) else {
        return Ok(());
    };
    for task_id in &projection.order {
        let Some(task) = projection.tasks.get(task_id) else {
            continue;
        };
        let raw_state = enum_name(task.state);
        let kind = enum_name(task.kind);
        items.insert(
            task.id.clone(),
            WorkProjection {
                id: task.id.clone(),
                kind: WorkKind::Task,
                state: map_state(&raw_state),
                raw_state,
                title: task
                    .current_step
                    .clone()
                    .unwrap_or_else(|| format!("{kind} {}", task.id)),
                parent_id: task
                    .lineage
                    .parent_task_id
                    .clone()
                    .or_else(|| goal_id.map(str::to_string)),
                session_id: Some(session_id.to_string()),
                current_step: task.current_step.clone(),
                progress: task.progress,
                result_ref: task.result_ref.clone(),
                artifact_refs: task.result_ref.clone().into_iter().collect(),
                evidence_refs: Vec::new(),
                error: task.durability.last_error.clone(),
                updated_at_ms: task
                    .durability
                    .last_heartbeat_epoch_ms
                    .or(task.durability.retry_at_epoch_ms)
                    .unwrap_or_default(),
            },
        );
    }
    Ok(())
}

fn collect_runtime_tasks(
    config: &Config,
    goal_id: Option<&str>,
    items: &mut BTreeMap<String, WorkProjection>,
) -> Result<(), String> {
    let root = config.project_root.to_string_lossy();
    // Stored project roots may use an OS-equivalent spelling (for example
    // `/var` versus `/private/var` on macOS). Filtering in SQL by the raw
    // string would discard the Task before the canonical scope check below.
    let tasks = NolaStore::open_default()?.list_all_runtime_tasks()?;
    for task in tasks {
        if !scope_matches(
            &task.project_root,
            task.owner_session_id.as_deref(),
            &root,
            config.session_id.as_deref(),
        ) {
            continue;
        }
        let projection = WorkProjection {
            id: task.id.clone(),
            kind: match task.kind.as_str() {
                "bash" => WorkKind::BackgroundProcess,
                "external_executor" => WorkKind::ExternalExecutor,
                _ => WorkKind::Task,
            },
            state: map_state(&task.status),
            raw_state: task.status,
            title: task.title,
            parent_id: task
                .parent_task_id
                .clone()
                .or_else(|| goal_id.map(str::to_string)),
            session_id: task.owner_session_id,
            current_step: task.progress_summary.or(task.last_activity),
            progress: None,
            result_ref: task.output_path.or(task.result_summary),
            artifact_refs: Vec::new(),
            evidence_refs: Vec::new(),
            error: task.error.or(task.last_error),
            updated_at_ms: task.updated_at_ms,
        };
        items
            .entry(projection.id.clone())
            .and_modify(|existing| merge_runtime_fields(existing, &projection))
            .or_insert(projection);
    }
    Ok(())
}

fn collect_background_processes(
    config: &Config,
    goal_id: Option<&str>,
    items: &mut BTreeMap<String, WorkProjection>,
) -> Result<(), String> {
    for task in crate::tools::background_bash_task_summaries(
        &config.allowed_dirs,
        config.session_id.as_deref(),
    )? {
        items.entry(task.id.clone()).or_insert(WorkProjection {
            id: task.id,
            kind: WorkKind::BackgroundProcess,
            state: map_state(&task.status),
            raw_state: task.status,
            title: task.command,
            parent_id: goal_id.map(str::to_string),
            session_id: task.owner_session_id,
            current_step: Some(format!("PID {} · {:.1}s", task.pid, task.elapsed_secs)),
            progress: None,
            result_ref: None,
            artifact_refs: Vec::new(),
            evidence_refs: Vec::new(),
            error: None,
            updated_at_ms: 0,
        });
    }
    Ok(())
}

fn merge_runtime_fields(existing: &mut WorkProjection, runtime: &WorkProjection) {
    if existing.title.starts_with("tool ")
        || existing.title.starts_with("skill ")
        || existing.title.starts_with("agent ")
    {
        existing.title.clone_from(&runtime.title);
    }
    existing.updated_at_ms = existing.updated_at_ms.max(runtime.updated_at_ms);
    existing.current_step = runtime
        .current_step
        .clone()
        .or_else(|| existing.current_step.clone());
    existing.result_ref = runtime
        .result_ref
        .clone()
        .or_else(|| existing.result_ref.clone());
    if existing.artifact_refs.is_empty() {
        existing.artifact_refs.clone_from(&runtime.artifact_refs);
    }
    if existing.evidence_refs.is_empty() {
        existing.evidence_refs.clone_from(&runtime.evidence_refs);
    }
    existing.error = runtime.error.clone().or_else(|| existing.error.clone());
}

fn scope_matches(
    task_root: &str,
    task_session: Option<&str>,
    project_root: &str,
    session_id: Option<&str>,
) -> bool {
    roots_refer_to_same_directory(task_root, project_root)
        && match session_id {
            Some(session_id) => task_session == Some(session_id),
            None => matches!(task_session, None | Some("interactive")),
        }
}



fn map_state(status: &str) -> WorkState {
    match status.trim().to_ascii_lowercase().as_str() {
        "queued" | "pending" => WorkState::Queued,
        "planning" | "planned" => WorkState::Planning,
        "ready" => WorkState::Ready,
        "running" | "in_progress" | "executing" => WorkState::Running,
        "waiting_permission" | "permission_requested" => WorkState::WaitingPermission,
        "waiting_user" | "needs_user" => WorkState::WaitingUser,
        "waiting" | "waiting_external" | "retry_waiting" => WorkState::WaitingExternal,
        "paused" | "suspended" => WorkState::Paused,
        "verifying" | "awaiting_audit" | "auditing" => WorkState::Verifying,
        "compensating" => WorkState::Compensating,
        "completed" | "complete" | "finished" | "succeeded" => WorkState::Completed,
        "failed" | "error" => WorkState::Failed,
        "cancelled" | "canceled" => WorkState::Cancelled,
        _ => WorkState::Ready,
    }
}

fn state_rank(state: WorkState) -> u8 {
    match state {
        WorkState::WaitingPermission | WorkState::WaitingUser => 0,
        WorkState::Running | WorkState::Verifying | WorkState::Compensating => 1,
        WorkState::Planning | WorkState::Queued | WorkState::Ready => 2,
        WorkState::WaitingExternal | WorkState::Paused => 3,
        WorkState::Failed => 4,
        WorkState::Completed | WorkState::Cancelled => 5,
    }
}

fn work_marker(state: WorkState) -> &'static str {
    match state {
        WorkState::Completed => "✓",
        WorkState::Failed => "!",
        WorkState::Cancelled => "×",
        WorkState::WaitingPermission | WorkState::WaitingUser => "?",
        WorkState::Paused | WorkState::WaitingExternal => "◦",
        _ => "●",
    }
}

fn kind_label(kind: WorkKind) -> &'static str {
    match kind {
        WorkKind::Goal => ui_text(UiText::WorkGoal),
        WorkKind::Sop => ui_text(UiText::WorkSop),
        WorkKind::Skill => ui_text(UiText::WorkSkill),
        WorkKind::Task => ui_text(UiText::WorkTask),
        WorkKind::BackgroundProcess => ui_text(UiText::WorkProcess),
        WorkKind::ExternalExecutor => ui_text(UiText::WorkExecutor),
    }
}

fn state_label(state: WorkState) -> &'static str {
    match state {
        WorkState::Queued => ui_text(UiText::WorkQueued),
        WorkState::Planning => ui_text(UiText::WorkPlanning),
        WorkState::Ready => ui_text(UiText::WorkReady),
        WorkState::Running => ui_text(UiText::Running),
        WorkState::WaitingPermission => ui_text(UiText::WaitingPermission),
        WorkState::WaitingUser => ui_text(UiText::WorkWaitingUser),
        WorkState::WaitingExternal => ui_text(UiText::WorkWaitingExternal),
        WorkState::Paused => ui_text(UiText::WorkPaused),
        WorkState::Verifying => ui_text(UiText::WorkVerifying),
        WorkState::Compensating => ui_text(UiText::WorkCompensating),
        WorkState::Completed => ui_text(UiText::Completed),
        WorkState::Failed => ui_text(UiText::Failed),
        WorkState::Cancelled => ui_text(UiText::Cancelled),
    }
}

fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut result = value
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versioned_sop(id: &str, version: u32) -> nola_core::agent::sop::SopDefinition {
        let mut definition = crate::host_prompts::current_host_sops()
            .into_iter()
            .next()
            .expect("bundled SOP definition");
        definition.id = id.to_string();
        definition.version = version;
        definition
    }

    #[test]
    fn canonical_state_mapping_covers_every_work_state() {
        let cases = [
            ("queued", WorkState::Queued),
            ("planned", WorkState::Planning),
            ("ready", WorkState::Ready),
            ("running", WorkState::Running),
            ("waiting_permission", WorkState::WaitingPermission),
            ("needs_user", WorkState::WaitingUser),
            ("retry_waiting", WorkState::WaitingExternal),
            ("suspended", WorkState::Paused),
            ("awaiting_audit", WorkState::Verifying),
            ("completed", WorkState::Completed),
            ("failed", WorkState::Failed),
            ("cancelled", WorkState::Cancelled),
        ];
        for (raw, expected) in cases {
            assert_eq!(map_state(raw), expected, "{raw}");
        }
    }

    #[test]
    fn sop_resume_context_declares_exact_typed_tool_lineage_protocol() {
        let protocol = sop_tool_invocation_protocol();
        assert_eq!(protocol["argument"], "_nolaExecution");
        assert_eq!(
            protocol["value"],
            serde_json::json!({
                "sopRunId": "<exact runId from this context>",
                "sopStepId": "<exact ready or active step id from the same run>"
            })
        );
        assert!(protocol["semantics"]
            .as_str()
            .is_some_and(|value| value.contains("Host validates")));
    }

    #[test]
    fn sop_version_selection_uses_latest_compatible_definition_regardless_of_catalog_order() {
        let definitions = vec![
            versioned_sop("nola.test", 2),
            versioned_sop("nola.other", 99),
            versioned_sop("nola.test", 5),
            versioned_sop("nola.test", 3),
        ];

        let latest =
            latest_sop_definition(definitions.iter(), "nola.test", 2).expect("matching definition");
        assert_eq!(latest.version, 5);
        assert!(latest_sop_definition(definitions.iter(), "nola.test", 6).is_none());
    }

    #[test]
    fn recovered_sop_visibility_is_fenced_by_typed_session_owner() {
        assert!(sop_run_is_visible_to_session(None, Some("session-current")));
        assert!(sop_run_is_visible_to_session(
            Some("session-current"),
            Some("session-current")
        ));
        assert!(!sop_run_is_visible_to_session(
            Some("session-foreign"),
            Some("session-current")
        ));
        assert!(!sop_run_is_visible_to_session(Some("session-owned"), None));
    }

    #[test]
    fn multilingual_natural_text_cannot_select_a_foreign_sop_run() {
        for _prompt in [
            "继续当前流程并搜索可用工具",
            "Continúa el procedimiento actual",
            "تابع الإجراء الحالي وابحث عن الأداة",
        ] {
            assert!(!sop_run_is_visible_to_session(
                Some("session-foreign"),
                Some("session-current")
            ));
        }
    }

    #[test]
    fn runtime_fields_enrich_core_projection_without_replacing_identity() {
        let mut core = WorkProjection {
            id: "task-1".to_string(),
            kind: WorkKind::Task,
            state: WorkState::Running,
            raw_state: "running".to_string(),
            title: "tool task-1".to_string(),
            parent_id: Some("goal-1".to_string()),
            session_id: Some("session-1".to_string()),
            current_step: None,
            progress: Some(0.5),
            result_ref: None,
            artifact_refs: Vec::new(),
            evidence_refs: Vec::new(),
            error: None,
            updated_at_ms: 1,
        };
        let runtime = WorkProjection {
            id: "task-1".to_string(),
            kind: WorkKind::BackgroundProcess,
            state: WorkState::Completed,
            raw_state: "completed".to_string(),
            title: "Compile release build".to_string(),
            parent_id: None,
            session_id: None,
            current_step: Some("linking".to_string()),
            progress: None,
            result_ref: Some("/tmp/build.log".to_string()),
            artifact_refs: vec!["/tmp/build.log".to_string()],
            evidence_refs: Vec::new(),
            error: None,
            updated_at_ms: 8,
        };
        merge_runtime_fields(&mut core, &runtime);
        assert_eq!(core.id, "task-1");
        assert_eq!(core.parent_id.as_deref(), Some("goal-1"));
        assert_eq!(core.title, "Compile release build");
        assert_eq!(core.current_step.as_deref(), Some("linking"));
        assert_eq!(core.result_ref.as_deref(), Some("/tmp/build.log"));
        assert_eq!(core.updated_at_ms, 8);
    }
}
