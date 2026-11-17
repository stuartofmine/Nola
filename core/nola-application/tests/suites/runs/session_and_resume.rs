// SPDX-License-Identifier: Apache-2.0
//! Session projection resume, core session identity, idle checkpoints, resume snapshots, pre-start cancellation, and WAL payloads.

mod session_projection_resume_e2e {
    use crate::support;
    use nola_application::{
        execute_application_command, AcknowledgeResumeSnapshot, ApplicationCommand, CreateRuntimeTask,
        NewPermissionRequest, PermissionGrantDecision, PermissionGrantInput, PermissionGrantTarget,
        PermissionRequestCommand, ResumeAction, ResumeSnapshot, RuntimeTask, RuntimeTaskAttempt,
        RuntimeTaskCommand, RuntimeTaskLeaseWitness, RuntimeTaskPriority, RuntimeTaskResourceBudget,
        RuntimeTaskResourceUsage, RuntimeTaskScheduling, SessionProjection, WorkStepTransactionCommand,
        WorkStepTransactionPhase,
    };
    use nola_agent_engine::agent::recovery::CheckpointReason;
    use nola_agent_engine::agent::{
        AgentCore, AgentCoreConfig, GoalRecordStatus, GoalRequirement, GoalRuntimeCommand,
        GoalRuntimeCommandEnvelope, SessionId,
    };
    use nola_store::{CreateConversationInput, NolaStore, SaveCheckpointInput};
    use tempfile::tempdir;

    fn seed_active_goal(
        store: &mut NolaStore,
        conversation_id: &str,
        project_root: &str,
        updated_at_ms: i64,
    ) {
        store
            .create_conversation(&CreateConversationInput {
                id: conversation_id.to_string(),
                title: format!("Goal {conversation_id}"),
                revision: 1,
                settings_json: Some(serde_json::json!({"project_root": project_root}).to_string()),
                created_at_ms: updated_at_ms,
                updated_at_ms,
            })
            .expect("create conversation");

        let core_config = AgentCoreConfig {
            turn_identity: support::model_identity::identity(),
            session_id: Some(SessionId::from_string(conversation_id.to_string())),
            model: Some("test-model".to_string()),
            ..AgentCoreConfig::default()
        };
        support::goal_execution::admit_start(
            store,
            nola_application::ReduceAndCommitIdleSubmission {
                conversation_id: conversation_id.into(),
                core_config,
                submission: nola_agent_engine::agent::AgentSubmission::GoalRuntime {
                    envelope: GoalRuntimeCommandEnvelope {
                        operation_id: format!("start-{conversation_id}"),
                        command: GoalRuntimeCommand::Start {
                            id: format!("goal-{conversation_id}"),
                            objective: format!("Resume {conversation_id}"),
                            constraints: Vec::new(),
                            requirements: vec![GoalRequirement {
                                acceptance_policy: nola_agent_engine::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                                semantic_revision: 1, id: "resume-after-restart".to_string(),
                                description: "Resume from the durable typed snapshot".to_string(),
                                dependencies: Vec::new(),
                                status: GoalRecordStatus::Pending,
                                evidence_refs: Vec::new(),
                            }],
                            now_epoch_ms: updated_at_ms,
                        },
                    },
                },
                committed_at_ms: updated_at_ms,
            },
        )
        .expect("commit real Goal start baseline");
    }

