use std::collections::BTreeMap;

use nola_application::{
    ApplicationCommand, CreateRuntimeTask, RuntimeTask, RuntimeTaskAttempt, RuntimeTaskCommand,
    RuntimeTaskLeaseWitness, RuntimeTaskResourceUsage,
};
use serde_json::Value;
#[cfg(test)]
use nola_runtime_driver::task_execution_authority::release_runtime_task_attempt;
pub use nola_runtime_driver::task_execution_authority::{
    RuntimeTaskExecutionAuthority, RuntimeTaskLeaseHandle, accept_runtime_task_attempt,
    register_runtime_task_attempt, release_runtime_task_handle,
    runtime_task_lease_state,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RuntimeTaskResultData {
    RuntimeTaskResultDelivery {
        cursor: nola_application::AcknowledgeRuntimeTaskResult,
        /// Actual business result for model continuation, not an authority
        /// parsed from human-facing presentation.
        text: String,
    },
}

pub(crate) fn result_delivery_cursor(
    task: &RuntimeTask,
) -> Result<nola_application::AcknowledgeRuntimeTaskResult, String> {
    let receipt: Option<nola_application::RuntimeTaskTerminalReceipt> =
        crate::application_port::CliApplicationPort::persistent_default()?.execute(
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::TerminalReceipt {
                    task_id: task.id.clone(),
                },
            },
        )?;
    let receipt = receipt
        .filter(|receipt| {
            receipt.task_id == task.id
                && Some(receipt.owner_session_id.as_str()) == task.owner_session_id.as_deref()
                && receipt.status == task.status
                && receipt.child_session_id == task.child_session_id
                && receipt.input_watermark == task.input_watermark
                && receipt.completion_watermark == task.completion_watermark
                && Some(receipt.ended_at_ms) == task.ended_at_ms
        })
        .ok_or("Task result has no exact immutable terminal receipt")?;
    Ok(nola_application::AcknowledgeRuntimeTaskResult {
        owner_conversation_id: receipt.owner_session_id,
        task_id: task.id.clone(),
        terminal_receipt_id: receipt.receipt_id,
        expected_updated_at_ms: task.updated_at_ms,
    })
}

#[cfg(test)]
#[path = "task_execution_authority_tests.rs"]
mod execution_authority_tests;

/// The immutable admission duration is retained. Checking and renewing share
/// the handle lock, so a concurrent heartbeat cannot make this CAS stale.
pub(crate) fn refresh_lease_before_execution(
    handle: &RuntimeTaskLeaseHandle,
    duration_ms: i64,
) -> Result<(), String> {
    if duration_ms <= 0 {
        return Err("task admission has no positive lease duration".into());
    }
    handle.with_attempt(|attempt| {
        let now_ms = operation_time_ms();
        if attempt.lease_expires_at_ms <= now_ms {
            return Err("task execution grant expired before start".into());
        }
        let requested_expiry_ms = now_ms
            .checked_add(duration_ms)
            .ok_or("task lease expiry overflow")?;
        if requested_expiry_ms <= attempt.lease_expires_at_ms {
            return Ok(());
        }
        let next: RuntimeTaskAttempt =
            crate::application_port::CliApplicationPort::persistent_default()?.execute(
                ApplicationCommand::RuntimeTask {
                    command: RuntimeTaskCommand::RenewLease {
                        task_id: attempt.task_id.clone(),
                        attempt_id: attempt.id.clone(),
                        expected_attempt_revision: attempt.revision,
                        worker_generation: attempt.worker_generation.clone(),
                        lease_token: attempt.lease_token.clone(),
                        current_expiry_ms: attempt.lease_expires_at_ms,
                        now_ms,
                        requested_expiry_ms,
                    },
                },
            )?;
        accept_runtime_task_attempt(attempt, &next, requested_expiry_ms)
    })
}

pub fn worker_generation() -> &'static str {
    static GENERATION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    GENERATION
        .get_or_init(|| {
            format!(
                "cli-process-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            )
        })
        .as_str()
}

pub fn create(input: CreateRuntimeTask) -> Result<RuntimeTask, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create { input },
        },
    )
}

pub fn get(task_id: &str) -> Result<Option<RuntimeTask>, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Get {
                id: task_id.to_string(),
            },
        },
    )
}

