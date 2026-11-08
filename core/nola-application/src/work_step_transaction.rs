// SPDX-License-Identifier: Apache-2.0
//! Durable transaction boundary for one side-effecting Work step.

use nola_store_port::work_step_transaction::{
    StoredWorkStepTransaction, WorkStepTransactionRepository,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStepTransactionPhase {
    Prepared,
    Checkpointed,
    EffectObserved,
    Committed,
    Failed,
}

impl WorkStepTransactionPhase {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Checkpointed => "checkpointed",
            Self::EffectObserved => "effect_observed",
            Self::Committed => "committed",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "checkpointed" => Ok(Self::Checkpointed),
            "effect_observed" => Ok(Self::EffectObserved),
            "committed" => Ok(Self::Committed),
            "failed" => Ok(Self::Failed),
            _ => Err(format!("unknown work step transaction phase: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkStepTransaction {
    pub id: String,
    pub work_id: String,
    pub step_id: String,
    pub tool_call_id: String,
    pub root: String,
    pub effect_kind: String,
    pub phase: WorkStepTransactionPhase,
    pub workspace_checkpoint_id: Option<String>,
    pub request: Value,
    pub result: Option<Value>,
    pub result_ref: Option<String>,
    pub error: Option<String>,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkStepTransactionPreparation {
    pub transaction: WorkStepTransaction,
    pub inserted: bool,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkStepTransactionCommand {
    /// Recovery is opportunistic: another owner may already have finalized
    /// this exact observed revision. It must not abort an unrelated Host.
    ReconcileObserved {
        id: String,
        expected_revision: i64,
        updated_at_ms: i64,
    },
    Prepare {
        id: String,
        work_id: String,
        step_id: String,
        tool_call_id: String,
        root: String,
        effect_kind: String,
        workspace_checkpoint_id: Option<String>,
        request: Value,
        created_at_ms: i64,
    },
    Advance {
        id: String,
        expected_revision: i64,
        phase: WorkStepTransactionPhase,
        workspace_checkpoint_id: Option<String>,
        result: Option<Value>,
        result_ref: Option<String>,
        error: Option<String>,
        updated_at_ms: i64,
    },
    Get {
        id: String,
    },
    ListIncomplete {
        work_id: Option<String>,
    },
}

pub(crate) fn execute_work_step_transaction_command<S: WorkStepTransactionRepository + ?Sized>(
    store: &mut S,
    command: WorkStepTransactionCommand,
) -> Result<Value, String> {
    match command {
        WorkStepTransactionCommand::ReconcileObserved { id, expected_revision, updated_at_ms } => {
            let current = store.work_step_transaction(&id).map_err(|e| e.to_string())?
                .ok_or_else(|| format!("unknown work step transaction: {id}"))?;
            if current.revision != expected_revision || current.phase != "effect_observed" {
                return encode(None::<WorkStepTransaction>);
            }
            let observed = transaction(current)?;
            let is_error = observed.result.as_ref().and_then(|value| value.get("error"))
                .and_then(Value::as_bool).unwrap_or(false);
            let phase = if is_error { WorkStepTransactionPhase::Failed } else { WorkStepTransactionPhase::Committed };
            if observed.result.is_none() && observed.result_ref.is_none() {
                return Err("observed work step transaction has no durable result".into());
            }
            let error = is_error.then(|| observed.result.as_ref()
                .and_then(|value| value.get("text")).and_then(Value::as_str)
                .unwrap_or("the observed Host side effect failed").to_owned());
            let result_json = observed.result.as_ref().map(serde_json::to_string).transpose()
                .map_err(|e| e.to_string())?;
            let committed = advance_with_plan_link(store, &id, expected_revision, &phase,
                observed.workspace_checkpoint_id.as_deref(), result_json.as_deref(),
                observed.result_ref.as_deref(), error.as_deref(), updated_at_ms)?;
            // None is an explicit lost CAS, not a swallowed storage error.
            encode(committed.map(transaction).transpose()?)
        }
        WorkStepTransactionCommand::Prepare {
            id,
            work_id,
            step_id,
            tool_call_id,
            root,
            effect_kind,
            workspace_checkpoint_id,
            request,
            created_at_ms,
        } => {
            let request_json = serde_json::to_string(&request)
                .map_err(|error| format!("encode work step request failed: {error}"))?;
            let (stored, inserted) = store
                .prepare_work_step_transaction(&StoredWorkStepTransaction {
                    id,
                    work_id,
                    step_id,
                    tool_call_id,
                    root,
                    effect_kind,
                    phase: "prepared".to_string(),
                    workspace_checkpoint_id,
                    request_json,
                    result_json: None,
                    result_ref: None,
                    error: None,
                    revision: 1,
                    created_at_ms,
                    updated_at_ms: created_at_ms,
                })
                .map_err(|error| error.to_string())?;
            encode(WorkStepTransactionPreparation {
                transaction: transaction(stored)?,
                inserted,
            })
        }
        WorkStepTransactionCommand::Advance {
            id,
            expected_revision,
            phase,
            workspace_checkpoint_id,
            result,
            result_ref,
            error,
            updated_at_ms,
        } => {
            if phase == WorkStepTransactionPhase::Prepared {
                return Err("a work step transaction cannot advance to prepared".to_string());
            }
            if phase == WorkStepTransactionPhase::Committed
                && result.is_none()
                && result_ref.is_none()
            {
                return Err(
                    "a committed work step transaction requires a result or result reference"
                        .to_string(),
                );
            }
            if phase == WorkStepTransactionPhase::Failed
                && error.as_deref().is_none_or(|value| value.trim().is_empty())
            {
                return Err("a failed work step transaction requires an error".to_string());
            }
            let result_json = result
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|encode_error| {
                    format!("encode work step result failed: {encode_error}")
                })?;
            let stored = advance_with_plan_link(
                store,
                &id,
                expected_revision,
                &phase,
                workspace_checkpoint_id.as_deref(),
                result_json.as_deref(),
                result_ref.as_deref(),
                error.as_deref(),
                updated_at_ms,
            )
            .map_err(|store_error| store_error.to_string())?
            .ok_or_else(|| {
                "work step transaction changed before transition was committed".to_string()
            })?;
            encode(transaction(stored)?)
        }
        WorkStepTransactionCommand::Get { id } => {
            let value = store
                .work_step_transaction(&id)
                .map_err(|error| error.to_string())?
                .map(transaction)
                .transpose()?;
            encode(value)
        }
        WorkStepTransactionCommand::ListIncomplete { work_id } => {
            let values = store
                .incomplete_work_step_transactions(work_id.as_deref())
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(transaction)
                .collect::<Result<Vec<_>, _>>()?;
            encode(values)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn advance_with_plan_link<S: WorkStepTransactionRepository + ?Sized>(
    store: &mut S,
    id: &str,
    expected_revision: i64,
    phase: &WorkStepTransactionPhase,
    workspace_checkpoint_id: Option<&str>,
    result_json: Option<&str>,
    result_ref: Option<&str>,
    error: Option<&str>,
    updated_at_ms: i64,
) -> Result<Option<StoredWorkStepTransaction>, String> {
    if matches!(
        phase,
        WorkStepTransactionPhase::Committed | WorkStepTransactionPhase::Failed
    ) {
        if let Some(transaction) = store.work_step_transaction(id).map_err(|e| e.to_string())? {
            if let Some(stored_plan) = store
                .conversation_plan(&transaction.work_id)
                .map_err(|e| e.to_string())?
            {
                let mut plan = super::conversation_plan::conversation_plan(stored_plan)?;
                if let Some(step) = plan
                    .todos
                    .iter_mut()
                    .find(|step| step.id == transaction.step_id)
                {
                    if !step
                        .work_step_transaction_refs
                        .iter()
                        .any(|value| value == id)
                    {
                        step.work_step_transaction_refs.push(id.to_string());
                    }
                    for artifact_ref in artifact_refs_from_result_json(result_json)? {
                        if !step
                            .artifact_refs
                            .iter()
                            .any(|value| value == &artifact_ref)
                        {
                            step.artifact_refs.push(artifact_ref);
                        }
                    }
                    super::conversation_plan::validate_plan_steps(&plan.todos)?;
                    let todos_json =
                        serde_json::to_string(&plan.todos).map_err(|e| e.to_string())?;
                    return store
                        .advance_work_step_transaction_and_link_plan(
                            id,
                            expected_revision,
                            phase.as_str(),
                            workspace_checkpoint_id,
                            result_json,
                            result_ref,
                            error,
                            updated_at_ms,
                            &plan.conversation_id,
                            plan.revision,
                            &plan.sop.sop_id,
                            plan.sop.sop_version,
                            &plan.sop.run_id,
                            &todos_json,
                        )
                        .map_err(|e| e.to_string());
                }
            }
        }
    }
    store
        .advance_work_step_transaction(
            id,
            expected_revision,
            phase.as_str(),
            workspace_checkpoint_id,
            result_json,
            result_ref,
            error,
            updated_at_ms,
        )
        .map_err(|e| e.to_string())
}

fn artifact_refs_from_result_json(result_json: Option<&str>) -> Result<Vec<String>, String> {
    let Some(result_json) = result_json else {
        return Ok(Vec::new());
    };
    let result: serde_json::Value = serde_json::from_str(result_json).map_err(|e| e.to_string())?;
    let Some(artifacts) = result.get("artifacts") else {
        return Ok(Vec::new());
    };
    let artifacts: Vec<nola_host_protocol::HostArtifact> =
        serde_json::from_value(artifacts.clone()).map_err(|e| e.to_string())?;
    nola_host_protocol::validate_host_artifacts(&artifacts)?;
    Ok(artifacts.into_iter().map(|artifact| artifact.uri).collect())
}

pub(crate) fn transaction(value: StoredWorkStepTransaction) -> Result<WorkStepTransaction, String> {
    Ok(WorkStepTransaction {
        id: value.id,
        work_id: value.work_id,
        step_id: value.step_id,
        tool_call_id: value.tool_call_id,
        root: value.root,
        effect_kind: value.effect_kind,
        phase: WorkStepTransactionPhase::parse(&value.phase)?,
        workspace_checkpoint_id: value.workspace_checkpoint_id,
        request: serde_json::from_str(&value.request_json)
            .map_err(|error| format!("decode work step request failed: {error}"))?,
        result: value
            .result_json
            .map(|raw| {
                serde_json::from_str(&raw)
                    .map_err(|error| format!("decode work step result failed: {error}"))
            })
            .transpose()?,
        result_ref: value.result_ref,
        error: value.error,
        revision: value.revision,
        created_at_ms: value.created_at_ms,
        updated_at_ms: value.updated_at_ms,
    })
}

fn encode(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value)
        .map_err(|error| format!("encode work step transaction result failed: {error}"))
}
