use super::{DbRuntimeTask, DbRuntimeTaskEvent, NolaStore};
use crate::runtime_task_port::RuntimeTaskLeaseHandle;
use nola_application::{
    ApplicationCommand, CreateRuntimeTask, RuntimeTaskCommand, RuntimeTaskLeaseWitness,
};
use serde_json::Value;

impl NolaStore {
    fn canonical_task_application(
        &self,
    ) -> Result<crate::application_port::CliApplicationPort, String> {
        crate::application_port::CliApplicationPort::persistent(&self.path)
    }

    pub fn create_runtime_task(&self, task: &DbRuntimeTask) -> Result<(), String> {
        let mut application = self.canonical_task_application()?;
        let _: Value = application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: CreateRuntimeTask::try_from(task.clone())?,
            },
        })?;
        Ok(())
    }

    pub fn runtime_task(&self, id: &str) -> Result<Option<DbRuntimeTask>, String> {
        let mut application = self.canonical_task_application()?;
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Get { id: id.to_string() },
        })
    }

    pub fn list_runtime_tasks(&self, project_root: &str) -> Result<Vec<DbRuntimeTask>, String> {
        let mut application = self.canonical_task_application()?;
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::ListProject {
                project_root: project_root.to_string(),
            },
        })
    }

    pub fn list_all_runtime_tasks(&self) -> Result<Vec<DbRuntimeTask>, String> {
        let mut application = self.canonical_task_application()?;
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::ListAll,
        })
    }

    pub fn claim_runtime_task(
        &self,
        task_id: &str,
        worker_id: &str,
        worker_generation: &str,
        ownership_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<Option<nola_application::RuntimeTaskAttempt>, String> {
        let mut application = self.canonical_task_application()?;
        let attempt: Option<nola_application::RuntimeTaskAttempt> =
            application.execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Claim {
                    task_id: task_id.to_string(),
                    worker_id: worker_id.to_string(),
                    worker_generation: worker_generation.to_string(),
                    lease_token: ownership_token.to_string(),
                    now_ms,
                    lease_until_ms,
                },
            })?;
        if let Some(attempt) = attempt.as_ref() {
            crate::runtime_task_port::register_runtime_task_attempt(attempt.clone());
        }
        Ok(attempt)
    }

    pub fn claim_runtime_task_fair(
        &self,
        task_id: &str,
        worker_id: &str,
        worker_generation: &str,
        ownership_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
        max_running_global: u32,
        max_running_project: u32,
    ) -> Result<Option<nola_application::RuntimeTaskAttempt>, String> {
        let mut application = self.canonical_task_application()?;
        if let Some(task) = self.runtime_task(task_id)? {
            if task.kind == "agent" {
                let owner_conversation_id = task
                    .owner_session_id
                    .ok_or("agent fair claim has no parent")?;
                let attempt: Option<nola_application::RuntimeTaskAttempt> =
                    application.execute(ApplicationCommand::ClaimAgentTaskFair {
                        input: nola_application::ClaimAgentTaskFair {
                            owner_conversation_id,
                            task_id: task_id.into(),
                            worker_id: worker_id.into(),
                            worker_generation: worker_generation.into(),
                            lease_token: ownership_token.into(),
                            now_ms,
                            lease_until_ms,
                            max_running_global,
                            max_running_project,
                        },
                    })?;
                if let Some(attempt) = &attempt {
                    crate::runtime_task_port::register_runtime_task_attempt(attempt.clone());
                }
                return Ok(attempt);
            }
        }
        let attempt: Option<nola_application::RuntimeTaskAttempt> =
            application.execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::ClaimFair {
                    task_id: task_id.to_string(),
                    worker_id: worker_id.to_string(),
                    worker_generation: worker_generation.to_string(),
                    lease_token: ownership_token.to_string(),
                    now_ms,
                    lease_until_ms,
                    max_running_global,
                    max_running_project,
                },
            })?;
        if let Some(attempt) = attempt.as_ref() {
            crate::runtime_task_port::register_runtime_task_attempt(attempt.clone());
        }
        Ok(attempt)
    }

    pub fn request_runtime_task_cancel(&self, task_id: &str, now_ms: i64) -> Result<bool, String> {
        let mut application = self.canonical_task_application()?;
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::RequestCancel {
                task_id: task_id.to_string(),
                now_ms,
            },
        })
    }

    pub fn renew_runtime_task_lease(
        &self,
        handle: &RuntimeTaskLeaseHandle,
        now_ms: i64,
        requested_expiry_ms: i64,
    ) -> Result<nola_application::RuntimeTaskAttempt, String> {
        handle.with_attempt(|attempt| {
            let mut application = self.canonical_task_application()?;
            let next: nola_application::RuntimeTaskAttempt =
                application.execute(ApplicationCommand::RuntimeTask {
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
                })?;
            crate::runtime_task_port::accept_runtime_task_attempt(
                attempt,
                &next,
                requested_expiry_ms,
            )?;
            Ok(next)
        })
    }

    pub fn retry_runtime_task(
        &self,
        handle: &RuntimeTaskLeaseHandle,
        retry_at_ms: i64,
        error: &str,
        now_ms: i64,
    ) -> Result<bool, String> {
        let retried = handle.with_attempt(|attempt| {
            let mut application = self.canonical_task_application()?;
            application.execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Retry {
                    lease: RuntimeTaskLeaseWitness::from(&*attempt),
                    retry_at_ms,
                    error: error.to_string(),
                    now_ms,
                },
            })
        })?;
        if retried {
            crate::runtime_task_port::release_runtime_task_handle(handle);
        }
        Ok(retried)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_runtime_task_detailed_with_event(
        &self,
        handle: &RuntimeTaskLeaseHandle,
        status: &str,
        completion_watermark: Option<i64>,
        progress_summary: Option<&str>,
        result_summary: Option<&str>,
        error: Option<&str>,
        output_path: Option<&str>,
        unread: bool,
        event_kind: &str,
        event_status: &str,
        event_summary: Option<&str>,
        event_detail_json: &str,
        now_ms: i64,
    ) -> Result<bool, String> {
        let finished = handle.with_attempt(|attempt| {
            let mut application = self.canonical_task_application()?;
            application.execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::FinishDetailedWithEvent {
                    lease: RuntimeTaskLeaseWitness::from(&*attempt),
                    status: status.to_string(),
                    completion_watermark,
                    progress_summary: progress_summary.map(str::to_string),
                    result_summary: result_summary.map(str::to_string),
                    error: error.map(str::to_string),
                    output_path: output_path.map(str::to_string),
                    unread,
                    event_kind: event_kind.to_string(),
                    event_status: event_status.to_string(),
                    event_summary: event_summary.map(str::to_string),
                    event_detail_json: event_detail_json.to_string(),
                    final_observation: None,
                    now_ms,
                },
            })
        })?;
        if finished {
            crate::runtime_task_port::release_runtime_task_handle(handle);
        }
        Ok(finished)
    }

    pub fn fail_expired_runtime_task(
        &self,
        task_id: &str,
        error: &str,
        now_ms: i64,
    ) -> Result<bool, String> {
        let mut application = self.canonical_task_application()?;
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::FailExpired {
                task_id: task_id.to_string(),
                error: error.to_string(),
                now_ms,
            },
        })
    }

    pub fn fail_expired_runtime_task_with_event(
        &self,
        task_id: &str,
        error: &str,
        event_kind: &str,
        event_status: &str,
        event_summary: Option<&str>,
        event_detail_json: &str,
        now_ms: i64,
    ) -> Result<bool, String> {
        let mut application = self.canonical_task_application()?;
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::FailExpiredWithEvent {
                task_id: task_id.to_string(),
                error: error.to_string(),
                event_kind: event_kind.to_string(),
                event_status: event_status.to_string(),
                event_summary: event_summary.map(str::to_string),
                event_detail_json: event_detail_json.to_string(),
                now_ms,
            },
        })
    }

    pub fn recover_expired_runtime_task(
        &self,
        task_id: &str,
        error: &str,
        retry_at_ms: i64,
        now_ms: i64,
    ) -> Result<bool, String> {
        let mut application = self.canonical_task_application()?;
        if let Some(task) = self.runtime_task(task_id)? {
            if task.kind == "agent" {
                let current: Option<nola_application::RuntimeTaskAttempt> =
                    application.execute(ApplicationCommand::RuntimeTask {
                        command: RuntimeTaskCommand::CurrentAttempt {
                            task_id: task_id.into(),
                        },
                    })?;
                let Some(attempt) = current else {
                    return Ok(false);
                };
                let owner_conversation_id = task
                    .owner_session_id
                    .ok_or("agent recovery has no parent")?;
                let lease = nola_application::RuntimeTaskLeaseWitness {
                    task_id: attempt.task_id,
                    attempt_id: attempt.id,
                    expected_attempt_revision: attempt.revision,
                    worker_generation: attempt.worker_generation,
                    lease_token: attempt.lease_token,
                    current_expiry_ms: attempt.lease_expires_at_ms,
                };
                return application.execute(ApplicationCommand::RecoverExpiredAgentTask {
                    input: nola_application::RecoverExpiredAgentTask {
                        owner_conversation_id,
                        lease,
                        error: error.into(),
                        retry_at_ms,
                        now_ms,
                    },
                });
            }
        }
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::RecoverExpired {
                task_id: task_id.to_string(),
                error: error.to_string(),
                retry_at_ms,
                now_ms,
            },
        })
    }

    pub fn append_runtime_task_event(
        &self,
        task_id: &str,
        kind: &str,
        status: &str,
        summary: Option<&str>,
        detail_json: &str,
        created_at_ms: i64,
    ) -> Result<(), String> {
        let mut application = self.canonical_task_application()?;
        let _: i64 = application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::AppendEvent {
                task_id: task_id.to_string(),
                kind: kind.to_string(),
                status: status.to_string(),
                summary: summary.map(str::to_string),
                detail_json: detail_json.to_string(),
                created_at_ms,
            },
        })?;
        Ok(())
    }

    pub fn runtime_task_events(
        &self,
        task_id: &str,
        limit: usize,
    ) -> Result<Vec<DbRuntimeTaskEvent>, String> {
        let mut application = self.canonical_task_application()?;
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::ListEvents {
                task_id: task_id.to_string(),
                limit,
            },
        })
    }

    pub fn runtime_task_attempt_retired_for_user_input(&self, task_id: &str, attempt_id: &str) -> Result<bool, String> {
        self.canonical_task_application()?.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::AttemptRetiredForUserInput { task_id: task_id.into(), attempt_id: attempt_id.into() },
        })
    }

    pub fn latest_runtime_task_event_id(&self) -> Result<i64, String> {
        let mut application = self.canonical_task_application()?;
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::LatestEventCursor,
        })
    }

    pub fn runtime_task_events_after_session_cursor(
        &self,
        owner_session_id: &str,
        after_cursor: i64,
        limit: usize,
    ) -> Result<Vec<DbRuntimeTaskEvent>, String> {
        let mut application = self.canonical_task_application()?;
        application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::EventsAfterSessionCursor {
                owner_session_id: owner_session_id.to_string(),
                after_cursor,
                limit,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_task_lease_events_and_completion_survive_service_cold_reopen() {
        let root = std::env::temp_dir().join(format!(
            "nola-runtime-task-service-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("nola.db");
        let store = NolaStore::open(&database).unwrap();
        let mut task = DbRuntimeTask {
            id: "task-service-e2e".to_string(),
            // This exercises generic lease/receipt persistence, not Agent birth.
            kind: "shell_process".to_string(),
            status: "queued".to_string(),
            title: "Service task".to_string(),
            project_root: "/workspace".to_string(),
            owner_session_id: Some("conversation-service-e2e".to_string()),
            parent_task_id: None,
            parent_tool_call_id: Some("call-service-e2e".to_string()),
            child_session_id: None,
            agent_type: Some("explore".to_string()),
            prompt: Some("Inspect through the host".to_string()),
            progress_summary: None,
            last_activity: None,
            result_summary: None,
            error: None,
            output_path: None,
            unread: false,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: Some(10),
            completion_watermark: None,
            created_at_ms: 1_000,
            started_at_ms: None,
            ended_at_ms: None,
            updated_at_ms: 1_000,
            metadata_json: "{}".to_string(),
        };
        task.set_scheduling(nola_application::RuntimeTaskScheduling {
            priority: nola_application::RuntimeTaskPriority::Normal,
            queued_at_ms: task.created_at_ms,
            resource_budget: nola_application::RuntimeTaskResourceBudget::default(),
            output_bytes: 0,
            resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        })
        .unwrap();

        store.create_runtime_task(&task).unwrap();
        let attempt = store
            .claim_runtime_task(
                &task.id,
                "worker-a",
                "generation-a",
                "lease-a",
                1_001,
                61_001,
            )
            .unwrap()
            .expect("typed runtime task attempt");
        assert_eq!(attempt.worker_generation, "generation-a");
        assert_eq!(attempt.lease_token, "lease-a");
        assert_eq!(attempt.lease_expires_at_ms, 61_001);
        assert_eq!(attempt.revision, 1);
        let lease = crate::runtime_task_port::register_runtime_task_attempt(attempt);
        let attempt = store
            .renew_runtime_task_lease(&lease, 1_002, 61_002)
            .unwrap();
        assert_eq!(attempt.revision, 2);
        assert_eq!(attempt.lease_expires_at_ms, 61_002);
        // Generic execution has one durable authority, never a synthetic parent
        // Core lease. The negative finish below still checks the exact witness.
        assert_eq!(
            store.runtime_task(&task.id).unwrap().unwrap().status,
            "running"
        );
        store
            .append_runtime_task_event(
                &task.id,
                "tool",
                "running",
                Some("Reading source"),
                r#"{"path":"src/main.rs"}"#,
                1_003,
            )
            .unwrap();
        let mut wrong_attempt = attempt.clone();
        wrong_attempt.lease_token = "wrong-lease".to_string();
        let wrong_lease = crate::runtime_task_port::register_runtime_task_attempt(wrong_attempt);
        assert!(!store
            .finish_runtime_task_detailed_with_event(
                &wrong_lease,
                "completed",
                Some(10),
                Some("completed"),
                None,
                None,
                None,
                true,
                "finished",
                "completed",
                None,
                "{}",
                1_004,
            )
            .unwrap());
        assert!(store
            .finish_runtime_task_detailed_with_event(
                &lease,
                "completed",
                Some(10),
                Some("completed"),
                None,
                None,
                None,
                true,
                "finished",
                "completed",
                None,
                "{}",
                1_005,
            )
            .unwrap());
        drop(store);

        let reopened = NolaStore::open(&database).unwrap();
        let loaded = reopened.runtime_task(&task.id).unwrap().unwrap();
        assert_eq!(loaded.status, "completed");
        assert_eq!(loaded.completion_watermark, Some(10));
        let events = reopened
            .runtime_task_events_after_session_cursor("conversation-service-e2e", 0, 20)
            .unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "created")
                .count(),
            1
        );
        let created = events.iter().find(|event| event.kind == "created").unwrap();
        assert_eq!(created.status, "queued");
        assert_eq!(created.summary.as_deref(), Some(task.title.as_str()));
        assert!(events.iter().any(|event| {
            event.summary.as_deref() == Some("Reading source")
                && event.detail_json.contains("src/main.rs")
        }));
        assert!(events.iter().any(|event| event.kind == "finished"));
        drop(reopened);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_event_and_notification_ack_are_atomic_and_never_restore_a_stale_status() {
        let root = std::env::temp_dir().join(format!(
            "nola-runtime-task-terminal-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("nola.db");
        let store = NolaStore::open(&database).unwrap();
        let mut task = DbRuntimeTask {
            id: "task-terminal-atomic".to_string(),
            kind: "shell_process".to_string(),
            status: "queued".to_string(),
            title: "Atomic terminal task".to_string(),
            project_root: "/workspace".to_string(),
            owner_session_id: Some("conversation-terminal".to_string()),
            parent_task_id: None,
            parent_tool_call_id: None,
            child_session_id: None,
            agent_type: None,
            prompt: None,
            progress_summary: None,
            last_activity: None,
            result_summary: None,
            error: None,
            output_path: None,
            unread: false,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: None,
            completion_watermark: None,
            created_at_ms: 1_000,
            started_at_ms: None,
            ended_at_ms: None,
            updated_at_ms: 1_000,
            metadata_json: "{}".to_string(),
        };
        task.set_scheduling(nola_application::RuntimeTaskScheduling {
            priority: nola_application::RuntimeTaskPriority::Normal,
            queued_at_ms: task.created_at_ms,
            resource_budget: nola_application::RuntimeTaskResourceBudget::default(),
            output_bytes: 0,
            resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        })
        .unwrap();
        store.create_runtime_task(&task).unwrap();
        let attempt = store
            .claim_runtime_task(&task.id, "worker", "generation", "lease", 1_001, 61_001)
            .unwrap()
            .expect("claim terminal attempt");
        let lease = crate::runtime_task_port::register_runtime_task_attempt(attempt);
        task = store.runtime_task(&task.id).unwrap().unwrap();
        assert_eq!(task.status, "running");
        assert!(store
            .finish_runtime_task_detailed_with_event(
                &lease,
                "completed",
                None,
                Some("completed"),
                Some("exited(0)"),
                None,
                None,
                true,
                "finished",
                "completed",
                Some("exited(0)"),
                r#"{"exit_code":0}"#,
                1_002,
            )
            .unwrap());
        let mut application = store.canonical_task_application().unwrap();
        let receipt: Option<nola_application::RuntimeTaskTerminalReceipt> = application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::TerminalReceipt { task_id: task.id.clone() },
        }).unwrap();
        let notification = nola_application::RuntimeTaskNotificationDelivery {
            owner_conversation_id: task.owner_session_id.clone(), task_id: task.id.clone(),
            occurrence_id: format!("terminal:{}", receipt.unwrap().receipt_id), consumer_id: "cli".into(), command_id: "delivery".into(),
        };
        let delivered: nola_application::RuntimeTaskNotificationAcknowledgement = application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::AcknowledgeDelivery { delivery: notification.clone() },
        }).unwrap();
        assert_eq!(delivered, nola_application::RuntimeTaskNotificationAcknowledgement::Delivered);

        let loaded = store.runtime_task(&task.id).unwrap().unwrap();
        assert_eq!(loaded.status, "completed");
        assert_eq!(loaded.ended_at_ms, Some(1_002));
        let delivered: bool = application.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::NotificationDelivered { delivery: notification },
        }).unwrap();
        assert!(delivered);
        let events = store.runtime_task_events(&task.id, 10).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind, "notification_delivered");
        assert_eq!(events[1].kind, "finished");
        assert_eq!(events[1].status, "completed");
        assert_eq!(events[2].kind, "created");
        assert_eq!(events[2].status, "queued");
        assert_eq!(events[2].summary.as_deref(), Some(task.title.as_str()));
        let inspection = rusqlite::Connection::open(&database).unwrap();
        let parent_commands: i64 = inspection.query_row("SELECT COUNT(*) FROM agent_commands", [], |row| row.get(0)).unwrap();
        assert_eq!(parent_commands, 0, "Shell terminal evidence is not an Agent parent reconciliation");
        drop(inspection);

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}
