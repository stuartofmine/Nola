use nola_application::{
    ApplicationCommand, WorkStepTransaction,
    WorkStepTransactionCommand, WorkStepTransactionPhase, WorkStepTransactionPreparation,
};
use serde_json::Value;

pub fn work_step_transaction(id: &str) -> Result<Option<WorkStepTransaction>, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::WorkStepTransaction {
            command: WorkStepTransactionCommand::Get { id: id.to_string() },
        },
    )
}

pub fn incomplete_work_step_transactions(
    work_id: Option<&str>,
) -> Result<Vec<WorkStepTransaction>, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::WorkStepTransaction {
            command: WorkStepTransactionCommand::ListIncomplete {
                work_id: work_id.map(str::to_string),
            },
        },
    )
}


/// Finalizes outcomes that were durably observed before a Host crash.
/// Prepared calls remain replayable by their original ToolCall identity;
/// checkpointed calls remain unresolved because execution may already have
/// crossed the external side-effect boundary.
pub fn reconcile_observed_work_step_transactions() -> Result<Vec<String>, String> {
    let mut reconciled = Vec::new();
    for transaction in incomplete_work_step_transactions(None)? {
        if transaction.phase != WorkStepTransactionPhase::EffectObserved {
            continue;
        }
        let committed: Option<WorkStepTransaction> =
            crate::application_port::CliApplicationPort::persistent_default()?.execute(
                ApplicationCommand::WorkStepTransaction {
                    command: WorkStepTransactionCommand::ReconcileObserved {
                        id: transaction.id.clone(), expected_revision: transaction.revision,
                        updated_at_ms: chrono::Utc::now().timestamp_millis(),
                    },
                },
            )?;
        if committed.is_some() {
            reconciled.push(transaction.id);
        }
    }
    Ok(reconciled)
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_work_step_transaction(
    id: &str,
    work_id: &str,
    step_id: &str,
    tool_call_id: &str,
    root: &str,
    effect_kind: &str,
    workspace_checkpoint_id: Option<String>,
    request: Value,
) -> Result<WorkStepTransactionPreparation, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::WorkStepTransaction {
            command: WorkStepTransactionCommand::Prepare {
                id: id.to_string(),
                work_id: work_id.to_string(),
                step_id: step_id.to_string(),
                tool_call_id: tool_call_id.to_string(),
                root: root.to_string(),
                effect_kind: effect_kind.to_string(),
                workspace_checkpoint_id,
                request,
                created_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn advance_work_step_transaction(
    id: &str,
    expected_revision: i64,
    phase: WorkStepTransactionPhase,
    workspace_checkpoint_id: Option<String>,
    result: Option<Value>,
    result_ref: Option<String>,
    error: Option<String>,
) -> Result<WorkStepTransaction, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::WorkStepTransaction {
            command: WorkStepTransactionCommand::Advance {
                id: id.to_string(),
                expected_revision,
                phase,
                workspace_checkpoint_id,
                result,
                result_ref,
                error,
                updated_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cold_start_commits_observed_result_but_preserves_uncertain_checkpoint() {
        let _environment = crate::store::test_env_lock();
        let directory =
            std::env::temp_dir().join(format!("nola-work-step-reconcile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).expect("temporary transaction directory");
        let path = directory.join("nola.db");
        std::env::set_var("NOLA_DB_PATH", &path);

        let observed = prepare_work_step_transaction(
            "transaction-observed",
            "conversation-alpha",
            "step-1",
            "call-1",
            directory.to_str().unwrap(),
            "mutate",
            None,
            json!({"capability_id":"write_file"}),
        )
        .expect("prepare observed transaction")
        .transaction;
        let observed = advance_work_step_transaction(
            &observed.id,
            observed.revision,
            WorkStepTransactionPhase::Checkpointed,
            None,
            None,
            None,
            None,
        )
        .expect("claim observed transaction");
        advance_work_step_transaction(
            &observed.id,
            observed.revision,
            WorkStepTransactionPhase::EffectObserved,
            observed.workspace_checkpoint_id,
            Some(json!({"text":"written","display":"Written","error":false,"activation":null})),
            None,
            None,
        )
        .expect("record observed result");

        let uncertain = prepare_work_step_transaction(
            "transaction-uncertain",
            "conversation-alpha",
            "step-2",
            "call-2",
            directory.to_str().unwrap(),
            "mutate",
            None,
            json!({"capability_id":"append_file"}),
        )
        .expect("prepare uncertain transaction")
        .transaction;
        advance_work_step_transaction(
            &uncertain.id,
            uncertain.revision,
            WorkStepTransactionPhase::Checkpointed,
            None,
            None,
            None,
            None,
        )
        .expect("claim uncertain transaction");
        assert_eq!(
            reconcile_observed_work_step_transactions().expect("reconcile cold-start state"),
            vec!["transaction-observed".to_string()]
        );
        assert_eq!(
            work_step_transaction("transaction-observed")
                .expect("read committed transaction")
                .expect("committed transaction exists")
                .phase,
            WorkStepTransactionPhase::Committed
        );
        let incomplete = incomplete_work_step_transactions(Some("conversation-alpha"))
            .expect("list incomplete transactions");
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].id, "transaction-uncertain");
        assert_eq!(incomplete[0].phase, WorkStepTransactionPhase::Checkpointed);
        std::env::remove_var("NOLA_DB_PATH");
        let _ = std::fs::remove_dir_all(directory);
    }
}
