// SPDX-License-Identifier: Apache-2.0
//! Task atomic ownership, task result delivery, agent queue port receipts, and goal execution bindings.

mod task_atomic_ownership_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Actual source admission and a single durable Task authority; no parent-body mirror.
    use crate::support;
    use nola_application::{
        ApplicationCommand, ClaimAgentTaskFair, RecoverExpiredAgentTask, RuntimeTaskAttempt,
        RuntimeTaskCommand, RuntimeTaskLeaseWitness,
    };
    use nola_domain::agent_task_admission::{
        RuntimeAgentTaskAuthority, RuntimeAgentTaskAuthorityPhase,
    };
    use nola_store::NolaStore;
    use serde_json::json;

    fn seed(path: &std::path::Path) {
        support::agent_task_admission::admitted(path, None);
        assert!(
            NolaStore::open(path)
                .unwrap()
                .load_checkpoint("parent")
                .unwrap()
                .is_none(),
            "Task ownership must not copy the source parent's checkpoint or transcript"
        );
    }
    fn claim(token: &str, at: i64) -> ApplicationCommand {
        ApplicationCommand::ClaimAgentTaskFair {
            input: ClaimAgentTaskFair {
                owner_conversation_id: "parent".into(),
                task_id: "task".into(),
                worker_id: format!("worker-{token}"),
                worker_generation: format!("generation-{token}"),
                lease_token: token.into(),
                now_ms: at,
                lease_until_ms: at + 1000,
                max_running_global: 4,
                max_running_project: 4,
            },
        }
    }
    fn recover(attempt: &RuntimeTaskAttempt, at: i64) -> ApplicationCommand {
        ApplicationCommand::RecoverExpiredAgentTask {
            input: RecoverExpiredAgentTask {
                owner_conversation_id: "parent".into(),
                lease: RuntimeTaskLeaseWitness::from(attempt),
                error: "worker expired before child admission".into(),
                retry_at_ms: at,
                now_ms: at,
            },
        }
    }
    fn task(path: &std::path::Path) -> nola_store_port::runtime_task::StoredRuntimeTask {
        NolaStore::open(path)
            .unwrap()
            .runtime_task("task")
            .unwrap()
            .unwrap()
    }
    fn leaf(path: &std::path::Path) -> RuntimeAgentTaskAuthority {
        NolaStore::open(path)
            .unwrap()
            .runtime_agent_task_authority("task")
            .unwrap()
            .unwrap()
    }
    fn rows(path: &std::path::Path) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
        let db = rusqlite::Connection::open(path).unwrap();
        let tables=db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != 'work_archive_publication' ORDER BY name").unwrap()
            .query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        tables
            .into_iter()
            .map(|name| {
                let mut query = db
                    .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                    .unwrap();
                let count = query.column_count();
                let mut rows = query
                    .query_map([], |r| {
                        (0..count)
                            .map(|i| r.get::<_, rusqlite::types::Value>(i))
                            .collect()
                    })
                    .unwrap()
                    .collect::<Result<Vec<Vec<_>>, _>>()
                    .unwrap();
                rows.sort_by_cached_key(|r| format!("{r:?}"));
                (name, rows)
            })
            .collect()
    }
    #[test]
    fn real_cross_store_admission_short_expiry_and_successor_have_one_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("task.db");
        seed(&path);
        let initial = leaf(&path);
        assert_eq!(initial.phase, RuntimeAgentTaskAuthorityPhase::Admitted);
        let mut app = support::open_runtime(&path).unwrap();
        let old: RuntimeTaskAttempt =
            serde_json::from_value(app.execute(claim("old", 1000)).unwrap()).unwrap();
        assert_eq!(task(&path).status, "running");
        assert_eq!(
            task(&path).ownership_token.as_deref(),
            Some(old.lease_token.as_str())
        );
        drop(app);
        let mut cold = support::open_runtime(&path).unwrap();
        assert_eq!(cold.execute(recover(&old, 2000)).unwrap(), json!(true));
        assert_eq!(task(&path).status, "retry_waiting");
        assert_eq!(task(&path).retry_count, 1);
        assert!(task(&path).ownership_token.is_none());
        let next: RuntimeTaskAttempt =
            serde_json::from_value(cold.execute(claim("next", 2001)).unwrap()).unwrap();
        assert_ne!(next.id, old.id);
        assert_ne!(next.lease_token, old.lease_token);
        assert_eq!(next.attempt_number, 2);
        assert_eq!(
            task(&path).ownership_token.as_deref(),
            Some(next.lease_token.as_str())
        );
        assert!(
            cold.execute(claim("illegal-running", 2002)).is_err(),
            "Running claim exclusion is strict"
        );
        assert_eq!(
            cold.execute(recover(&old, 3000)).unwrap(),
            json!(false),
            "old witness cannot retire successor"
        );
        let current = leaf(&path);
        assert_eq!(current.admission, initial.admission);
        assert!(current.authority_revision > initial.authority_revision);
    }
    #[test]
    fn source_not_committed_and_prepared_task_never_grant_execution() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prepared.db");
        let mut target = support::open_runtime(&path).unwrap();
        support::agent_task_admission::draft(&mut target, None);
        let before = rows(&path);
        assert!(target.execute(claim("unproven", 1000)).is_err());
        assert_eq!(rows(&path), before);
        let mut source = support::open_runtime(":memory:").unwrap();
        assert!(source
            .execute(ApplicationCommand::ExportAgentTaskAdmission {
                conversation_id: "parent".into(),
                run_id: "parent-run".into(),
                task_id: "task".into(),
                event_cursor: 1
            })
            .is_err());
        assert_eq!(leaf(&path).phase, RuntimeAgentTaskAuthorityPhase::Prepared);
        let fact = support::agent_task_admission::source_fact(&mut source);
        let args = serde_json::json!({"type":"import_host_agent_task_admission","admission":fact});
        assert!(
            serde_json::from_value::<ApplicationCommand>(args).is_err(),
            "ordinary JSON cannot import a self-made source proof"
        );
        target
            .execute(ApplicationCommand::ImportHostAgentTaskAdmission {
                admission: fact.clone(),
            })
            .unwrap();
        let admitted = leaf(&path);
        let immutable = rows(&path);
        target
            .execute(ApplicationCommand::ImportHostAgentTaskAdmission { admission: fact })
            .unwrap();
        assert_eq!(leaf(&path), admitted);
        assert_eq!(
            rows(&path),
            immutable,
            "same actual source fact is idempotent"
        );
    }
    #[test]
    fn authority_failure_rolls_back_grant_and_expiry_retirement() {
        for during_claim in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("rollback.db");
            seed(&path);
            let mut app = support::open_runtime(&path).unwrap();
            let old = if during_claim {
                None
            } else {
                Some(
                    serde_json::from_value::<RuntimeTaskAttempt>(
                        app.execute(claim("old", 1000)).unwrap(),
                    )
                    .unwrap(),
                )
            };
            let before = rows(&path);
            rusqlite::Connection::open(&path).unwrap().execute_batch(
                "CREATE TRIGGER reject_authority_update BEFORE UPDATE OF authority_revision ON runtime_agent_task_authorities
                 BEGIN SELECT RAISE(ABORT,'injected authority commit failure'); END;").unwrap();
            assert!(app
                .execute(
                    old.as_ref()
                        .map_or_else(|| claim("new", 1000), |old| recover(old, 2000))
                )
                .is_err());
            assert_eq!(
                rows(&path),
                before,
                "Task/Attempt/leaf/events/budgets commit or roll back together"
            );
        }
    }
    #[test]
    fn renewal_cas_winner_is_not_retired_and_expired_grants_are_not_revived() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("renew.db");
        seed(&path);
        let mut app = support::open_runtime(&path).unwrap();
        let old: RuntimeTaskAttempt =
            serde_json::from_value(app.execute(claim("old", 1000)).unwrap()).unwrap();
        let next: RuntimeTaskAttempt = serde_json::from_value(
            app.execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::RenewLease {
                    task_id: old.task_id.clone(),
                    attempt_id: old.id.clone(),
                    expected_attempt_revision: old.revision,
                    worker_generation: old.worker_generation.clone(),
                    lease_token: old.lease_token.clone(),
                    current_expiry_ms: old.lease_expires_at_ms,
                    now_ms: 1999,
                    requested_expiry_ms: 2999,
                },
            })
            .unwrap(),
        )
        .unwrap();
        let before = rows(&path);
        assert_eq!(app.execute(recover(&old, 2001)).unwrap(), json!(false));
        assert_eq!(rows(&path), before);
        assert!(app
            .execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::RenewLease {
                    task_id: next.task_id.clone(),
                    attempt_id: next.id.clone(),
                    expected_attempt_revision: next.revision,
                    worker_generation: next.worker_generation.clone(),
                    lease_token: next.lease_token.clone(),
                    current_expiry_ms: next.lease_expires_at_ms,
                    now_ms: 2999,
                    requested_expiry_ms: 3999
                }
            })
            .is_err());
        assert_eq!(app.execute(recover(&next, 3000)).unwrap(), json!(true));
    }
    #[test]
    fn lost_claim_ack_uses_the_same_receipt_and_never_creates_another_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt.db");
        seed(&path);
        let command = claim("old", 1000);
        let raw = serde_json::to_string(&command).unwrap();
        let request = nola_application::ExecuteReceivedCommand {
            client_id: "worker".into(),
            request_id: "stable-claim".into(),
            command_type: "claim_agent_task_fair".into(),
            command_json: raw.clone(),
            expected_revision: None,
            received_at: "1000".into(),
        };
        let mut app = support::open_runtime(&path).unwrap();
        let committed = app
            .execute_received_command(request.clone(), serde_json::from_str(&raw).unwrap())
            .unwrap();
        assert!(matches!(
            committed.outcome,
            nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        drop(app);
        let before = rows(&path);
        let mut cold = support::open_runtime(&path).unwrap();
        assert!(
            cold.execute_received_command(request, serde_json::from_str(&raw).unwrap())
                .unwrap()
                .replayed
        );
        assert_eq!(rows(&path), before);
        let count: i64 = rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("SELECT count(*) FROM runtime_task_attempts", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
}

mod task_result_delivery_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Exact result consumption must not acknowledge an undelivered peer or UI.
    use crate::support;

    use nola_application::{
        AcknowledgeRuntimeTaskResult, ApplicationCommand, ApplicationCommandReceiptOutcome,
        CreateRuntimeTask, ExecuteReceivedCommand, RuntimeTaskCommand, RuntimeTaskLeaseWitness,
    };
    use nola_agent_engine::agent::{host::*, AgentEvent, SessionId, TaskKind, TaskLineage, TaskState};
    use nola_store::NolaStore;
    use serde_json::{json, Value};

    fn seed(path: &std::path::Path) -> [AcknowledgeRuntimeTaskResult; 2] {
        let mut app = support::open_runtime(path).unwrap();
        for (id, child, call) in [
            ("task", "child", "task-call"),
            ("peer", "peer-child", "peer-call"),
        ] {
            app.execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Create {
                    input: CreateRuntimeTask {
                        id: id.into(),
                        kind: "agent".into(),
                        title: id.into(),
                        project_root: "/fixture".into(),
                        owner_session_id: Some("parent".into()),
                        parent_task_id: None,
                        parent_tool_call_id: Some(call.into()),
                        child_session_id: Some(child.into()),
                        agent_type: Some("explore".into()),
                        prompt: Some("Execute this task once".into()),
                        initial_progress_summary: None,
                        initial_activity: None,
                        output_path: None,
                        initially_unread: false,
                        input_watermark: Some(7),
                        created_at_ms: 100,
                        metadata: json!({}),
                        scheduling: nola_application::RuntimeTaskScheduling {
                            priority: nola_application::RuntimeTaskPriority::Normal,
                            queued_at_ms: 100,
                            resource_budget: Default::default(),
                            resource_usage: Default::default(),
                            output_bytes: 0,
                            budget_exhaustion: None,
                        },
                    },
                },
            })
            .unwrap();
        }
        let first = support::agent_task_admission::source_fact(&mut app);
        app.execute(ApplicationCommand::ImportHostAgentTaskAdmission { admission: first })
            .unwrap();

        // Produce the peer from the SAME actual parent Run, not a second fabricated
        // source with colliding conversation/Run identities.
        let store = NolaStore::open(path).unwrap();
        let run = store.active_run("parent").unwrap().unwrap();
        let digest = run.invocation_config_digest.unwrap();
        let claim = |app: &mut support::ApplicationRuntime, at| -> HostRequestEnvelope {
            let requests: nola_application::CommittedHostRequests = serde_json::from_value(
                app.execute(ApplicationCommand::ClaimCommittedHostRequests {
                    input: nola_application::ClaimCommittedHostRequests {
                        run_id: run.run_id.clone(),
                        worker_generation: "peer-source".into(),
                        lease_owner: "source".into(),
                        claimed_at_ms: at,
                        lease_expires_at_ms: 10000,
                        limit: 16,
                    },
                })
                .unwrap(),
            )
            .unwrap();
            assert_eq!(requests.requests.len(), 1);
            requests.requests.into_iter().next().unwrap()
        };
        let accept = |app: &mut support::ApplicationRuntime,
                      request: &HostRequestEnvelope,
                      sequence,
                      payload,
                      at|
         -> nola_application::AcceptedHostResponse {
            serde_json::from_value(
                app.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                    input: nola_application::ReduceAndAcceptHostResponse {
                        conversation_id: "parent".into(),
                        run_id: run.run_id.clone(),
                        invocation_config_digest: digest.clone(),
                        received_at_ms: at,
                        response: HostResponseEnvelope {
                            attempt_id: request.attempt_id.clone(),
                            delegated_invocation_id: request.delegated_invocation_id.clone(),
                            request_id: request.request_id.clone(),
                            effect_id: request.effect_id.clone(),
                            host_id: request.host_id.clone(),
                            lease_id: request.lease_id.clone(),
                            session_id: request.session_id.clone(),
                            turn_id: request.turn_id.clone(),
                            catalog_revision: request.catalog_revision,
                            availability_epoch: request.availability_epoch,
                            delivery: request.delivery.clone(),
                            sequence,
                            payload,
                        },
                    },
                })
                .unwrap(),
            )
            .unwrap()
        };
        let mut model = claim(&mut app, 700);
        assert!(matches!(model.payload, HostRequest::ModelStream { .. }));
        accept(&mut app, &model, 1, HostResponse::ExecutionStarted, 701);
        model.resume_after_sequence = Some(1);
        accept(
            &mut app,
            &model,
            2,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "fixture-model".into(),
                reason: "initial".into(),
            },
            702,
        );
        model.resume_after_sequence = Some(2);
        accept(
            &mut app,
            &model,
            3,
            HostResponse::ModelCompleted {
                items: vec![],
                tool_calls: vec![
                    json!({"call_id":"peer-call","name":"fixture.task","arguments":{"task_id":"peer"}}),
                ],
                usage: json!({}),
            },
            703,
        );
        let tool = claim(&mut app, 710);
        assert!(matches!(tool.payload, HostRequest::ExecuteTool { .. }));
        accept(&mut app, &tool, 1, HostResponse::ExecutionStarted, 711);
        let promoted = accept(
            &mut app,
            &tool,
            2,
            HostResponse::TaskPromoted {
                task_id: "peer".into(),
                input_watermark: Some(7),
                kind: TaskKind::Agent,
                state: TaskState::Queued,
                progress: None,
                current_step: Some("Queued".into()),
                lineage: TaskLineage {
                    child_session_id: Some(SessionId::from_string("peer-child".into())),
                    parent_call_id: Some(nola_agent_engine::agent::CallId::from_string("peer-call".into())),
                    ..Default::default()
                },
            },
            712,
        );
        let cursor = promoted
            .events
            .iter()
            .find_map(|event| match &event.event {
                AgentEvent::TaskUpdated { task, .. } if task.id == "peer" => {
                    Some(event.cursor.sequence)
                }
                _ => None,
            })
            .unwrap();
        let admission = serde_json::from_value(
            app.execute(ApplicationCommand::ExportAgentTaskAdmission {
                conversation_id: "parent".into(),
                run_id: run.run_id.clone(),
                task_id: "peer".into(),
                event_cursor: cursor,
            })
            .unwrap(),
        )
        .unwrap();
        app.execute(ApplicationCommand::ImportHostAgentTaskAdmission { admission })
            .unwrap();
        drop(app);
        let mut store = NolaStore::open(path).unwrap();
        ["task", "peer"].map(|id| {
            let attempt = store
                .claim_runtime_task(
                    id,
                    &format!("worker-{id}"),
                    &format!("generation-{id}"),
                    &format!("token-{id}"),
                    1000,
                    5000,
                )
                .unwrap()
                .unwrap();
            assert!(store
                .finish_runtime_task_detailed_with_event(
                    &RuntimeTaskLeaseWitness::from(&attempt),
                    "failed",
                    Some(7),
                    Some("done"),
                    Some(id),
                    None,
                    None,
                    true,
                    "failed",
                    "failed",
                    Some("done"),
                    "{}",
                    None,
                    1100
                )
                .unwrap());
            let receipt = store.runtime_task_terminal_receipt(id).unwrap().unwrap();
            let task = store.runtime_task(id).unwrap().unwrap();
            assert!(task.unread);
            AcknowledgeRuntimeTaskResult {
                owner_conversation_id: "parent".into(),
                task_id: id.into(),
                terminal_receipt_id: receipt.receipt_id,
                expected_updated_at_ms: task.updated_at_ms,
            }
        })
    }

    fn command(input: AcknowledgeRuntimeTaskResult) -> ApplicationCommand {
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::AcknowledgeResult { input },
        }
    }

    fn state(path: &std::path::Path) -> Value {
        let store = NolaStore::open(path).unwrap();
        let receipts =
            ["task", "peer"].map(|id| store.runtime_task_terminal_receipt(id).unwrap().unwrap());
        let attempts = receipts
            .iter()
            .flat_map(|receipt| &receipt.attempts)
            .map(|attempt| {
                store
                    .runtime_task_attempt(&attempt.id)
                    .unwrap()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        json!([
            store.runtime_task("task").unwrap(),
            store.runtime_task("peer").unwrap(),
            store.runtime_task_terminal_receipt("task").unwrap(),
            store.runtime_task_terminal_receipt("peer").unwrap(),
            store.runtime_agent_task_authority("task").unwrap(),
            store.runtime_agent_task_authority("peer").unwrap(),
            attempts
        ])
    }

    #[test]
    fn exact_result_ack_preserves_peer_and_ui_and_rejects_foreign_or_stale_proofs() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("results.db");
        let [first, peer] = seed(&path);
        let mut app = support::open_runtime(&path).unwrap();
        let before = state(&path);
        for input in [
            AcknowledgeRuntimeTaskResult {
                owner_conversation_id: "foreign".into(),
                ..first.clone()
            },
            AcknowledgeRuntimeTaskResult {
                terminal_receipt_id: peer.terminal_receipt_id.clone(),
                ..first.clone()
            },
            AcknowledgeRuntimeTaskResult {
                expected_updated_at_ms: first.expected_updated_at_ms - 1,
                ..first.clone()
            },
        ] {
            assert!(app.execute(command(input)).is_err());
            assert_eq!(state(&path), before);
        }
        assert_eq!(app.execute(command(first.clone())).unwrap(), json!(true));
        let after = state(&path);
        assert_eq!(
            after[1], before[1],
            "undelivered peer must stay entirely unchanged"
        );
        assert_eq!(after[2], before[2]);
        assert_eq!(after[3], before[3]);
        assert_eq!(after[0]["unread"], false);
        let old_metadata: Value =
            serde_json::from_str(before[0]["metadata_json"].as_str().unwrap()).unwrap();
        let metadata: Value =
            serde_json::from_str(after[0]["metadata_json"].as_str().unwrap()).unwrap();
        assert_eq!(metadata["model_notified"], true);
        assert_eq!(metadata["ui_notified"], old_metadata["ui_notified"]);
        assert_eq!(app.execute(command(first.clone())).unwrap(), json!(false));
        assert_eq!(state(&path), after);
        drop(app);
        let mut cold = support::open_runtime(&path).unwrap();
        assert_eq!(cold.execute(command(first)).unwrap(), json!(false));
        assert_eq!(state(&path), after);
        assert_eq!(cold.execute(command(peer)).unwrap(), json!(true));
    }

    #[test]
    fn result_ack_receipt_failure_rolls_back_and_lost_ack_replays_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollback.db");
        let [first, _] = seed(&path);
        let mut app = support::open_runtime(&path).unwrap();
        let encoded = serde_json::to_string(&command(first)).unwrap();
        let envelope = || ExecuteReceivedCommand {
            client_id: "result-delivery".into(),
            request_id: "exact-result".into(),
            command_type: "runtime_task".into(),
            command_json: encoded.clone(),
            expected_revision: None,
            received_at: "2026-10-08T00:00:00Z".into(),
        };
        let before = state(&path);
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER fail_result_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'result receipt fault'); END;").unwrap();
        let error = app
            .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
            .unwrap_err();
        assert!(error.to_string().contains("result receipt fault"));
        assert_eq!(state(&path), before);
        db.execute_batch("DROP TRIGGER fail_result_receipt")
            .unwrap();
        let accepted = app
            .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
            .unwrap();
        let ApplicationCommandReceiptOutcome::Committed { ref result, .. } = accepted.outcome else {
            panic!("result delivery did not commit");
        };
        assert_eq!(*result, json!(true));
        let after = state(&path);
        assert_eq!(after[1], before[1]);
        drop(app);
        let mut cold = support::open_runtime(&path).unwrap();
        let replay = cold
            .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, accepted.outcome);
        assert_eq!(state(&path), after);
    }
}

