// SPDX-License-Identifier: Apache-2.0
//! Durable SOP execution boundary.
//!
//! The Host reports typed facts. Core owns transition validity and Application
//! commits the resulting record with revision CAS against its pinned definition.

use nola_agent_engine::agent::sop::{
    SopDefinition, SopRunEvent, SopRunRecord, SopRunStatus, SopStepRunStatus, SopWaitDisposition,
    SopWaitKind, SopWaitRecord,
};
use nola_store_port::sop_run::{SopRunRepository, StoredSopRun, VisibleActiveSopRuns};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SopRunProjection {
    pub definition: SopDefinition,
    pub record: SopRunRecord,
    pub owner_session_id: Option<String>,
    pub project_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SopRunCommand {
    VisibleActive { query: VisibleActiveSopRuns },
    Create {
        definition: SopDefinition,
        run_id: String,
        inputs: Value,
        owner_session_id: Option<String>,
        project_root: String,
        now_ms: i64,
    },
    Apply {
        run_id: String,
        expected_revision: u64,
        event: SopRunEvent,
        now_ms: i64,
    },
    Load {
        run_id: String,
    },
    ValidateInvocation {
        run_id: String,
        step_id: String,
        project_root: String,
        owner_session_id: String,
    },
    ValidatePermissionInvocation {
        run_id: String,
        step_id: String,
        project_root: String,
        owner_session_id: String,
        reference_id: String,
    },
    EnsurePermissionWait {
        run_id: String,
        step_id: String,
        reference_id: String,
        reason: String,
        now_ms: i64,
    },
    EnsureWait {
        run_id: String,
        step_id: String,
        wait: SopWaitRecord,
        now_ms: i64,
    },
    ResolvePermissionWait {
        run_id: String,
        step_id: String,
        reference_id: String,
        disposition: SopWaitDisposition,
        evidence_refs: Vec<String>,
        now_ms: i64,
    },
    List {
        project_root: Option<String>,
        owner_session_id: Option<String>,
        limit: usize,
    },
}

pub(crate) fn execute_sop_run_command(
    store: &mut impl SopRunRepository,
    command: SopRunCommand,
) -> Result<Value, String> {
    match command {
        SopRunCommand::VisibleActive { query } => encode(
            store.visible_active_sop_runs(&query)
                .map_err(|error| format!("load visible active SOP context failed: {error}"))?
                .into_iter().map(projection).collect::<Result<Vec<_>, _>>()?,
        ),
        SopRunCommand::Create {
            definition,
            run_id,
            inputs,
            owner_session_id,
            project_root,
            now_ms,
        } => {
            if project_root.trim().is_empty() {
                return Err("SOP project root must not be empty".into());
            }
            let record = SopRunRecord::new(&definition, run_id, inputs, now_ms)
                .map_err(|error| format!("create SOP run failed: {error}"))?;
            let projection = SopRunProjection {
                definition,
                record,
                owner_session_id,
                project_root,
            };
            store
                .create_sop_run(&stored_projection(&projection)?)
                .map_err(|error| format!("persist SOP run failed: {error}"))?;
            encode(projection)
        }
        SopRunCommand::Apply {
            run_id,
            expected_revision,
            event,
            now_ms,
        } => {
            let stored = store
                .sop_run(&run_id)
                .map_err(|error| format!("load SOP run failed: {error}"))?
                .ok_or_else(|| format!("SOP run `{run_id}` does not exist"))?;
            if stored.revision != expected_revision as i64 {
                return Err(format!(
                    "SOP run revision conflict: expected {expected_revision}, found {}",
                    stored.revision
                ));
            }
            let mut projection = projection(stored)?;
            if projection.record.revision != expected_revision {
                return Err("SOP run Store and Core revisions diverged".into());
            }
            projection
                .record
                .apply(&projection.definition, event, now_ms)
                .map_err(|error| format!("apply SOP event failed: {error}"))?;
            let next = stored_projection(&projection)?;
            if !store
                .update_sop_run_cas(&next, expected_revision as i64)
                .map_err(|error| format!("commit SOP event failed: {error}"))?
            {
                return Err("SOP run revision conflict during commit".into());
            }
            encode(projection)
        }
        SopRunCommand::Load { run_id } => encode(
            store
                .sop_run(&run_id)
                .map_err(|error| format!("load SOP run failed: {error}"))?
                .map(projection)
                .transpose()?,
        ),
        SopRunCommand::ValidateInvocation {
            run_id,
            step_id,
            project_root,
            owner_session_id,
        } => validate_sop_invocation(
            store,
            &run_id,
            &step_id,
            &project_root,
            &owner_session_id,
            None,
        ),
        SopRunCommand::ValidatePermissionInvocation {
            run_id,
            step_id,
            project_root,
            owner_session_id,
            reference_id,
        } => validate_sop_invocation(
            store,
            &run_id,
            &step_id,
            &project_root,
            &owner_session_id,
            Some(&reference_id),
        ),
        SopRunCommand::EnsurePermissionWait {
            run_id,
            step_id,
            reference_id,
            reason,
            now_ms,
        } => {
            let stored = store
                .sop_run(&run_id)
                .map_err(|error| format!("load SOP run failed: {error}"))?
                .ok_or_else(|| format!("unknown SOP run: {run_id}"))?;
            let expected_revision = stored.revision;
            let mut projection = projection(stored)?;
            let step = projection
                .record
                .steps
                .get(&step_id)
                .ok_or_else(|| format!("unknown SOP step: {step_id}"))?;
            if step.current_wait.as_ref().is_some_and(|wait| {
                wait.kind == SopWaitKind::Permission && wait.reference_id == reference_id
            }) {
                return encode(projection);
            }
            if let Some(wait) = step.wait_history.iter().rev().find(|wait| {
                wait.kind == SopWaitKind::Permission && wait.reference_id == reference_id
            }) {
                if wait.reason != reason || wait.resolved_at_epoch_ms.is_none() {
                    return Err(format!(
                        "SOP permission wait {reference_id} conflicts with its durable history"
                    ));
                }
                return encode(projection);
            }
            if step.status != SopStepRunStatus::Running {
                return Err(format!("SOP step {step_id} is not running"));
            }
            projection
                .record
                .apply(
                    &projection.definition,
                    SopRunEvent::SuspendStep {
                        step_id,
                        wait: SopWaitRecord {
                            kind: SopWaitKind::Permission,
                            reference_id,
                            reason,
                            requested_at_epoch_ms: now_ms,
                            disposition: None,
                            evidence_refs: Vec::new(),
                            resolved_at_epoch_ms: None,
                        },
                    },
                    now_ms,
                )
                .map_err(|error| format!("suspend SOP permission wait failed: {error}"))?;
            commit_projection(store, &projection, expected_revision)?;
            encode(projection)
        }
        SopRunCommand::EnsureWait {
            run_id,
            step_id,
            wait,
            now_ms,
        } => {
            let stored = store
                .sop_run(&run_id)
                .map_err(|error| format!("load SOP run failed: {error}"))?
                .ok_or_else(|| format!("unknown SOP run: {run_id}"))?;
            let expected_revision = stored.revision;
            let mut projection = projection(stored)?;
            let step = projection
                .record
                .steps
                .get(&step_id)
                .ok_or_else(|| format!("unknown SOP step: {step_id}"))?;
            if step.current_wait.as_ref().is_some_and(|current| {
                current.kind == wait.kind && current.reference_id == wait.reference_id
            }) {
                return encode(projection);
            }
            if step.status != SopStepRunStatus::Running {
                return Err(format!("SOP step {step_id} is not running"));
            }
            projection
                .record
                .apply(
                    &projection.definition,
                    SopRunEvent::SuspendStep { step_id, wait },
                    now_ms,
                )
                .map_err(|error| format!("suspend SOP wait failed: {error}"))?;
            commit_projection(store, &projection, expected_revision)?;
            encode(projection)
        }
        SopRunCommand::ResolvePermissionWait {
            run_id,
            step_id,
            reference_id,
            disposition,
            evidence_refs,
            now_ms,
        } => {
            let stored = store
                .sop_run(&run_id)
                .map_err(|error| format!("load SOP run failed: {error}"))?
                .ok_or_else(|| format!("unknown SOP run: {run_id}"))?;
            let expected_revision = stored.revision;
            let mut projection = projection(stored)?;
            let current_wait = projection
                .record
                .steps
                .get(&step_id)
                .ok_or_else(|| format!("unknown SOP step: {step_id}"))?
                .current_wait
                .clone();
            if let Some(wait) = current_wait {
                if wait.kind != SopWaitKind::Permission || wait.reference_id != reference_id {
                    return Err(format!("SOP step {step_id} is waiting for another event"));
                }
            } else {
                let resolved = projection
                    .record
                    .steps
                    .get(&step_id)
                    .and_then(|step| {
                        step.wait_history.iter().rev().find(|wait| {
                            wait.kind == SopWaitKind::Permission
                                && wait.reference_id == reference_id
                        })
                    })
                    .ok_or_else(|| format!("SOP step {step_id} has no pending wait"))?;
                if resolved.disposition.as_ref() != Some(&disposition)
                    || resolved.evidence_refs != evidence_refs
                    || resolved.resolved_at_epoch_ms.is_none()
                {
                    return Err(format!(
                        "SOP permission wait {reference_id} conflicts with its durable resolution"
                    ));
                }
                return encode(projection);
            }
            projection
                .record
                .apply(
                    &projection.definition,
                    SopRunEvent::ResumeStep {
                        step_id,
                        disposition,
                        evidence_refs,
                    },
                    now_ms,
                )
                .map_err(|error| format!("resolve SOP permission wait failed: {error}"))?;
            commit_projection(store, &projection, expected_revision)?;
            encode(projection)
        }
        SopRunCommand::List {
            project_root,
            owner_session_id,
            limit,
        } => encode(
            store
                .list_sop_runs(project_root.as_deref(), owner_session_id.as_deref(), limit)
                .map_err(|error| format!("list SOP runs failed: {error}"))?
                .into_iter()
                .map(projection)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    }
}

fn validate_sop_invocation(
    store: &impl SopRunRepository,
    run_id: &str,
    step_id: &str,
    project_root: &str,
    owner_session_id: &str,
    permission_reference: Option<&str>,
) -> Result<Value, String> {
    let stored = store
        .sop_run(run_id)
        .map_err(|error| format!("load SOP run failed: {error}"))?
        .ok_or_else(|| format!("unknown SOP run: {run_id}"))?;
    let projection = projection(stored)?;
    if !same_workspace(&projection.project_root, project_root) {
        return Err("SOP execution lineage belongs to another workspace".into());
    }
    if projection
        .owner_session_id
        .as_deref()
        .is_some_and(|owner| owner != owner_session_id)
    {
        return Err("SOP execution lineage belongs to another session".into());
    }
    let step = projection
        .record
        .steps
        .get(step_id)
        .ok_or_else(|| format!("unknown SOP step: {step_id}"))?;
    let continuing_permission = permission_reference.is_some_and(|reference| {
        projection.record.status == SopRunStatus::WaitingPermission
            && step.status == SopStepRunStatus::Waiting
            && step.current_wait.as_ref().is_some_and(|wait| {
                wait.kind == SopWaitKind::Permission
                    && wait.reference_id == reference
                    && wait.resolved_at_epoch_ms.is_none()
            })
    });
    let executable = projection.record.status == SopRunStatus::Running
        && step.status == SopStepRunStatus::Running;
    if !executable && !continuing_permission {
        return Err(format!(
            "SOP run and step {step_id} are not executable or awaiting this permission"
        ));
    }
    encode(projection)
}

fn commit_projection(
    store: &mut impl SopRunRepository,
    projection: &SopRunProjection,
    expected_revision: i64,
) -> Result<(), String> {
    let next = stored_projection(projection)?;
    if !store
        .update_sop_run_cas(&next, expected_revision)
        .map_err(|error| format!("commit SOP event failed: {error}"))?
    {
        return Err("SOP run revision conflict during commit".into());
    }
    Ok(())
}

fn same_workspace(stored: &str, active: &str) -> bool {
    if std::path::Path::new(stored) == std::path::Path::new(active) {
        return true;
    }
    matches!(
        (
            std::path::Path::new(stored).canonicalize(),
            std::path::Path::new(active).canonicalize(),
        ),
        (Ok(stored), Ok(active)) if stored == active
    )
}

fn projection(stored: StoredSopRun) -> Result<SopRunProjection, String> {
    let definition = serde_json::from_str::<SopDefinition>(&stored.definition_json)
        .map_err(|error| format!("decode pinned SOP definition failed: {error}"))?;
    let record = serde_json::from_str::<SopRunRecord>(&stored.record_json)
        .map_err(|error| format!("decode SOP run record failed: {error}"))?;
    if definition.id != stored.sop_id
        || definition.version as i64 != stored.sop_version
        || record.run_id != stored.run_id
        || record.revision as i64 != stored.revision
    {
        return Err("persisted SOP identity or revision is inconsistent".into());
    }
    Ok(SopRunProjection {
        definition,
        record,
        owner_session_id: stored.owner_session_id,
        project_root: stored.project_root,
    })
}

fn stored_projection(projection: &SopRunProjection) -> Result<StoredSopRun, String> {
    Ok(StoredSopRun {
        run_id: projection.record.run_id.clone(),
        sop_id: projection.definition.id.clone(),
        sop_version: projection.definition.version as i64,
        owner_session_id: projection.owner_session_id.clone(),
        project_root: projection.project_root.clone(),
        status: run_status(projection.record.status).into(),
        definition_json: serde_json::to_string(&projection.definition)
            .map_err(|error| format!("encode SOP definition failed: {error}"))?,
        record_json: serde_json::to_string(&projection.record)
            .map_err(|error| format!("encode SOP record failed: {error}"))?,
        revision: projection.record.revision as i64,
        created_at_ms: projection.record.created_at_epoch_ms,
        updated_at_ms: projection.record.updated_at_epoch_ms,
    })
}

fn run_status(status: SopRunStatus) -> &'static str {
    match status {
        SopRunStatus::Queued => "queued",
        SopRunStatus::Running => "running",
        SopRunStatus::WaitingPermission => "waiting_permission",
        SopRunStatus::WaitingUser => "waiting_user",
        SopRunStatus::WaitingExternal => "waiting_external",
        SopRunStatus::Paused => "paused",
        SopRunStatus::Verifying => "verifying",
        SopRunStatus::Compensating => "compensating",
        SopRunStatus::Completed => "completed",
        SopRunStatus::Failed => "failed",
        SopRunStatus::Cancelled => "cancelled",
    }
}

fn encode(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| format!("encode SOP result failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_agent_engine::agent::sop::{
        SopCompensationContract, SopCompletionContract, SopExecutionMode, SopRetryPolicy, SopStep,
        SopStepBudget, SopVerificationContract, SopWaitDisposition, SopWaitKind, SopWaitRecord,
        SOP_SCHEMA_VERSION,
    };
    use nola_store::NolaStore;
    use serde_json::json;
    use tempfile::tempdir;

    fn definition() -> SopDefinition {
        SopDefinition {
            schema_version: SOP_SCHEMA_VERSION,
            id: "application.sop".into(),
            version: 1,
            title: "Application SOP".into(),
            inputs: json!({}),
            steps: vec![SopStep {
                id: "work".into(),
                title: "Work".into(),
                depends_on: Vec::new(),
                mode: SopExecutionMode::Inline,
                concurrent: false,
                inputs: json!({}),
                outputs: json!({}),
                preconditions: Vec::new(),
                completion: SopCompletionContract {
                    description: "work is complete".into(),
                    evidence_required: true,
                },
                required_capabilities: Vec::new(),
                required_permissions: Vec::new(),
                budget: SopStepBudget::default(),
                retry: SopRetryPolicy::default(),
                artifacts: Vec::new(),
                verifier: None,
                resume_key: Some("work-key".into()),
            }],
            verification: SopVerificationContract::default(),
            compensation: SopCompensationContract::default(),
        }
    }

    #[test]
    fn visible_active_query_preserves_pinned_projection_and_owner_scope() {
        let directory = tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("nola.db")).unwrap();
        for (run_id, owner) in [("owned", Some("session")), ("shared", None), ("foreign", Some("other"))] {
            execute_sop_run_command(&mut store, SopRunCommand::Create {
                definition: definition(), run_id: run_id.into(), inputs: json!({}),
                owner_session_id: owner.map(str::to_owned), project_root: "/workspace".into(), now_ms: 1,
            }).unwrap();
        }
        let value = execute_sop_run_command(&mut store, SopRunCommand::VisibleActive {
            query: VisibleActiveSopRuns { project_root: "/workspace".into(), owner_session_id: Some("session".into()) },
        }).unwrap();
        let runs: Vec<SopRunProjection> = serde_json::from_value(value).unwrap();
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|run| run.record.run_id != "foreign"));
        assert!(runs.iter().all(|run| run.definition == definition()));
    }

    #[test]
    fn persisted_run_uses_revision_cas_and_recovers_the_pinned_definition() {
        let directory = tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("nola.db")).unwrap();
        let created = execute_sop_run_command(
            &mut store,
            SopRunCommand::Create {
                definition: definition(),
                run_id: "run-1".into(),
                inputs: json!({"input": true}),
                owner_session_id: Some("session-1".into()),
                project_root: "/workspace".into(),
                now_ms: 10,
            },
        )
        .unwrap();
        assert_eq!(created["record"]["revision"], 1);

        let started = execute_sop_run_command(
            &mut store,
            SopRunCommand::Apply {
                run_id: "run-1".into(),
                expected_revision: 1,
                event: SopRunEvent::StartStep {
                    step_id: "work".into(),
                    executor_ref: "worker-1".into(),
                },
                now_ms: 11,
            },
        )
        .unwrap();
        assert_eq!(started["record"]["revision"], 2);
        assert!(execute_sop_run_command(
            &mut store,
            SopRunCommand::Apply {
                run_id: "run-1".into(),
                expected_revision: 1,
                event: SopRunEvent::Cancel,
                now_ms: 12,
            },
        )
        .unwrap_err()
        .contains("revision conflict"));

        let suspended = execute_sop_run_command(
            &mut store,
            SopRunCommand::Apply {
                run_id: "run-1".into(),
                expected_revision: 2,
                event: SopRunEvent::SuspendStep {
                    step_id: "work".into(),
                    wait: SopWaitRecord {
                        kind: SopWaitKind::UserInput,
                        reference_id: "user-input:pending-1".into(),
                        reason: "Choose the replacement destination".into(),
                        requested_at_epoch_ms: 12,
                        disposition: None,
                        evidence_refs: Vec::new(),
                        resolved_at_epoch_ms: None,
                    },
                },
                now_ms: 12,
            },
        )
        .unwrap();
        assert_eq!(suspended["record"]["revision"], 3);

        drop(store);
        let mut reopened = NolaStore::open(directory.path().join("nola.db")).unwrap();
        let loaded = execute_sop_run_command(
            &mut reopened,
            SopRunCommand::Load {
                run_id: "run-1".into(),
            },
        )
        .unwrap();
        assert_eq!(loaded["record"]["steps"]["work"]["attempt"], 1);
        assert_eq!(
            loaded["record"]["steps"]["work"]["currentWait"]["referenceId"],
            "user-input:pending-1"
        );
        assert_eq!(loaded["definition"]["id"], "application.sop");

        let resumed = execute_sop_run_command(
            &mut reopened,
            SopRunCommand::Apply {
                run_id: "run-1".into(),
                expected_revision: 3,
                event: SopRunEvent::ResumeStep {
                    step_id: "work".into(),
                    disposition: SopWaitDisposition::Continue,
                    evidence_refs: vec!["user-input:answer-1".into()],
                },
                now_ms: 13,
            },
        )
        .unwrap();
        assert_eq!(resumed["record"]["revision"], 4);
        assert!(resumed["record"]["steps"]["work"]["currentWait"].is_null());
        assert_eq!(
            resumed["record"]["steps"]["work"]["waitHistory"][0]["referenceId"],
            "user-input:pending-1"
        );
    }

    #[test]
    fn invocation_validation_targets_exact_running_parallel_step_and_workspace() {
        let directory = tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let mut parallel = definition();
        parallel.steps[0].concurrent = true;
        let mut second = parallel.steps[0].clone();
        second.id = "review".into();
        second.title = "Review".into();
        second.resume_key = Some("review-key".into());
        parallel.steps.push(second);
        let mut store = NolaStore::open(directory.path().join("nola.db")).unwrap();
        execute_sop_run_command(
            &mut store,
            SopRunCommand::Create {
                definition: parallel,
                run_id: "parallel-run".into(),
                inputs: json!({}),
                owner_session_id: Some("session-1".into()),
                project_root: workspace.to_string_lossy().into_owned(),
                now_ms: 10,
            },
        )
        .unwrap();
        for (revision, step_id) in [(1, "work"), (2, "review")] {
            execute_sop_run_command(
                &mut store,
                SopRunCommand::Apply {
                    run_id: "parallel-run".into(),
                    expected_revision: revision,
                    event: SopRunEvent::StartStep {
                        step_id: step_id.into(),
                        executor_ref: format!("worker-{step_id}"),
                    },
                    now_ms: 10 + revision as i64,
                },
            )
            .unwrap();
        }

        for step_id in ["work", "review"] {
            let validated = execute_sop_run_command(
                &mut store,
                SopRunCommand::ValidateInvocation {
                    run_id: "parallel-run".into(),
                    step_id: step_id.into(),
                    project_root: workspace.to_string_lossy().into_owned(),
                    owner_session_id: "session-1".into(),
                },
            )
            .unwrap();
            assert_eq!(validated["record"]["steps"][step_id]["status"], "running");
        }
        assert!(execute_sop_run_command(
            &mut store,
            SopRunCommand::ValidateInvocation {
                run_id: "parallel-run".into(),
                step_id: "missing".into(),
                project_root: workspace.to_string_lossy().into_owned(),
                owner_session_id: "session-1".into(),
            },
        )
        .unwrap_err()
        .contains("unknown SOP step"));
        assert!(execute_sop_run_command(
            &mut store,
            SopRunCommand::ValidateInvocation {
                run_id: "parallel-run".into(),
                step_id: "work".into(),
                project_root: directory
                    .path()
                    .join("other")
                    .to_string_lossy()
                    .into_owned(),
                owner_session_id: "session-1".into(),
            },
        )
        .unwrap_err()
        .contains("another workspace"));
        assert!(execute_sop_run_command(
            &mut store,
            SopRunCommand::ValidateInvocation {
                run_id: "parallel-run".into(),
                step_id: "work".into(),
                project_root: workspace.to_string_lossy().into_owned(),
                owner_session_id: "session-2".into(),
            },
        )
        .unwrap_err()
        .contains("another session"));

        let waiting = execute_sop_run_command(
            &mut store,
            SopRunCommand::EnsurePermissionWait {
                run_id: "parallel-run".into(),
                step_id: "work".into(),
                reference_id: "permission:request-1".into(),
                reason: "Allow the write".into(),
                now_ms: 20,
            },
        )
        .unwrap();
        assert_eq!(waiting["record"]["revision"], 4);
        assert_eq!(waiting["record"]["status"], "waiting_permission");
        let repeated = execute_sop_run_command(
            &mut store,
            SopRunCommand::EnsurePermissionWait {
                run_id: "parallel-run".into(),
                step_id: "work".into(),
                reference_id: "permission:request-1".into(),
                reason: "Allow the write".into(),
                now_ms: 21,
            },
        )
        .unwrap();
        assert_eq!(repeated["record"]["revision"], 4, "replay is idempotent");
        assert_eq!(
            repeated["record"]["steps"]["review"]["status"], "running",
            "one parallel step waiting must not hide the other running step"
        );

        drop(store);
        let mut reopened = NolaStore::open(directory.path().join("nola.db")).unwrap();
        let resumed = execute_sop_run_command(
            &mut reopened,
            SopRunCommand::ResolvePermissionWait {
                run_id: "parallel-run".into(),
                step_id: "work".into(),
                reference_id: "permission:request-1".into(),
                disposition: SopWaitDisposition::Continue,
                evidence_refs: vec!["permission-result:request-1:allowed".into()],
                now_ms: 22,
            },
        )
        .unwrap();
        assert_eq!(resumed["record"]["revision"], 5);
        assert_eq!(resumed["record"]["steps"]["work"]["status"], "running");
        assert_eq!(
            resumed["record"]["steps"]["work"]["waitHistory"][0]["referenceId"],
            "permission:request-1"
        );
        let replayed_wait = execute_sop_run_command(
            &mut reopened,
            SopRunCommand::EnsurePermissionWait {
                run_id: "parallel-run".into(),
                step_id: "work".into(),
                reference_id: "permission:request-1".into(),
                reason: "Allow the write".into(),
                now_ms: 23,
            },
        )
        .unwrap();
        assert_eq!(replayed_wait["record"]["revision"], 5);
        assert_eq!(
            replayed_wait["record"]["steps"]["work"]["status"],
            "running"
        );
        assert_eq!(
            replayed_wait["record"]["steps"]["work"]["waitHistory"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "a receipt retry must not suspend the resolved SOP step again"
        );
        let replayed_resolution = execute_sop_run_command(
            &mut reopened,
            SopRunCommand::ResolvePermissionWait {
                run_id: "parallel-run".into(),
                step_id: "work".into(),
                reference_id: "permission:request-1".into(),
                disposition: SopWaitDisposition::Continue,
                evidence_refs: vec!["permission-result:request-1:allowed".into()],
                now_ms: 24,
            },
        )
        .unwrap();
        assert_eq!(replayed_resolution["record"]["revision"], 5);
        assert!(execute_sop_run_command(
            &mut reopened,
            SopRunCommand::ResolvePermissionWait {
                run_id: "parallel-run".into(),
                step_id: "work".into(),
                reference_id: "permission:request-1".into(),
                disposition: SopWaitDisposition::Replan,
                evidence_refs: vec!["permission-result:request-1:not_allowed".into()],
                now_ms: 25,
            },
        )
        .unwrap_err()
        .contains("conflicts with its durable resolution"));

        let reconciliation_wait = SopWaitRecord {
            kind: SopWaitKind::UserInput,
            reference_id: "runtime-task:task-1:reconciliation".into(),
            reason: "The worker exited after an observable side effect".into(),
            requested_at_epoch_ms: 23,
            disposition: None,
            evidence_refs: Vec::new(),
            resolved_at_epoch_ms: None,
        };
        let waiting = execute_sop_run_command(
            &mut reopened,
            SopRunCommand::EnsureWait {
                run_id: "parallel-run".into(),
                step_id: "work".into(),
                wait: reconciliation_wait.clone(),
                now_ms: 23,
            },
        )
        .unwrap();
        assert_eq!(waiting["record"]["revision"], 6);
        assert_eq!(waiting["record"]["status"], "waiting_user");
        let repeated = execute_sop_run_command(
            &mut reopened,
            SopRunCommand::EnsureWait {
                run_id: "parallel-run".into(),
                step_id: "work".into(),
                wait: reconciliation_wait,
                now_ms: 24,
            },
        )
        .unwrap();
        assert_eq!(repeated["record"]["revision"], 6);
        assert_eq!(repeated["record"]["steps"]["review"]["status"], "running");
    }

    #[test]
    fn paused_run_rejects_new_tool_invocations_for_running_steps() {
        let directory = tempdir().unwrap();
        let workspace = directory.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let mut store = NolaStore::open(directory.path().join("nola.db")).unwrap();
        execute_sop_run_command(
            &mut store,
            SopRunCommand::Create {
                definition: definition(),
                run_id: "paused-run".into(),
                inputs: json!({}),
                owner_session_id: Some("session-1".into()),
                project_root: workspace.to_string_lossy().into_owned(),
                now_ms: 10,
            },
        )
        .unwrap();
        for (revision, event) in [
            (
                1,
                SopRunEvent::StartStep {
                    step_id: "work".into(),
                    executor_ref: "worker-1".into(),
                },
            ),
            (2, SopRunEvent::Pause),
        ] {
            execute_sop_run_command(
                &mut store,
                SopRunCommand::Apply {
                    run_id: "paused-run".into(),
                    expected_revision: revision,
                    event,
                    now_ms: 10 + revision as i64,
                },
            )
            .unwrap();
        }

        let error = execute_sop_run_command(
            &mut store,
            SopRunCommand::ValidateInvocation {
                run_id: "paused-run".into(),
                step_id: "work".into(),
                project_root: workspace.to_string_lossy().into_owned(),
                owner_session_id: "session-1".into(),
            },
        )
        .unwrap_err();
        assert!(error.contains("not executable"), "{error}");
    }
    include!("sop_port_tests.rs");
}