    #[test]
    fn narrow_goal_query_matches_canonical_projection_after_cold_open() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("narrow-goal.db");
        let mut store = NolaStore::open(&path).unwrap();
        seed_active_goal(&mut store, "conversation", "/workspace", 100);
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        let expected = nola_application::load_session_projection(&store, "conversation")
            .unwrap().unwrap().goal_projection;
        let actual = execute_application_command(&mut store,
            ApplicationCommand::LoadSessionGoalProjection { conversation_id: "conversation".into() })
            .unwrap();
        assert_eq!(actual, serde_json::to_value(Some(expected)).unwrap());
        let missing = execute_application_command(&mut store,
            ApplicationCommand::LoadSessionGoalProjection { conversation_id: "missing".into() })
            .unwrap();
        assert!(missing.is_null());
    }

    #[test]
    fn corrupted_newer_checkpoint_is_reported_instead_of_skipping_to_older_goal() {
        use nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository;
        for missing_body in [true, false] {
            let directory = tempdir().unwrap();
            let path = directory.path().join("corrupt-resume.db");
            let mut store = NolaStore::open(&path).unwrap();
            seed_active_goal(&mut store, "older", "/workspace", 100);
            seed_session_without_goal(&mut store, "broken", "/workspace", 200);
            drop(store);
            let sql = rusqlite::Connection::open(&path).unwrap();
            sql.execute("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES('broken-run','broken','maintenance','running',1,200,200)", []).unwrap();
            if missing_body {
                sql.execute(
                    "DELETE FROM core_checkpoint_bodies WHERE conversation_id='broken'",
                    [],
                )
                .unwrap();
            } else {
                // Deliberate storage corruption bypasses the normal writer trigger,
                // which correctly refuses malformed checkpoint JSON at publication.
                sql.execute_batch("DROP TRIGGER core_delivery_witness_update;
                    UPDATE core_checkpoint_bodies SET checkpoint_json='not-json' WHERE conversation_id='broken';").unwrap();
            }
            drop(sql);
            let mut store = NolaStore::open(&path).unwrap();
            assert!(
                nola_application::load_resume_snapshot(&mut store, "/workspace", None, 300).is_err()
            );
            assert!(nola_application::load_session_projection(&store, "broken").is_err());
            if missing_body {
                let error = match store.inspect_exact_run("broken-run") {
                    Err(error) => error.to_string(),
                    Ok(_) => panic!("broken Run unexpectedly recovered without its checkpoint body"),
                };
                assert!(error.contains("header/body is incomplete"), "{error}");
            }
        }
    }

    #[test]
    fn cancelled_goal_control_commits_baseline_and_reveals_older_eligible_goal() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("goal-controls.db");
        let mut store = NolaStore::open(&path).unwrap();
        seed_active_goal(&mut store, "older", "/workspace", 100);
        seed_active_goal(&mut store, "newer", "/workspace", 200);
        nola_application::reduce_and_commit_idle_submission(
            &mut store,
            nola_application::ReduceAndCommitIdleSubmission {
                conversation_id: "newer".into(),
                core_config: AgentCoreConfig {
                    turn_identity: support::model_identity::identity(),
                    session_id: Some(SessionId::from_string("newer".into())),
                    model: Some("test-model".into()),
                    ..Default::default()
                },
                submission: nola_agent_engine::agent::AgentSubmission::GoalRuntime {
                    envelope: GoalRuntimeCommandEnvelope {
                        operation_id: "cancel-newer".into(),
                        command: GoalRuntimeCommand::Cancel { now_epoch_ms: 300 },
                    },
                },
                committed_at_ms: 300,
            },
        )
        .unwrap();
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        let value = execute_application_command(
            &mut store,
            ApplicationCommand::FindResumableGoalSession {
                project_root: "/workspace".into(),
            },
        )
        .unwrap();
        let projection: SessionProjection = serde_json::from_value(value).unwrap();
        assert_eq!(projection.conversation_id, "older");
        let newer = nola_application::load_session_projection(&store, "newer")
            .unwrap()
            .unwrap();
        assert!(newer.goal_projection.active.unwrap().status.is_terminal());
    }

    fn seed_session_without_goal(
        store: &mut NolaStore,
        conversation_id: &str,
        project_root: &str,
        updated_at_ms: i64,
    ) {
        store
            .create_conversation(&CreateConversationInput {
                id: conversation_id.to_string(),
                title: format!("No Goal {conversation_id}"),
                revision: 1,
                settings_json: Some(serde_json::json!({"project_root": project_root}).to_string()),
                created_at_ms: updated_at_ms,
                updated_at_ms,
            })
            .expect("create conversation without Goal");
        let core = AgentCore::try_new(AgentCoreConfig {
            turn_identity: support::model_identity::identity(),
            session_id: Some(SessionId::from_string(conversation_id.to_string())),
            model: Some("test-model".to_string()),
            ..AgentCoreConfig::default()
        })
        .expect("create Core without Goal");
        let checkpoint = core.checkpoint(CheckpointReason::Manual);
        store
            .save_checkpoint(&SaveCheckpointInput {
                conversation_id: conversation_id.to_string(),
                expected_revision: 0,
                revision: 1,
                event_cursor: checkpoint.last_event_cursor.sequence as i64,
                checkpoint_json: serde_json::to_string(&checkpoint).expect("encode checkpoint"),
                updated_at_ms,
            })
            .expect("save checkpoint without Goal");
    }

    fn scheduled_task(mut task: RuntimeTask) -> RuntimeTask {
        task.set_scheduling(RuntimeTaskScheduling {
            priority: RuntimeTaskPriority::Normal,
            queued_at_ms: task.created_at_ms,
            resource_budget: RuntimeTaskResourceBudget {
                max_wall_time_ms: Some(60_000),
                max_output_bytes: Some(1_048_576),
                max_cpu_time_ms: None,
                max_memory_bytes: None,
                max_processes: None,
            },
            output_bytes: 0,
            resource_usage: RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        })
        .expect("valid runtime task scheduling");
        task
    }

    fn queued_agent_task(id: &str, owner_session_id: Option<&str>, created_at_ms: i64) -> RuntimeTask {
        scheduled_task(RuntimeTask {
            id: id.into(),
            kind: "agent".into(),
            status: "queued".into(),
            title: id.into(),
            project_root: "/workspace/resume-scope".into(),
            owner_session_id: owner_session_id.map(str::to_string),
            parent_task_id: None,
            parent_tool_call_id: Some(format!("call-{id}")),
            child_session_id: Some(format!("child-{id}")),
            agent_type: Some("general".into()),
            prompt: Some("continue durable work".into()),
            progress_summary: Some("queued".into()),
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
            input_watermark: Some(created_at_ms),
            completion_watermark: None,
            created_at_ms,
            started_at_ms: None,
            ended_at_ms: None,
            updated_at_ms: created_at_ms,
            metadata_json: "{}".into(),
        })
    }

    #[test]
    fn resumable_goal_lookup_returns_one_typed_projection_for_the_requested_workspace() {
        let directory = tempdir().expect("temp directory");
        let database = directory.path().join("session-projection-resume.db");

        {
            let mut store = NolaStore::open(&database).expect("open store");
            seed_active_goal(&mut store, "target-session", "/workspace/target", 100);
            seed_active_goal(&mut store, "newer-other-session", "/workspace/other", 200);
            for index in 0..128 {
                seed_session_without_goal(
                    &mut store,
                    &format!("newer-history-{index}"),
                    "/workspace/target",
                    1_000 + index,
                );
            }
            let candidates =
                nola_store_port::resume_snapshot::ResumeSnapshotRepository::resumable_conversation_ids(
                    &store,
                    "/workspace/target",
                )
                .expect("query bounded resumable candidates");
            assert_eq!(candidates, vec!["target-session"]);
        }

        let mut reopened = NolaStore::open(&database).expect("cold reopen store");
        let value = execute_application_command(
            &mut reopened,
            ApplicationCommand::FindResumableGoalSession {
                project_root: "/workspace/target".to_string(),
            },
        )
        .expect("find resumable Goal session");
        let projection: Option<SessionProjection> =
            serde_json::from_value(value).expect("decode typed SessionProjection");
        let projection = projection.expect("target workspace must have a resumable Goal");

        assert_eq!(projection.conversation_id, "target-session");
        let current = support::current_checkpoint(&database, "target-session");
        assert_eq!(projection.store_revision, current.revision);
        assert_eq!(
            projection.event_cursor,
            u64::try_from(current.event_cursor).unwrap()
        );
        assert!(
            projection.event_cursor > 0,
            "real Goal admission emits committed Core control facts"
        );
        assert_eq!(
            projection.goal_projection.active.as_ref().unwrap().id,
            "goal-target-session"
        );
        assert!(
            !projection
                .goal_projection
                .active
                .as_ref()
                .unwrap()
                .status
                .is_terminal(),
            "resume lookup must return a nonterminal Goal"
        );

        let missing = execute_application_command(
            &mut reopened,
            ApplicationCommand::FindResumableGoalSession {
                project_root: "/workspace/missing".to_string(),
            },
        )
        .expect("query missing workspace");
        assert_eq!(missing, serde_json::Value::Null);
    }

    #[test]
    fn cold_resume_includes_workspace_tasks_but_excludes_another_conversations_tasks() {
        let directory = tempdir().expect("temp directory");
        let database = directory.path().join("resume-task-scope.db");
        {
            let mut store = NolaStore::open(&database).expect("open store");
            seed_active_goal(
                &mut store,
                "resume-scope-owner",
                "/workspace/resume-scope",
                100,
            );
            for task in [
                queued_agent_task("task-owned", Some("resume-scope-owner"), 101),
                queued_agent_task("task-workspace", None, 102),
                queued_agent_task("task-foreign", Some("another-conversation"), 103),
            ] {
                execute_application_command(
                    &mut store,
                    ApplicationCommand::RuntimeTask {
                        command: RuntimeTaskCommand::Create {
                            input: CreateRuntimeTask::try_from(task).expect("create scoped task input"),
                        },
                    },
                )
                .expect("create scoped runtime task");
            }
        }

        let mut reopened = NolaStore::open(&database).expect("cold reopen store");
        let snapshot: Option<ResumeSnapshot> = serde_json::from_value(
            execute_application_command(
                &mut reopened,
                ApplicationCommand::LoadResumeSnapshot {
                    project_root: "/workspace/resume-scope".into(),
                    host_id: Some("host-local".into()),
                    requested_at_ms: 200,
                },
            )
            .expect("load scoped resume snapshot"),
        )
        .expect("decode scoped resume snapshot");
        let snapshot = snapshot.expect("scoped Goal is resumable");
        let task_ids = snapshot
            .runtime_tasks
            .iter()
            .map(|task| task.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            task_ids,
            std::collections::BTreeSet::from(["task-owned", "task-workspace"])
        );
        assert!(!task_ids.contains("task-foreign"));
    }

    #[test]
    fn resume_snapshot_reads_goal_tasks_permissions_and_effects_from_one_cold_view() {
        let directory = tempdir().expect("temp directory");
        let database = directory.path().join("resume-snapshot.db");
        {
            let mut store = NolaStore::open(&database).expect("open store");
            seed_active_goal(&mut store, "resume-session", "/workspace/resume", 100);
            execute_application_command(
                &mut store,
                ApplicationCommand::RuntimeTask {
                    command: RuntimeTaskCommand::Create {
                        input: CreateRuntimeTask::try_from(scheduled_task(RuntimeTask {
                            id: "task-finished".into(),
                            kind: "shell".into(),
                            status: "queued".into(),
                            title: "Generate artifact".into(),
                            project_root: "/workspace/resume".into(),
                            owner_session_id: Some("resume-session".into()),
                            parent_task_id: None,
                            parent_tool_call_id: Some("tool-call-1".into()),
                            child_session_id: None,
                            agent_type: None,
                            prompt: None,
                            progress_summary: Some("queued".into()),
                            last_activity: Some("bash".into()),
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
                            created_at_ms: 110,
                            started_at_ms: None,
                            ended_at_ms: None,
                            updated_at_ms: 110,
                            metadata_json: "{}".into(),
                        }))
                        .expect("create resume task input"),
                    },
                },
            )
            .expect("create runtime task");
            let claimed = execute_application_command(
                &mut store,
                ApplicationCommand::RuntimeTask {
                    command: RuntimeTaskCommand::Claim {
                        task_id: "task-finished".into(),
                        worker_id: "resume-worker".into(),
                        worker_generation: "resume-generation".into(),
                        lease_token: "resume-lease".into(),
                        now_ms: 111,
                        lease_until_ms: 1_000,
                    },
                },
            )
            .expect("claim runtime task");
            let claimed_attempt: RuntimeTaskAttempt =
                serde_json::from_value(claimed).expect("decode runtime task claim authority");
            execute_application_command(
                &mut store,
                ApplicationCommand::RuntimeTask {
                    command: RuntimeTaskCommand::FinishDetailedWithEvent {
                        final_observation: None,
                        lease: RuntimeTaskLeaseWitness::from(&claimed_attempt),
                        status: "completed".into(),
                        completion_watermark: Some(1),
                        progress_summary: Some("Completed".into()),
                        result_summary: Some("artifact.txt".into()),
                        error: None,
                        output_path: Some("/workspace/resume/artifact.txt".into()),
                        unread: true,
                        event_kind: "finished".into(),
                        event_status: "completed".into(),
                        event_summary: Some("artifact.txt".into()),
                        event_detail_json: "{}".into(),
                        now_ms: 120,
                    },
                },
            )
            .expect("finish runtime task");
            execute_application_command(
                &mut store,
                ApplicationCommand::UpsertPermissionGrant {
                    input: PermissionGrantInput {
                        id: "grant-session".into(),
                        target: PermissionGrantTarget::Conversation {
                            conversation_id: "resume-session".into(),
                        },
                        capability_pattern: "bash:*".into(),
                        resource_pattern: "/workspace/resume/**".into(),
                        decision: PermissionGrantDecision::Allow,
                        created_at_ms: 121,
                        updated_at_ms: 121,
                    },
                },
            )
            .expect("save permission grant");
            // A terminal task cannot acquire new pending authority. Keep its
            // mailbox result, and bind the independent pending request to a live
            // task in the same owner scope.
            let mut pending_task: RuntimeTask = serde_json::from_value(serde_json::to_value(
                store.runtime_task("task-finished").unwrap().unwrap(),
            ).unwrap()).unwrap();
            pending_task.id = "task-permission".into();
            pending_task.status = "queued".into();
            pending_task.unread = false;
            pending_task.result_summary = None;
            pending_task.output_path = None;
            pending_task.worker_id = None;
            pending_task.ownership_token = None;
            pending_task.lease_until_ms = None;
            pending_task.heartbeat_at_ms = None;
            pending_task.completion_watermark = None;
            pending_task.started_at_ms = None;
            pending_task.ended_at_ms = None;
            pending_task.created_at_ms = 121;
            pending_task.updated_at_ms = 121;
            execute_application_command(&mut store, ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Create {
                    input: CreateRuntimeTask::try_from(pending_task).unwrap(),
                },
            }).expect("create live permission task");
            execute_application_command(
                &mut store,
                ApplicationCommand::PermissionRequest {
                    command: PermissionRequestCommand::Enqueue {
                        input: NewPermissionRequest {
                            core_permission_request_id: Some("core-fixture-request".into()),
                            permission_instructions: Vec::new(),
                            request_id: "permission-pending".into(),
                            task_id: "task-permission".into(),
                            child_session_id: "child-1".into(),
                            capability: "file_write".into(),
                            input: serde_json::json!({"path": "/workspace/resume/report.md"}),
                            scope_rule: "/workspace/resume/**".into(),
                            created_at_ms: 122,
                        },
                    },
                },
            )
            .expect("save pending permission");
            execute_application_command(
                &mut store,
                ApplicationCommand::WorkStepTransaction {
                    command: WorkStepTransactionCommand::Prepare {
                        id: "effect-1".into(),
                        work_id: "goal-resume-session".into(),
                        step_id: "step-write".into(),
                        tool_call_id: "tool-call-1".into(),
                        root: "/workspace/resume".into(),
                        effect_kind: "file_write".into(),
                        workspace_checkpoint_id: None,
                        request: serde_json::json!({"path": "report.md"}),
                        created_at_ms: 123,
                    },
                },
            )
            .expect("prepare side effect");
            execute_application_command(
                &mut store,
                ApplicationCommand::WorkStepTransaction {
                    command: WorkStepTransactionCommand::Prepare {
                        id: "effect-uncertain".into(),
                        work_id: "goal-resume-session".into(),
                        step_id: "step-external".into(),
                        tool_call_id: "tool-call-uncertain".into(),
                        root: "/workspace/resume".into(),
                        effect_kind: "external_mutation".into(),
                        workspace_checkpoint_id: None,
                        request: serde_json::json!({"operation": "external"}),
                        created_at_ms: 124,
                    },
                },
            )
            .expect("prepare uncertain side effect");
            execute_application_command(
                &mut store,
                ApplicationCommand::WorkStepTransaction {
                    command: WorkStepTransactionCommand::Advance {
                        id: "effect-uncertain".into(),
                        expected_revision: 1,
                        phase: WorkStepTransactionPhase::Checkpointed,
                        workspace_checkpoint_id: None,
                        result: None,
                        result_ref: None,
                        error: None,
                        updated_at_ms: 125,
                    },
                },
            )
            .expect("checkpoint uncertain side effect");
        }

        let mut reopened = NolaStore::open(&database).expect("cold reopen store");
        let value = execute_application_command(
            &mut reopened,
            ApplicationCommand::LoadResumeSnapshot {
                project_root: "/workspace/resume".into(),
                host_id: Some("host-local".into()),
                requested_at_ms: 200,
            },
        )
        .expect("load one-shot resume snapshot");
        let snapshot: Option<ResumeSnapshot> =
            serde_json::from_value(value).expect("decode resume snapshot");
        let snapshot = snapshot.expect("resumable snapshot");

        assert_eq!(snapshot.snapshot_version, 1);
        assert_eq!(snapshot.projection.conversation_id, "resume-session");
        assert_eq!(snapshot.projection.store_revision, 1);
        assert_eq!(snapshot.runtime_tasks.len(), 2);
        assert_eq!(snapshot.mailbox_tasks[0].id, "task-finished");
        assert_eq!(snapshot.effective_permission_grants[0].id, "grant-session");
        assert_eq!(
            snapshot.pending_permission_requests[0].request_id,
            "permission-pending"
        );
        assert_eq!(snapshot.pending_permission_requests[0].task_id, "task-permission");
        assert_eq!(snapshot.incomplete_work_step_transactions.len(), 2);
        assert_eq!(snapshot.action, ResumeAction::ReconcileSideEffects);
        assert_eq!(snapshot.supervisor_reconciliation.tasks.len(), 2);
        assert_eq!(
            snapshot.supervisor_reconciliation.tasks.iter()
                .find(|task| task.task_id == "task-finished").unwrap().disposition,
            nola_application::SupervisorTaskDisposition::DeliverEvidence
        );
        assert_eq!(snapshot.supervisor_reconciliation.side_effects.len(), 2);
        assert_eq!(
            snapshot
                .supervisor_reconciliation
                .side_effects
                .iter()
                .find(|effect| effect.transaction_id == "effect-1")
                .expect("prepared effect decision")
                .disposition,
            nola_application::SupervisorSideEffectDisposition::ReadyToExecute
        );
        assert_eq!(
            snapshot
                .supervisor_reconciliation
                .side_effects
                .iter()
                .find(|effect| effect.transaction_id == "effect-uncertain")
                .expect("uncertain effect decision")
                .disposition,
            nola_application::SupervisorSideEffectDisposition::RequiresIntervention
        );
        assert!(snapshot.active_run.is_none());

        let mut changed_task = snapshot.runtime_tasks.iter()
            .find(|task| task.id == "task-finished").unwrap().clone();
        changed_task.id = "task-after-snapshot".into();
        changed_task.status = "queued".into();
        changed_task.title = "New mailbox task".into();
        changed_task.progress_summary = Some("queued".into());
        changed_task.result_summary = None;
        changed_task.output_path = None;
        changed_task.unread = true;
        changed_task.worker_id = None;
        changed_task.ownership_token = None;
        changed_task.lease_until_ms = None;
        changed_task.heartbeat_at_ms = None;
        changed_task.completion_watermark = None;
        changed_task.started_at_ms = None;
        changed_task.ended_at_ms = None;
        changed_task.created_at_ms = 201;
        changed_task.updated_at_ms = 201;
        execute_application_command(
            &mut reopened,
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Create {
                    input: CreateRuntimeTask::try_from(changed_task)
                        .expect("create post-snapshot task input"),
                },
            },
        )
        .expect("create task after snapshot");
        let claimed = execute_application_command(
            &mut reopened,
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Claim {
                    task_id: "task-after-snapshot".into(),
                    worker_id: "mailbox-worker".into(),
                    worker_generation: "mailbox-generation".into(),
                    lease_token: "mailbox-lease".into(),
                    now_ms: 202,
                    lease_until_ms: 1_000,
                },
            },
        )
        .expect("claim task after snapshot");
        let claimed_attempt: RuntimeTaskAttempt =
            serde_json::from_value(claimed).expect("decode mailbox claim authority");
        execute_application_command(
            &mut reopened,
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::FinishDetailedWithEvent {
                    final_observation: None,
                    lease: RuntimeTaskLeaseWitness::from(&claimed_attempt),
                    status: "failed".into(),
                    completion_watermark: Some(2),
                    progress_summary: Some("Completed".into()),
                    result_summary: Some("new mailbox result".into()),
                    error: None,
                    output_path: None,
                    unread: true,
                    event_kind: "finished".into(),
                    event_status: "failed".into(),
                    event_summary: Some("new mailbox result".into()),
                    event_detail_json: "{}".into(),
                    now_ms: 203,
                },
            },
        )
        .expect("finish task after snapshot");
        let stale_ack = execute_application_command(
            &mut reopened,
            ApplicationCommand::AcknowledgeResumeSnapshot {
                input: AcknowledgeResumeSnapshot {
                    conversation_id: snapshot.projection.conversation_id.clone(),
                    checkpoint_revision: snapshot.projection.store_revision,
                    mailbox_cursor: snapshot.mailbox_cursor.clone(),
                },
            },
        );
        assert!(
            stale_ack.is_err(),
            "a stale mailbox cursor must be rejected"
        );

        let fresh: Option<ResumeSnapshot> = serde_json::from_value(
            execute_application_command(
                &mut reopened,
                ApplicationCommand::LoadResumeSnapshot {
                    project_root: "/workspace/resume".into(),
                    host_id: Some("host-local".into()),
                    requested_at_ms: 204,
                },
            )
            .expect("reload fresh resume snapshot"),
        )
        .expect("decode fresh snapshot");
        let fresh = fresh.expect("fresh resume snapshot");
        assert_eq!(fresh.mailbox_tasks[0].updated_at_ms, 203);
        execute_application_command(
            &mut reopened,
            ApplicationCommand::AcknowledgeResumeSnapshot {
                input: AcknowledgeResumeSnapshot {
                    conversation_id: fresh.projection.conversation_id.clone(),
                    checkpoint_revision: fresh.projection.store_revision,
                    mailbox_cursor: fresh.mailbox_cursor.clone(),
                },
            },
        )
        .expect("acknowledge fresh snapshot");
        let acknowledged: Option<ResumeSnapshot> = serde_json::from_value(
            execute_application_command(
                &mut reopened,
                ApplicationCommand::LoadResumeSnapshot {
                    project_root: "/workspace/resume".into(),
                    host_id: Some("host-local".into()),
                    requested_at_ms: 203,
                },
            )
            .expect("reload acknowledged snapshot"),
        )
        .expect("decode acknowledged snapshot");
        assert!(
            acknowledged
                .expect("acknowledged snapshot")
                .mailbox_tasks
                .is_empty(),
            "acknowledged mailbox must not be injected again"
        );
    }
}

