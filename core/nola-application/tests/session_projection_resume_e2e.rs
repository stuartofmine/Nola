mod support;
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

    assert_eq!(snapshot.snapshot_version, 2);
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