pub(crate) fn current_attempt(task_id: &str) -> Result<Option<RuntimeTaskAttempt>, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::CurrentAttempt { task_id: task_id.into() },
        },
    )
}

pub(crate) fn submit_terminal_input(
    input: nola_domain::terminal_input::TerminalInputRequest,
    now_ms: i64,
) -> Result<nola_domain::terminal_input::TerminalInputReceipt, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::SubmitTerminalInput {
                input,
                now_ms,
            },
        },
    )
}

pub(crate) fn observe_permission_wait_budget(task_id: &str, now_ms: i64) -> Result<RuntimeTask, String> {
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let suspension: Option<nola_application::RuntimeTaskPermissionSuspension> = application.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::PendingPermissionSuspension { task_id: task_id.into() },
        },
    )?;
    let suspension = suspension.ok_or("waiting Task has no pending permission suspension")?;
    application.execute(ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::ObservePermissionWaitBudget {
            task_id: task_id.into(), suspension_id: suspension.id,
            expected_suspension_revision: suspension.revision, now_ms,
        },
    })
}

pub(crate) fn run_binding(
    task_id: &str,
) -> Result<Option<nola_application::RuntimeTaskRunBinding>, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::RunBinding {
                task_id: task_id.to_string(),
            },
        },
    )
}

pub(crate) fn reconcile_bound_run_terminal(task_id: &str, now_ms: i64) -> Result<bool, String> {
    let Some(binding) = run_binding(task_id)
        .map_err(|error| task_recovery_diagnostic(task_id, "binding_read", error))? else {
        return Ok(false);
    };
    let reconciled: bool = crate::application_port::CliApplicationPort::persistent_default()
        .map_err(|error| task_recovery_diagnostic(task_id, "terminal_application_open", error))?
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
                task_id: task_id.to_string(),
                expected_run_id: binding.run_id,
                now_ms,
            },
        }).map_err(|error| task_recovery_diagnostic(task_id, "bound_terminal_commit", error))?;
    if reconciled {
        // Either side may crash between these durable commands. The immutable
        // receipt and stable enqueue identity make the next attempt identical.
        enqueue_terminal_reconciliation(task_id)?;
    }
    Ok(reconciled)
}

/// Observability only: never classifies a String error into execution certainty
/// or changes its canonical cause. Production excludes all free-form payloads.
pub(crate) fn task_recovery_diagnostic(task_id: &str, stage: &'static str, error: String) -> String {
    use sha2::{Digest, Sha256};
    eprintln!("nola_task_recovery stage={stage} code=operation_failed task={:x} error_digest={:x}",
        Sha256::digest(task_id.as_bytes()), Sha256::digest(error.as_bytes()));
    #[cfg(test)]
    eprintln!("nola_task_recovery_fixture stage={stage} cause={:?}", error.chars().take(1024).collect::<String>());
    error
}

/// Only the Store's terminal receipt can authorize a parent terminal update.
/// The acknowledgement is durable enqueue, not proof the parent applied it.
pub(crate) fn enqueue_terminal_reconciliation(
    task_id: &str,
) -> Result<nola_application::TaskTerminalReconciliationEnqueued, String> {
    let mut application = crate::application_port::CliApplicationPort::persistent_default()
        .map_err(|error| task_recovery_diagnostic(task_id, "receipt_application_open", error))?;
    let receipt: Option<nola_application::RuntimeTaskTerminalReceipt> =
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::TerminalReceipt {
                task_id: task_id.to_string(),
            },
        }).map_err(|error| task_recovery_diagnostic(task_id, "terminal_receipt_read", error))?;
    let receipt = receipt.ok_or_else(|| task_recovery_diagnostic(task_id, "terminal_receipt_missing",
        "terminal task has no durable terminal receipt".into()))?;
    let ack: nola_application::TaskTerminalReconciliationEnqueued =
        application.execute(ApplicationCommand::EnqueueTaskTerminalReconciliation {
            task_id: task_id.to_string(),
            expected_receipt_id: receipt.receipt_id.clone(),
        }).map_err(|error| task_recovery_diagnostic(task_id, "parent_enqueue", error))?;
    if ack.receipt_id != receipt.receipt_id
        || ack.conversation_id != receipt.owner_session_id
        || ack.command_id != format!("terminal:{}", receipt.receipt_id)
    {
        return Err(task_recovery_diagnostic(task_id, "parent_ack_identity",
            "terminal reconciliation acknowledgement belongs to another receipt".into()));
    }
    crate::core_agent_runtime::drain_registered_agent_task_inbox(&ack.conversation_id)
        .map_err(|error| task_recovery_diagnostic(task_id, "parent_wake", error))?;
    Ok(ack)
}