mod core_session_identity_execution_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Canonical conversation and typed Core session are independent identities.
    use crate::support;
    use nola_application::cancellation_delivery::{
        load_run_cancellation_delivery_facts, CancellationRunIdentity,
    };
    use nola_application::*;
    use nola_agent_engine::agent::{host::*, protocol::*};
    use nola_store::NolaStore;

    const CONVERSATION: &str = "canonical-conversation";
    const CORE_SESSION: &str = "independent-core-session";

    fn fixture(path: &std::path::Path) -> String {
        let mut store = NolaStore::open(path).unwrap();
        let capability = HostCapabilityDescriptor {
            id: "model.stream".into(),
            title: "Isolated model".into(),
            route: HostCapabilityRoute::Model,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Internal,
            permission_domain: None,
            input_schema: serde_json::json!({"type":"object"}),
            input_normalization: Default::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 4000,
                persist_full_output: true,
                allow_artifacts: true,
            },
            supports_streaming: true,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::Serial,
            cancellation: HostCancellationMode::Cooperative,
            replay_safety: HostReplaySafety::Recoverable,
        };
        let begun = begin_assistant_turn(
            &mut store,
            BeginAssistantTurn {
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                conversation_id: CONVERSATION.into(),
                conversation_title: "Identity fixture".into(),
                user_message_id: "user".into(),
                user_part_id: "user-part".into(),
                user_text: "Work".into(),
                user_content_json: None,
                assistant_message_id: "assistant".into(),
                context_checkpoint_id: None,
                core_config: AgentCoreConfig {
                    turn_identity: support::model_identity::identity(),
                    session_id: Some(SessionId::from_string(CORE_SESSION.into())),
                    model: Some("fixture".into()),
                    host_catalog: Some(HostCapabilityCatalog {
                        host_id: HostId("isolated-host".into()),
                        revision: 1,
                        availability_epoch: 1,
                        lease: None,
                        capabilities: [("model.stream".into(), capability)].into_iter().collect(),
                    }),
                    ..Default::default()
                },
                submission: AgentSubmission::UserInput {
                    items: vec![AgentItem::user_message(
                        ItemId::from_string("user".into()),
                        "Work",
                    )],
                    turn_context_items: vec![],
                    turn_context_platform: Default::default(),
                    turn_context_components: vec![],
                    submission_id: Some("submission".into()),
                    submitted_at_epoch_ms: Some(1000),
                    delivery: AgentInputDelivery::CurrentTurn,
                },
                created_at_ms: 1000,
            },
        )
        .unwrap();
        assert_eq!(begun.execution.control.session_id.as_str(), CORE_SESSION);
        claim_committed_host_requests(
            &mut store,
            ClaimCommittedHostRequests {
                run_id: begun.begin.run_id.clone(),
                worker_generation: "fixture-generation".into(),
                lease_owner: "fixture-worker".into(),
                claimed_at_ms: 1100,
                lease_expires_at_ms: 100_000,
                limit: 1,
            },
        )
        .unwrap();
        begun.begin.run_id
    }
    fn cancel(run: &str) -> RequestRunCancellation {
        RequestRunCancellation {
            conversation_id: CONVERSATION.into(),
            expected_run_id: run.into(),
            assistant_message_id: "assistant".into(),
            cancellation_id: "cancellation".into(),
            reason: None,
            requested_at_ms: 1200,
        }
    }
    fn recovery() -> ApplicationCommand {
        ApplicationCommand::RecoverOrphanedRuns {
            conversation_id: Some(CONVERSATION.into()),
            reason: "isolated_recovery_check".into(),
            interrupted_at: "before-restart".into(),
            interrupted_at_ms: 1150,
        }
    }
    fn checkpoint(db: &rusqlite::Connection) -> String {
        db.query_row(
            "SELECT checkpoint_json FROM core_checkpoint_bodies WHERE conversation_id=?1",
            [CONVERSATION],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn independent_core_session_recovers_cancels_and_loads_cold_delivery_facts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("execution.db");
        let run = fixture(&path);
        let db = rusqlite::Connection::open(&path).unwrap();
        let before = checkpoint(&db);
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime.execute(recovery()).unwrap();
        assert_eq!(checkpoint(&db), before);
        let receipt: RunCancellationReceipt = serde_json::from_value(
            runtime
                .execute(ApplicationCommand::RequestRunCancellation {
                    input: cancel(&run),
                })
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            receipt.disposition,
            RunCancellationDisposition::CancellationCommitted
        );
        assert_eq!(receipt.terminal_result, None);
        drop(runtime);
        let store = NolaStore::open(&path).unwrap();
        let identity = CancellationRunIdentity {
            conversation_id: CONVERSATION.into(),
            expected_run_id: run.clone(),
            assistant_message_id: "assistant".into(),
        };
        let facts = load_run_cancellation_delivery_facts(&store, &identity).unwrap();
        assert!(!facts.targets.is_empty());
        assert!(facts
            .targets
            .iter()
            .all(|fact| fact.cancellation.session_id == CORE_SESSION
                && fact
                    .target
                    .as_ref()
                    .is_none_or(|target| target.session_id == CORE_SESSION)));
        assert!(load_run_cancellation_delivery_facts(
            &store,
            &CancellationRunIdentity {
                conversation_id: "foreign-conversation".into(),
                ..identity.clone()
            }
        )
        .is_err());
        assert!(load_run_cancellation_delivery_facts(
            &store,
            &CancellationRunIdentity {
                expected_run_id: "foreign-run".into(),
                ..identity
            }
        )
        .is_err());
    }

    #[test]
    fn inconsistent_core_session_and_foreign_host_session_fail_without_canonical_writes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rejected-identity.db");
        let run = fixture(&path);
        let db = rusqlite::Connection::open(&path).unwrap();
        let original = checkpoint(&db);
        let mut invalid: serde_json::Value = serde_json::from_str(&original).unwrap();
        invalid["state"]["session_id"] = serde_json::json!("foreign-core-session");
        let invalid = serde_json::to_string(&invalid).unwrap();
        db.execute(
            "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id=?2",
            [&invalid, CONVERSATION],
        )
        .unwrap();
        let mut runtime = support::open_runtime(&path).unwrap();
        assert!(runtime
            .execute(ApplicationCommand::RequestRunCancellation {
                input: cancel(&run)
            })
            .is_err());
        assert!(runtime.execute(recovery()).is_err());
        assert_eq!(checkpoint(&db), invalid);
        db.execute(
            "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id=?2",
            [&original, CONVERSATION],
        )
        .unwrap();
        runtime
            .execute(ApplicationCommand::RequestRunCancellation {
                input: cancel(&run),
            })
            .unwrap();
        drop(runtime);
        let committed = checkpoint(&db);
        let (effect_id, original_request): (String, String) = db.query_row(
            "SELECT effect_id,payload_json FROM effect_outbox JOIN effect_outbox_payloads USING(effect_id) WHERE json_extract(payload_json,'$.payload.type')='cancel' LIMIT 1",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        let original_binding: String = db.query_row("SELECT binding_json FROM effect_outbox WHERE effect_id=?1", [&effect_id], |row| row.get(0)).unwrap();
        let store = NolaStore::open(&path).unwrap();
        let identity = CancellationRunIdentity {
            conversation_id: CONVERSATION.into(),
            expected_run_id: run,
            assistant_message_id: "assistant".into(),
        };
        for (field, value) in [
            ("$.session_id", "foreign-core-session"),
            ("$.request_id", "foreign-request"),
            ("$.effect_id", "foreign-effect"),
            ("$.host_id", "foreign-host"),
            ("$.lease_id", "foreign-lease"),
            ("$.turn_id", "foreign-turn"),
            ("$.payload.target_effect_id", "foreign-target"),
        ] {
            let (table,column,original) = if field.starts_with("$.payload.") {
                ("effect_outbox_payloads","payload_json",&original_request)
            } else { ("effect_outbox","binding_json",&original_binding) };
            db.execute(&format!("UPDATE {table} SET {column}=json_set(?1,?2,?3) WHERE effect_id=?4"),
                rusqlite::params![original, field, value, effect_id]).unwrap();
            assert!(load_run_cancellation_delivery_facts(&store, &identity).is_err(), "{field}");
            db.execute(&format!("UPDATE {table} SET {column}=?1 WHERE effect_id=?2"),
                rusqlite::params![original, effect_id]).unwrap();
            assert!(load_run_cancellation_delivery_facts(&store, &identity).is_ok());
        }
        assert_eq!(checkpoint(&db), committed);
        // Well-typed envelope mutations must not be hidden by proof admission.
        for (field, value) in [
            ("$.catalog_revision", serde_json::json!(999)),
            ("$.availability_epoch", serde_json::json!(999)),
            ("$.attempt_id", serde_json::json!("foreign-attempt")),
            ("$.attempt_started", serde_json::json!(true)),
            ("$.created_at_ms", serde_json::json!(999)),
            ("$.deadline_ms", serde_json::json!(999)),
            ("$.batch", serde_json::json!({"batch_id":"foreign-batch","index":0,"size":1})),
        ] {
            db.execute("UPDATE effect_outbox SET binding_json=json_set(?1,?2,json(?3)) WHERE effect_id=?4",
                rusqlite::params![original_binding, field, value.to_string(), effect_id]).unwrap();
            let before_version: i64 = db.query_row("PRAGMA data_version", [], |row| row.get(0)).unwrap();
            assert!(load_run_cancellation_delivery_facts(&store, &identity).is_err(), "{field}");
            assert_eq!(db.query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0)).unwrap(),
                before_version, "rejected read must not commit through the Store connection: {field}");
            assert_eq!(checkpoint(&db), committed, "{field}");
            db.execute("UPDATE effect_outbox SET binding_json=?1 WHERE effect_id=?2",
                rusqlite::params![original_binding, effect_id]).unwrap();
            assert!(load_run_cancellation_delivery_facts(&store, &identity).is_ok());
        }
    }
}

