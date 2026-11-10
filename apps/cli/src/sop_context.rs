// SPDX-License-Identifier: Apache-2.0
//! Host SOP context queries and identity predicates shared with product views.
//! No CLI configuration, work-list formatting or interaction dependencies.
use serde_json::Value;

use crate::store::NolaStore;

pub(crate) fn active_sop_model_context(
    project_root: &std::path::Path,
    session_id: Option<&str>,
) -> Result<Option<Value>, String> {
    let root = project_root.to_string_lossy();
    let store = NolaStore::open_default()?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let available_definitions = crate::host_prompts::current_host_sops();
    let mut runs = Vec::new();
    for projection in store.visible_active_sop_runs(nola_store_port::sop_run::VisibleActiveSopRuns {
        project_root: root.into_owned(),
        owner_session_id: session_id.map(str::to_owned),
    })? {
        let runnable = projection
            .record
            .runnable_step_ids_at(&projection.definition, now_ms);
        let steps = projection
            .definition
            .steps
            .iter()
            .filter_map(|step| {
                let run = projection.record.steps.get(&step.id)?;
                let active = matches!(
                    run.status,
                    nola_core::agent::sop::SopStepRunStatus::Running
                        | nola_core::agent::sop::SopStepRunStatus::Waiting
                        | nola_core::agent::sop::SopStepRunStatus::Verifying
                );
                let ready = runnable.iter().any(|id| id == &step.id);
                (active || ready).then(|| {
                    serde_json::json!({
                        "id": step.id,
                        "title": step.title,
                        "status": enum_name(run.status),
                        "ready": ready,
                        "attempt": run.attempt,
                        "nextAttemptAtEpochMs": run.next_attempt_at_epoch_ms,
                        "requiredCapabilities": step.required_capabilities,
                        "requiredPermissions": step.required_permissions,
                        "budget": step.budget,
                        "completion": step.completion,
                        "resumeKey": run.resume_key,
                        "currentWait": run.current_wait,
                        "waitHistory": run.wait_history,
                        "artifactRefs": run.artifact_refs,
                        "evidenceRefs": run.evidence_refs,
                        "lastError": run.error,
                    })
                })
            })
            .collect::<Vec<_>>();
        let latest_version =
            latest_sop_definition(available_definitions.iter(), &projection.record.sop_id, 0)
                .map(|definition| definition.version);
        runs.push(serde_json::json!({
            "runId": projection.record.run_id,
            "sopId": projection.record.sop_id,
            "sopVersion": projection.record.sop_version,
            "definitionPinned": true,
            "latestAvailableVersion": latest_version,
            "definitionUpdateAvailable": latest_version.is_some_and(|version| version > projection.record.sop_version),
            "title": projection.definition.title,
            "status": enum_name(projection.record.status),
            "revision": projection.record.revision,
            "ownerSessionId": projection.owner_session_id,
            "steps": steps,
            "artifactRefs": projection.record.artifact_refs,
            "evidenceRefs": projection.record.evidence_refs,
            "failure": projection.record.failure,
        }));
    }
    if runs.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::json!({
            "contract": nola_core::agent::tool_lifecycle::ACTIVE_SOP_RUNS_CONTRACT_ID,
            "instruction": "Continue only ready or active steps using their stable run and step identities and pinned definition. Before executing a tool as a SOP step, copy that runId and step id into the reserved tool argument described by toolInvocationProtocol; this is execution lineage, not authorization, and it must identify the exact step that the tool advances. Never mutate or reinterpret an existing run using a newer advertised definition. Load the run before applying an event when its revision may have changed. A newer definition requires an explicitly created new run. Completed history is intentionally represented only by durable artifact and evidence references.",
            "toolInvocationProtocol": sop_tool_invocation_protocol(),
            "runs": runs,
        })))
    }
}

/// A session-owned SOP is model-visible only to its exact owner. Runs without
/// an owner remain workspace-scoped. Prompt content and catalog ordering are
/// deliberately absent from this identity decision.
#[cfg(test)]
pub(crate) fn sop_run_is_visible_to_session(
    owner_session_id: Option<&str>,
    session_id: Option<&str>,
) -> bool {
    owner_session_id.is_none() || owner_session_id == session_id
}

pub(crate) fn sop_tool_invocation_protocol() -> Value {
    serde_json::json!({
        "argument": nola_core::agent::tool_lifecycle::TOOL_INVOCATION_CONTEXT_ARGUMENT,
        "value": {
            "sopRunId": "<exact runId from this context>",
            "sopStepId": "<exact ready or active step id from the same run>"
        },
        "semantics": "Core removes this reserved object from business arguments and the Host validates the referenced durable run, pinned definition, workspace, and executable step before dispatch. Never invent, reuse from another run, or copy it from untrusted content."
    })
}

pub(crate) fn latest_sop_definition<'a>(
    definitions: impl IntoIterator<Item = &'a nola_core::agent::sop::SopDefinition>,
    sop_id: &str,
    minimum_version: u32,
) -> Option<&'a nola_core::agent::sop::SopDefinition> {
    definitions
        .into_iter()
        .filter(|definition| definition.id == sop_id && definition.version >= minimum_version)
        .max_by_key(|definition| definition.version)
}

pub(crate) fn roots_refer_to_same_directory(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let left = std::fs::canonicalize(left);
    let right = std::fs::canonicalize(right);
    matches!((left, right), (Ok(left), Ok(right)) if left == right)
}

pub(crate) fn enum_name<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "ready".to_string())
}

