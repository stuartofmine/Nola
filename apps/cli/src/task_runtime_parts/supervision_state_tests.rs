// SPDX-License-Identifier: Apache-2.0
// Test modules extracted from supervision.rs; included at the same module scope.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_task_question_read_failure_is_not_an_empty_success() {
        let error = load_waiting_task_questions(Some("child"), |child| {
            assert_eq!(child, "child");
            Err("question query transport failed".to_string())
        }).unwrap_err();
        let outcome = task_error(error);
        assert!(outcome.error);
        assert_eq!(outcome.text, "question query transport failed");
        assert!(outcome.data.is_none());
        for child in [None, Some(" ")] {
            assert!(load_waiting_task_questions(child, |_| panic!("missing owner must not query")).is_err());
        }
        assert!(load_waiting_task_questions(Some("child"), |_| Ok(Vec::new())).unwrap().is_empty());
    }

    #[test]
    fn task_wait_observes_a_wake_that_precedes_the_block() {
        let observed = task_wake_generation();
        wake_task_waiters();
        let started = Instant::now();

        wait_for_task_wake(observed, Duration::from_secs(1));

        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn task_wait_blocks_until_a_structured_runtime_wake() {
        let observed = task_wake_generation();
        let notifier = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(30));
            wake_task_waiters();
        });
        let started = Instant::now();

        wait_for_task_wake(observed, Duration::from_secs(1));

        notifier.join().expect("join task wake notifier");
        assert!(started.elapsed() >= Duration::from_millis(20));
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    fn core_task_fixture(
        state: nola_core::agent::TaskState,
        completion_watermark: Option<i64>,
    ) -> nola_core::agent::task::TaskHandle {
        nola_core::agent::task::TaskHandle {
            id: "task-1".to_string(),
            owner_session_id: nola_core::agent::SessionId::from_string("parent".to_string()),
            owner_turn_id: nola_core::agent::TurnId::from_string("turn-1".to_string()),
            kind: nola_core::agent::TaskKind::Agent,
            state,
            progress: None,
            current_step: None,
            result_ref: None,
            cancel_requested: false,
            cancel_requested_at_epoch_ms: None,
            lineage: Default::default(),
            durability: nola_core::agent::TaskDurability {
                completion_watermark,
                ..Default::default()
            },
        }
    }

    #[test]
    fn goal_auditor_task_slice_contains_only_tasks_owned_by_executor_turn() {
        let mut projection = nola_core::agent::task::TaskProjection::default();
        let current = core_task_fixture(nola_core::agent::TaskState::Sealed, Some(7));
        projection.order.push(current.id.clone());
        projection.tasks.insert(current.id.clone(), current);
        let mut older = core_task_fixture(nola_core::agent::TaskState::Sealed, Some(5));
        older.id = "older-task".to_string();
        older.owner_turn_id = nola_core::agent::TurnId::from_string("older-turn".to_string());
        projection.order.push(older.id.clone());
        projection.tasks.insert(older.id.clone(), older);

        let selected = task_ids_owned_by_turn(&projection, "turn-1");

        assert_eq!(selected.len(), 1);
        assert!(selected.contains("task-1"));
        assert!(!selected.contains("older-task"));
    }

    #[test]
    fn single_terminal_result_primitive_preserves_independent_ui_delivery() {
        let _environment = crate::store::test_env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-wait-task-delivery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create test root");
        let database = root.join("nola.db");
        std::env::set_var("NOLA_DB_PATH", &database);

        let session_id = "wait-task-parent";
        let task_id = "wait-task-result";
        let mut canonical = nola_store::NolaStore::open(&database).expect("open canonical store");
        canonical
            .create_conversation(&nola_store::CreateConversationInput {
                id: session_id.into(),
                title: "Wait task delivery".into(),
                revision: 1,
                settings_json: Some(format!(
                    "{{\"project_root\":{}}}",
                    serde_json::to_string(root.to_string_lossy().as_ref()).unwrap()
                )),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .expect("create conversation");
        let stored: nola_store_port::runtime_task::StoredRuntimeTask =
            serde_json::from_value(serde_json::json!({
                "id": task_id,
                "kind": "agent",
                "status": "queued",
                "title": "Delivered result",
                "project_root": root.to_string_lossy(),
                "owner_session_id": session_id,
                "parent_tool_call_id": "wait-task-call",
                "child_session_id": "wait-task-child",
                "agent_type": "general",
                "result_summary": "WAIT-TASK-RESULT",
                "unread": true,
                "cancel_requested": false,
                "retry_count": 0,
                "input_watermark": 7,
                "created_at_ms": 1,
                "updated_at_ms": 2,
                "metadata_json": "{}"
            }))
            .expect("stored runtime task");
        canonical
            .create_runtime_task_with_event(&stored)
            .expect("create runtime task");
        // wait_task acknowledges the actual source conversation's resume
        // cursor, so retain its real Core admission, not a fabricated parent.
        let mut source = source_admission_support::open_runtime(&database).unwrap();
        let fact = source_admission_support::agent_task_admission::source_fact_for(
            &mut source,
            task_id,
            session_id,
            "wait-task-child",
            "wait-task-call",
            Some(7),
        );
        source
            .execute(
                nola_application::ApplicationCommand::ImportHostAgentTaskAdmission {
                    admission: fact,
                },
            )
            .unwrap();
        drop(source);
        let attempt = canonical
            .claim_runtime_task(
                task_id,
                "delivery-worker",
                "delivery-generation",
                "delivery-token",
                3,
                1000,
            )
            .unwrap()
            .unwrap();
        assert!(canonical
            .finish_runtime_task_detailed_with_event(
                &nola_application::RuntimeTaskLeaseWitness::from(&attempt),
                "failed",
                Some(7),
                Some("failed"),
                Some("WAIT-TASK-RESULT"),
                None,
                None,
                true,
                "finished",
                "failed",
                None,
                "{}",
                None,
                4,
            )
            .unwrap());
        drop(canonical);

        let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().expect("wait task config");
        config.session.session_id = Some(session_id.into());
        config.tool_settings.project_root = root.clone();
        let task = NolaStore::open_default()
            .expect("open cli store")
            .runtime_task(task_id)
            .expect("load runtime task")
            .expect("runtime task exists");

        let cursor = crate::runtime_task_port::result_delivery_cursor(&task).unwrap();
        let business = json!({"completion_confirmed":true,"task":task_json(&task)}).to_string();
        let prepared = completed_wait_task_result(&task, business.clone(), "failed".into());
        assert!(!prepared.error);
        let typed: crate::runtime_task_port::RuntimeTaskResultData =
            serde_json::from_value(prepared.data.clone().unwrap()).unwrap();
        let crate::runtime_task_port::RuntimeTaskResultData::RuntimeTaskResultDelivery {
            cursor: prepared_cursor,
            text,
        } = typed;
        assert_eq!(prepared_cursor, cursor);
        assert_eq!(text, business);
        assert!(
            NolaStore::open_default()
                .unwrap()
                .runtime_task(task_id)
                .unwrap()
                .unwrap()
                .unread,
            "preparing a wait result must not consume a Task before its Host response commits"
        );
        session::acknowledge_wait_task_result(&cursor, "primitive-test")
            .expect("consume exact result primitive");

        let consumed = NolaStore::open_default()
            .expect("reopen cli store")
            .runtime_task(task_id)
            .expect("reload runtime task")
            .expect("runtime task remains");
        assert!(!consumed.unread);
        let metadata: Value = serde_json::from_str(&consumed.metadata_json).unwrap();
        assert_eq!(metadata["model_notified"], true);
        assert_ne!(
            metadata["ui_notified"], true,
            "model delivery is not UI presentation"
        );
        let args = Map::from_iter([("task_id".into(), Value::String(task_id.into()))]);
        let skills = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
            config.tool_settings.skill_dirs.clone(),
        );
        assert!(!read_task(&config, &args, &skills).error);
        assert!(!manage_runtime_tasks(&config, "status", Some(task_id)).error);
        let after_reads = NolaStore::open_default().unwrap().runtime_task(task_id).unwrap().unwrap();
        assert_eq!(after_reads.metadata_json, consumed.metadata_json,
            "model reads and UI detail preparation must not acknowledge presentation");
        let notifications = take_runtime_task_notifications(&config).unwrap();
        assert_eq!(
            notifications.len(),
            1,
            "a model-delivered result remains visible to the UI"
        );
        assert_eq!(notifications[0].id, task_id);
        assert_eq!(take_runtime_task_notifications(&config).unwrap().len(), 1,
            "preparing a notification, including after a lost renderer, is not delivery");
        assert!(deliver_runtime_task_notification(&notifications[0], || Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "renderer flush failed"))).is_err());
        assert_eq!(take_runtime_task_notifications(&config).unwrap().len(), 1,
            "failed presentation cannot acknowledge a notification");
        assert_eq!(deliver_runtime_task_notification(&notifications[0], || Ok(crate::terminal_ui::HistoryPresentation::Deferred)).unwrap(), None);
        assert_eq!(take_runtime_task_notifications(&config).unwrap().len(), 1);
        assert_eq!(deliver_runtime_task_notification(&notifications[0], || Ok(crate::terminal_ui::HistoryPresentation::Presented)).unwrap(), Some(nola_application::RuntimeTaskNotificationAcknowledgement::Delivered));
        assert_eq!(acknowledge_runtime_task_notification_delivery(&notifications[0]).unwrap(), nola_application::RuntimeTaskNotificationAcknowledgement::AlreadyDelivered);
        assert!(take_runtime_task_notifications(&config).unwrap().is_empty());

        std::env::remove_var("NOLA_DB_PATH");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn worker_terminal_state_is_not_completion_until_core_matches() {
        let running = core_task_fixture(nola_core::agent::TaskState::Running, None);
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&running),
            Some("receipt"),
            None
        ));

        let stale = core_task_fixture(nola_core::agent::TaskState::Sealed, Some(6));
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&stale),
            Some("receipt"),
            None
        ));

        let mut committed = core_task_fixture(nola_core::agent::TaskState::Sealed, Some(7));
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&committed),
            Some("receipt"),
            None
        ));
        committed.durability.terminal_receipt_id = Some("receipt".into());
        assert!(terminal_states_match(
            "sealed",
            Some(7),
            Some(&committed),
            Some("receipt"),
            None
        ));
        assert!(!terminal_states_match(
            "failed",
            Some(7),
            Some(&committed),
            Some("receipt"),
            None
        ));
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&committed),
            None,
            None
        ));
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&committed),
            Some("foreign"),
            None
        ));
        assert!(!terminal_states_match(
            "sealed",
            Some(7),
            Some(&committed),
            Some("receipt"),
            Some("foreign-result")
        ));
    }

    #[test]
    fn cold_stop_cannot_invent_denial_for_unbound_permission() {
        let _environment = crate::store::test_env_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-cold-stop-permission-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create cold stop test root");
        std::env::set_var("NOLA_DB_PATH", root.join("nola.db"));
        let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().expect("cold stop test config");
        config.tool_settings.project_root = root.clone();
        config.tool_settings.allowed_dirs = vec![root.clone()];
        config.session.session_id = Some("cold-stop-owner".to_string());
        let task_id = format!("task-{}", uuid::Uuid::new_v4());
        let mut task = DbRuntimeTask {
            id: task_id.clone(),
            kind: "agent".to_string(),
            status: "queued".to_string(),
            title: "Waiting for permission".to_string(),
            project_root: root.to_string_lossy().into_owned(),
            owner_session_id: config.session.session_id.clone(),
            parent_task_id: None,
            parent_tool_call_id: Some("spawn-call".to_string()),
            child_session_id: Some("cold-stop-child".to_string()),
            agent_type: Some("general".to_string()),
            prompt: Some("continue".to_string()),
            progress_summary: Some("waiting".to_string()),
            last_activity: None,
            result_summary: None,
            error: None,
            output_path: None,
            unread: true,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: Some(1),
            completion_watermark: None,
            created_at_ms: 1,
            started_at_ms: None,
            ended_at_ms: None,
            updated_at_ms: 1,
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
        .expect("attach queued scheduling metadata");
        NolaStore::open_default()
            .expect("open cold stop store")
            .create_runtime_task(&task)
            .expect("create waiting task");
        let permission = crate::permission_request_port::ensure_core_permission(
            nola_application::NewPermissionRequest {
                request_id: "cold-stop-permission".to_string(),
                task_id: task_id.clone(),
                child_session_id: "cold-stop-child".to_string(),
                capability: "write_file".to_string(),
                input: json!({"path": root.join("result.txt")}),
                scope_rule: format!("write_file:{}", root.display()),
                created_at_ms: 2,
                core_permission_request_id: Some("core-cold-stop-permission".to_string()),
                permission_instructions: Vec::new(),
            },
        )
        .expect("persist pending permission");

        let mut other_config = config.clone();
        other_config.session.session_id = Some("other-session".to_string());
        assert!(permission_prompt_for_task(&other_config, &task_id).is_none());
        let displayed = TaskPermissionPrompt {
            prompt: tools::permission_prompt_for_tool_request(&tools::ToolRequest {
                id: permission.request_id.clone(), name: permission.capability.clone(),
                args: permission.input.as_object().cloned().unwrap_or_default(),
            }),
            permission: permission.clone(),
        };
        let cross_session_resolution =
            resolve_task_permission_choice(&other_config, &displayed, PermissionChoice::Deny);
        assert!(cross_session_resolution.error);
        let still_pending =
            crate::permission_request_port::permission_request(&permission.request_id)
                .expect("read cross-session permission")
                .expect("permission exists");
        assert_eq!(
            still_pending.status,
            nola_application::PermissionRequestStatus::Pending
        );

        let outcome = stop_task(
            &config,
            &Map::from_iter([("task_id".to_string(), Value::String(task_id.clone()))]),
            &crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
                config.tool_settings.skill_dirs.clone(),
            ),
        );
        assert!(!outcome.error, "{}", outcome.text);
        let retained = crate::permission_request_port::permission_request(&permission.request_id)
            .expect("read permission")
            .expect("permission exists");
        // This fixture has no actual Core Run/effect or suspension. The Task's
        // own cancellation receipt closes its permission surface without
        // pretending that the user denied it or a Core effect was withdrawn.
        assert_eq!(retained.status, nola_application::PermissionRequestStatus::Withdrawn);
        assert_eq!(retained.resolution, permission.resolution);
        assert_eq!(retained.resolved_at_ms, permission.resolved_at_ms);
        assert!(matches!(retained.closure.as_ref().unwrap().source,
            nola_domain::permission_closure::PermissionClosureSource::UnboundTaskTerminal { .. }));
        assert!(matches!(
            crate::permission_request_port::authorize_permission(&permission.request_id,
                nola_application::PermissionAuthorizationDecision::AllowOnce, unix_epoch_millis()).unwrap(),
            nola_application::PermissionResolutionOutcome::Closed { request }
                if request == retained
        ));
    }

    #[test]
    fn persisted_return_to_composer_maps_to_background_cancellation_decision() {
        let request = nola_application::PermissionRequest {
            closure: None,
            core_permission_request_id: Some("core-fixture-request".into()),
            permission_instructions: Vec::new(),
            request_id: "permission-1".to_string(),
            task_id: "task-1".to_string(),
            child_session_id: "child-1".to_string(),
            capability: "write_file".to_string(),
            input: json!({}),
            scope_rule: "write_file:/tmp".to_string(),
            status: nola_application::PermissionRequestStatus::Resolved,
            resolution: Some(
                nola_application::PermissionRequestResolution::Intervention {
                    intervention: nola_application::PermissionIntervention::ReturnToComposer,
                },
            ),
            created_at_ms: 1,
            resolved_at_ms: Some(2),
        };
        assert!(matches!(
            background_permission_decision(request),
            Some(BackgroundPermissionDecision::ReturnToComposer)
        ));
    }

    #[test]
    fn task_json_exposes_stable_public_state() {
        let mut task = DbRuntimeTask {
            id: "task-1".to_string(),
            kind: "agent".to_string(),
            status: "running".to_string(),
            title: "Inspect code".to_string(),
            project_root: "/workspace".to_string(),
            owner_session_id: Some("parent".to_string()),
            parent_task_id: None,
            parent_tool_call_id: Some("call".to_string()),
            child_session_id: Some("child".to_string()),
            agent_type: Some("explore".to_string()),
            prompt: Some("Inspect".to_string()),
            progress_summary: Some("Searching".to_string()),
            last_activity: Some("grep".to_string()),
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
            created_at_ms: 1,
            started_at_ms: Some(2),
            ended_at_ms: None,
            updated_at_ms: 3,
            metadata_json: "{}".to_string(),
        };
        let value = task_json(&task);
        assert_eq!(value["task_id"], "task-1");
        assert_eq!(value["status"], "running");
        assert_eq!(value["agent_type"], "explore");
        assert!(value["parent_task_id"].is_null());
        assert!(value.get("prompt").is_none());
        assert!(task_scope_matches_values(
            &task,
            "/workspace",
            Some("parent")
        ));
        assert!(!task_scope_matches_values(
            &task,
            "/workspace",
            Some("other")
        ));
        assert!(!task_scope_matches_values(&task, "/workspace", None));
        task.owner_session_id = None;
        assert!(!task_scope_matches_values(
            &task,
            "/workspace",
            Some("other")
        ));
        assert!(task_scope_matches_values(&task, "/workspace", None));
        assert!(!task_scope_matches_values(&task, "/other", Some("parent")));
    }

    #[test]
    fn another_session_cannot_observe_control_or_consume_a_persisted_agent_task() {
        let _environment = crate::store::test_env_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-cross-session-task-scope-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create task scope test root");
        let database = root.join("nola.db");
        std::env::set_var("NOLA_DB_PATH", &database);

        let mut owner_config = crate::host_invocation_request::HostInvocationRequest::for_test().expect("owner task config");
        owner_config.tool_settings.project_root = root.clone();
        owner_config.tool_settings.allowed_dirs = vec![root.clone()];
        owner_config.session.session_id = Some("owner-session".to_string());

        let task_id = format!("task-{}", uuid::Uuid::new_v4());
        let now = unix_epoch_millis();
        let mut task = DbRuntimeTask {
            id: task_id.clone(),
            kind: "agent".to_string(),
            status: "queued".to_string(),
            title: "Private child task".to_string(),
            project_root: root.to_string_lossy().into_owned(),
            owner_session_id: owner_config.session.session_id.clone(),
            parent_task_id: None,
            parent_tool_call_id: Some("private-spawn-call".to_string()),
            child_session_id: Some("private-child".to_string()),
            agent_type: Some("general".to_string()),
            prompt: Some("private prompt".to_string()),
            progress_summary: Some("queued".to_string()),
            last_activity: None,
            result_summary: None,
            error: None,
            output_path: None,
            unread: true,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: Some(now),
            completion_watermark: None,
            created_at_ms: now,
            started_at_ms: None,
            ended_at_ms: None,
            updated_at_ms: now,
            metadata_json: "{}".to_string(),
        };
        task.set_scheduling(nola_application::RuntimeTaskScheduling {
            priority: nola_application::RuntimeTaskPriority::Normal,
            queued_at_ms: now,
            resource_budget: nola_application::RuntimeTaskResourceBudget::default(),
            output_bytes: 0,
            resource_usage: nola_application::RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        })
        .expect("attach scheduling metadata");
        NolaStore::open(&database)
            .expect("open task scope store")
            .create_runtime_task(&task)
            .expect("persist private task");
        source_admission_support::agent_task_admission::admit_existing(
            &database,
            &task_id,
            "owner-session",
            "private-child",
            "private-spawn-call",
            Some(now),
        );

        let mut other_config = owner_config.clone();
        other_config.session.session_id = Some("other-session".to_string());
        let args = Map::from_iter([("task_id".to_string(), Value::String(task_id.clone()))]);
        assert_eq!(running_task_count(&other_config), 0);
        let listed: Value = serde_json::from_str(&list_tasks(&other_config).text)
            .expect("decode task list for other session");
        assert_eq!(listed["count"], 0);
        let skills = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(
            other_config.tool_settings.skill_dirs.clone(),
        );
        let read = read_task(&other_config, &args, &skills);
        assert!(read.error);
        assert!(read.text.contains("Unknown task"));
        let stop = stop_task(&other_config, &args, &skills);
        assert!(stop.error);
        assert!(stop.text.contains("Unknown task"));

        let reopened = NolaStore::open(&database).expect("reopen task scope store");
        let persisted = reopened
            .runtime_task(&task_id)
            .expect("read private task")
            .expect("private task exists");
        assert!(!persisted.cancel_requested);
        assert_eq!(persisted.owner_session_id.as_deref(), Some("owner-session"));
        assert_eq!(running_task_count(&owner_config), 1);

        assert!(!list_tasks(&owner_config).error);
        assert!(!manage_runtime_tasks(&owner_config, "list", None).error);
        assert!(!manage_runtime_tasks(&owner_config, "status", Some(&task_id)).error);
        assert!(pending_task_context(&owner_config).is_none());
        let observed = reopened.runtime_task(&task_id).unwrap().unwrap();
        assert_eq!(observed.status, "queued", "queries must not launch an admitted worker");
        assert!(observed.worker_id.is_none());
        let database_check = rusqlite::Connection::open(&database).unwrap();
        let attempts: i64 = database_check.query_row(
            "SELECT COUNT(*) FROM runtime_task_attempts WHERE task_id=?1",
            [&task_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(attempts, 0, "query composition must not create attempts");

        let attempt = reopened
            .claim_runtime_task(
                &task_id,
                "scope-test-worker",
                "scope-test-generation",
                "scope-test-token",
                now + 1,
                now + 10_000,
            )
            .expect("claim private task")
            .expect("private task is claimable");
        assert_eq!(attempt.task_id, task_id);
        let lease = crate::runtime_task_port::register_runtime_task_attempt(attempt);
        // A cancelled wait is a completed local Execute invocation, not proof
        // that the independently leased child stopped. Exercise the actual
        // wait_task wrapper against this admitted, running durable task.
        let cancellation = RuntimeExecutionControl::default();
        cancellation.request();
        let context = ToolContext {
            memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
            language_services: Default::default(),
            output_store: tools::ToolOutputStore::for_test(),
            skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
            allowed_dirs: vec![root.clone()],
            policy: tools::ToolExecutionPolicy::new(Vec::new(), false, "ask".into(), Vec::new()),
            permission_scope: Default::default(),
            cancellation,
            plan_state: Default::default(),
            browser: tools::BrowserRuntime::new(root.join("browser")),
        };
        let waited = wait_task(&owner_config, &args, &context);
        assert!(waited.error, "cancelled waiting must remain an error result");
        assert!(waited.text.contains("Task wait cancelled"), "{}", waited.text);
        assert_eq!(waited.execution.as_ref().map(|receipt| receipt.state),
            Some(nola_host_protocol::tool_result::ExecutionState::Completed));
        let still_running = reopened.runtime_task(&task_id).unwrap().unwrap();
        assert_eq!(still_running.status, "running");
        assert!(!still_running.cancel_requested, "wait cancellation cannot cancel its child");
        assert!(still_running.ended_at_ms.is_none());
        assert_eq!(still_running.ownership_token, Some("scope-test-token".into()));
        let attempt_id = lease.snapshot().unwrap().id;
        reopened.append_runtime_task_event(&task_id, "worker_process_exited", "suspended",
            Some("worker exited before completion"), &json!({"attempt_id":attempt_id}).to_string(),
            unix_epoch_millis()).unwrap();
        let mut context = context;
        context.cancellation = RuntimeExecutionControl::default();
        let mut exit_args = args.clone();
        exit_args.insert("timeout_sec".into(), json!(1));
        let exit = wait_task(&owner_config, &exit_args, &context);
        assert!(!exit.error, "{}", exit.text);
        let exit:Value=serde_json::from_str(&exit.text).unwrap();
        assert_eq!(exit["action_required"], "recovery");
        assert_eq!(exit["wait_timed_out"], false);
        assert_eq!(exit["completion_confirmed"], false);
        assert_eq!(reopened.runtime_task(&task_id).unwrap().unwrap().status, "running");
        assert!(reopened
            .finish_runtime_task_detailed_with_event(
                &lease,
                "failed",
                Some(now),
                Some("failed"),
                Some("private result"),
                None,
                None,
                true,
                "finished",
                "failed",
                Some("private task finished"),
                "{}",
                now + 2,
            )
            .expect("finish private task"));
        assert!(take_runtime_task_notifications(&other_config)
            .expect("read other-session notifications")
            .is_empty());
        assert!(
            reopened
                .runtime_task(&task_id)
                .expect("read private notification state")
                .expect("private task exists")
                .unread
        );
        let owner_notifications =
            take_runtime_task_notifications(&owner_config).expect("read owner notifications");
        assert_eq!(owner_notifications.len(), 1);
        assert_eq!(owner_notifications[0].id, task_id);

        std::env::remove_var("NOLA_DB_PATH");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn completed_agent_task_can_resume_only_inside_its_owner_boundary() {
        let mut task = DbRuntimeTask {
            id: "task-1".to_string(),
            kind: "agent".to_string(),
            status: "sealed".to_string(),
            title: "Inspect code".to_string(),
            project_root: "/workspace".to_string(),
            owner_session_id: Some("parent".to_string()),
            parent_task_id: None,
            parent_tool_call_id: Some("call".to_string()),
            child_session_id: Some("child".to_string()),
            agent_type: Some("explore".to_string()),
            prompt: Some("Inspect".to_string()),
            progress_summary: None,
            last_activity: None,
            result_summary: Some("Done".to_string()),
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
            input_watermark: Some(1),
            completion_watermark: Some(2),
            created_at_ms: 1,
            started_at_ms: Some(1),
            ended_at_ms: Some(2),
            updated_at_ms: 2,
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            resumable_child_session(&task, "/workspace", "parent", "explore").unwrap(),
            "child"
        );
        assert!(resumable_child_session(&task, "/other", "parent", "explore").is_err());
        assert!(resumable_child_session(&task, "/workspace", "other", "explore").is_err());
        task.status = "running".to_string();
        assert!(resumable_child_session(&task, "/workspace", "parent", "explore").is_err());
    }

    #[test]
    fn bounded_output_preserves_utf8_and_requests_cancellation_once_limit_is_reached() {
        let mut output = String::new();
        let failure = Arc::new(Mutex::new(None));
        let cancellation = RuntimeExecutionControl::default();
        let observed = std::sync::atomic::AtomicU64::new(0);
        append_bounded_output(&mut output, "ab水面", &observed, 5, &failure, &cancellation);
        assert_eq!(output, "ab水");
        assert_eq!(observed.load(std::sync::atomic::Ordering::Relaxed), 8);
        assert!(cancellation.is_requested());
        assert!(failure
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|reason| reason.contains("5 bytes")));
        append_bounded_output(&mut output, "later", &observed, 5, &failure, &cancellation);
        assert_eq!(output, "ab水");
    }
}

#[cfg(test)]
mod internal_task_context_authorization_tests {
    use super::*;

    #[test]
    fn supervisor_context_survives_shared_authorization_gate() {
        for title in ["Supervisor recovery report", "Background task mailbox"] {
            let component = ContextComponent {
                kind: ContextComponentKind::PlatformData,
                id: "task-ledger-context".into(),
                content: Value::String("Task is waiting for user input".into()),
                source: Some(internal_task_context_source(title)),
            };
            let prepared = nola_core::context_engine::prepare_context_components(
                ContextPlatform::Cli,
                &[component],
                4096,
            );
            assert!(serde_json::to_string(&prepared.messages)
                .unwrap()
                .contains("Task is waiting for user input"));
            assert!(prepared
                .component_metrics
                .iter()
                .any(|metric| metric.used_ids.contains(&"task-ledger-context".to_string())));
        }
    }
}