mod core_session_read_identity_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Canonical conversation keys and Core session identities are separate facts.

    use crate::support::model_identity as fixture_model_identity;
    use std::collections::BTreeMap;

    use nola_application::{
        begin_assistant_turn, execute_application_command, load_conversation_projection_v1,
        load_conversation_projection_window_v1, load_session_projection,
        reduce_and_commit_active_submission, ApplicationCommand, BeginAssistantTurn,
        ReduceAndCommitActiveSubmission,
    };
    use nola_agent_engine::agent::host::*;
    use nola_agent_engine::agent::protocol::*;
    use nola_domain::MessagePartView;
    use nola_store::NolaStore;

    const CONVERSATION: &str = "canonical-conversation";
    const CORE_SESSION: &str = "actual-core-session";

    fn config() -> AgentCoreConfig {
        AgentCoreConfig {
            turn_identity: fixture_model_identity::identity(),
            session_id: Some(SessionId::from_string(CORE_SESSION.into())),
            model: Some("fixture-model".into()),
            host_catalog: Some(HostCapabilityCatalog {
                host_id: HostId("fixture-host".into()),
                revision: 1,
                availability_epoch: 1,
                lease: None,
                capabilities: BTreeMap::from([(
                    "model.stream".into(),
                    HostCapabilityDescriptor {
                        id: "model.stream".into(),
                        title: "model.stream".into(),
                        route: HostCapabilityRoute::Model,
                        availability: CapabilityAvailability::Available,
                        exposure: HostCapabilityExposure::Internal,
                        permission_domain: None,
                        input_schema: serde_json::json!({"type":"object"}),
                        input_normalization: Default::default(),
                        output_policy: HostOutputPolicy {
                            preview_char_limit: 4000,
                            persist_full_output: true,
                            allow_artifacts: true,
                        },
                        supports_streaming: true,
                        supports_argument_streaming: false,
                        parallelism: HostCapabilityParallelism::Serial,
                        cancellation: HostCancellationMode::Cooperative,
                        replay_safety: HostReplaySafety::Recoverable,
                    },
                )]),
            }),
            ..AgentCoreConfig::default()
        }
    }

    fn begin(store: &mut NolaStore) -> nola_application::AcceptedAssistantTurn {
        begin_assistant_turn(
            store,
            BeginAssistantTurn {
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                conversation_id: CONVERSATION.into(),
                conversation_title: "Separate identities".into(),
                user_message_id: "user".into(),
                user_part_id: "user:body".into(),
                user_text: "Continue the task".into(),
                user_content_json: None,
                assistant_message_id: "visible-assistant".into(),
                context_checkpoint_id: None,
                core_config: config(),
                submission: AgentSubmission::UserInput {
                    items: vec![AgentItem::user_message(
                        ItemId::from_string("user".into()),
                        "Continue the task",
                    )],
                    turn_context_items: vec![],
                    turn_context_platform: Default::default(),
                    turn_context_components: vec![],
                    submission_id: Some("submission".into()),
                    submitted_at_epoch_ms: Some(1000),
                    delivery: AgentInputDelivery::CurrentTurn,
                },
                created_at_ms: 1000,
            },
        )
        .unwrap()
    }

    fn start_skill(store: &mut NolaStore, begun: &nola_application::AcceptedAssistantTurn) {
        let envelope = serde_json::from_value(serde_json::json!({
            "operation_id":"start-skill",
            "command":{
                "type":"start", "run_id":"skill-run",
                "owner":{"session_id":CORE_SESSION,"parent_turn_id":begun.begin.run_id},
                "descriptor":{
                    "skill_id":"skill", "version":"1", "display_name":"Active skill",
                    "instructions_ref":format!("skill#sha256={}", "a".repeat(64)),
                    "instructions_digest":"a".repeat(64)
                },
                "mode":"inline",
                "budget":{"max_model_turns":100,"max_tool_calls":100,"max_elapsed_millis":null}
            }
        }))
        .unwrap();
        reduce_and_commit_active_submission(
            store,
            ReduceAndCommitActiveSubmission {
                conversation_id: CONVERSATION.into(),
                run_id: begun.begin.run_id.clone(),
                invocation_config_digest: begun.invocation_config_digest.clone(),
                transition_id: "skill-start-transition".into(),
                submission: AgentSubmission::SkillRuntime { envelope },
                committed_at_ms: 1100,
            },
        )
        .unwrap();
    }

    fn assert_reads(store: &mut NolaStore, _run: &str) -> nola_domain::MessageView {
        let local = load_conversation_projection_v1(store, CONVERSATION).unwrap();
        assert_eq!(local.conversation_id.0, CONVERSATION);
        assert_eq!(local.messages.len(), 2);
        assert!(local.messages[1].parts.iter().any(|part| {
            matches!(part, MessagePartView::Skill { run_id, .. } if run_id == "skill-run")
        }));
        let window =
            load_conversation_projection_window_v1(store, CONVERSATION, None, 1, 32_768).unwrap();
        assert_eq!(window.projection.messages[0], local.messages[1]);
        let context: nola_domain::ContextUsageView = serde_json::from_value(
            execute_application_command(
                store,
                ApplicationCommand::LoadConversationContextUsageV1 {
                    conversation_id: CONVERSATION.into(),
                },
            )
            .unwrap(),
        )
        .unwrap();
        // Both endpoints consume the same exact-Run committed control fact.
        assert_eq!(context.used_tokens, local.context_usage.used_tokens);
        assert_eq!(
            context.context_window_tokens,
            local.context_usage.context_window_tokens
        );
        assert_eq!(
            context.prompt_token_limit,
            local.context_usage.prompt_token_limit
        );
        assert_eq!(context.estimated, local.context_usage.estimated);
        assert_eq!(
            context.measured_at_revision,
            local.context_usage.measured_at_revision
        );
        let session = load_session_projection(store, CONVERSATION)
            .unwrap()
            .unwrap();
        assert_eq!(session.conversation_id, CONVERSATION);
        assert_eq!(session.skill_runtime_projection.session_id, CORE_SESSION);
        assert_eq!(
            session.skill_runtime_projection.runs["skill-run"]
                .owner
                .session_id,
            CORE_SESSION
        );
        let permissions = execute_application_command(
            store,
            ApplicationCommand::LoadConversationPermissionProjectionV1 {
                conversation_id: CONVERSATION.into(),
                run_id: _run.into(),
                scope: nola_application::PermissionProjectionReadScope::CurrentRun,
            },
        )
        .unwrap();
        assert!(permissions.is_object());
        local.messages[1].clone()
    }

    #[test]
    fn production_begin_distinct_core_session_reads_after_cold_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("local.db");
        let mut store = NolaStore::open(&database).unwrap();
        let begun = begin(&mut store);
        assert_eq!(begun.execution.control.session_id.as_str(), CORE_SESSION);
        assert_eq!(begun.execution.configuration.session_id.as_ref().unwrap().as_str(), CORE_SESSION);
        start_skill(&mut store, &begun);
        let before = assert_reads(&mut store, &begun.begin.run_id);
        drop(store);
        let mut reopened = NolaStore::open(&database).unwrap();
        let after = assert_reads(&mut reopened, &begun.begin.run_id);
        assert_eq!(before, after);
    }

    #[test]
    fn owned_checkpoint_rejects_foreign_root_state_or_skill_session() {
        for foreign in ["root", "state", "skill"] {
            let directory = tempfile::tempdir().unwrap();
            let database = directory.path().join("foreign.db");
            let mut store = NolaStore::open(&database).unwrap();
            let begun = begin(&mut store);
            start_skill(&mut store, &begun);
            let usage_before = execute_application_command(
                &mut store,
                ApplicationCommand::LoadConversationContextUsageV1 {
                    conversation_id: CONVERSATION.into(),
                },
            )
            .unwrap();
            let stored = store.load_checkpoint(CONVERSATION).unwrap().unwrap();
            let mut checkpoint: serde_json::Value =
                serde_json::from_str(&stored.checkpoint_json).unwrap();
            match foreign {
                "root" => checkpoint["session_id"] = serde_json::json!("foreign-core"),
                "state" => checkpoint["state"]["session_id"] = serde_json::json!("foreign-core"),
                "skill" => {
                    checkpoint["state"]["skill_runtime"]["runs"]["skill-run"]["owner"]["session_id"] =
                        serde_json::json!("foreign-core")
                }
                _ => unreachable!(),
            }
            // Isolated corruption is deliberate: the canonical key stays owned by
            // this conversation, so dropping identity checks would accept it.
            let inspect = rusqlite::Connection::open(&database).unwrap();
            inspect
                .execute(
                    "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id=?2",
                    rusqlite::params![checkpoint.to_string(), CONVERSATION],
                )
                .unwrap();
            assert!(load_conversation_projection_v1(&store, CONVERSATION).is_err());
            assert!(load_session_projection(&store, CONVERSATION).is_err());
            assert_eq!(
                execute_application_command(
                    &mut store,
                    ApplicationCommand::LoadConversationContextUsageV1 {
                        conversation_id: CONVERSATION.into()
                    }
                )
                .unwrap(),
                usage_before,
                "committed usage does not decode the independently corrupt execution checkpoint"
            );
            if foreign != "skill" {
                assert!(execute_application_command(
                    &mut store,
                    ApplicationCommand::LoadConversationPermissionProjectionV1 {
                        conversation_id: CONVERSATION.into(),
                        run_id: begun.begin.run_id.clone(),
                        scope: nola_application::PermissionProjectionReadScope::CurrentRun,
                    },
                )
                .is_err());
            }
        }
    }
}