mod agent_queue_port_receipt_e2e {
    // SPDX-License-Identifier: Apache-2.0
    use crate::support;
    use nola_application::{
        AgentCommand, ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
    };
    use serde_json::json;

    #[test]
    fn empty_queue_observation_has_no_lease_or_mutation_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("queue-observation.sqlite");
        let mut runtime = support::open_runtime(&database).unwrap();
        let db = rusqlite::Connection::open(&database).unwrap();
        let before: i64 = db.query_row("SELECT count(*) FROM command_receipts", [], |row| row.get(0)).unwrap();
        for _ in 0..100 {
            let command = ApplicationCommand::PeekAgentCommand { session_id: "owner".into() };
            assert!(command.is_read_only_query());
            assert_eq!(runtime.execute(command).unwrap(), json!(null));
        }
        let after: i64 = db.query_row("SELECT count(*) FROM command_receipts", [], |row| row.get(0)).unwrap();
        assert_eq!(after, before);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_commands", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn public_queue_rejects_invalid_or_internal_payloads_without_inserting_rows() {
        let directory=tempfile::tempdir().unwrap();let database=directory.path().join("invalid.sqlite");
        let mut runtime=support::open_runtime(&database).unwrap();
        for payload in ["{", "{}", "null", r#"{"type":"future_unknown"}"#,
            r#"{"type":"user_input"}"#,
            r#"{"type":"task_terminal_available","task_id":"t","receipt_id":"r"}"#,
            r#"{"type":"task_guidance_available","task_id":"t","message_id":"m","settlement":true}"#,
            r#"{"type":"permission_policy_available","policy":{"run_id":"r","configuration_revision":2,"profile":"allow_all"}}"#,
            r#"{"type":"apply_permission_policy","policy":{"run_id":"r","configuration_revision":2,"profile":"allow_all"}}"#,
            r#"{"type":"submission","submission":{"type":"interrupt","reason":null}}"#] {
            assert!(runtime.execute(ApplicationCommand::EnqueueAgentCommand {input:AgentCommand {
                session_id:"s".into(),command_id:"same-untrusted-id".into(),payload_json:payload.into(),created_at_ms:1,
            }}).is_err(),"accepted invalid queue input {payload}");
        }
        let db=rusqlite::Connection::open(&database).unwrap();
        assert_eq!(db.query_row("SELECT COUNT(*) FROM agent_commands",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    }

    #[test]
    fn queue_claim_port_rolls_back_receipt_failure_and_cold_replays_lease() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("queue-port.db");
        let mut runtime = support::open_runtime(&database).unwrap();
        runtime
            .execute(ApplicationCommand::EnqueueAgentCommand {
                input: AgentCommand {
                    session_id: "s".into(),
                    command_id: "c".into(),
                    payload_json: serde_json::to_string(&nola_agent_engine::agent::AgentSubmission::UserInput {
                        items:vec![nola_agent_engine::agent::AgentItem::user_message(nola_agent_engine::agent::ItemId::from_string("input".into()),"继续")],
                        turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
                        submission_id:Some("submission".into()),submitted_at_epoch_ms:Some(1),
                        delivery:nola_agent_engine::agent::AgentInputDelivery::NextTurn,
                    }).unwrap(),
                    created_at_ms: 1,
                },
            })
            .unwrap();
        let command = ApplicationCommand::ClaimNextAgentCommand {
            session_id: "s".into(),
            lease_owner: "host".into(),
            now_ms: 2,
            lease_until_ms: 20,
        };
        let head = runtime.execute(ApplicationCommand::PeekAgentCommand { session_id: "s".into() }).unwrap();
        assert_eq!(head["command_id"], "c");
        assert_eq!(head["lease_owner"], json!(null));
        assert!(head.get("payload_json").is_none(), "observation is not command delivery");
        let encoded = serde_json::to_string(&command).unwrap();
        let envelope = || ExecuteReceivedCommand {
            client_id: "queue-test".into(),
            request_id: "claim".into(),
            command_type: "claim_next_agent_command".into(),
            command_json: encoded.clone(),
            expected_revision: None,
            received_at: "2026-09-08T00:00:00Z".into(),
        };
        let fault = rusqlite::Connection::open(&database).unwrap();
        fault.execute_batch("CREATE TRIGGER fail_queue_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'queue receipt fault'); END;").unwrap();
        assert!(runtime
            .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
            .unwrap_err()
            .to_string()
            .contains("queue receipt fault"));
        assert_eq!(
            runtime
                .execute(ApplicationCommand::LeasedAgentCommands {
                    session_id: "s".into()
                })
                .unwrap(),
            json!([])
        );
        fault
            .execute_batch("DROP TRIGGER fail_queue_receipt")
            .unwrap();
        let committed = runtime
            .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
            .unwrap();
        let ApplicationCommandReceiptOutcome::Committed { ref result, .. } = committed.outcome else {
            panic!("claim not committed")
        };
        assert_eq!(result["lease_owner"], "host");
        assert_eq!(result["delivery_attempt"], 1);
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        let replay = runtime
            .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, committed.outcome);
        let leases = runtime
            .execute(ApplicationCommand::LeasedAgentCommands {
                session_id: "s".into(),
            })
            .unwrap();
        assert_eq!(leases[0]["delivery_attempt"], 1);
        assert_eq!(
            runtime
                .execute(ApplicationCommand::AcknowledgeAgentCommand {
                    session_id: "s".into(),
                    command_id: "c".into(),
                    lease_owner: "wrong".into()
                })
                .unwrap(),
            json!(false)
        );
        assert_eq!(
            runtime
                .execute(ApplicationCommand::AcknowledgeAgentCommand {
                    session_id: "s".into(),
                    command_id: "c".into(),
                    lease_owner: "host".into()
                })
                .unwrap(),
            json!(true)
        );
    }

    #[test]
    fn queued_turn_rejects_a_previous_claim_from_the_same_owner() {
        use nola_agent_engine::agent::{AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId};
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("queued-admission-fence.sqlite");
        let mut runtime = support::open_runtime(&database).unwrap();
        let submission = AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(ItemId::from_string("ui-user".into()), "card action")],
            turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
            submission_id: Some("ui-command".into()), submitted_at_epoch_ms: Some(1),
            delivery: AgentInputDelivery::NextTurn,
        };
        runtime.execute(ApplicationCommand::EnqueueAgentCommand { input: AgentCommand {
            session_id: "chat".into(), command_id: "ui-command".into(),
            payload_json: serde_json::to_string(&submission).unwrap(), created_at_ms: 1,
        }}).unwrap();
        let claim = |now_ms| ApplicationCommand::ClaimNextAgentCommand {
            session_id: "chat".into(), lease_owner: "same-host".into(), now_ms, lease_until_ms: 100,
        };
        let first = runtime.execute(claim(2)).unwrap();
        runtime.execute(ApplicationCommand::ReleaseAgentCommandLease {
            session_id: "chat".into(), command_id: "ui-command".into(), lease_owner: "same-host".into(),
        }).unwrap();
        let second = runtime.execute(claim(3)).unwrap();
        assert_ne!(first["delivery_attempt"], second["delivery_attempt"]);
        let error = runtime.execute(ApplicationCommand::BeginQueuedUserTurn {
            input: nola_application::BeginQueuedUserTurn {
                conversation_id: "chat".into(), command_id: "ui-command".into(), lease_owner: "same-host".into(),
                delivery_attempt: first["delivery_attempt"].as_i64().unwrap(),
                expected_configuration_revision: 1, expected_parent_message_id: None,
                core_config: AgentCoreConfig::default(), turn_context_items: vec![],
                turn_context_platform: Default::default(), turn_context_components: vec![], now_ms: 4,
            },
        }).unwrap_err();
        assert!(error.contains("lease is stale"), "{error}");
        let head = runtime.execute(ApplicationCommand::PeekAgentCommand { session_id: "chat".into() }).unwrap();
        assert_eq!(head["command_id"], "ui-command");
        let db = rusqlite::Connection::open(database).unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM runs", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn rejected_ui_dispatch_can_be_cancelled_without_replaying_or_erasing_receipts() {
        use nola_domain::remote_submission::RemoteSubmissionIdentity;
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("cancel-ui.sqlite");
        let mut store = nola_store::NolaStore::open(&database).unwrap();
        let identity = RemoteSubmissionIdentity { source_host_id:"host".into(), client_id:"client".into(),
            conversation_id:"conversation".into(), submission_id:"action".into(),
            user_message_id:"ui:action:user".into(), assistant_message_id:"ui:action:assistant".into() };
        let surface = json!({"owner":{"authority_id":"host","conversation_id":"conversation","participant_id":"assistant",
            "context_id":"conversation","originating_run_id":"source","message_id":"source:assistant","part_id":"tool"},"surface_id":"form","generation":1});
        let action = json!({"surface":{"identity":surface,"revision":1},"summary":"Submitted form · submit",
            "envelope":{"type":"ui_action","surface":surface,"message":{"version":"v0.9.1","action":{"surfaceId":"form","name":"submit","sourceComponentId":"button","timestamp":"2026-10-21T00:00:00Z"}}}});
        let item: nola_agent_engine::agent::AgentItem = serde_json::from_value(json!({"type":"ui_action","id":"ui:action:user","action":action})).unwrap();
        let payload = nola_agent_engine::agent::host::AgentBoundaryInput::Submission {
            submission:nola_agent_engine::agent::AgentSubmission::UserInput {
                items:vec![item],turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
                submission_id:Some("action".into()),submitted_at_epoch_ms:Some(1),delivery:nola_agent_engine::agent::AgentInputDelivery::NextTurn }};
        store.enqueue_agent_command(&nola_store_port::agent_queue::EnqueueAgentCommandInput {
            session_id:"conversation".into(),command_id:"action".into(),payload_json:serde_json::to_string(&payload).unwrap(),created_at_ms:1 }).unwrap();
        let registered = store.register_source_submission(&identity, &json!({"original":"frozen"})).unwrap();
        let digest = registered.receipt.request_digest;
        drop(store);
        let mut runtime = support::open_runtime(&database).unwrap();
        let cancel = |identity: RemoteSubmissionIdentity, now_ms| ApplicationCommand::CancelRejectedQueuedUiAction { identity,request_digest:digest.clone(),now_ms };
        assert!(runtime.execute(cancel(identity.clone(),10)).is_err(), "prepared is uncertain, never cancellable as rejected");
        runtime.execute(ApplicationCommand::RejectSourceSubmission { identity:identity.clone(),request_digest:digest.clone(),rejection_code:"dispatch_not_admitted".into() }).unwrap();
        let mut foreign = identity.clone(); foreign.client_id="other".into();
        assert!(runtime.execute(cancel(foreign,10)).is_err());
        runtime.execute(ApplicationCommand::ClaimNextAgentCommand {session_id:"conversation".into(),lease_owner:"worker".into(),now_ms:10,lease_until_ms:20}).unwrap();
        assert!(runtime.execute(cancel(identity.clone(),15)).is_err(), "live dispatch lease blocks cancellation");
        let db = rusqlite::Connection::open(&database).unwrap();
        db.execute_batch("CREATE TRIGGER reject_ui_cancel BEFORE DELETE ON agent_commands BEGIN SELECT RAISE(ABORT, 'injected cancellation failure'); END;").unwrap();
        assert!(runtime.execute(cancel(identity.clone(),21)).is_err());
        let retained = runtime.execute(ApplicationCommand::PeekAgentCommand {session_id:"conversation".into()}).unwrap();
        assert_eq!(retained["lease_owner"], "worker", "failed cancellation must roll back its temporary lease");
        db.execute_batch("DROP TRIGGER reject_ui_cancel;").unwrap();
        assert_eq!(runtime.execute(cancel(identity.clone(),21)).unwrap()["cancelled"], true);
        assert_eq!(runtime.execute(cancel(identity.clone(),22)).unwrap()["cancelled"], true);
        assert_eq!(runtime.execute(ApplicationCommand::PeekAgentCommand {session_id:"conversation".into()}).unwrap(),json!(null));
        let receipt = runtime.execute(ApplicationCommand::ReadSourceSubmissionReceipt {identity,request_digest:digest}).unwrap();
        assert_eq!(receipt["phase"],"rejected");
        assert_eq!(receipt["rejection_code"],"dispatch_not_admitted");
        assert_eq!(receipt["admission"],json!(null));
    }
}