pub fn list_all() -> Result<Vec<RuntimeTask>, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::ListAll,
        },
    )
}

pub(crate) fn agent_authority(
    task_id: &str,
) -> Result<Option<nola_domain::agent_task_admission::RuntimeAgentTaskAuthority>, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::AgentAuthority {
                task_id: task_id.into(),
            },
        },
    )
}

pub fn list_background() -> Result<Vec<crate::store::DbBackgroundTask>, String> {
    let mut records = Vec::new();
    for task in list_all()? {
        if task.kind != "shell_process" { continue; }
        let terminal = matches!(task.status.as_str(), "sealed" | "completed" | "failed" | "cancelled");
        // A cold process must reconstruct undelivered terminal notifications,
        // not only live worker handles. Model consumption is independent.
        if (terminal && !notification_delivered(&terminal_notification_delivery(&task)?)?)
            || (!terminal && !task.cancel_requested)
        {
            records.push(crate::store::background_task_from_runtime(task)?);
        }
    }
    Ok(records)
}

pub fn terminal_notification_delivery(task: &RuntimeTask) -> Result<nola_application::RuntimeTaskNotificationDelivery, String> {
    let receipt: Option<nola_application::RuntimeTaskTerminalReceipt> =
        crate::application_port::CliApplicationPort::persistent_default()?.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::TerminalReceipt { task_id: task.id.clone() },
        })?;
    let receipt = receipt.filter(|receipt| receipt.task_id == task.id
        && receipt.owner_session_id == task.owner_session_id.as_deref().unwrap_or_default()
        && receipt.status == task.status && Some(receipt.ended_at_ms) == task.ended_at_ms)
        .ok_or("Task notification has no exact immutable terminal receipt")?;
    let occurrence_id = format!("terminal:{}", receipt.receipt_id);
    Ok(nola_application::RuntimeTaskNotificationDelivery {
        owner_conversation_id: task.owner_session_id.clone(), task_id: task.id.clone(),
        command_id: format!("cli-presentation:{}:{occurrence_id}", task.id),
        occurrence_id, consumer_id: "cli-interactive".into(),
    })
}

pub fn notification_delivered(delivery: &nola_application::RuntimeTaskNotificationDelivery) -> Result<bool, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::NotificationDelivered { delivery: delivery.clone() },
    })
}

pub fn acknowledge_notification_delivery(delivery: &nola_application::RuntimeTaskNotificationDelivery) -> Result<nola_application::RuntimeTaskNotificationAcknowledgement, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::AcknowledgeDelivery { delivery: delivery.clone() },
    })
}

pub fn request_cancel(task_id: &str, now_ms: i64) -> Result<bool, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::RequestCancel {
                task_id: task_id.to_string(),
                now_ms,
            },
        },
    )
}

pub fn fail_expired_with_event(
    task_id: &str,
    error: String,
    event_kind: &str,
    event_status: &str,
    event_summary: Option<String>,
    event_detail_json: String,
    now_ms: i64,
) -> Result<bool, String> {
    let handle = runtime_task_lease_state(task_id).ok();
    let finished = crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::FailExpiredWithEvent {
                task_id: task_id.to_string(),
                error,
                event_kind: event_kind.to_string(),
                event_status: event_status.to_string(),
                event_summary,
                event_detail_json,
                now_ms,
            },
        },
    )?;
    if finished {
        if let Some(handle) = handle {
            release_runtime_task_handle(&handle);
        }
    }
    Ok(finished)
}

pub fn acknowledge_model_delivery(task_id: &str) -> Result<bool, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::AcknowledgeModelDelivery {
                task_id: task_id.to_string(),
            },
        },
    )
}