mod idle_checkpoint_port_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Idle checkpoint, memory deltas and publication share a real isolated Store UoW.
    use crate::support::model_identity as fixture_model_identity;
    use nola_application::{reduce_and_commit_idle_submission, ReduceAndCommitIdleSubmission};
    use nola_agent_engine::agent::{AgentCoreConfig, AgentSubmission, SessionId};

    fn store(path: &std::path::Path) -> nola_store::NolaStore {
        let mut store=nola_store::NolaStore::open(path).unwrap();
        store.create_conversation(&nola_store::CreateConversationInput {
            id:"idle".into(),title:"Idle fixture".into(),revision:1,settings_json:None,
            created_at_ms:1,updated_at_ms:1,
        }).unwrap();
        store
    }

    fn command(submission:AgentSubmission) -> ReduceAndCommitIdleSubmission {
        ReduceAndCommitIdleSubmission {
            conversation_id:"idle".into(),core_config:AgentCoreConfig {
                turn_identity: fixture_model_identity::identity(),
                session_id:Some(SessionId::from_string("idle".into())),..Default::default()
            },submission,committed_at_ms:10,
        }
    }

    fn usage() -> AgentSubmission {
        AgentSubmission::MemoryRuntime {
            envelope:nola_agent_engine::agent::MemoryRuntimeCommandEnvelope {
                operation_id:"usage-op".into(),command:nola_agent_engine::agent::MemoryRuntimeCommand::RecordUsage {
                    entry:nola_agent_engine::agent::MemoryUsageLedgerEntry {
                        usage_id:"usage-id".into(),memory_id:"memory-id".into(),session_id:"idle".into(),
                        turn_id:"turn-id".into(),used_at_epoch_ms:10,context_payload_digest:"digest".into(),
                        citation_ids:vec!["citation".into()],
                    },
                },
            },
        }
    }

    #[test]
    fn idle_memory_usage_is_forwarded_once_with_the_checkpoint_delta() {
        let directory=tempfile::tempdir().unwrap();let path=directory.path().join("usage.sqlite");
        let mut facts=store(&path);
        facts.upsert_memory_record(&nola_store_port::memory::StoredMemoryRecord {
            id:"memory-id".into(),owner_json:r#"{"kind":"profile","profile_id":"local"}"#.into(),namespace_json:r#"{"kind":"personal"}"#.into(),scope:"global".into(),kind:"reference".into(),
            content:"A durable fixture fact".into(),summary:None,tags_json:"[]".into(),confidence:1.0,
            importance:0.5,sensitivity:None,retention_json:"{}".into(),expires_at:None,
            supersedes_json:"[]".into(),provenance_json:"[]".into(),created_at_ms:1,updated_at_ms:1,
            last_used_at_ms:None,deleted_at_ms:None,
        }).unwrap();
        let accepted = reduce_and_commit_idle_submission(&mut facts,command(usage())).unwrap();
        let wire = serde_json::to_value(&accepted).unwrap();
        assert!(wire.get("checkpoint").is_none());
        assert!(wire.get("snapshot").is_none());
        assert_eq!(wire["execution"]["control"]["session_id"], "idle");
        assert!(wire["execution"]["configuration"]["initial_items"].as_array().unwrap().is_empty());
        let db=rusqlite::Connection::open(&path).unwrap();
        let row:(String,String)=db.query_row("SELECT usage_id,citation_ids_json FROM memory_usage_ledger",[],
            |row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(row.0,"usage-id");
        assert_eq!(serde_json::from_str::<Vec<String>>(&row.1).unwrap(),vec!["citation"]);
        assert_eq!(facts.load_checkpoint("idle").unwrap().unwrap().revision,1);
        // Even INSERT OR IGNORE would trigger this failure: repeated reduction
        // must emit no usage delta, rather than depend on database deduplication.
        db.execute_batch("CREATE TRIGGER forbid_repeated_usage BEFORE INSERT ON memory_usage_ledger
            BEGIN SELECT RAISE(ABORT,'usage delta was forwarded twice'); END;").unwrap();
        drop(facts);
        let mut reopened=nola_store::NolaStore::open(&path).unwrap();
        reduce_and_commit_idle_submission(&mut reopened,command(usage())).unwrap();
        assert_eq!(reopened.load_checkpoint("idle").unwrap().unwrap().revision,2);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM memory_usage_ledger",[],|row|row.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM assistant_journal_records",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    }

    #[test]
    fn real_idle_core_commits_only_after_repository_success_and_restores() {
        let directory=tempfile::tempdir().unwrap();let path=directory.path().join("rollback.sqlite");
        let mut facts=store(&path);let db=rusqlite::Connection::open(&path).unwrap();
        let submission=||AgentSubmission::RebaseConversation {items:vec![],item_ownership:Default::default()};
        db.execute_batch("CREATE TRIGGER fail_checkpoint AFTER INSERT ON core_checkpoints
            BEGIN SELECT RAISE(ABORT,'checkpoint commit unavailable'); END;").unwrap();
        assert!(reduce_and_commit_idle_submission(&mut facts,command(submission())).unwrap_err()
            .contains("checkpoint commit unavailable"));
        assert!(facts.load_checkpoint("idle").unwrap().is_none());
        assert!(!facts.has_write_unit_of_work(),"failed reducer must close its owned transaction");
        drop(facts);
        let mut reopened=nola_store::NolaStore::open(&path).unwrap();
        assert!(reopened.load_checkpoint("idle").unwrap().is_none());
        assert_eq!(db.query_row("SELECT COUNT(*) FROM assistant_journal_publications",[],|row|row.get::<_,i64>(0)).unwrap(),0);
        db.execute_batch("DROP TRIGGER fail_checkpoint;").unwrap();
        reduce_and_commit_idle_submission(&mut reopened,command(submission())).unwrap();
        assert_eq!(reopened.load_checkpoint("idle").unwrap().unwrap().revision,1);
        drop(reopened);
        let mut reopened=nola_store::NolaStore::open(&path).unwrap();
        reduce_and_commit_idle_submission(&mut reopened,command(submission())).unwrap();
        assert_eq!(reopened.load_checkpoint("idle").unwrap().unwrap().revision,2);
    }
}

mod resume_snapshot_port_e2e {
    // SPDX-License-Identifier: Apache-2.0
    use crate::support;
    use nola_application::{
        acknowledge_resume_snapshot, load_resume_snapshot, AcknowledgeResumeSnapshot,
        ResumeMailboxCursor,
    };
    use nola_agent_engine::agent::recovery::CheckpointReason;
    use nola_agent_engine::agent::{
        AgentCore, AgentCoreConfig, GoalRecordStatus, GoalRequirement, GoalRuntimeCommand,
        GoalRuntimeCommandEnvelope, SessionId,
    };
    use nola_store_port::{
        consistent_read::ConsistentReadRepository,
        remote_session::{AuthoritativeSessionRecord, SessionProjectionRepository},
        resume_snapshot::*,
        run_effect::StoredCheckpoint,
        runtime_task::StoredRuntimeTask,
    };
    fn checkpoint() -> StoredCheckpoint {
        let core = AgentCore::try_new(AgentCoreConfig {
            turn_identity: support::model_identity::identity(),
            session_id: Some(SessionId::from_string("c".into())),
            ..Default::default()
        })
        .unwrap();
        let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
        checkpoint
            .state
            .goal_projection
            .apply(GoalRuntimeCommandEnvelope {
                operation_id: "start".into(),
                command: GoalRuntimeCommand::Start {
                    id: "g".into(),
                    objective: "resume".into(),
                    constraints: vec![],
                    requirements: vec![GoalRequirement {
                        acceptance_policy: nola_agent_engine::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                        semantic_revision: 1, id: "check".into(),
                        description: "verify".into(),
                        dependencies: vec![],
                        status: GoalRecordStatus::Pending,
                        evidence_refs: vec![],
                    }],
                    now_epoch_ms: 1,
                },
            })
            .unwrap();
        StoredCheckpoint {
            conversation_id: "c".into(),
            revision: 1,
            event_cursor: checkpoint.last_event_cursor.sequence as i64,
            checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
            updated_at_ms: 1,
        }
    }
    fn task() -> StoredRuntimeTask {
        serde_json::from_value(serde_json::json!({"id":"task","kind":"shell","status":"completed","title":"Evidence","project_root":"/workspace","owner_session_id":"c","unread":true,"cancel_requested":false,"retry_count":0,"created_at_ms":1,"updated_at_ms":2,"metadata_json":"{}"})).unwrap()
    }
    #[test]
    fn real_resume_helper_reads_snapshot_and_only_explicit_ack_consumes_cursor() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("resume-helper.db");
        let mut store = seed(&path);
        let result = load_resume_snapshot(&mut store, " /workspace ", Some("host"), 3)
            .unwrap().unwrap();
        assert!(store.runtime_task("task").unwrap().unwrap().unread);
        assert_eq!(result.mailbox_cursor, vec![ResumeMailboxCursor {
            task_id: "task".into(), updated_at_ms: 2,
        }]);
        acknowledge_resume_snapshot(&mut store, AcknowledgeResumeSnapshot {
            conversation_id: "c".into(), checkpoint_revision: 1,
            mailbox_cursor: result.mailbox_cursor,
        }).unwrap();
        drop(store);
        let store = nola_store::NolaStore::open(&path).unwrap();
        assert!(!store.runtime_task("task").unwrap().unwrap().unread);
    }
    fn seed(path: &std::path::Path) -> nola_store::NolaStore {
        let mut store = nola_store::NolaStore::open(path).unwrap();
        store
            .create_conversation(&nola_store::CreateConversationInput {
                id: "c".into(),
                title: "Goal".into(),
                revision: 1,
                settings_json: Some(r#"{"project_root":"/workspace"}"#.into()),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        let cp = checkpoint();
        store
            .save_checkpoint(&nola_store::SaveCheckpointInput {
                conversation_id: "c".into(),
                expected_revision: 0,
                revision: 1,
                event_cursor: cp.event_cursor,
                checkpoint_json: cp.checkpoint_json,
                updated_at_ms: 1,
            })
            .unwrap();
        store.create_runtime_task_with_event(&task()).unwrap();
        store
    }
    #[test]
    fn sqlite_port_pins_candidates_session_and_facts_across_another_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.db");
        let mut store = seed(&path);
        let writer = rusqlite::Connection::open(&path).unwrap();
        ConsistentReadRepository::with_consistent_read(&mut store,|store| {
            assert_eq!(store.resumable_conversation_ids("/workspace").unwrap(),vec!["c"]);
            writer.execute_batch("BEGIN;UPDATE core_checkpoints SET revision=2 WHERE conversation_id='c';UPDATE durable_tasks SET updated_at_ms=3 WHERE id='task';COMMIT").unwrap();
            let Some(AuthoritativeSessionRecord::Local(cp))=store.load_authoritative_session("c").unwrap() else {panic!()};
            let facts=store.load_resume_facts("c","/workspace",None).unwrap();
            assert_eq!(cp.recovery.checkpoint.revision,1);assert_eq!(facts.runtime_tasks[0].updated_at_ms,2);Ok(())
        }).unwrap();
        assert_eq!(store.load_checkpoint("c").unwrap().unwrap().revision, 2);
        assert_eq!(
            store.runtime_task("task").unwrap().unwrap().updated_at_ms,
            3
        );
        let rejected: Result<(), String> =
            ConsistentReadRepository::with_consistent_read(&mut store, |_| Err("read rejected".into()));
        assert!(rejected.is_err());
        assert_eq!(store.load_checkpoint("c").unwrap().unwrap().revision, 2);
    }
    #[test]
    fn mailbox_ack_rolls_back_with_receipt_and_replays_without_double_consumption() {
        use nola_application::{
            ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
        };
        use nola_protocol::CommandMetadata;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ack.db");
        drop(seed(&path));
        let command = || ApplicationCommand::AcknowledgeResumeSnapshot {
            input: AcknowledgeResumeSnapshot {
                conversation_id: "c".into(),
                checkpoint_revision: 1,
                mailbox_cursor: vec![ResumeMailboxCursor {
                    task_id: "task".into(),
                    updated_at_ms: 2,
                }],
            },
        };
        let admission = |id: &str| ExecuteReceivedCommand {
            client_id: "resume-port".into(),
            request_id: id.into(),
            command_type: command().wire_command_name().into(),
            command_json: serde_json::to_string(&command()).unwrap(),
            expected_revision: None,
            received_at: "2026-09-08T00:00:00Z".into(),
        };
        let db = rusqlite::Connection::open(&path).unwrap();
        let unread = || {
            db.query_row(
                "SELECT unread FROM durable_tasks WHERE id='task'",
                [],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
        };
        db.execute_batch("CREATE TRIGGER fail_ack BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'receipt failure');END").unwrap();
        let mut runtime = support::open_runtime(&path).unwrap();
        let result = runtime.execute_received_command(admission("failed"), command());
        if let Ok(receipt) = result {
            assert!(matches!(
                receipt.outcome,
                ApplicationCommandReceiptOutcome::Rejected { .. }
            ));
        }
        assert!(unread());
        db.execute_batch("DROP TRIGGER fail_ack").unwrap();
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        let received = admission("success");
        let receipt = runtime
            .execute_received_command(received.clone(), command())
            .unwrap();
        assert!(matches!(
            receipt.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        assert!(!unread());
        let replay = runtime
            .execute_received_command(received, command())
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(
            replay.into_protocol_receipt(),
            receipt.into_protocol_receipt()
        );
        assert!(!unread());
    }
}

mod run_not_started_cancellation_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Transactional non-dispatch proof cannot be inferred from an empty Host registry.
    use crate::support;
    use nola_agent_engine::agent::{host::*, protocol::*};
    use nola_application::*;
    use nola_domain::{EffectDeliveryAttemptState, EffectDeliverySettlement};
    use nola_store::NolaStore;
    use nola_store_port::assistant_journal::AssistantJournalRepository;

    fn admission() -> BeginAssistantTurn {
        let capability = HostCapabilityDescriptor {
            id: "model.stream".into(),
            title: "Unsent fixture model".into(),
            route: HostCapabilityRoute::Model,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Internal,
            permission_domain: None,
            input_schema: serde_json::json!({"type":"object"}),
            input_normalization: Default::default(),
            output_policy: HostOutputPolicy {
                preview_char_limit: 4000, persist_full_output: true, allow_artifacts: true,
            },
            supports_streaming: true,
            supports_argument_streaming: false,
            parallelism: HostCapabilityParallelism::Serial,
            cancellation: HostCancellationMode::Cooperative,
            replay_safety: HostReplaySafety::Recoverable,
        };
        BeginAssistantTurn {
            conversation_id: "conversation".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_title: "Cancel before provider entry".into(),
            user_message_id: "user".into(),
            user_part_id: "user-part".into(),
            user_text: "Work".into(),
            user_content_json: None,
            assistant_message_id: "assistant".into(),
            context_checkpoint_id: None,
            core_config: AgentCoreConfig {
                turn_identity: support::model_identity::identity(),
                session_id: Some(SessionId::from_string("conversation".into())),
                model: Some("fixture".into()),
                host_catalog: Some(HostCapabilityCatalog {
                    host_id: HostId("fixture".into()), revision: 1, availability_epoch: 1,
                    lease: None,
                    capabilities: [("model.stream".into(), capability)].into_iter().collect(),
                }),
                ..Default::default()
            },
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(ItemId::from_string("user".into()), "Work")],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("run".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        }
    }

    fn cancellation(run: &str, at: i64) -> ApplicationCommand {
        ApplicationCommand::RequestRunCancellation {
            input: RequestRunCancellation {
                conversation_id: "conversation".into(), expected_run_id: run.into(),
                assistant_message_id: "assistant".into(), cancellation_id: "cancel-before-entry".into(),
                reason: Some("Escape before provider entry".into()), requested_at_ms: at,
            },
        }
    }

    fn claim(store: &mut NolaStore, run: &str, at: i64) -> HostRequestEnvelope {
        let mut claimed = claim_committed_host_requests(store, ClaimCommittedHostRequests {
            run_id: run.into(), worker_generation: "generation".into(), lease_owner: "owner".into(),
            claimed_at_ms: at, lease_expires_at_ms: at + 10_000, limit: 64,
        }).unwrap();
        assert_eq!(claimed.requests.len(), 1);
        claimed.requests.remove(0)
    }

    fn response(begun: &AcceptedAssistantTurn, request: &HostRequestEnvelope,
        sequence: u64, at: i64, payload: HostResponse) -> ApplicationCommand {
        ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: "conversation".into(), run_id: begun.begin.run_id.clone(),
                invocation_config_digest: begun.invocation_config_digest.clone(), received_at_ms: at,
                response: HostResponseEnvelope {
                    request_id: request.request_id.clone(), effect_id: request.effect_id.clone(),
                    attempt_id: request.attempt_id.clone(), delegated_invocation_id: request.delegated_invocation_id.clone(),
                    host_id: request.host_id.clone(), lease_id: request.lease_id.clone(),
                    session_id: request.session_id.clone(), turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
                    delivery: request.delivery.clone(), sequence, payload,
                },
            },
        }
    }

    fn snapshot(path: &std::path::Path) -> Vec<String> {
        let db = rusqlite::Connection::open(path).unwrap();
        [
            "SELECT record_json FROM effect_delivery_attempts ORDER BY id",
            "SELECT payload_json FROM host_event_inbox ORDER BY event_id",
            "SELECT payload_json FROM run_events ORDER BY run_id,sequence",
            "SELECT checkpoint_json FROM core_checkpoint_bodies ORDER BY conversation_id",
        ].iter().map(|query| {
            let mut statement = db.prepare(query).unwrap();
            let rows = statement.query_map([], |row| row.get::<_, String>(0)).unwrap();
            serde_json::to_string(&rows.collect::<Result<Vec<_>, _>>().unwrap()).unwrap()
        }).collect()
    }

    #[test]
    fn cold_nonexecution_proof_revokes_prepared_or_leased_target_and_rejects_late_start() {
        for stage in ["unclaimed", "prepared", "leased"] {
            let temporary = tempfile::tempdir().unwrap();
            let path = temporary.path().join("not-started.db");
            let mut store = NolaStore::open(&path).unwrap();
            let begun = begin_assistant_turn(&mut store, admission()).unwrap();
            let run = &begun.begin.run_id;
            let target = if stage == "leased" {
                claim(&mut store, run, 1100)
            } else {
                load_committed_host_requests(&store, run).unwrap().requests.remove(0)
            };
            let original_attempt = if stage == "prepared" {
                let mutation = nola_kernel::prepare_effect_delivery_attempt(
                    "prepared-target", run, &target.effect_id.0, &target.request_id, 1,
                    nola_domain::EffectDeliveryReplaySafety::Recoverable, None, 1100,
                ).unwrap();
                store.commit_effect_delivery_attempt_mutations(&[mutation]).unwrap();
                Some("prepared-target".to_owned())
            } else {
                target.delivery.as_ref().map(|lease| lease.attempt_id.clone())
            };
            let original_record = original_attempt.as_ref()
                .map(|id| store.effect_delivery_attempt(id).unwrap().unwrap());
            drop(store);
            let mut runtime = support::open_runtime(&path).unwrap();
            runtime.execute(cancellation(run, 1200)).unwrap();
            drop(runtime);

            // Reopen, rather than trusting a hot Host's absent local registry.
            let mut store = NolaStore::open(&path).unwrap();
            let control = claim(&mut store, run, 1300);
            let HostRequest::Cancel { target_effect_id, target_not_started: Some(proof) } = &control.payload else {
                panic!("{stage} must produce exact durable non-execution proof")
            };
            proof.validate().unwrap();
            assert!(proof.matches_control(&control.host_id.0, &control.session_id,
                control.turn_id.as_deref(), &control.effect_id.0, &control.request_id, &target.effect_id.0));
            assert_eq!(target_effect_id, &target.effect_id);
            assert_eq!(proof.conversation_id, "conversation");
            assert_eq!(proof.target_request_id, target.request_id);
            assert_eq!(proof.revoked_attempts.len(), usize::from(stage != "unclaimed"));
            if let Some(id) = &original_attempt {
                let revoked = store.effect_delivery_attempt(id).unwrap().unwrap();
                assert_eq!(revoked.state, EffectDeliveryAttemptState::Settled);
                assert_eq!(revoked.settlement, Some(EffectDeliverySettlement::RejectedBeforeExecution));
                assert_eq!(revoked.execution_started_at_ms, None);
                assert_eq!(proof.revoked_attempts[0].attempt_id, *id);
                assert_eq!(proof.revoked_attempts[0].revision, revoked.revision);
            }
            if stage == "prepared" {
                let stale_claim = nola_kernel::claim_effect_delivery_attempt(
                    original_record.as_ref().unwrap(), "late-generation", "late-owner", "late-token", 10_000, 1301,
                ).unwrap();
                let before = snapshot(&path);
                assert!(store.commit_effect_delivery_attempt_mutations(&[stale_claim]).is_err());
                assert_eq!(snapshot(&path), before, "a pre-revocation Prepared snapshot cannot be claimed late");
            }
            drop(store);
            let mut runtime = support::open_runtime(&path).unwrap();
            if stage == "leased" {
                let before = snapshot(&path);
                assert!(runtime.execute(response(&begun, &target, 1, 1301, HostResponse::ExecutionStarted)).is_err());
                assert_eq!(snapshot(&path), before, "the revoked old lease cannot start or leave an inbox fact");
            }
            runtime.execute(response(&begun, &control, 1, 1302, HostResponse::ExecutionStarted)).unwrap();
            runtime.execute(response(&begun, &control, 2, 1303, HostResponse::CancellationConfirmed {
                target_effect_id: target.effect_id.clone(),
            })).unwrap();
            drop(runtime);
            let store = NolaStore::open(&path).unwrap();
            let seal = store.local_assistant_run_terminal_seal("conversation", run).unwrap().unwrap();
            assert!(matches!(seal.result.value, nola_domain::run_result::RunResultValue::Cancelled { .. }));
            assert!(load_committed_host_requests(&store, run).unwrap().requests.is_empty());
            let db = rusqlite::Connection::open(&path).unwrap();
            let starts: i64 = db.query_row(
                "SELECT count(*) FROM host_event_inbox WHERE json_extract(payload_json,'$.effect_id')=?1
                 AND json_extract(payload_json,'$.payload.type') IN ('execution_started','model_attempt_started','model_completed')",
                [&target.effect_id.0], |row| row.get(0),
            ).unwrap();
            assert_eq!(starts, 0, "cancelling unsent work must not fabricate provider execution");
        }
    }

    #[test]
    fn a_started_predecessor_prevents_proof_even_when_current_successor_has_not_started() {
        for successor in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let path = temporary.path().join("started-history.db");
            let mut store = NolaStore::open(&path).unwrap();
            let begun = begin_assistant_turn(&mut store, admission()).unwrap();
            let run = &begun.begin.run_id;
            let target = claim(&mut store, run, 1100);
            drop(store);
            let mut runtime = support::open_runtime(&path).unwrap();
            runtime.execute(response(&begun, &target, 1, 1101, HostResponse::ExecutionStarted)).unwrap();
            drop(runtime);
            if successor {
                let mut store = NolaStore::open(&path).unwrap();
                let current = store.effect_delivery_attempt(&target.delivery.as_ref().unwrap().attempt_id).unwrap().unwrap();
                let interrupted = nola_kernel::interrupt_effect_delivery_attempt(
                    &current, current.worker_generation.as_deref().unwrap(), current.lease_owner.as_deref().unwrap(),
                    current.lease_token.as_deref().unwrap(), nola_domain::EffectDeliveryRecoveryPlan::Resume,
                    "original worker lost after entry", 1150,
                ).unwrap();
                let next = nola_kernel::claim_interrupted_effect_delivery_successor(
                    &interrupted.record, "unstarted-successor", &target.request_id,
                    "successor-generation", "successor-owner", "successor-token", 10_000, 1160,
                ).unwrap();
                store.commit_effect_delivery_attempt_mutations(&[interrupted, next]).unwrap();
            }
            let mut runtime = support::open_runtime(&path).unwrap();
            runtime.execute(cancellation(run, 1200)).unwrap();
            drop(runtime);
            let mut store = NolaStore::open(&path).unwrap();
            let control = claim(&mut store, run, 1300);
            assert!(matches!(&control.payload, HostRequest::Cancel { target_not_started: None, .. }),
                "one unstarted successor cannot erase its started predecessor");
            assert!(store.local_assistant_run_terminal_seal("conversation", run).unwrap().is_none());
            assert_eq!(store.effect_delivery_attempt(&target.delivery.as_ref().unwrap().attempt_id)
                .unwrap().unwrap().execution_started_at_ms, Some(1101));
        }
    }

    #[test]
    fn failed_revocation_rolls_back_cancel_control_and_checkpoint_without_revoking_old_lease() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("revocation-rollback.db");
        let mut store = NolaStore::open(&path).unwrap();
        let begun = begin_assistant_turn(&mut store, admission()).unwrap();
        let target = claim(&mut store, &begun.begin.run_id, 1100);
        drop(store);
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER fail_nonexecution_revocation BEFORE UPDATE OF state ON effect_delivery_attempts
            WHEN NEW.state='settled' AND json_extract(NEW.record_json,'$.settlement')='rejected_before_execution'
            BEGIN SELECT RAISE(ABORT,'nonexecution revocation fault'); END;").unwrap();
        let before = snapshot(&path);
        let mut runtime = support::open_runtime(&path).unwrap();
        assert!(runtime.execute(cancellation(&begun.begin.run_id, 1200)).is_err());
        assert_eq!(snapshot(&path), before, "failed proof construction is not a partial revocation");
        let store = NolaStore::open(&path).unwrap();
        let current = store.effect_delivery_attempt(&target.delivery.as_ref().unwrap().attempt_id).unwrap().unwrap();
        assert_eq!(current.state, EffectDeliveryAttemptState::Leased);
        assert!(load_committed_host_requests(&store, &begun.begin.run_id).unwrap().requests
            .iter().all(|request| !matches!(request.payload, HostRequest::Cancel { .. })));
    }

    #[test]
    fn hot_cancel_claim_preserves_atomically_admitted_proof_after_late_start_rejection() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("hot-not-started.db");
        let mut store = NolaStore::open(&path).unwrap();
        let begun = begin_assistant_turn(&mut store, admission()).unwrap();
        let target = claim(&mut store, &begun.begin.run_id, 1100);
        drop(store);
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime.execute(cancellation(&begun.begin.run_id, 1200)).unwrap();
        assert!(runtime.execute(response(&begun, &target, 1, 1201, HostResponse::ExecutionStarted)).is_err());
        let controls: CommittedHostRequests = serde_json::from_value(runtime.execute(
            ApplicationCommand::ClaimCommittedHostRequests { input: ClaimCommittedHostRequests {
                run_id: begun.begin.run_id.clone(), worker_generation: "control-worker".into(),
                lease_owner: "control-owner".into(), claimed_at_ms: 1300,
                lease_expires_at_ms: 10_000, limit: 64,
            }}).unwrap()).unwrap();
        assert_eq!(controls.requests.len(), 1);
        let control = &controls.requests[0];
        let HostRequest::Cancel { target_effect_id, target_not_started: Some(proof) } = &control.payload else {
            panic!("same hot runtime must materialize durable cancellation proof")
        };
        assert_eq!(target_effect_id, &target.effect_id);
        assert_eq!(proof.revoked_attempts[0].attempt_id, target.delivery.as_ref().unwrap().attempt_id);
        runtime.execute(response(&begun, control, 1, 1301, HostResponse::ExecutionStarted)).unwrap();
        runtime.execute(response(&begun, control, 2, 1302, HostResponse::CancellationConfirmed {
            target_effect_id: target.effect_id,
        })).unwrap();
        drop(runtime);
        let store = NolaStore::open(&path).unwrap();
        let seal = store.local_assistant_run_terminal_seal("conversation", &begun.begin.run_id).unwrap().unwrap();
        assert!(matches!(seal.result.value, nola_domain::run_result::RunResultValue::Cancelled { .. }));
    }
}