mod goal_execution_binding_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Real SQLite atomic admission and cold reads; no provider or plaintext secrets.
    use crate::support;
    use nola_agent_engine::agent::{
        protocol::AgentCoreConfig, AgentSubmission, GoalRuntimeCommand, GoalRuntimeCommandEnvelope,
    };
    use nola_application::goal_execution::*;
    use nola_domain::goal_execution::*;
    use nola_store::NolaStore;
    use nola_store_port::goal_execution::GoalExecutionRepository;
    use serde_json::json;
    fn config() -> AgentCoreConfig {
        let route = json!({"kind":"http","protocol":"chat_completions","output_token_field":"max_tokens","provider":"fixture", "endpoint":"https://fixture.invalid/v1",
            "endpoint_query_sha256":null,"model":"original-model","credential_sha256":"a".repeat(64),
            "credential_source":{"kind":"ephemeral"},"endpoint_query_source":{"kind":"none"}});
        let mut config = support::model_identity::config();
        config.host_catalog =
            Some(nola_agent_engine::agent::host::HostCapabilityCatalog::empty("host"));
        config.session_id = Some(nola_agent_engine::agent::protocol::SessionId::from_string(
            "core-c".into(),
        ));
        config.tools = vec![json!({
            "type": "function",
            "function": {"name": "fixture_tool", "parameters": {"type": "object", "properties": {}}},
            "private_context_marker": "must-not-be-disclosed"
        })];
        config.context_window.context_window_tokens = 4096;
        config.context_window.reserved_output_tokens = 64;
        config.context_window.prompt_token_limit = Some(2048);
        config.context_window.auto_compact_tokens = Some(3000);
        config.context_window.compaction_target_tokens = Some(1500);
        config.context_window.auto_compact_percent = 75;
        config.context_window.compaction_target_percent = 40;
        config.turn_identity.remote_tool_plan = Some(serde_json::from_value(json!({
            "schema_version":1,"reference":{"manifest_id":"frozen-plan","host_id":"host","client_id":"client",
                "session_id":"c","catalog_revision":"a".repeat(64)},
            "issued_at_ms":1,"expires_at_ms":300001,"tools":[]
        })).unwrap());
        config.turn_identity.host_invocation = Some(serde_json::from_value(json!({
            "version":nola_host_protocol::HOST_INVOCATION_BINDING_VERSION,
            "primary":route,"fallbacks":[],
            "generation":{"temperature":0,"top_p":0,"max_output_tokens":16384,
                "max_retries":2,"parallel_tool_calls":null,
                "reasoning_selection":null,"sampling_override":false,"generation_options":{},
                "reasoning_tier":"balanced",
                "thinking_enabled":false,"prompt_cache":false,"prompt_cache_key_sha256":null,
                "prompt_cache_key_prefix_sha256":null,"prompt_cache_retention":null},
            "output_admission":{"version":nola_host_protocol::HOST_OUTPUT_ADMISSION_VERSION,
                "routes":[{"route":route,"max_input_tokens":null,"max_output_tokens":null,"max_total_tokens":null}],
                "preference":"default","resolved":{"tokens":16384,"source":"product_default"}},
            "execution_policy_sha256":"b".repeat(64)
        })).unwrap());
        config.turn_identity.host_recovery = Some(nola_host_protocol::HostRunRecoveryReference {
            adapter: "fixture.host".into(),
            handle: "opaque-descriptor".into(),
            sha256: "c".repeat(64),
        });
        config
    }

    fn input() -> StartGoalExecution {
        StartGoalExecution {
            identity:GoalExecutionBindingIdentity {conversation_id:"c".into(),goal_id:"g".into(),operation_id:"start-g".into(),source_host_id:"host".into()},
            template:GoalExecutionTemplate {model_selection:GoalModelSelection::Profile {profile_id:"chosen".into(),revision:1},core_config:config()},
            envelope:GoalRuntimeCommandEnvelope {operation_id:"start-g".into(),command:GoalRuntimeCommand::Start {
                id:"g".into(),objective:"verify result".into(),constraints:vec![],now_epoch_ms:100,
                requirements:vec![serde_json::from_value(json!({"id":"r","semantic_revision":1,"description":"verify result","dependencies":[],"status":"pending","evidence_refs":[],"acceptance_policy":{"kind":"model_judgment"}})).unwrap()],
            }},
        }
    }
    fn seed(path: &std::path::Path) -> NolaStore {
        let mut store = NolaStore::open(path).unwrap();
        use nola_store_port::model::ModelRepository;
        store
            .model_repository()
            .upsert_model_profile(&nola_store_port::model::StoredModelProfile {
                id: "chosen".into(),
                display_name: "Chosen".into(),
                provider: "fixture".into(),
                model_name: "original-model".into(),
                runtime: "remote".into(),
                source_json: "{}".into(),
                parameters_json: "{}".into(),
                selected: false,
                enabled: true,
                revision: 1,
                created_at_ms: 1,
                updated_at_ms: 1,
                deleted_at_ms: None,
            })
            .unwrap();
        // The Store allocates revisions; caller-supplied numbers cannot invent one.
        assert_eq!(store.model_repository().model_profile("chosen").unwrap().unwrap().revision, 1);
        rusqlite::Connection::open(path).unwrap().execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','test',1,1,1)",[]).unwrap();
        store
    }
    #[test]
    fn goal_binding_is_atomic_immutable_and_survives_cold_open_before_any_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("goal.sqlite");
        let mut store = seed(&path);
        start_goal_execution(&mut store, input()).unwrap();
        let original = load_goal_execution_binding(&store, "c", "g")
            .unwrap()
            .unwrap();
        start_goal_execution(&mut store, input()).unwrap();
        let mut changed = input();
        changed.template.model_selection = GoalModelSelection::DirectRoute;
        assert!(start_goal_execution(&mut store, changed)
            .unwrap_err()
            .contains("immutable"));
        drop(store);
        let reopened = NolaStore::open(&path).unwrap();
        let restored = load_goal_execution_binding(&reopened, "c", "g")
            .unwrap()
            .unwrap();
        assert_eq!(restored.binding_sha256, original.binding_sha256);
        assert_eq!(
            restored.template.model_selection,
            GoalModelSelection::Profile {
                profile_id: "chosen".into(),
                revision: 1
            }
        );
        assert!(load_goal_execution_binding(&reopened, "foreign", "g")
            .unwrap()
            .is_none());
        let sql = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            sql.query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        sql.execute("UPDATE goal_execution_bindings SET binding_json='{}'", [])
            .unwrap();
        assert!(load_goal_execution_binding(&reopened, "c", "g")
            .unwrap_err()
            .contains("digest"));
    }
    #[test]
    fn failed_goal_reduction_rolls_back_binding_and_unbound_start_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollback.sqlite");
        let mut store = seed(&path);
        let mut bad = input();
        if let GoalRuntimeCommand::Start { objective, .. } = &mut bad.envelope.command {
            *objective = String::new();
        }
        assert!(start_goal_execution(&mut store, bad).is_err());
        assert!(store
            .load_goal_execution_binding("c", "g")
            .unwrap()
            .is_none());
        let i = input();
        assert!(nola_application::reduce_and_commit_idle_submission(
            &mut store,
            nola_application::ReduceAndCommitIdleSubmission {
                conversation_id: "c".into(),
                core_config: i.template.core_config,
                submission: AgentSubmission::GoalRuntime {
                    envelope: i.envelope
                },
                committed_at_ms: 100,
            }
        )
        .unwrap_err()
        .contains("atomic execution binding"));
        start_goal_execution(&mut store, input()).unwrap();
    }

    #[test]
    fn goal_reconstruction_reuses_exact_run_policy_validator() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("authority.sqlite");
        let mut store = seed(&path);
        start_goal_execution(&mut store, input()).unwrap();
        let frozen = load_goal_execution_binding(&store, "c", "g")
            .unwrap()
            .unwrap();
        let config = &frozen.template.core_config;
        let current = nola_application::RunRecoveryAuthority {
            host_catalog: config.host_catalog.clone(),
            tools: config.tools.clone(),
            permission_rules: config.permission_rules.clone(),
            context_window: config.context_window.clone(),
            host_invocation: config.turn_identity.host_invocation.clone().unwrap(),
        };
        validate_goal_execution_authority(
            &store,
            "c",
            "g",
            "host",
            &frozen.binding_sha256,
            &current,
            101,
        )
        .unwrap();
        assert!(validate_goal_execution_authority(
            &store,
            "c",
            "g",
            "foreign",
            &frozen.binding_sha256,
            &current,
            101
        )
        .is_err());
        let mut changed = current.clone();
        changed.tools.clear();
        assert!(validate_goal_execution_authority(
            &store,
            "c",
            "g",
            "host",
            &frozen.binding_sha256,
            &changed,
            101
        )
        .is_err());
        let mut changed = current.clone();
        changed.context_window.context_window_tokens += 1;
        assert!(validate_goal_execution_authority(
            &store,
            "c",
            "g",
            "host",
            &frozen.binding_sha256,
            &changed,
            101
        )
        .is_err());
    }
    #[test]
    fn goal_binding_requires_recoverable_template_and_canonical_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.sqlite");
        let mut store = seed(&path);
        let mut missing = input();
        missing.template.core_config.turn_identity.host_recovery = None;
        assert!(start_goal_execution(&mut store, missing)
            .unwrap_err()
            .contains("recovery reference"));
        assert!(store
            .load_goal_execution_binding("c", "g")
            .unwrap()
            .is_none());
        start_goal_execution(&mut store, input()).unwrap();
        let row = store
            .load_goal_execution_binding("c", "g")
            .unwrap()
            .unwrap();
        assert!(store
            .insert_goal_execution_binding(&row)
            .unwrap_err()
            .to_string()
            .contains("transaction"));
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute(
            "UPDATE goal_execution_bindings SET source_host_id='foreign'",
            [],
        )
        .unwrap();
        assert!(load_goal_execution_binding(&store, "c", "g")
            .unwrap_err()
            .contains("sealed payload"));
    }

    #[test]
    fn checkpoint_write_failure_rolls_back_goal_binding_in_the_same_sqlite_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commit-failure.sqlite");
        let mut store = seed(&path);
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("CREATE TRIGGER reject_goal_checkpoint BEFORE INSERT ON core_checkpoint_bodies BEGIN SELECT RAISE(ABORT,'fixture checkpoint failure'); END;").unwrap();
        assert!(start_goal_execution(&mut store, input()).is_err());
        assert!(store
            .load_goal_execution_binding("c", "g")
            .unwrap()
            .is_none());
        assert_eq!(
            sql.query_row("SELECT count(*) FROM core_checkpoints", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        sql.execute_batch("DROP TRIGGER reject_goal_checkpoint")
            .unwrap();
        start_goal_execution(&mut store, input()).unwrap();
    }

    #[test]
    fn disabled_or_revised_profile_blocks_execution_but_not_inspection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("revocation.sqlite");
        let mut store = seed(&path);
        start_goal_execution(&mut store, input()).unwrap();
        prepare_goal_execution(&mut store, "c", "g").unwrap();
        let sql = rusqlite::Connection::open(&path).unwrap();
        sql.execute("UPDATE model_profiles SET enabled=0 WHERE id='chosen'", [])
            .unwrap();
        assert!(prepare_goal_execution(&mut store, "c", "g")
            .unwrap_err()
            .contains("disabled"));
        assert!(load_goal_execution_binding(&store, "c", "g")
            .unwrap()
            .is_some());
        sql.execute(
            "UPDATE model_profiles SET enabled=1,revision=2 WHERE id='chosen'",
            [],
        )
        .unwrap();
        assert!(prepare_goal_execution(&mut store, "c", "g")
            .unwrap_err()
            .contains("changed"));
    }
}