pub fn append_event(
    task_id: &str,
    kind: &str,
    status: &str,
    summary: Option<String>,
    detail_json: String,
    created_at_ms: i64,
) -> Result<i64, String> {
    crate::application_port::CliApplicationPort::persistent_default()?.execute(
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::AppendEvent {
                task_id: task_id.to_string(),
                kind: kind.to_string(),
                status: status.to_string(),
                summary,
                detail_json,
                created_at_ms,
            },
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn finish_detailed_with_event(
    handle: &RuntimeTaskLeaseHandle,
    status: &str,
    completion_watermark: Option<i64>,
    progress_summary: Option<String>,
    result_summary: Option<String>,
    error: Option<String>,
    output_path: Option<String>,
    unread: bool,
    event_kind: &str,
    event_status: &str,
    event_summary: Option<String>,
    event_detail_json: String,
    now_ms: i64,
) -> Result<bool, String> {
    finish_detailed_with_event_clock(
        handle,
        status,
        completion_watermark,
        progress_summary,
        result_summary,
        error,
        output_path,
        unread,
        event_kind,
        event_status,
        event_summary,
        event_detail_json,
        None,
        || now_ms,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn finish_detailed_with_event_now(
    handle: &RuntimeTaskLeaseHandle,
    status: &str,
    completion_watermark: Option<i64>,
    progress_summary: Option<String>,
    result_summary: Option<String>,
    error: Option<String>,
    output_path: Option<String>,
    unread: bool,
    event_kind: &str,
    event_status: &str,
    event_summary: Option<String>,
    event_detail_json: String,
    final_observation: Option<nola_application::RuntimeTaskFinalObservation>,
) -> Result<bool, String> {
    finish_detailed_with_event_clock(
        handle,
        status,
        completion_watermark,
        progress_summary,
        result_summary,
        error,
        output_path,
        unread,
        event_kind,
        event_status,
        event_summary,
        event_detail_json,
        final_observation,
        operation_time_ms,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_detailed_with_event_clock(
    handle: &RuntimeTaskLeaseHandle,
    status: &str,
    completion_watermark: Option<i64>,
    progress_summary: Option<String>,
    result_summary: Option<String>,
    error: Option<String>,
    output_path: Option<String>,
    unread: bool,
    event_kind: &str,
    event_status: &str,
    event_summary: Option<String>,
    event_detail_json: String,
    final_observation: Option<nola_application::RuntimeTaskFinalObservation>,
    clock: impl FnOnce() -> i64,
) -> Result<bool, String> {
    let finished = handle.with_attempt(|attempt| {
        let mut port = crate::application_port::CliApplicationPort::persistent_default()?;
        let now_ms = clock();
        port.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::FinishDetailedWithEvent {
                lease: RuntimeTaskLeaseWitness::from(&*attempt),
                status: status.to_string(),
                completion_watermark,
                progress_summary,
                result_summary,
                error,
                output_path,
                unread,
                event_kind: event_kind.to_string(),
                event_status: event_status.to_string(),
                event_summary,
                event_detail_json,
                final_observation,
                now_ms,
            },
        })
    })?;
    if finished {
        release_runtime_task_handle(handle);
    }
    Ok(finished)
}

#[allow(clippy::too_many_arguments)]
pub fn report_progress(
    handle: &RuntimeTaskLeaseHandle,
    progress_summary: Option<String>,
    last_activity: Option<String>,
    output_bytes: Option<u64>,
    resource_usage: Option<RuntimeTaskResourceUsage>,
    metadata_patch: BTreeMap<String, Option<Value>>,
    now_ms: i64,
) -> Result<nola_application::RuntimeTaskProgressAck, String> {
    report_progress_clock(
        handle,
        progress_summary,
        last_activity,
        output_bytes,
        resource_usage,
        metadata_patch,
        || now_ms,
    )
}

/// Progress and its activity event share one exact-lease Store transaction.
/// The clock is captured after acquiring the original attempt witness.
pub(crate) fn report_activity_now(
    handle: &RuntimeTaskLeaseHandle,
    summary: String,
    activity: String,
    detail_json: String,
) -> Result<nola_application::RuntimeTaskProgressAck,String> {
    handle.with_attempt(|attempt| {
        let ack:nola_application::RuntimeTaskProgressAck=
            crate::application_port::CliApplicationPort::persistent_default()?.execute(
                ApplicationCommand::RuntimeTask {command:RuntimeTaskCommand::ReportActivity {
                    lease:RuntimeTaskLeaseWitness::from(&*attempt),summary,activity,detail_json,
                    now_ms:operation_time_ms(),
                }})?;
        let expiry=attempt.lease_expires_at_ms;
        accept_runtime_task_attempt(attempt,&ack.attempt,expiry)?;
        Ok(ack)
    })
}

/// Operation time belongs to the same serialized authority scope as the CAS
/// witness. Process observations retain their own timestamps in metadata.
#[allow(clippy::too_many_arguments)]
pub fn report_progress_now(
    handle: &RuntimeTaskLeaseHandle,
    progress_summary: Option<String>,
    last_activity: Option<String>,
    output_bytes: Option<u64>,
    resource_usage: Option<RuntimeTaskResourceUsage>,
    metadata_patch: BTreeMap<String, Option<Value>>,
) -> Result<nola_application::RuntimeTaskProgressAck, String> {
    report_progress_clock(
        handle,
        progress_summary,
        last_activity,
        output_bytes,
        resource_usage,
        metadata_patch,
        operation_time_ms,
    )
}

fn operation_time_ms() -> i64 {
    nola_runtime_driver::monotonic_epoch_millis()
}

pub(crate) fn advance_terminal_input(handle: &RuntimeTaskLeaseHandle,
    previous: &nola_domain::terminal_input::TerminalInputReceipt,
    next: &nola_domain::terminal_input::TerminalInputReceipt) -> Result<(),String> {
    handle.with_attempt(|attempt| {
        let mut port = crate::application_port::CliApplicationPort::persistent_default()?;
        let ack: nola_application::RuntimeTaskProgressAck = port.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::AdvanceTerminalInput {
                lease:RuntimeTaskLeaseWitness::from(&*attempt),previous:previous.clone(),next:next.clone(),
                now_ms:operation_time_ms(),
            },
        })?;
        let expiry = attempt.lease_expires_at_ms;
        accept_runtime_task_attempt(attempt,&ack.attempt,expiry)?;
        Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
fn report_progress_clock(
    handle: &RuntimeTaskLeaseHandle,
    progress_summary: Option<String>,
    last_activity: Option<String>,
    output_bytes: Option<u64>,
    resource_usage: Option<RuntimeTaskResourceUsage>,
    metadata_patch: BTreeMap<String, Option<Value>>,
    clock: impl FnOnce() -> i64,
) -> Result<nola_application::RuntimeTaskProgressAck, String> {
    handle.with_attempt(|attempt| {
        let mut port = crate::application_port::CliApplicationPort::persistent_default()?;
        let now_ms = clock();
        let ack: nola_application::RuntimeTaskProgressAck =
            port.execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::ReportProgress {
                    task_id: attempt.task_id.clone(),
                    attempt_id: attempt.id.clone(),
                    expected_attempt_revision: attempt.revision,
                    worker_generation: attempt.worker_generation.clone(),
                    lease_token: attempt.lease_token.clone(),
                    current_expiry_ms: attempt.lease_expires_at_ms,
                    progress_summary,
                    last_activity,
                    output_bytes,
                    resource_usage,
                    metadata_patch,
                    now_ms,
                },
            })?;
        let expected_expiry_ms = attempt.lease_expires_at_ms;
        accept_runtime_task_attempt(attempt, &ack.attempt, expected_expiry_ms)?;
        Ok(ack)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_operation_clock_is_serialized_without_rewriting_process_observation() {
        let _environment = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("progress-clock.sqlite");
        let previous = std::env::var_os("NOLA_DB_PATH");
        std::env::set_var("NOLA_DB_PATH", &path);
        let store = crate::store::NolaStore::open(&path).unwrap();
        let mut task: crate::store::DbRuntimeTask = serde_json::from_value(serde_json::json!({
            "id":"progress-clock", "kind":"shell_process", "status":"queued",
            "title":"Process observation", "project_root":"/fixture",
            "unread":false, "cancel_requested":false, "retry_count":0,
            "created_at_ms":100, "updated_at_ms":100, "metadata_json":"{}"
        }))
        .unwrap();
        task.set_scheduling(nola_application::RuntimeTaskScheduling {
            priority: nola_application::RuntimeTaskPriority::Normal,
            queued_at_ms: 100,
            resource_budget: Default::default(),
            resource_usage: Default::default(),
            output_bytes: 0,
            budget_exhaustion: None,
        })
        .unwrap();
        store.create_runtime_task(&task).unwrap();
        let first = store
            .claim_runtime_task(&task.id, "worker", "generation", "token", 200, 1000)
            .unwrap()
            .unwrap();
        let handle = register_runtime_task_attempt(first);
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let contender = handle.clone();
        let first_progress = std::thread::spawn(move || {
            report_progress_clock(
                &contender,
                Some("first".into()),
                None,
                None,
                None,
                BTreeMap::new(),
                || {
                    locked_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    300
                },
            )
            .unwrap()
        });
        locked_rx.recv().unwrap();
        // The process observation predates a competing committed control update.
        // It must remain an observation, not the timestamp paired with its CAS.
        let process_observed_at_ms = 250;
        let (waiting_tx, waiting_rx) = std::sync::mpsc::channel();
        let observer = handle.clone();
        let second_progress = std::thread::spawn(move || {
            waiting_tx.send(()).unwrap();
            report_progress_clock(
                &observer,
                None,
                Some("pid observed".into()),
                None,
                None,
                BTreeMap::from([(
                    "processStartedAtMs".into(),
                    Some(Value::from(process_observed_at_ms)),
                )]),
                || {
                    400
                },
            )
            .unwrap()
        });
        waiting_rx.recv().unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(first_progress.join().unwrap().attempt.revision, 2);
        assert_eq!(second_progress.join().unwrap().attempt.revision, 3);
        let committed = store.runtime_task(&task.id).unwrap().unwrap();
        assert_eq!(committed.updated_at_ms, 400);
        assert_eq!(
            serde_json::from_str::<Value>(&committed.metadata_json).unwrap()["processStartedAtMs"],
            250
        );
        let authority = handle.snapshot().unwrap();
        for at in [399, 1000] {
            let error = report_progress_clock(
                &handle,
                Some("must not commit".into()),
                None,
                None,
                None,
                BTreeMap::new(),
                || at,
            )
            .unwrap_err();
            assert!(
                error.contains(if at == 399 {
                    "moved backwards"
                } else {
                    "expired"
                }),
                "{error}"
            );
            assert_eq!(handle.snapshot().unwrap(), authority);
            assert_eq!(store.runtime_task(&task.id).unwrap().unwrap(), committed);
        }
        release_runtime_task_handle(&handle);
        match previous {
            Some(value) => std::env::set_var("NOLA_DB_PATH", value),
            None => std::env::remove_var("NOLA_DB_PATH"),
        }
    }

    #[test]
    fn worker_authority_is_neither_serialized_nor_accepted_from_config() {
        let _environment = crate::store::test_env_lock();
        let mut config = crate::config::Config::for_test().unwrap();
        let lease = register_runtime_task_attempt(attempt("config-authority", "config-attempt", 1));
        config.host_task_execution = Some(RuntimeTaskExecutionAuthority::new(lease.clone()));
        let mut json = serde_json::to_value(&config).unwrap();
        assert!(json.get("host_task_execution").is_none());
        assert!(!format!("{:?}", config.host_task_execution).contains("token-1"));
        json["host_task_execution"] = serde_json::json!({"lease_token": "injected"});
        let restored: crate::config::Config = serde_json::from_value(json).unwrap();
        assert!(restored.host_task_execution.is_none());
        release_runtime_task_handle(&lease);
    }

    fn attempt(
        task_id: &str,
        id: &str,
        attempt_number: i64,
    ) -> nola_application::RuntimeTaskAttempt {
        nola_application::RuntimeTaskAttempt {
            id: id.into(),
            task_id: task_id.into(),
            attempt_number,
            worker_id: "worker".into(),
            worker_generation: format!("generation-{attempt_number}"),
            lease_token: format!("token-{attempt_number}"),
            lease_expires_at_ms: 100 + attempt_number,
            state: "leased".into(),
            revision: 1,
            claimed_at_ms: attempt_number,
        }
    }

    #[test]
    fn stale_registration_and_cleanup_cannot_remove_a_new_attempt() {
        let task_id = format!("registry-aba-{}", uuid::Uuid::new_v4());
        let first = attempt(&task_id, "attempt-1", 1);
        let second = attempt(&task_id, "attempt-2", 2);
        register_runtime_task_attempt(first.clone());
        register_runtime_task_attempt(second.clone());
        register_runtime_task_attempt(first.clone());
        assert_eq!(
            runtime_task_lease_state(&task_id)
                .unwrap()
                .snapshot()
                .unwrap()
                .id,
            second.id
        );

        release_runtime_task_attempt(&task_id, &first.id);
        assert_eq!(
            runtime_task_lease_state(&task_id)
                .unwrap()
                .snapshot()
                .unwrap()
                .id,
            second.id
        );
        release_runtime_task_attempt(&task_id, &second.id);
        assert!(runtime_task_lease_state(&task_id).is_err());
    }

    #[test]
    fn retained_handle_cannot_borrow_a_successor_even_when_tokens_match() {
        let task_id = format!("bound-owner-{}", uuid::Uuid::new_v4());
        let mut first = attempt(&task_id, "original-attempt", 1);
        let mut second = attempt(&task_id, "successor-attempt", 2);
        first.lease_token = "same-token".into();
        second.lease_token = first.lease_token.clone();
        let original = register_runtime_task_attempt(first.clone());
        let successor = register_runtime_task_attempt(second.clone());
        assert!(!original.same_owner(&successor));
        let witness = RuntimeTaskLeaseWitness::from(&original.snapshot().unwrap());
        assert_eq!(witness.attempt_id, first.id);
        assert_eq!(witness.worker_generation, first.worker_generation);
        assert_ne!(witness.attempt_id, successor.snapshot().unwrap().id);
        release_runtime_task_handle(&original);
        assert!(runtime_task_lease_state(&task_id)
            .unwrap()
            .same_owner(&successor));
        release_runtime_task_handle(&successor);
    }

    #[test]
    fn original_handle_retains_accepted_revision_for_terminal_witnesses() {
        let task_id = format!("refresh-owner-{}", uuid::Uuid::new_v4());
        let first = attempt(&task_id, "refresh-attempt", 1);
        let handle = register_runtime_task_attempt(first.clone());
        let heartbeat = handle.clone();
        heartbeat
            .with_attempt(|current| {
                let mut next = current.clone();
                next.revision += 1;
                next.lease_expires_at_ms = 200;
                accept_runtime_task_attempt(current, &next, 200)
            })
            .unwrap();
        let transported = register_runtime_task_attempt(first);
        assert!(transported.same_owner(&handle));
        let witness = RuntimeTaskLeaseWitness::from(&handle.snapshot().unwrap());
        assert_eq!(witness.expected_attempt_revision, 2);
        assert_eq!(witness.current_expiry_ms, 200);
        release_runtime_task_handle(&handle);
    }

    #[test]
    fn mismatched_response_cannot_change_the_retained_authority() {
        let first = attempt("response-task", "response-attempt", 1);
        for mutation in 0..12 {
            let mut current = first.clone();
            let mut response = first.clone();
            response.revision += 1;
            match mutation {
                0 => response.task_id = "different-task".into(),
                1 => response.id = "different-attempt".into(),
                2 => response.worker_id = "different-worker".into(),
                3 => response.worker_generation = "different-generation".into(),
                4 => response.lease_token = "different-token".into(),
                5 => response.attempt_number += 1,
                6 => response.claimed_at_ms += 1,
                7 => response.state = "finished".into(),
                8 => response.revision += 1,
                9 => response.revision = first.revision,
                10 => response.lease_expires_at_ms += 1,
                _ => response.lease_expires_at_ms -= 1,
            }
            assert!(accept_runtime_task_attempt(
                &mut current,
                &response,
                first.lease_expires_at_ms
            )
            .is_err());
            assert_eq!(current.id, first.id);
            assert_eq!(current.revision, first.revision);
            assert_eq!(current.lease_expires_at_ms, first.lease_expires_at_ms);
        }
    }
}