mod run_payload_wal_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Real Core/provider-response commits must not multiply frozen configuration
    //! bytes in Run rows or full checkpoint bodies on the incremental hot path.
    use crate::committed_usage_stream as committed_usage;
    use nola_store::NolaStore;
    use nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository;

    fn stream_wal(config_padding: usize) -> u64 {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("run.db");
        let wal = directory.path().join("run.db-wal");
        let mut store = NolaStore::open(&path).unwrap();
        let reader = rusqlite::Connection::open(&path).unwrap();
        let mut start = 0;
        let mut bytes = 0;
        let mut brackets = 0;
        let (_, receipt) = committed_usage::seed_stream_with_observer(
            &mut store,
            Some(5432),
            config_padding,
            64,
            |begin| {
                if begin {
                    // A pinned reader prevents autocheckpoint from recycling the WAL
                    // during measurement; file growth therefore counts actual frames.
                    let busy: i64 = reader
                        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
                        .unwrap();
                    assert_eq!(busy, 0);
                    reader.execute_batch("BEGIN").unwrap();
                    reader
                        .query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))
                        .unwrap();
                    start = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
                } else {
                    bytes = std::fs::metadata(&wal).unwrap().len() - start;
                    reader.execute_batch("ROLLBACK").unwrap();
                }
                brackets += 1;
            },
        );
        assert_eq!(brackets, 2);
        assert!(bytes > 0);
        let original = store
            .load_run_invocation_config("usage-run")
            .unwrap()
            .unwrap();
        assert!(original.invocation_config_json.len() > config_padding);
        assert_eq!(
            reader
                .query_row("SELECT count(*) FROM run_invocation_payloads", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(!reader.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('runs') WHERE name='invocation_config_json')",[],|r|r.get::<_,bool>(0)).unwrap());
        assert!(!reader.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('core_checkpoints') WHERE name='checkpoint_json')",[],|r|r.get::<_,bool>(0)).unwrap());
        let baseline = store.load_checkpoint("usage").unwrap().unwrap();
        let bodies: i64=reader.query_row("SELECT count(*) FROM core_checkpoint_bodies",[],|r|r.get(0)).unwrap();
        assert_eq!(bodies,1);
        drop(store);
        drop(reader);
        let reopened = NolaStore::open(&path).unwrap();
        let reopened_baseline = reopened.load_checkpoint("usage").unwrap().unwrap();
        assert_eq!(reopened_baseline.checkpoint_json,baseline.checkpoint_json);
        assert_eq!(reopened_baseline.revision,baseline.revision);
        let identity = receipt.admission.unwrap().identity;
        let exact = reopened
            .load_exact_run_invocation(
                &identity.conversation_id,
                &identity.run_id,
                &identity.assistant_message_id,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            exact.invocation_config_json,
            original.invocation_config_json
        );
        assert_eq!(
            exact.invocation_config_digest,
            original.invocation_config_digest
        );
        assert!(reopened
            .load_exact_run_invocation("foreign", &identity.run_id, &identity.assistant_message_id)
            .is_err());
        bytes
    }

    #[test]
    fn large_frozen_configuration_does_not_multiply_stream_wal_writes() {
        let small = stream_wal(4096);
        let large = stream_wal(2 * 1024 * 1024);
        assert!(
            large <= small + 8 * 1024 * 1024,
            "payload-sized write amplification: small={small},large={large}"
        );
        assert!(
            large < 32 * 1024 * 1024,
            "64 bounded commits exceeded WAL budget: {large}"
        );
    }
}
